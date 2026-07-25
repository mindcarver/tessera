//! Source Registry integration tests (Story 1.3 / spec-1-3-source-confirm.md).
//!
//! Drives the full application → policy → registry → SQLite stack against the
//! spec's I/O matrix, idempotency / wake-up rules (AD-33/AD-35), and NFR-1
//! (zero source-file mutation). No `std::env::set_var` (parallel-test races);
//! candidates are built directly against tempdir roots and passed to the
//! application layer, exactly as the IPC commands would.
//!
//! Coverage:
//! - migration id 2 (`v1_source_registry`) applies and schema_version = 4
//!   (after the appended 1.4 migration id 3, `v2_scan_generations`).
//! - confirm new candidate → `src_<n>` + `confirmed` + fingerprint persisted.
//! - idempotent re-confirm → same `source_id`, no new row.
//! - reject → `rejected`; confirm a previously-rejected candidate wakes it to
//!   `confirmed` with the SAME `source_id`.
//! - disable → `disabled`; row preserved; **source files unchanged** (NFR-1).
//! - disable unknown `source_id` → `SourceNotFound`.
//! - `find_by_fingerprint` exact match (no fuzzy merge — AD-35).
//! - same path but inode changed (rebuilt dir) → different fingerprint →
//!   different Source (no auto-merge).
//! - canonicalize resolves symlinks (tempdir symlink).
//! - list returns every row ordered by id.

use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

use rusqlite::Connection;
use tempfile::tempdir;

use tessera_lib::adapters::claude_code::ClaudeCodeAdapter;
use tessera_lib::adapters::codex::CodexAdapter;
use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis, ProviderAdapter,
};
use tessera_lib::domain::source::{SourceFingerprint, SourceId, SourceLifecycle};
use tessera_lib::index::migrations;
use tessera_lib::index::SourceRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a fresh in-memory DB and apply all migrations (v0_meta +
/// v1_source_registry + v2_scan_generations). Returns a connection at
/// schema_version 4 with foreign-key enforcement ON (matching boot).
fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign_keys pragma must apply");
    migrations::apply(&mut conn).expect("migrations apply on fresh db");
    conn
}

/// Build a real Codex-shaped candidate for a tempdir root. The root must
/// already exist as a directory; the candidate carries the non-canonicalized
/// `root_path` exactly as the discover layer would.
fn candidate_for(root: &std::path::Path) -> CandidateSource {
    CandidateSource {
        provider: "codex".to_string(),
        root_path: root.to_string_lossy().to_string(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CodexAdapter.coverage_level(),
        native_project: None,
    }
}

/// Create a memories-shaped directory under a tempdir and return its path.
fn make_memories(parent: &std::path::Path) -> PathBuf {
    let memories = parent.join("memories");
    fs::create_dir_all(&memories).expect("create memories dir");
    memories
}

/// Snapshot the (mtime, size, content-hash-lite) of every file under `root`
/// so we can assert NFR-1 zero-write after disable. Returns a sorted vec of
/// (path, mtime_nanos, size, content) tuples.
fn snapshot_tree(root: &std::path::Path) -> Vec<(PathBuf, SystemTime, u64, Vec<u8>)> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(dir: &std::path::Path, out: &mut Vec<(PathBuf, SystemTime, u64, Vec<u8>)>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let meta = entry.metadata().expect("metadata");
        if meta.is_dir() {
            walk(&path, out);
        } else {
            let mtime = meta.modified().expect("modified");
            let content = fs::read(&path).expect("read file content");
            out.push((path, mtime, meta.len(), content));
        }
    }
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

/// Migration id 2 (`v1_source_registry`) applies on a fresh DB. Stories 1.4
/// and 1.5 append migrations 3 and 4, so `schema_version` advances to 4; the
/// v1 table and its unique index still exist and the v1 audit row is recorded.
#[test]
fn migration_v1_source_registry_applies_and_sets_current_schema_version() {
    let conn = fresh_db();
    let v: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    assert_eq!(v, "5", "schema_version must be 5 after Story 1.8 migration");

    // The table + unique index exist.
    let table: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'source_registry'",
            [],
            |row| row.get(0),
        )
        .expect("check table");
    assert_eq!(table, 1);
    let idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'source_registry_fingerprint'",
            [],
            |row| row.get(0),
        )
        .expect("check index");
    assert_eq!(idx, 1);

    // An audit row was recorded.
    let (id, name): (i64, String) = conn
        .query_row(
            "SELECT id, name FROM tessera_migrations_applied WHERE id = 2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("v1 audit row");
    assert_eq!(id, 2);
    assert_eq!(name, "v1_source_registry");
}

