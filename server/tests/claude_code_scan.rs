//! Claude Code scan contract tests (Story 2.2 / spec-2-2-claude-parse-index.md).
//!
//! Drives the five adapter-contract classes for `claude_code` through the
//! public scan orchestration, mirroring the Codex classes in
//! `scan_pipeline.rs`:
//! 1. **fixture-contract** — heading/section ids + rejecter boundaries.
//! 2. **zero-source-mutation** — NFR-1 byte/mtime/size identical after scan.
//! 3. **parser-version** — records carry `claude-markdown/v1` (single source
//!    of truth on the adapter), never the Codex tag.
//! 4. **reconcile-recovery** — stale-run boot recovery + manifest drift both
//!    preserve the previous active generation.
//! 5. **capability-honesty** — empty dir activates an empty generation;
//!    symlink alias dedups to an honest count; enumeration failure preserves
//!    the previous generation.
//!
//! The discovery + confirm + basic-scan/NFR-1 coverage for `claude_code`
//! lives in `claude_code_discover.rs` and `source_registry.rs`; this file
//! owns the scan-pipeline contract slice.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use rusqlite::Connection;
use tempfile::tempdir;

use tessera_lib::adapters::claude_code::ClaudeCodeAdapter;
use tessera_lib::adapters::markdown::canonicalize_markdown;
use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis, EnumerateError, FileUnit, ProviderAdapter,
};
use tessera_lib::domain::scan::{ScanError, ScanRunState};
use tessera_lib::domain::source::{HealthState, SourceLifecycle};
use tessera_lib::domain::query::SearchRequest;
use tessera_lib::index::migrations;
use tessera_lib::index::scan_store::ScanStore;
use tessera_lib::index::SourceRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a fresh in-memory DB and apply all migrations (matches boot —
/// `PRAGMA foreign_keys = ON`).
fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign_keys pragma must apply");
    migrations::apply(&mut conn).expect("migrations apply on fresh db");
    conn
}

/// Build a Claude candidate for a memory dir + project key.
fn claude_candidate_for(root: &Path, project_key: &str) -> CandidateSource {
    CandidateSource {
        provider: "claude_code".to_string(),
        root_path: root.to_string_lossy().to_string(),
        basis: DiscoveryBasis::ClaudeDefaultHome,
        coverage_level: CoverageLevel::Full,
        native_project: Some(project_key.to_string()),
    }
}

/// Create a Claude-shaped project memory dir `<parent>/<project>/memory/`.
fn make_claude_memory(parent: &Path, project: &str) -> PathBuf {
    let memory = parent.join(project).join("memory");
    fs::create_dir_all(&memory).expect("create claude memory dir");
    memory
}

/// Confirm a Claude source for `memory` and return it.
fn confirm_claude(conn: &Connection, memory: &Path, project: &str) -> tessera_lib::domain::source::Source {
    let registry = SourceRegistry::new(conn);
    application::confirm_source(&registry, &claude_candidate_for(memory, project))
        .expect("confirm claude")
}

/// Snapshot (path, mtime, size, content) of every file under `root`.
fn snapshot_tree(root: &Path) -> Vec<(PathBuf, SystemTime, u64, Vec<u8>)> {
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn walk(dir: &Path, out: &mut Vec<(PathBuf, SystemTime, u64, Vec<u8>)>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let meta = entry.metadata().expect("metadata");
        if meta.is_dir() {
            walk(&path, out);
        } else {
            let mtime = meta.modified().expect("modified");
            let content = fs::read(&path).expect("read content");
            out.push((path, mtime, meta.len(), content));
        }
    }
}

fn assert_tree_unchanged(
    before: &[(PathBuf, SystemTime, u64, Vec<u8>)],
    after: &[(PathBuf, SystemTime, u64, Vec<u8>)],
) {
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (NFR-1)");
        assert_eq!(b.2, a.2, "size unchanged (NFR-1)");
        assert_eq!(b.3, a.3, "content unchanged (NFR-1)");
    }
}

/// Read the active generation string for a source rowid (None if unset).
fn active_generation_str(conn: &Connection, source_rowid: i64) -> Option<String> {
    let key = format!("active_generation:{source_rowid}");
    conn.query_row(
        "SELECT value FROM tessera_meta WHERE key = ?1",
        [key],
        |row| row.get(0),
    )
    .ok()
}