// ---------------------------------------------------------------------------
// Confirm + idempotency
// ---------------------------------------------------------------------------

/// I/O matrix row 1: confirm a real existing Codex memories dir →
/// `src_<n>` + `confirmed` + `coverage_level=full` + normalized path +
/// fingerprint persisted.
#[test]
fn confirm_new_candidate_persists_confirmed_source_with_fingerprint() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let source = application::confirm_source(&registry, &candidate).expect("confirm ok");
    assert!(source.source_id.0.starts_with("src_"), "src_ prefix");
    assert_eq!(source.provider, "codex");
    assert_eq!(source.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(source.coverage_level, CoverageLevel::Full);
    assert!(
        source.normalized_root_path.starts_with('/'),
        "normalized abs"
    );
    assert!(!source.fingerprint.0.is_empty(), "fingerprint stored");

    // persisted: re-reading via the fingerprint returns the same row.
    let reread = registry
        .find_by_fingerprint(&source.fingerprint)
        .expect("db ok")
        .expect("row exists");
    assert_eq!(reread.source_id, source.source_id);
    assert_eq!(reread.lifecycle_state, SourceLifecycle::Confirmed);
}

/// I/O matrix row 2: idempotent re-confirm with the same root (same
/// path+inode) returns the SAME `source_id`, no new row.
#[test]
fn reconfirm_same_root_is_idempotent_same_source_id() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let first = application::confirm_source(&registry, &candidate).expect("first confirm");
    let second = application::confirm_source(&registry, &candidate).expect("second confirm");

    assert_eq!(first.source_id, second.source_id, "idempotent: same id");
    // Exactly one row in the registry.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "no duplicate row on re-confirm");
}

// ---------------------------------------------------------------------------
// Reject + wake-up
// ---------------------------------------------------------------------------

/// I/O matrix row 3: reject a candidate → `rejected` row; confirm later wakes
/// it to `confirmed` with the SAME `source_id` (idempotent wake-up).
#[test]
fn reject_then_confirm_wakes_to_confirmed_same_source_id() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let rejected = application::reject_source(&registry, &candidate).expect("reject");
    assert_eq!(rejected.lifecycle_state, SourceLifecycle::Rejected);
    let rejected_id = rejected.source_id.clone();

    // Confirm the same candidate → wake-up, same id, now confirmed.
    let confirmed = application::confirm_source(&registry, &candidate).expect("wake-up confirm");
    assert_eq!(confirmed.source_id, rejected_id, "wake-up keeps source_id");
    assert_eq!(confirmed.lifecycle_state, SourceLifecycle::Confirmed);

    // Still one row.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

/// Confirm then disable then re-confirm → wake-up from disabled to confirmed,
/// same source_id.
#[test]
fn confirm_then_disable_then_reconfirm_wakes_from_disabled() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let confirmed = application::confirm_source(&registry, &candidate).expect("confirm");
    let disabled = application::disable_source(&registry, &confirmed.source_id).expect("disable");
    assert_eq!(disabled.lifecycle_state, SourceLifecycle::Disabled);
    assert_eq!(disabled.source_id, confirmed.source_id);

    let rewoken = application::confirm_source(&registry, &candidate).expect("re-confirm");
    assert_eq!(rewoken.source_id, confirmed.source_id);
    assert_eq!(rewoken.lifecycle_state, SourceLifecycle::Confirmed);
}

// ---------------------------------------------------------------------------
// Disable + NFR-1 zero write + unknown id
// ---------------------------------------------------------------------------

/// I/O matrix row 5 + NFR-1: disable a confirmed Source → `disabled`, row
/// preserved, AND the source files (mtime / content / size) are UNCHANGED.
#[test]
fn disable_preserves_row_and_does_not_mutate_source_files_nfr1() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    // Put a real artifact so the snapshot is non-empty.
    fs::write(memories.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let confirmed = application::confirm_source(&registry, &candidate).expect("confirm");

    let before = snapshot_tree(&memories);

    let disabled = application::disable_source(&registry, &confirmed.source_id).expect("disable");
    assert_eq!(disabled.lifecycle_state, SourceLifecycle::Disabled);

    // Row still present.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "disabled row is preserved");

    // NFR-1: source files byte-identical with identical mtime.
    let after = snapshot_tree(&memories);
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (NFR-1)");
        assert_eq!(b.2, a.2, "size unchanged (NFR-1)");
        assert_eq!(b.3, a.3, "content unchanged (NFR-1)");
    }
}

/// NFR-1 (extended to confirm/reject): confirm and reject must NOT mutate the
/// source filesystem (mtime / size / content unchanged). The disable test
/// snapshots AFTER confirm, so it only pins disable's write path. This test
/// snapshots BEFORE confirm so confirm's and reject's own paths — canonicalize
/// + metadata (reads) + registry SQL (Tessera DB) — are covered.
#[test]
fn confirm_and_reject_do_not_mutate_source_files_nfr1() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");
    fs::write(memories.join("raw_memories.md"), "raw").expect("write raw");

    let candidate = candidate_for(&memories);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let before = snapshot_tree(&memories);

    let _confirmed = application::confirm_source(&registry, &candidate).expect("confirm");
    // Reject the same root: idempotent-by-fingerprint, flips the confirmed row
    // to rejected in place — exercises reject_source's write path without
    // touching a new root.
    let _rejected = application::reject_source(&registry, &candidate).expect("reject");

    let after = snapshot_tree(&memories);
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (NFR-1)");
        assert_eq!(b.2, a.2, "size unchanged (NFR-1)");
        assert_eq!(b.3, a.3, "content unchanged (NFR-1)");
    }
}

/// I/O matrix row 6: disable an unknown `source_id` → `SourceNotFound`.
#[test]
fn disable_unknown_source_id_returns_source_not_found() {
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let bogus = SourceId("src_99999".to_string());
    let err = application::disable_source(&registry, &bogus).expect_err("unknown id");
    assert!(matches!(err, application::SourceError::SourceNotFound));
}

// ---------------------------------------------------------------------------
// Fingerprint exactness: path/inode separation (AD-35)
// ---------------------------------------------------------------------------

/// I/O matrix row 8: same path but inode changed (directory rebuilt) →
/// different fingerprint → different Source (no auto-merge; degraded handling
/// is Story 4.3).
#[cfg(unix)]
#[test]
fn same_path_different_inode_yields_different_source_no_merge() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let first = application::confirm_source(&registry, &candidate_for(&memories)).expect("first");

    // Rebuild the directory while keeping the old inode allocated, so filesystems
    // that aggressively reuse recently freed inodes cannot collapse the test.
    let old_memories = tmp.path().join("memories-old");
    fs::rename(&memories, &old_memories).expect("rename old memories aside");
    fs::create_dir_all(&memories).expect("recreate");

    let second = application::confirm_source(&registry, &candidate_for(&memories)).expect("second");

    assert_ne!(
        first.fingerprint, second.fingerprint,
        "rebuilt dir has a new inode → different fingerprint"
    );
    assert_ne!(
        first.source_id, second.source_id,
        "different fingerprint → different Source (no auto-merge)"
    );

    // Both rows preserved.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 2);
}

/// find_by_fingerprint is an exact equality lookup (AD-35 "no fuzzy merge").
/// A fingerprint that is a prefix / substring of a stored one does NOT match.
#[test]
fn find_by_fingerprint_is_exact_equality() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let source =
        application::confirm_source(&registry, &candidate_for(&memories)).expect("confirm");
    let stored = &source.fingerprint;

    // Exact match.
    let exact = registry
        .find_by_fingerprint(stored)
        .expect("db ok")
        .expect("exact match");
    assert_eq!(exact.source_id, source.source_id);

    // Truncated fingerprint does NOT match.
    let truncated = SourceFingerprint(stored.0[..stored.0.len() - 1].to_string());
    let none = registry.find_by_fingerprint(&truncated).expect("db ok");
    assert!(none.is_none(), "truncated fingerprint must not match");

    // Empty fingerprint does not match.
    let empty = registry
        .find_by_fingerprint(&SourceFingerprint(String::new()))
        .expect("db ok");
    assert!(empty.is_none());
}