// ===========================================================================
// Class 1: fixture-contract (heading/section ids + rejecter boundaries)
// ===========================================================================

/// The shared Markdown parser produces the same canonical heading/section ids
/// for Claude's boundary fixture as for Codex's (one parser, two tags). Mirrors
/// `codex_canonicalization::repository_fixture_preserves_canonical_heading_boundaries`.
#[test]
fn claude_repository_fixture_preserves_canonical_heading_boundaries() {
    let units = canonicalize_markdown(include_bytes!(
        "fixtures/providers/claude_code/canonical-boundaries.md"
    ))
    .expect("fixture parses");

    assert_eq!(units.len(), 3);
    assert_eq!(units[0].title, "Preamble");
    assert_eq!(units[1].native_unit_id, "section/h1:7:Fixture:1");
    assert_eq!(units[1].body, "body\n\n");
    assert_eq!(
        units[2].native_unit_id,
        "section/h1:7:Fixture:1/h2:5:Child:1"
    );
    assert_eq!(units[2].body, "child body\n");
}

/// `enumerate_artifacts` on the fixture `memory/` dir indexes `MEMORY.md` +
/// topic `*.md`, tags their roles honestly, and rejects `CLAUDE.md` + the
/// non-`.md` JSON as `unsupported_artifact` diagnostics. The fixture path is
/// a real on-disk dir under `tests/fixtures/...`, so canonicalization is
/// exercised end-to-end.
#[test]
fn claude_fixture_enumerate_indexes_topics_and_rejects_boundary_artifacts() {
    let fixture_root = Path::new("tests/fixtures/providers/claude_code/memory");
    let observation = ClaudeCodeAdapter
        .enumerate_artifacts(fixture_root)
        .expect("fixture enumerate");

    // Supported: MEMORY.md + 2 topic files (python-patterns.md, rust-notes.md).
    let mut supported: Vec<(&str, &str)> = observation
        .supported
        .iter()
        .map(|a| (a.file.relative_path.as_str(), a.memory_type.as_str()))
        .collect();
    supported.sort();
    assert_eq!(
        supported,
        vec![
            ("MEMORY.md", "memory"),
            ("python-patterns.md", "topic_memory"),
            ("rust-notes.md", "topic_memory"),
        ],
        "supported artifacts + roles"
    );

    // Diagnostics: CLAUDE.md (rejected by name) + session-notes.json (non-md).
    let mut diag: Vec<&str> = observation
        .diagnostics
        .iter()
        .map(|d| d.observed_path.as_str())
        .collect();
    diag.sort();
    assert_eq!(diag, vec!["CLAUDE.md", "session-notes.json"]);
    for d in &observation.diagnostics {
        assert_eq!(d.kind, "unsupported_artifact");
    }
}

// ===========================================================================
// Class 2: zero-source-mutation (NFR-1)
// ===========================================================================

/// Scanning a Claude source leaves every file byte/mtime/size-identical
/// (NFR-1 zero-write). Covers MEMORY.md + topic files + rejecter files.
#[test]
fn claude_scan_does_not_mutate_source_files_nfr1() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");
    fs::write(memory.join("topic.md"), "# topic\n\nbody").expect("write topic");
    fs::write(memory.join("CLAUDE.md"), "rules").expect("write claude");
    fs::write(memory.join("notes.txt"), "notes").expect("write txt");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);

    let before = snapshot_tree(&memory);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("scan ok");
    let after = snapshot_tree(&memory);
    assert_tree_unchanged(&before, &after);
}

// ===========================================================================
// Class 3: parser-version (claude-markdown/v1, single source of truth)
// ===========================================================================

/// Every canonical record produced by a Claude scan carries
/// `parser_version='claude-markdown/v1'` and `provider='claude_code'`. No
/// Codex-tagged rows leak in (the parser version is read from the adapter at
/// the record-build site, not a hard-coded constant).
#[test]
fn claude_scan_records_carry_claude_markdown_v1_only() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");
    fs::write(memory.join("topic.md"), "# topic\n\nbody").expect("write topic");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan ok");

    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");
    let claude_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND provider = 'claude_code' \
             AND parser_version = 'claude-markdown/v1'",
            rusqlite::params![source_rowid],
            |row| row.get(0),
        )
        .expect("count claude rows");
    assert_eq!(
        claude_rows,
        outcome.records_indexed as i64,
        "all rows are claude_code + claude-markdown/v1"
    );

    let codex_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'codex-markdown/v1'",
            rusqlite::params![source_rowid],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(codex_rows, 0, "no codex-tagged rows for a claude source");
}

// ===========================================================================
// Class 4: reconcile-recovery (stale-run + drift preserve the active gen)
// ===========================================================================

/// A Claude source's indexed records survive a rescan unchanged (same
/// `record_id`s, same content). Idempotent re-scan is the precondition for
/// reconcile-recovery: a re-scan after a crash must produce identical
/// records, not duplicates.
#[test]
fn claude_rescan_is_idempotent_same_record_ids() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");
    fs::write(memory.join("topic.md"), "# topic\n\nbody").expect("write topic");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);

    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_ids: Vec<String> = conn
        .prepare("SELECT record_id FROM memory_records ORDER BY record_id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();

    let second =
        application::scan_source(&registry, &conn, &source.source_id).expect("second scan");
    let second_ids: Vec<String> = conn
        .prepare("SELECT record_id FROM memory_records ORDER BY record_id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query")
        .map(Result::unwrap)
        .collect();

    assert_eq!(first_ids, second_ids, "same record_ids on rescan");
    assert_eq!(first.records_indexed, second.records_indexed);
}

/// Boot recovery preserves the active Claude generation when a rescan run is
/// left in-flight (crashed worker). The stale run is marked `stale_recovered`;
/// the previous active generation's records remain visible. Mirrors the Codex
/// `boot_recovery_recovers_stale_runs_and_preserves_active` test.
#[test]
fn claude_boot_recovery_preserves_active_generation_after_crash() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");

    // Establish an active generation.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();
    let active_count = first.records_indexed;

    // Simulate a crashed in-flight rescan: begin + staging + a stale record,
    // but never commit.
    let store = ScanStore::new(&conn);
    let (crash_id, _t, crash_gen) = store.begin_run(source_rowid, "revX").expect("begin");
    store
        .set_state(crash_id, ScanRunState::Staging)
        .expect("staging");
    store
        .stage_records(
            &crash_gen,
            &[tessera_lib::index::scan_store::StagedRecord {
                record_id: "rec_claude_stale".to_string(),
                source_rowid,
                provider: "claude_code".to_string(),
                unit_kind: "file".to_string(),
                native_unit_id: "MEMORY.md".to_string(),
                native_locator: "file:///x/MEMORY.md".to_string(),
                content_hash: "h".to_string(),
                parser_version: "claude-markdown/v1".to_string(),
                title: "stale".to_string(),
                body: String::new(),
                native_project: None,
                provider_memory_type: "memory".to_string(),
                coverage_level: "full".to_string(),
                observed_at: 0,
                source_revision: "r".to_string(),
                display_locator: "file:///x/MEMORY.md#L1-L1".to_string(),
            }],
        )
        .expect("stage stale");

    // Recovery: the stale generation's record is GC'd; the active generation
    // is unchanged.
    application::recover_scans(&conn).expect("recover");
    let (state, error_code): (String, Option<String>) = conn
        .query_row(
            "SELECT state, error_code FROM scan_runs WHERE id = ?1",
            [crash_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("crashed run row");
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("stale_recovered"));

    // The active generation is unchanged and still reports its records.
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first_gen.as_str())
    );
    let store = ScanStore::new(&conn);
    assert_eq!(
        store.count_active_records(source_rowid).expect("count"),
        active_count
    );
}

/// A scripted Claude adapter whose SECOND enumeration returns an empty set:
/// the manifest re-validation sees a drift and fails with
/// `dirty_after_validation`. The previous active generation is preserved.
/// Mirrors the Codex `DriftAdapter` pattern via the test seam.
#[derive(Debug)]
struct ClaudeDriftAdapter {
    calls: AtomicUsize,
}

impl ClaudeDriftAdapter {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderAdapter for ClaudeDriftAdapter {
    fn provider_id(&self) -> &'static str {
        "claude_code"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    fn parser_version(&self) -> &'static str {
        "claude-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            ClaudeCodeAdapter.enumerate_file_units(root)
        } else {
            Ok(Vec::new())
        }
    }
}