// ---------------------------------------------------------------------------
// Canonicalize resolves symlinks (AD-4)
// ---------------------------------------------------------------------------

/// Confirm canonicalizes via `std::fs::canonicalize`, so a symlink root
/// resolves to the real directory. Two confirms — one via the symlink, one
/// via the real path — produce the SAME fingerprint and therefore the SAME
/// source_id (symlinks collapse, no duplicate).
#[cfg(unix)]
#[test]
fn confirm_via_symlink_collapses_to_same_source_as_real_path() {
    let tmp = tempdir().expect("tempdir");
    let real = tmp.path().join("real_memories");
    let link = tmp.path().join("link_memories");
    fs::create_dir_all(&real).expect("mkdir real");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let via_link = application::confirm_source(&registry, &candidate_for(&link)).expect("via link");
    let via_real = application::confirm_source(&registry, &candidate_for(&real)).expect("via real");

    // canonicalize collapses the symlink → same normalized path → same fp.
    assert_eq!(via_link.fingerprint, via_real.fingerprint);
    assert_eq!(via_link.source_id, via_real.source_id, "symlink collapsed");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1);
}

// ---------------------------------------------------------------------------
// Confirm failure: root invalid (NFR-5/6)
// ---------------------------------------------------------------------------

/// I/O matrix row 4: confirm a candidate whose root vanished between discover
/// and confirm → `ConfirmFailed`. No registry write.
#[test]
fn confirm_failed_when_root_does_not_exist_no_registry_write() {
    let bogus = CandidateSource {
        provider: "codex".to_string(),
        root_path: "/this/does/not/exist/tessera-1-3".to_string(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    };
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let err = application::confirm_source(&registry, &bogus).expect_err("missing root");
    assert!(matches!(err, application::SourceError::ConfirmFailed));

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 0, "no row written on confirm_failed");
}

/// I/O matrix row 4 variant: confirm a candidate whose root is a regular file
/// (not a directory) → `ConfirmFailed`.
#[test]
fn confirm_failed_when_root_is_a_regular_file() {
    let tmp = tempdir().expect("tempdir");
    let file = tmp.path().join("not_a_dir");
    fs::write(&file, "x").expect("write file");
    let candidate = candidate_for(&file);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let err = application::confirm_source(&registry, &candidate).expect_err("file not dir");
    assert!(matches!(err, application::SourceError::ConfirmFailed));
}

/// Confirming an unknown provider id → `ConfirmFailed` (no adapter). No row.
#[test]
fn confirm_failed_for_unknown_provider() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let mut candidate = candidate_for(&memories);
    candidate.provider = "unknown_provider".to_string();

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let err = application::confirm_source(&registry, &candidate).expect_err("unknown provider");
    assert!(matches!(err, application::SourceError::ConfirmFailed));
}

// ---------------------------------------------------------------------------
// list_sources
// ---------------------------------------------------------------------------