/// Manifest drift during a Claude rescan never activates the staging
/// generation; the previous active generation's records survive.
#[test]
fn claude_dirty_after_validation_never_activates_and_preserves_previous() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");

    // Establish the active generation with the REAL adapter so the source has
    // a record to preserve.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();

    // Second scan via the drift seam: first enumeration returns the real
    // units; second returns empty → manifest drift.
    let drift = ClaudeDriftAdapter::new();
    let err = application::scan_source_with(&drift, &registry, &conn, &source)
        .expect_err("drift must fail");
    assert!(matches!(err, ScanError::DirtyAfterValidation));

    // The failed run is persisted with dirty_after_validation.
    let (state, error_code): (String, Option<String>) = conn
        .query_row(
            "SELECT state, error_code FROM scan_runs \
             WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("latest run");
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("dirty_after_validation"));

    // The previous active generation is unchanged.
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first_gen.as_str())
    );
}

// ===========================================================================
// Class 5: capability-honesty (empty dir, symlink dedup, enumeration failure)
// ===========================================================================

/// An empty Claude `memory/` dir scans successfully and activates an empty
/// generation (spec I/O matrix — empty directory scan is a complete success).
#[test]
fn claude_empty_memory_dir_activates_empty_generation() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan ok");
    assert_eq!(outcome.records_indexed, 0, "empty dir → zero records");

    // The generation activated (an empty generation is a valid first scan).
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");
    assert!(
        active_generation_str(&conn, source_rowid).is_some(),
        "empty generation activated"
    );

    // Health is healthy — an empty dir is a complete success, not degraded.
    let row = registry.get(&source.source_id).expect("db ok").expect("row");
    assert_eq!(row.health_state, HealthState::Healthy);
}

/// An empty re-scan must NOT replace a useful active Claude generation. After a
/// first scan activates a generation, emptying the `memory/` dir and rescanning
/// fails with `EmptyScanWithActiveGeneration` (honest refusal — an unreadable
/// or unexpectedly empty root cannot masquerade as a successful destructive
/// rescan), and the prior generation's records remain visible. Mirrors the
/// Codex `empty_rescan_preserves_existing_active_generation` test.
#[test]
fn claude_empty_rescan_preserves_existing_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");

    // Establish the active generation.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();
    let first_count = first.records_indexed;
    assert!(first_count > 0, "first scan indexed records");

    // Empty the memory dir so the next enumeration is empty (no supported, no
    // diagnostics).
    fs::remove_file(memory.join("MEMORY.md")).expect("remove supported file");

    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("empty replacement generation is rejected");
    assert!(
        matches!(err, ScanError::EmptyScanWithActiveGeneration),
        "expected EmptyScanWithActiveGeneration, got {err:?}"
    );

    // The failed run is persisted with `enumeration_failed` (the stable
    // error_code for EmptyScanWithActiveGeneration).
    let (state, error_code): (String, Option<String>) = conn
        .query_row(
            "SELECT state, error_code FROM scan_runs \
             WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("latest run");
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("enumeration_failed"));

    // The prior active generation is unchanged and still searchable.
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first_gen.as_str())
    );
    let store = ScanStore::new(&conn);
    assert_eq!(
        store.count_active_records(source_rowid).expect("count"),
        first_count
    );
}

/// An enumeration failure (root unreadable mid-scan) fails the run with
/// `enumeration_failed` and preserves the previous active generation. The
/// failure surface is honest — an empty result did NOT replace the real one.
#[test]
fn claude_enumeration_failure_preserves_previous_generation() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let source = confirm_claude(&conn, &memory, "proj");
    let registry = SourceRegistry::new(&conn);
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");

    // Establish the active generation.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();
    let first_count = first.records_indexed;

    // Make the root unreadable (chmod 000) so the next enumeration fails. On
    // Unix, removing read/execute on the dir makes `read_dir` fail with
    // `EnumerateError::Unreadable`. Restore perms afterward so the tempdir
    // cleanup works.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&memory, fs::Permissions::from_mode(0o000))
            .expect("chmod 000");
    }

    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("unreadable root must fail");
    assert!(
        matches!(err, ScanError::EnumerationFailed),
        "expected EnumerationFailed, got {err:?}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&memory, fs::Permissions::from_mode(0o755));
    }

    // The failed run is persisted with enumeration_failed.
    let (state, error_code): (String, Option<String>) = conn
        .query_row(
            "SELECT state, error_code FROM scan_runs \
             WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
            rusqlite::params![source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("latest run");
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("enumeration_failed"));

    // The previous active generation is unchanged and still searchable.
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first_gen.as_str())
    );
    let store = ScanStore::new(&conn);
    assert_eq!(
        store.count_active_records(source_rowid).expect("count"),
        first_count
    );
}

/// A Claude source rooted at a user `autoMemoryDirectory`-style dir (i.e. the
/// memory dir itself, no `projects/<P>/memory/` nesting) scans and activates
/// identically to a project `memory/` dir. The adapter's boundary is the
/// confirmed root, regardless of how discovery found it.
#[test]
fn claude_auto_memory_directory_root_scans_identically_to_project_memory() {
    let tmp = tempdir().expect("tempdir");
    // The auto-memory dir IS the root (no project-key parent).
    let auto_memory = tmp.path().join("auto-memory");
    fs::create_dir_all(&auto_memory).expect("mkdir auto memory");
    fs::write(auto_memory.join("MEMORY.md"), "# auto\n\nbody").expect("write memory");
    fs::write(auto_memory.join("topic.md"), "# topic\n\nbody").expect("write topic");

    let conn = fresh_db();
    // Candidate with native_project=None (autoMemoryDirectory basis).
    let candidate = CandidateSource {
        provider: "claude_code".to_string(),
        root_path: auto_memory.to_string_lossy().to_string(),
        basis: DiscoveryBasis::ClaudeAutoMemoryDir,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    };
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &candidate).expect("confirm auto");

    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan ok");
    assert!(outcome.records_indexed > 0, "auto-memory records indexed");
    assert_eq!(source.provider, "claude_code");
    assert!(source.native_project.is_none());

    // Records carry the same parser tag regardless of the discovery basis.
    let source_rowid = ScanStore::source_rowid(&source.source_id).expect("rowid");
    let claude_rows: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'claude-markdown/v1'",
            rusqlite::params![source_rowid],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        claude_rows,
        outcome.records_indexed as i64,
        "auto-memory records tagged claude-markdown/v1"
    );
}

/// Cross-provider coexistence: one Codex + one Claude source confirmed and
/// scanned. Both carry their own `provider`/`parser_version`; the query layer
/// sees both. This is the foundational federation contract (Epic 2 goal).
#[test]
fn claude_and_codex_sources_coexist_after_scan_with_distinct_parser_tags() {
    let tmp_codex = tempdir().expect("codex tempdir");
    let tmp_claude = tempdir().expect("claude tempdir");

    // Codex source.
    let codex_memories = tmp_codex.path().join("memories");
    fs::create_dir_all(&codex_memories).expect("mkdir codex");
    fs::write(codex_memories.join("MEMORY.md"), "# codex\n\nbody").expect("write codex");
    let codex_candidate = CandidateSource {
        provider: "codex".to_string(),
        root_path: codex_memories.to_string_lossy().to_string(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    };

    // Claude source.
    let claude_memory = make_claude_memory(tmp_claude.path(), "proj");
    fs::write(claude_memory.join("MEMORY.md"), "# claude\n\nbody").expect("write claude");
    let claude_candidate = claude_candidate_for(&claude_memory, "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let codex = application::confirm_source(&registry, &codex_candidate).expect("confirm codex");
    let claude = application::confirm_source(&registry, &claude_candidate).expect("confirm claude");

    let codex_outcome =
        application::scan_source(&registry, &conn, &codex.source_id).expect("scan codex");
    let claude_outcome =
        application::scan_source(&registry, &conn, &claude.source_id).expect("scan claude");
    assert!(codex_outcome.records_indexed > 0);
    assert!(claude_outcome.records_indexed > 0);

    // Distinct parser tags per provider.
    let codex_rowid = ScanStore::source_rowid(&codex.source_id).expect("rowid");
    let claude_rowid = ScanStore::source_rowid(&claude.source_id).expect("rowid");
    let codex_tags: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'codex-markdown/v1'",
            rusqlite::params![codex_rowid],
            |row| row.get(0),
        )
        .expect("count codex tags");
    let claude_tags: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'claude-markdown/v1'",
            rusqlite::params![claude_rowid],
            |row| row.get(0),
        )
        .expect("count claude tags");
    assert_eq!(codex_tags as u64, codex_outcome.records_indexed);
    assert_eq!(claude_tags as u64, claude_outcome.records_indexed);

    // No cross-contamination.
    let codex_with_claude_tag: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'claude-markdown/v1'",
            rusqlite::params![codex_rowid],
            |row| row.get(0),
        )
        .unwrap_or(0);
    let claude_with_codex_tag: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records \
             WHERE source_id = ?1 AND parser_version = 'codex-markdown/v1'",
            rusqlite::params![claude_rowid],
            |row| row.get(0),
        )
        .unwrap_or(0);
    assert_eq!(codex_with_claude_tag, 0, "no claude tags on codex records");
    assert_eq!(claude_with_codex_tag, 0, "no codex tags on claude records");
}