/// `list_sources` returns every registered row ordered by id, regardless of
/// lifecycle.
#[test]
fn list_sources_returns_all_rows_ordered_by_id() {
    let tmp_a = tempdir().expect("tempdir a");
    let tmp_b = tempdir().expect("tempdir b");
    let mem_a = make_memories(tmp_a.path());
    let mem_b = make_memories(tmp_b.path());

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let a = application::confirm_source(&registry, &candidate_for(&mem_a)).expect("confirm a");
    let _b = application::reject_source(&registry, &candidate_for(&mem_b)).expect("reject b");

    let all = application::list_sources(&registry).expect("list");
    assert_eq!(all.len(), 2);
    // Ordered by id ascending.
    assert_eq!(all[0].source_id, a.source_id);
    assert_eq!(all[0].lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(all[1].lifecycle_state, SourceLifecycle::Rejected);
}

// ---------------------------------------------------------------------------
// discover_sources still works (1.2 regression guard)
// ---------------------------------------------------------------------------

/// 1.2's `discover_sources` orchestrator moved to `application::source`; this
/// pins that the re-export still works and is infallible.
#[test]
fn discover_sources_still_orchestrates_after_move() {
    let candidates = application::discover_sources();
    // Infallible: returns a Vec, never panics. Every candidate declares a
    // known provider (Story 2.1 adds Claude Code alongside Codex) with Full
    // coverage.
    for c in &candidates {
        assert!(matches!(c.provider.as_str(), "codex" | "claude_code"));
        assert_eq!(c.coverage_level, CoverageLevel::Full);
    }
}

// ---------------------------------------------------------------------------
// Story 2.1: Claude Code confirm + coexistence + zero-mutation
// ---------------------------------------------------------------------------

/// Build a real Claude Code-shaped candidate for a tempdir memory root. The
/// root must already exist as a directory; the candidate carries the
/// non-canonicalized `root_path` exactly as the discover layer would, plus
/// the encoded project key as `native_project` (no reverse-mapping).
fn claude_candidate_for(root: &std::path::Path, project_key: &str) -> CandidateSource {
    CandidateSource {
        provider: "claude_code".to_string(),
        root_path: root.to_string_lossy().to_string(),
        basis: DiscoveryBasis::ClaudeDefaultHome,
        coverage_level: ClaudeCodeAdapter.coverage_level(),
        native_project: Some(project_key.to_string()),
    }
}

/// Create a Claude-shaped project memory dir under a tempdir and return its
/// path: `<parent>/<project>/memory/`.
fn make_claude_memory(parent: &std::path::Path, project: &str) -> PathBuf {
    let memory = parent.join(project).join("memory");
    fs::create_dir_all(&memory).expect("create claude memory dir");
    memory
}

/// Story 2.1 I/O matrix — confirm a Claude Code candidate →
/// `src_<n>` + `confirmed` + `provider=claude_code` + the project key as
/// Native Project + Full coverage. The reused Codex pipeline is provider-
/// neutral.
#[test]
fn confirm_claude_code_candidate_persists_confirmed_source_with_native_project() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "encoded-project-key");
    let candidate = claude_candidate_for(&memory, "encoded-project-key");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let source = application::confirm_source(&registry, &candidate).expect("confirm claude");
    assert!(source.source_id.0.starts_with("src_"), "src_ prefix");
    assert_eq!(source.provider, "claude_code");
    assert_eq!(source.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(source.coverage_level, CoverageLevel::Full);
    assert_eq!(source.native_project.as_deref(), Some("encoded-project-key"));
    assert!(source.normalized_root_path.starts_with('/'), "normalized abs");
    assert!(!source.fingerprint.0.is_empty(), "fingerprint stored");
}

/// Story 2.1 — re-confirming the same Claude Code candidate is idempotent:
/// same `source_id`, no new row.
#[test]
fn reconfirm_claude_code_candidate_is_idempotent_same_source_id() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    let candidate = claude_candidate_for(&memory, "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let first = application::confirm_source(&registry, &candidate).expect("first confirm");
    let second = application::confirm_source(&registry, &candidate).expect("second confirm");
    assert_eq!(first.source_id, second.source_id);

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "no duplicate row on re-confirm");
}

/// Story 2.1 AC — Claude and Codex Sources coexist as separate
/// `source_registry` rows (different fingerprints). Confirming one does not
/// disturb the other; re-confirm of the Claude one returns the SAME
/// `source_id`. The separate-row requirement is the spec's "separate
/// `source_registry` rows; re-confirm is an idempotent wake-up returning the
/// same `source_id`".
#[test]
fn claude_and_codex_sources_coexist_as_separate_rows() {
    let tmp_codex = tempdir().expect("codex tempdir");
    let tmp_claude = tempdir().expect("claude tempdir");
    let codex_memories = make_memories(tmp_codex.path());
    let claude_memory = make_claude_memory(tmp_claude.path(), "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let codex = application::confirm_source(&registry, &candidate_for(&codex_memories))
        .expect("confirm codex");
    let claude = application::confirm_source(
        &registry,
        &claude_candidate_for(&claude_memory, "proj"),
    )
    .expect("confirm claude");

    assert_ne!(codex.source_id, claude.source_id);
    assert_ne!(codex.fingerprint, claude.fingerprint);
    assert_eq!(codex.provider, "codex");
    assert_eq!(claude.provider, "claude_code");

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 2, "both sources persisted as separate rows");
}