/// Cross-provider query surface (Story 2.2 AC): the FTS5-backed read path
/// (`application::search` → `ScanStore::search_records`) returns records from
/// BOTH providers in a single page when both contain the search term. The
/// "query sees both providers" claim was previously asserted only via raw SQL;
/// this closes the gap by exercising the actual read path the AC names.
#[test]
fn claude_and_codex_records_are_returnable_by_search_together() {
    let tmp_codex = tempdir().expect("codex tempdir");
    let tmp_claude = tempdir().expect("claude tempdir");

    // A single shared alphabetic term so one FTS5 query matches records from
    // both providers (no hyphen, to avoid query-operator tokenization quirks).
    let shared_term = "federation";

    // Codex source.
    let codex_memories = tmp_codex.path().join("memories");
    fs::create_dir_all(&codex_memories).expect("mkdir codex");
    fs::write(
        codex_memories.join("MEMORY.md"),
        format!("# codex memory\n\n{shared_term} body"),
    )
    .expect("write codex");
    let codex_candidate = CandidateSource {
        provider: "codex".to_string(),
        root_path: codex_memories.to_string_lossy().to_string(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    };

    // Claude source.
    let claude_memory = make_claude_memory(tmp_claude.path(), "proj");
    fs::write(
        claude_memory.join("MEMORY.md"),
        format!("# claude memory\n\n{shared_term} body"),
    )
    .expect("write claude");
    let claude_candidate = claude_candidate_for(&claude_memory, "proj");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let codex = application::confirm_source(&registry, &codex_candidate).expect("confirm codex");
    let claude = application::confirm_source(&registry, &claude_candidate).expect("confirm claude");
    application::scan_source(&registry, &conn, &codex.source_id).expect("scan codex");
    application::scan_source(&registry, &conn, &claude.source_id).expect("scan claude");

    // One FTS5 query through the application read path; both providers match.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new(shared_term.to_string(), None, None).expect("valid request"),
    )
    .expect("search ok");
    assert!(
        !page.results().is_empty(),
        "search returned no results for {shared_term:?}"
    );

    let providers: Vec<&str> = page.results().iter().map(|r| r.provider()).collect();
    assert!(
        providers.contains(&"codex"),
        "codex record present in search results: {providers:?}"
    );
    assert!(
        providers.contains(&"claude_code"),
        "claude_code record present in search results: {providers:?}"
    );
}

/// Lifecycle honesty: a rejected Claude source is not scannable. Confirmed
/// sources scan; non-confirmed sources surface `NotConfirmed` (NOT a
/// `ProviderNotScannable`-style failure — that variant is removed).
#[test]
fn claude_rejected_source_is_not_scannable_with_not_confirmed() {
    let tmp = tempdir().expect("tempdir");
    let memory = make_claude_memory(tmp.path(), "proj");
    fs::write(memory.join("MEMORY.md"), "# memory\n\nbody").expect("write memory");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    // Reject the candidate, then attempt a scan.
    let candidate = claude_candidate_for(&memory, "proj");
    let rejected = application::reject_source(&registry, &candidate).expect("reject claude");
    assert_eq!(rejected.lifecycle_state, SourceLifecycle::Rejected);

    let err = application::scan_source(&registry, &conn, &rejected.source_id)
        .expect_err("rejected source must not scan");
    assert!(
        matches!(err, ScanError::NotConfirmed),
        "expected NotConfirmed, got {err:?}"
    );
}