/// Story 2.1 NFR-1 — confirming a Claude Code candidate must NOT mutate the
/// source filesystem (mtime / size / content unchanged). Snapshots the tree
/// BEFORE confirm so confirm's own path — canonicalize (read) + registry SQL
/// (Tessera DB) — is covered, then asserts zero drift.
#[test]
fn confirm_claude_code_candidate_does_not_mutate_source_files_nfr1() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    // Seed an artifact the matrix will accept in 2.2; we are not parsing it
    // here, just pinning that confirm leaves it byte-identical.
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let candidate = claude_candidate_for(&memory, "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let before = snapshot_tree(&memory);

    let _ = application::confirm_source(&registry, &candidate).expect("confirm claude");
    // Reject the same root: idempotent-by-fingerprint, flips the confirmed
    // row to rejected in place — exercises reject_source's write path on a
    // Claude Code row.
    let _ = application::reject_source(&registry, &candidate).expect("reject claude");

    let after = snapshot_tree(&memory);
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (NFR-1)");
        assert_eq!(b.2, a.2, "size unchanged (NFR-1)");
        assert_eq!(b.3, a.3, "content unchanged (NFR-1)");
    }
}

/// Story 2.1 AC — confirming a Claude Code source, then triggering a scan,
/// yields a structured `ScanError::ProviderNotScannable` (Claude parsing is
/// 2.2). The Codex parser is never applied to Claude files; the Source's
/// health stays `unknown` (a Claude Source is legitimately unscannable in
/// 2.1, not degraded).
#[test]
fn scan_claude_code_source_returns_provider_not_scannable_without_health_change() {
    use tessera_lib::domain::scan::ScanError;
    use tessera_lib::domain::source::HealthState;
    use tessera_lib::index::scan_store::ScanStore;

    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_source(&registry, &claude_candidate_for(&memory, "proj"))
            .expect("confirm claude");

    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("claude scan must not proceed in 2.1");
    assert!(
        matches!(err, ScanError::ProviderNotScannable),
        "expected ProviderNotScannable, got {err:?}"
    );

    // Health must remain `unknown` — the guard fires before any
    // health-write path.
    let row = registry.get(&source.source_id).expect("db ok").expect("row");
    assert_eq!(row.health_state, HealthState::Unknown);

    // No scan_runs row was created either — the guard fires before begin_run.
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");
    let run_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scan_runs WHERE source_id = ?1",
            rusqlite::params![source_rowid],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(run_count, 0, "no run row was created for the guarded scan");
}

/// Story 2.1 review fix — the RESCAN path (`scan_reserved_source`) marks the
/// reserved run row with the dedicated `error_code='provider_not_scannable'`
/// vocabulary value, NOT the `internal` catch-all. This is the persisted
/// surface of the spec AC "every surface … shows the provider-aware safe
/// message — never a generic `internal` code": the SSE terminal event and
/// inventory `latest_error` are both derived from this persisted code, so a
/// wrong value here propagates everywhere.
#[test]
fn rescan_claude_code_source_persists_provider_not_scannable_error_code() {
    use tessera_lib::domain::scan::{Generation, ScanError};
    use tessera_lib::domain::source::HealthState;
    use tessera_lib::index::scan_store::ScanStore;

    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_source(&registry, &claude_candidate_for(&memory, "proj"))
            .expect("confirm claude");

    // Mirror what the rescan HTTP handler does: begin_run BEFORE dispatch so
    // the run row exists, then drive the reserved scan path that the worker
    // thread uses. The guard inside `scan_reserved_source` marks the row
    // failed with the error_code under test.
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");
    let store = ScanStore::new(&conn);
    let (scan_id, fencing_token, _placeholder_generation) = store
        .begin_run(source_rowid, "pending")
        .expect("begin_run for reserved claude scan");
    // `Generation::from_rowid` is `pub(crate)`, so build the same opaque
    // handle via the public tuple constructor. The production path always
    // uses `from_rowid`; this mirrors its `gen_<rowid>` shape exactly.
    let generation = Generation(format!("gen_{scan_id}"));

    let err = application::scan_reserved_source(
        &registry,
        &conn,
        &source.source_id,
        scan_id,
        fencing_token,
        generation,
    )
    .expect_err("reserved claude scan must fail");
    assert!(matches!(err, ScanError::ProviderNotScannable));

    // The persisted `error_code` is the dedicated vocabulary value — NOT
    // `internal`. This is the load-bearing assertion for "every surface
    // shows the provider-aware message": inventory `latest_error` is derived
    // from this value via `safe_error_reason`, and a wrong code here would
    // make the inventory read "Tessera could not complete the last rescan."
    let persisted_code: String = conn
        .query_row(
            "SELECT error_code FROM scan_runs WHERE id = ?1",
            rusqlite::params![scan_id],
            |row| row.get(0),
        )
        .expect("scan_runs row readable");
    assert_eq!(
        persisted_code, "provider_not_scannable",
        "rescan must persist provider_not_scannable, NOT internal"
    );

    // Health must remain `unknown` — a Claude Source being unscannable in
    // 2.1 is expected, not degraded.
    let row = registry.get(&source.source_id).expect("db ok").expect("row");
    assert_eq!(row.health_state, HealthState::Unknown);
}

/// Story 2.1 pass-2 review fix — disable-by-`source_id` must work for a
/// `claude_code` Source, not just Codex. Mirrors the existing Codex-shaped
/// `confirm_then_disable_then_reconfirm_wakes_from_disabled` and
/// `disable_preserves_row_and_does_not_mutate_source_files_nfr1` shape: the
/// row flips to `disabled`, the `source_id` is unchanged, and the row is
/// preserved (no delete). A subsequent re-confirm wakes it back to
/// `confirmed` with the SAME `source_id`, exercising the Claude Code path
/// through the same idempotent-by-fingerprint lifecycle machine.
#[test]
fn disable_claude_code_source_flips_to_disabled_and_is_preserved() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");
    let candidate = claude_candidate_for(&memory, "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);

    let confirmed = application::confirm_source(&registry, &candidate).expect("confirm claude");

    // Snapshot the source files BEFORE disable so we can assert NFR-1
    // zero-mutation for the Claude Code path too (parity with the Codex
    // disable test).
    let before = snapshot_tree(&memory);

    let disabled = application::disable_source(&registry, &confirmed.source_id).expect("disable");
    assert_eq!(disabled.provider, "claude_code");
    assert_eq!(disabled.lifecycle_state, SourceLifecycle::Disabled);
    assert_eq!(disabled.source_id, confirmed.source_id, "same source_id");

    // Row still present.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "disabled Claude Code row is preserved");

    // Re-read via the registry — the persisted lifecycle is `disabled`.
    let reread = registry
        .get(&confirmed.source_id)
        .expect("db ok")
        .expect("row exists");
    assert_eq!(reread.lifecycle_state, SourceLifecycle::Disabled);
    assert_eq!(reread.provider, "claude_code");
    assert_eq!(
        reread.native_project.as_deref(),
        Some("proj"),
        "Claude project key preserved across disable"
    );

    // NFR-1: source files byte-identical with identical mtime.
    let after = snapshot_tree(&memory);
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (NFR-1)");
        assert_eq!(b.2, a.2, "size unchanged (NFR-1)");
        assert_eq!(b.3, a.3, "content unchanged (NFR-1)");
    }

    // Wake-up parity: re-confirm flips the SAME row back to `confirmed`
    // (idempotent-by-fingerprint lifecycle machine is provider-neutral).
    let rewoken = application::confirm_source(&registry, &candidate).expect("re-confirm");
    assert_eq!(rewoken.source_id, confirmed.source_id);
    assert_eq!(rewoken.lifecycle_state, SourceLifecycle::Confirmed);
    assert_eq!(rewoken.provider, "claude_code");

    // Still one row.
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "no duplicate row on re-confirm");
}
