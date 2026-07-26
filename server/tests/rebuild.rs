//! Story 4.4 — Full Derived Index rebuild (spec-4-4-rebuild-index.md).
//!
//! These tests pin every I/O matrix row of `application::rebuild_index` +
//! `http::start_rebuild`:
//! - Happy rebuild (idle, ≥1 Confirmed source): wipe + per-source re-scan to a
//!   fresh active generation; stable `record_id` + Provenance identical pre
//!   and post rebuild (only `generation` / `observed_at` may differ).
//! - Atomic wipe: EXACTLY `memory_records`, `scan_runs`, `scan_diagnostics`,
//!   and `tessera_meta` rows matching `active_generation:%` are empty after
//!   the wipe; `source_registry`, `tessera_meta.schema_version`, and
//!   `tessera_migrations_applied` are unchanged.
//! - In-flight race guard: rebuild while ANY source has a
//!   `queued/running/staging/committing` run → no state change, returns
//!   `RebuildError::InFlight`.
//! - Zero Confirmed sources: wipe still runs (clears leaked disabled /
//!   rejected records — first path that ever does); no rescans dispatched;
//!   returns an empty Confirmed list.
//! - Disabled/rejected leaked records: cleared by the wipe; that source's
//!   `source_registry` row + lifecycle + health unchanged; NOT re-scanned.
//! - Unreadable source isolation: one healthy Confirmed source + one
//!   Confirmed source whose root is unreadable; wipe proceeds, healthy source
//!   re-scans to a fresh active generation, unreadable source fails per 4.2
//!   source-scoped error isolation; rebuild still returns Ok(confirmed).
//! - Zero-source-mutation gate: source file set / content / size / mtime are
//!   unchanged across rebuild.

use std::fs;
use std::sync::Arc;
use std::time::SystemTime;

use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis,
};
use tessera_lib::domain::source::{HealthCause, HealthState, SourceLifecycle};
use tessera_lib::index::migrations;
use tessera_lib::index::scan_store::ScanStore;
use tessera_lib::index::SourceRegistry;

/// A tiny, dependency-free FNV-1a 64-bit content hash used by the zero-source-
/// mutation gate assertions (Patch I). This is NOT used by production code;
/// it lets the test assert content equality (not just size+mtime) without
/// pulling a hashing crate into the test deps. Mirrors the FNV-1a constants
/// the project already uses (see `domain/scan.rs`'s record_id derivation).
fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a fresh in-memory DB and apply all migrations. Returns a connection
/// with foreign-key enforcement ON (matching boot).
fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign_keys pragma must apply");
    migrations::apply(&mut conn).expect("migrations apply on fresh db");
    conn
}

/// Build a real Codex-shaped candidate for a root path. Codex's
/// `native_project` is `None` (global store).
fn codex_candidate(root: &std::path::Path) -> CandidateSource {
    CandidateSource {
        provider: "codex".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    }
}

/// Create a memories-shaped directory and return its path.
fn make_memories(parent: &std::path::Path) -> std::path::PathBuf {
    let memories = parent.join("memories");
    fs::create_dir_all(&memories).expect("create memories dir");
    memories
}

/// Confirm a Codex source at `root` and return the materialized `Source`.
fn confirm_codex(conn: &Connection, root: &std::path::Path) -> tessera_lib::domain::source::Source {
    let registry = SourceRegistry::new(conn);
    application::confirm_source(&registry, &codex_candidate(root)).expect("confirm codex")
}

/// Snapshot the file size, mtime, AND content hash of every file under
/// `root` so a test can assert the zero-source-mutation gate (NFR-1/NFR-10)
/// by comparing pre and post rebuild. Returns `(relative_path, size, mtime,
/// content_hash)` tuples (Patch I: added `content_hash` — size+mtime alone
/// could miss a same-size byte-level mutation).
fn snapshot_files(root: &std::path::Path) -> Vec<(std::path::PathBuf, u64, SystemTime, u64)> {
    let mut snap = Vec::new();
    walk(root, root, &mut snap);
    snap.sort_by(|a, b| a.0.cmp(&b.0));
    snap
}

fn walk(
    root: &std::path::Path,
    dir: &std::path::Path,
    out: &mut Vec<(std::path::PathBuf, u64, SystemTime, u64)>,
) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("dir entry");
        let path = entry.path();
        let meta = entry.metadata().expect("metadata");
        if meta.is_file() {
            let rel = path.strip_prefix(root).expect("rel").to_path_buf();
            let mtime = meta.modified().expect("modified");
            // Patch I — hash the bytes (FNV-1a 64-bit, project-local
            // dependency-free) so a same-size byte-level mutation cannot
            // pass the gate silently. Size+mtime alone catch most mutations;
            // content_hash catches the rare same-size edit.
            let bytes = fs::read(&path).expect("read file bytes");
            let hash = fnv1a_64(&bytes);
            out.push((rel, meta.len(), mtime, hash));
        } else if meta.is_dir() {
            walk(root, &path, out);
        }
    }
}

// ===========================================================================
// Atomic wipe — exact target set + preserved tables
// ===========================================================================

/// AC: rebuild wipes EXACTLY `memory_records`, `scan_runs`, `scan_diagnostics`,
/// and `tessera_meta` rows matching `key LIKE 'active_generation:%'`. It MUST
/// NOT touch `source_registry`, `tessera_meta.schema_version`, or
/// `tessera_migrations_applied`. The wipe is one transaction: any partial
/// state would surface as a count mismatch here.
#[test]
fn rebuild_wipes_exactly_the_four_targets_and_preserves_registry_and_schema() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "# memory\nbody").expect("fixture");

    let conn = fresh_db();
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    // Run a real scan to populate memory_records + scan_runs + scan_diagnostics
    // + the active_generation:* meta row.
    application::scan_source(&registry, &conn, &source.source_id)
        .expect("initial scan succeeds");
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Seed a non-active-generation staging row in memory_records to verify the
    // wipe clears EVERY memory_records row (not just the active generation).
    conn.execute(
        "INSERT INTO memory_records
            (record_id, source_id, generation, provider, unit_kind, native_unit_id,
             native_locator, content_hash, parser_version, title, body,
             native_project, provider_memory_type, coverage_level, observed_at,
             source_revision, display_locator)
         VALUES ('rec_leaked', ?1, 'gen_stale', 'codex', 'section', 'rec_leaked',
                 'file:///leaked#semantic', 'leaked-hash', 'v1', 'leaked', 'leak-body',
                 NULL, 'memory', 'full', 1, 'rev', 'file:///leaked#L1-L2')",
        params![source_rowid],
    )
    .expect("seed leaked row");
    // Seed a non-active-generation diagnostic to verify scan_diagnostics is wiped.
    conn.execute(
        "INSERT INTO scan_diagnostics (source_id, generation, kind, observed_path)
         VALUES (?1, 'gen_stale', 'unsupported_artifact', '/leaked/path')",
        params![source_rowid],
    )
    .expect("seed leaked diagnostic");

    // Seed a non-active_generation:* tessera_meta key to verify the wipe's
    // LIKE clause leaves it intact (the schema_version row + any future
    // mapping revision key MUST survive).
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('reserved_future_key', 'keep-me')",
        [],
    )
    .expect("seed reserved meta key");

    // Snapshot the preserved-tables state BEFORE rebuild.
    let pre_schema_version: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    let pre_audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tessera_migrations_applied",
            [],
            |row| row.get(0),
        )
        .expect("audit count");
    let pre_source_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("source count");
    // Patch J — snapshot the FULL source_registry row incl. `health_cause`
    // (4.2 health = state + cause). A regression that strips the cause
    // column on rebuild would silently break the inventory surface.
    let pre_source_row: (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT lifecycle_state, health_state, coverage_level, health_cause FROM source_registry WHERE id = ?1",
            params![source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("source row");

    // Rebuild — wipe + return Confirmed ids.
    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert_eq!(confirmed.len(), 1, "exactly one Confirmed source to rescan");
    assert_eq!(confirmed[0], source.source_id);

    // Post-wipe table counts: the four targets are EMPTY.
    let post_memory: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_memory, 0, "memory_records wiped");
    let post_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_runs, 0, "scan_runs wiped");
    let post_diag: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_diagnostics", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_diag, 0, "scan_diagnostics wiped");
    let post_active_gen: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(post_active_gen, 0, "active_generation:* meta rows wiped");

    // Preserved tables: source_registry, schema_version, audit log, and the
    // reserved non-active_generation:* meta key are ALL unchanged.
    let post_schema_version: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    assert_eq!(
        post_schema_version, pre_schema_version,
        "schema_version preserved"
    );
    let post_audit_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tessera_migrations_applied",
            [],
            |row| row.get(0),
        )
        .expect("audit count");
    assert_eq!(post_audit_count, pre_audit_count, "audit log preserved");
    let post_source_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("source count");
    assert_eq!(post_source_count, pre_source_count, "source_registry preserved");
    let post_source_row: (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT lifecycle_state, health_state, coverage_level, health_cause FROM source_registry WHERE id = ?1",
            params![source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("source row");
    assert_eq!(
        post_source_row, pre_source_row,
        "source_registry row unchanged (incl. health_cause)"
    );

    // The reserved non-active_generation:* meta key survived the wipe.
    let reserved: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'reserved_future_key'",
            [],
            |row| row.get(0),
        )
        .expect("reserved meta key readable");
    assert_eq!(reserved, "keep-me");

    // The active generation marker is gone, so the source's prior generation
    // is no longer queryable post-wipe (the rebuild's re-scan will produce a
    // fresh one — tested in happy_path_rebuild_restores_stable_identity_and_provenance).
    let active_after = ScanStore::new(&conn)
        .active_generation(source_rowid)
        .expect("active");
    assert_eq!(active_after, None, "active generation pointer wiped");
}

// ===========================================================================
// Stable identity + Provenance reproduction
// ===========================================================================

/// AC: after a rebuild, the post-rebuild active records have the SAME
/// `record_id` and Provenance fields (`native_locator`, `display_locator`,
/// `native_unit_id`, `provider`, `unit_kind`, `provider_memory_type`,
/// `native_project`) as pre-rebuild for unchanged source data. Only
/// `generation` and `observed_at` may differ. Because `record_id =
/// rec_<fnv1a(source_id|provider|native_locator|unit_kind)>` is a pure
/// function of source data + the preserved `src_<rowid>`, this is satisfied
/// by construction.
#[test]
fn happy_path_rebuild_restores_stable_identity_and_provenance() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "# title\nbody line").expect("fixture");

    let conn = fresh_db();
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let pre_outcome = application::scan_source(&registry, &conn, &source.source_id)
        .expect("initial scan succeeds");
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Snapshot pre-rebuild active records' identity + Provenance fields.
    let pre_active_gen = ScanStore::new(&conn)
        .active_generation(source_rowid)
        .expect("active")
        .expect("active generation exists");
    type IdentityRow = (String, String, String, String, String, String, String, Option<String>);
    let pre_records: Vec<IdentityRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT record_id, provider, unit_kind, native_unit_id, native_locator,
                        display_locator, provider_memory_type, native_project
                 FROM memory_records
                 WHERE source_id = ?1 AND generation = ?2
                 ORDER BY record_id ASC",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(
                params![source_rowid, pre_active_gen.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .expect("query_map");
        rows.map(|r| r.expect("row")).collect()
    };
    assert!(!pre_records.is_empty(), "pre-rebuild records exist");

    // Rebuild (wipe + return Confirmed ids). The HTTP layer would now spawn
    // per-source rescans; for this unit test we drive the SAME path directly
    // (application::scan_reserved_source is what the HTTP worker calls).
    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert_eq!(confirmed.len(), 1);
    // Reserve a run on the freshly-wiped DB and run the scan synchronously.
    // (Mirrors what http::start_rebuild does per Confirmed source.)
    let (scan_id, fencing_token, generation) =
        ScanStore::new(&conn).begin_run(source_rowid, "pending").expect("begin_run");
    let outcome = application::scan_reserved_source(
        &registry,
        &conn,
        &source.source_id,
        scan_id,
        fencing_token,
        generation,
    )
    .expect("post-rebuild scan succeeds");

    // Post-rebuild active records: SAME record_id + Provenance fields.
    let post_active_gen = ScanStore::new(&conn)
        .active_generation(source_rowid)
        .expect("active")
        .expect("post-rebuild active generation exists");
    let post_records: Vec<IdentityRow> = {
        let mut stmt = conn
            .prepare(
                "SELECT record_id, provider, unit_kind, native_unit_id, native_locator,
                        display_locator, provider_memory_type, native_project
                 FROM memory_records
                 WHERE source_id = ?1 AND generation = ?2
                 ORDER BY record_id ASC",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(
                params![source_rowid, post_active_gen.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .expect("query_map");
        rows.map(|r| r.expect("row")).collect()
    };

    assert_eq!(
        pre_records.len(),
        post_records.len(),
        "same record count post-rebuild"
    );
    for (pre, post) in pre_records.iter().zip(post_records.iter()) {
        assert_eq!(pre.0, post.0, "record_id stable across rebuild");
        assert_eq!(pre.1, post.1, "provider stable");
        assert_eq!(pre.2, post.2, "unit_kind stable");
        assert_eq!(pre.3, post.3, "native_unit_id stable");
        assert_eq!(pre.4, post.4, "native_locator stable");
        assert_eq!(pre.5, post.5, "display_locator stable");
        assert_eq!(pre.6, post.6, "provider_memory_type stable");
        assert_eq!(pre.7, post.7, "native_project stable");
    }

    // Generation DID advance (scan_runs was wiped and AUTOINCREMENT continues;
    // the post-rebuild generation is `gen_<new scan_id>`, distinct from the
    // pre-rebuild one). This is the spec's expected-and-harmless observation.
    assert_ne!(
        pre_active_gen.0, post_active_gen.0,
        "generation advances across rebuild (scan_runs wiped)"
    );
    assert_ne!(
        pre_outcome.generation.0, outcome.generation.0,
        "scan outcomes carry distinct generations"
    );
}

// ===========================================================================
// In-flight race guard
// ===========================================================================

/// AC: rebuild while ANY source has an in-flight (`queued/running/staging/
/// committing`) run → no state change (no wipe, no reservation), returns
/// `RebuildError::InFlight`. The HTTP layer maps this to a 409
/// `rebuild_failed` envelope.
#[test]
fn rebuild_rejects_when_any_scan_is_in_flight_with_no_state_change() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "body").expect("fixture");

    let conn = fresh_db();
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Inject an in-flight run for the source (mimics a reconcile/rescan that
    // has begun_run but not yet reached a terminal state).
    ScanStore::new(&conn)
        .begin_run(source_rowid, "pending")
        .expect("begin_run leaves a queued row");

    // Snapshot memory_records + scan_runs counts to assert no wipe happened.
    let pre_memory: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .expect("count");
    let pre_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
        .expect("count");
    assert!(pre_memory > 0, "fixture: pre-rebuild memory_records exist");
    assert!(pre_runs > 0, "fixture: pre-rebuild scan_runs exist");

    let err = application::rebuild_index(&conn).expect_err("in-flight rejects");
    assert!(
        matches!(err, application::RebuildError::InFlight),
        "expected InFlight, got {err:?}"
    );

    // No state change: counts unchanged, the in-flight run is still there.
    let post_memory: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .expect("count");
    let post_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_memory, pre_memory, "no wipe occurred");
    assert_eq!(post_runs, pre_runs, "no reservation cleanup occurred");
}

/// AC: an in-flight run on ANY source blocks the rebuild globally (not just
/// for that source). One source has an in-flight run; a SECOND source is
/// idle and Confirmed. The rebuild still rejects — the global race guard
/// prevents a wipe racing with any in-flight scan.
#[test]
fn rebuild_rejects_when_any_source_in_flight_globally() {
    let tmp = tempdir().expect("tempdir");
    let root_a = make_memories(&tmp.path().join("a"));
    let root_b = make_memories(&tmp.path().join("b"));
    fs::write(root_a.join("MEMORY.md"), "a").expect("a");
    fs::write(root_b.join("MEMORY.md"), "b").expect("b");

    let conn = fresh_db();
    let a = confirm_codex(&conn, &root_a);
    let b = confirm_codex(&conn, &root_b);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &a.source_id).expect("scan a");
    let _ = application::scan_source(&registry, &conn, &b.source_id).expect("scan b");

    // Mark source A as in-flight; source B stays idle.
    let a_rowid = a.source_id.to_rowid().expect("rowid");
    ScanStore::new(&conn)
        .begin_run(a_rowid, "pending")
        .expect("a in-flight");

    // Rebuild rejects even though B is idle.
    let err = application::rebuild_index(&conn).expect_err("in-flight rejects");
    assert!(matches!(err, application::RebuildError::InFlight));
}

// ===========================================================================
// Zero Confirmed sources
// ===========================================================================

/// AC: rebuild with zero Confirmed sources wipes (clearing any leaked
/// disabled / rejected records — the first path that ever does), dispatches
/// no rescans, returns an empty Confirmed list. The index is empty after.
#[test]
fn rebuild_with_zero_confirmed_sources_still_wipes_and_dispatches_no_rescans() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());

    let conn = fresh_db();
    // Reject the only candidate — zero Confirmed sources.
    let registry = SourceRegistry::new(&conn);
    let rejected =
        application::reject_source(&registry, &codex_candidate(&root)).expect("reject");

    // Seed leaked memory_records + scan_runs + active_generation meta for the
    // rejected source (simulating records from a prior confirm/scan that were
    // never cleaned when the source was rejected).
    let rejected_rowid = rejected.source_id.to_rowid().expect("rowid");
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (?1, 'gen_leaked', 'succeeded', 1, 'gen_leaked', 'leak')",
        params![rejected_rowid],
    )
    .expect("seed scan_runs");
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES (?1, 'gen_leaked')",
        params![format!("active_generation:{rejected_rowid}")],
    )
    .expect("seed active_generation meta");
    conn.execute(
        "INSERT INTO memory_records
            (record_id, source_id, generation, provider, unit_kind, native_unit_id,
             native_locator, content_hash, parser_version, title, body,
             native_project, provider_memory_type, coverage_level, observed_at,
             source_revision, display_locator)
         VALUES ('rec_leak', ?1, 'gen_leaked', 'codex', 'section', 'rec_leak',
                 'file:///leak', 'h', 'v1', 'leak', 'leak', NULL, 'memory', 'full', 1, 'r', 'file:///leak#L1')",
        params![rejected_rowid],
    )
    .expect("seed memory_records");

    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert!(confirmed.is_empty(), "no Confirmed sources to rescan");

    // The leaked records were cleared by the wipe (this is the first path
    // that ever clears a rejected source's leaked derived records).
    let post_memory: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_memory, 0);
    let post_runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
        .expect("count");
    assert_eq!(post_runs, 0);
    let post_active_gen: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(post_active_gen, 0);

    // The rejected source's registry row + lifecycle + health are unchanged.
    let post_rejected = registry
        .get(&rejected.source_id)
        .expect("db ok")
        .expect("rejected row");
    assert_eq!(post_rejected.lifecycle_state, SourceLifecycle::Rejected);
    assert_eq!(post_rejected.health_state, HealthState::Unknown);
    // Patch J — assert health_cause too (4.2 health = state + cause). A
    // rejected row that was never scanned carries (Unknown, None); a
    // regression that strips the cause column would silently break the
    // inventory surface.
    assert_eq!(
        post_rejected.health_cause,
        HealthCause::None,
        "Patch J: rejected source's health_cause preserved"
    );
}

// ===========================================================================
// Disabled/rejected leaked records cleared; not re-scanned
// ===========================================================================

/// AC: a Disabled or Rejected source that leaked derived records (from a
/// prior confirm/scan) has those records cleared by the wipe. Its
/// `source_registry` row + lifecycle + health are unchanged. It is NOT in
/// the rescan set (not Confirmed).
#[test]
fn rebuild_clears_leaked_disabled_records_without_rescanning_or_touching_registry_row() {
    let tmp = tempdir().expect("tempdir");
    let healthy_root = make_memories(&tmp.path().join("healthy"));
    let disabled_root = make_memories(&tmp.path().join("disabled"));
    fs::write(healthy_root.join("MEMORY.md"), "healthy").expect("healthy");
    fs::write(disabled_root.join("MEMORY.md"), "disabled").expect("disabled");

    let conn = fresh_db();
    let healthy = confirm_codex(&conn, &healthy_root);
    let disabled = confirm_codex(&conn, &disabled_root);
    let registry = SourceRegistry::new(&conn);

    // Both sources scan cleanly first.
    let _ = application::scan_source(&registry, &conn, &healthy.source_id).expect("scan healthy");
    let _ = application::scan_source(&registry, &conn, &disabled.source_id).expect("scan disabled");
    // Then disable the second one (a disabled source retains its prior derived
    // records — its active generation pointer + memory_records survive disable).
    let _ = application::disable_source(&registry, &disabled.source_id).expect("disable");

    let disabled_rowid = disabled.source_id.to_rowid().expect("rowid");
    let pre_disabled_records: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id = ?1",
            params![disabled_rowid],
            |row| row.get(0),
        )
        .expect("count");
    assert!(
        pre_disabled_records > 0,
        "fixture: disabled source has leaked records pre-rebuild"
    );

    // Rebuild — wipe + return Confirmed ids (only the healthy source).
    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert_eq!(confirmed.len(), 1, "only the healthy source is Confirmed");
    assert_eq!(confirmed[0], healthy.source_id);
    assert!(
        !confirmed.contains(&disabled.source_id),
        "disabled source is NOT in the rescan set"
    );

    // The disabled source's leaked derived records were cleared by the wipe.
    let post_disabled_records: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id = ?1",
            params![disabled_rowid],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(
        post_disabled_records, 0,
        "disabled source's leaked records cleared by wipe"
    );

    // The disabled source's registry row + lifecycle + health are unchanged.
    let post_disabled = registry
        .get(&disabled.source_id)
        .expect("db ok")
        .expect("disabled row");
    assert_eq!(post_disabled.lifecycle_state, SourceLifecycle::Disabled);
    // Patch J — assert the FULL health row (state + cause). The prior
    // successful scan wrote (Healthy, None); a regression that strips the
    // cause column would silently break the inventory surface.
    assert_eq!(post_disabled.health_state, HealthState::Healthy);
    assert_eq!(
        post_disabled.health_cause,
        HealthCause::None,
        "Patch J: disabled source's health_cause preserved (Healthy→None)"
    );
}

// ===========================================================================
// Unreadable source isolation (4.2 source-scoped error isolation)
// ===========================================================================

/// AC: one Confirmed source whose root is unreadable (4.2 PathMissing) +
/// one healthy Confirmed source. Rebuild wipes, then re-scans each Confirmed
/// source. The healthy source rebuilds fully (fresh active generation); the
/// unreadable source's re-scan fails per 4.2 source-scoped error isolation
/// (marked degraded + cause + last-success=None post-wipe + stale=false
/// because no active generation). The rebuild itself still returns
/// Ok(confirmed) with both sources in the rescan set; the per-source failure
/// surfaces on that source's inventory, not as a rebuild-wide error.
///
/// Patch I — also snapshot the healthy source's file content (size+mtime+
/// hash) before AND after the rebuild+re-scan. AC#4 couples the zero-source-
/// mutation gate to the unreadable-source scenario: the healthy source's
/// re-scan (the one that succeeds) MUST NOT mutate its files, and the
/// unreadable source's failed re-scan also MUST NOT mutate anything.
#[test]
fn rebuild_isolates_unreadable_source_failure_per_4_2_source_scoped_isolation() {
    let tmp = tempdir().expect("tempdir");
    let healthy_root = make_memories(&tmp.path().join("healthy"));
    let unreadable_root = make_memories(&tmp.path().join("gone"));
    fs::write(healthy_root.join("MEMORY.md"), "healthy body").expect("healthy");
    fs::write(unreadable_root.join("MEMORY.md"), "gone body").expect("gone");

    let conn = fresh_db();
    let healthy = confirm_codex(&conn, &healthy_root);
    let unreadable = confirm_codex(&conn, &unreadable_root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &healthy.source_id).expect("scan healthy");
    let _ = application::scan_source(&registry, &conn, &unreadable.source_id).expect("scan gone");

    // Patch I — snapshot the healthy source's files BEFORE we make the other
    // root unreadable + BEFORE the rebuild. The rebuild's re-scan of the
    // healthy source reads its files; this snapshot lets us prove no
    // mutation occurred (size + mtime + content hash).
    let healthy_pre = snapshot_files(&healthy_root);

    // Make the "gone" source's root unreadable by removing it (4.2 PathMissing
    // cause). The source's prior active generation survives the failed re-scan
    // (4.2 NFR-9), but the rebuild's WIPE already cleared it — so post-rebuild
    // the unreadable source has NO active generation and re-scan failure marks
    // it degraded + PathMissing + last_success=None + stale=false (no active
    // generation → no older results to be stale against).
    fs::remove_dir_all(&unreadable_root).expect("remove unreadable root");

    // Rebuild — wipe + return Confirmed ids.
    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert_eq!(confirmed.len(), 2, "both Confirmed sources in rescan set");
    assert!(confirmed.contains(&healthy.source_id));
    assert!(confirmed.contains(&unreadable.source_id));

    // Per-source re-dispatch (mirrors what http::start_rebuild does per
    // Confirmed source). The healthy source re-scans to a fresh active gen;
    // the unreadable source's re-scan fails (4.2 source-scoped isolation).
    for source_id in &confirmed {
        let rowid = source_id.to_rowid().expect("rowid");
        let (scan_id, fencing_token, generation) =
            ScanStore::new(&conn).begin_run(rowid, "pending").expect("begin_run");
        let _ = application::scan_reserved_source(
            &registry,
            &conn,
            source_id,
            scan_id,
            fencing_token,
            generation,
        );
    }

    // The healthy source rebuilt fully (fresh active generation).
    let healthy_rowid = healthy.source_id.to_rowid().expect("rowid");
    let healthy_active = ScanStore::new(&conn)
        .active_generation(healthy_rowid)
        .expect("active")
        .expect("healthy has an active generation post-rebuild");
    let healthy_records: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id = ?1 AND generation = ?2",
            params![healthy_rowid, healthy_active.0],
            |row| row.get(0),
        )
        .expect("count");
    assert!(healthy_records > 0, "healthy source rebuilt records");

    // The unreadable source failed re-scan → degraded + PathMissing + no
    // active generation (last_success was wiped) + stale=false (no active
    // generation → no older results to be stale against). 4.2 source-scoped
    // isolation: the failure is on THIS source's row, not the rebuild's.
    let unreadable_after = registry
        .get(&unreadable.source_id)
        .expect("db ok")
        .expect("unreadable row");
    assert_eq!(unreadable_after.health_state, HealthState::Degraded);
    assert_eq!(unreadable_after.health_cause, HealthCause::PathMissing);
    let unreadable_active = ScanStore::new(&conn)
        .active_generation(unreadable.source_id.to_rowid().expect("rowid"))
        .expect("active");
    assert_eq!(
        unreadable_active,
        None,
        "wipe cleared last-success; failed re-scan did not produce one"
    );

    // Inventory projection: stale=false (no active generation → not stale),
    // cause=PathMissing surfaced.
    let unreadable_inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == unreadable.source_id)
        .expect("unreadable row in inventory");
    assert_eq!(unreadable_inv.health_state, HealthState::Degraded);
    assert_eq!(unreadable_inv.cause, Some(HealthCause::PathMissing));
    assert!(
        !unreadable_inv.stale,
        "no active generation → unavailable, not stale"
    );
    assert_eq!(
        unreadable_inv.last_successful_scan, None,
        "wipe cleared scan_runs so derived last_success_at is None"
    );

    // The healthy source's inventory row is Healthy + cause=None + not stale.
    let healthy_inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == healthy.source_id)
        .expect("healthy row in inventory");
    assert_eq!(healthy_inv.health_state, HealthState::Healthy);
    assert_eq!(healthy_inv.cause, None);
    assert!(!healthy_inv.stale);

    // Patch I — zero-source-mutation gate applied to the AC#4 scenario: the
    // healthy source's re-scan (the one that succeeded) MUST NOT mutate its
    // files. The unreadable source's failed re-scan touched nothing (it
    // could not even open the root), so it is trivially honest.
    let healthy_post = snapshot_files(&healthy_root);
    assert_eq!(
        healthy_pre.len(),
        healthy_post.len(),
        "Patch I: same healthy-source file count pre and post rebuild"
    );
    for (pre, post) in healthy_pre.iter().zip(healthy_post.iter()) {
        assert_eq!(pre.0, post.0, "Patch I: same relative path");
        assert_eq!(pre.1, post.1, "Patch I: same size ({} bytes)", pre.1);
        assert_eq!(
            pre.2, post.2,
            "Patch I: same mtime for {:?}",
            pre.0
        );
        assert_eq!(
            pre.3, post.3,
            "Patch I: same content hash for {:?} (byte-level zero-source-mutation gate)",
            pre.0
        );
    }
}

// ===========================================================================
// Zero-source-mutation gate (NFR-1 / NFR-10)
// ===========================================================================

/// AC: source file set / content / size / mtime are unchanged across a
/// rebuild. The wipe + per-source re-scan must NEVER touch source files
/// (NFR-1/NFR-10 — zero-source-mutation gate).
#[test]
fn rebuild_does_not_mutate_source_files() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "# memory\nbody").expect("fixture");
    fs::write(root.join("memory_summary.md"), "summary body").expect("fixture 2");

    let pre = snapshot_files(&root);

    let conn = fresh_db();
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("initial scan");

    // Rebuild + a synchronous re-scan (the full rebuild pipeline minus the
    // HTTP worker spawn — same FS-touching code path).
    let confirmed = application::rebuild_index(&conn).expect("rebuild");
    assert_eq!(confirmed.len(), 1);
    let rowid = source.source_id.to_rowid().expect("rowid");
    let (scan_id, fencing_token, generation) =
        ScanStore::new(&conn).begin_run(rowid, "pending").expect("begin_run");
    let _ = application::scan_reserved_source(
        &registry,
        &conn,
        &source.source_id,
        scan_id,
        fencing_token,
        generation,
    )
    .expect("post-rebuild scan");

    let post = snapshot_files(&root);
    assert_eq!(
        pre.len(),
        post.len(),
        "same file count pre and post rebuild"
    );
    for (pre, post) in pre.iter().zip(post.iter()) {
        assert_eq!(pre.0, post.0, "same relative path");
        assert_eq!(pre.1, post.1, "same size ({} bytes)", pre.1);
        assert_eq!(
            pre.2, post.2,
            "same mtime for {:?} (zero-source-mutation gate, NFR-1/NFR-10)",
            pre.0
        );
        // Patch I — content hash equality (size+mtime alone could miss a
        // same-size byte-level mutation).
        assert_eq!(
            pre.3, post.3,
            "same content hash for {:?} (Patch I: zero-source-mutation byte-level gate)",
            pre.0
        );
    }
}

// ===========================================================================
// any_in_flight_run — ScanStore-level race guard
// ===========================================================================

/// `ScanStore::any_in_flight_run` returns true iff ANY source has a
/// non-terminal (`queued/running/staging/committing`) run. This is the
/// rebuild's primary race guard. Verified at the store level so a regression
/// in the SQL predicate surfaces directly.
#[test]
fn any_in_flight_run_reports_global_in_flight_state() {
    let conn = fresh_db();
    let store = ScanStore::new(&conn);

    // Empty scan_runs → no in-flight.
    assert!(!store.any_in_flight_run().expect("any_in_flight"));

    // Confirm + scan a source so a row exists; the row is terminal
    // (succeeded) → still no in-flight.
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "body").expect("fixture");
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    assert!(!store.any_in_flight_run().expect("any_in_flight"));

    // Inject a queued run on the source → in-flight is true.
    let rowid = source.source_id.to_rowid().expect("rowid");
    let _ = store.begin_run(rowid, "pending").expect("begin_run");
    assert!(store.any_in_flight_run().expect("any_in_flight"));

    // Terminal states (succeeded / failed / cancelled) do not count.
    conn.execute(
        "UPDATE scan_runs SET state = 'failed', finished_at = 1, error_code = 'internal'",
        [],
    )
    .expect("mark all failed");
    assert!(!store.any_in_flight_run().expect("any_in_flight"));
}

// ===========================================================================
// reset_derived_data — ScanStore-level wipe
// ===========================================================================

/// `ScanStore::reset_derived_data` wipes the four targets atomically and
/// preserves the schema_version row. Verified at the store level so the SQL
/// is pinned independently of the application orchestration.
#[test]
fn reset_derived_data_wipes_four_targets_preserves_schema_version() {
    let conn = fresh_db();
    let store = ScanStore::new(&conn);

    // Seed every target table with a sentinel row.
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "body").expect("fixture");
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    let _source_rowid = source.source_id.to_rowid().expect("rowid");

    // Seed a reserved (non-active_generation:*) meta key to verify it survives.
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('reserved', 'keep')",
        [],
    )
    .expect("seed reserved meta");

    // Sanity: at least one row in each target pre-wipe.
    assert!(count(&conn, "SELECT COUNT(*) FROM memory_records") > 0);
    assert!(count(&conn, "SELECT COUNT(*) FROM scan_runs") > 0);
    assert!(count(&conn, "SELECT COUNT(*) FROM scan_diagnostics") >= 0);
    assert!(
        count(&conn, "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'")
            > 0
    );

    store.reset_derived_data().expect("wipe");

    assert_eq!(count(&conn, "SELECT COUNT(*) FROM memory_records"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM scan_runs"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM scan_diagnostics"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'"),
        0
    );

    // schema_version + reserved meta key preserved. Story 5.1 bumped
    // schema_version from 6 to 7 (the v6_tessera_projects migration adds the
    // `tessera_projects` + `project_mappings` tables); Story 5.2 bumped it
    // from 7 to 8 (the v7_project_mapping_revision migration seeds
    // `project_mapping_revision`); the rebuild wipe preserves it.
    let schema_version: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    assert_eq!(schema_version, "8", "schema_version preserved");
    let reserved: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'reserved'",
            [],
            |row| row.get(0),
        )
        .expect("reserved meta readable");
    assert_eq!(reserved, "keep");

    // source_registry preserved.
    let source_count = count(&conn, "SELECT COUNT(*) FROM source_registry");
    assert_eq!(source_count, 1, "source_registry preserved");
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).expect("count")
}

// ===========================================================================
// Patch H — wipe / begin_run failures map to RebuildError::Internal
// ===========================================================================

/// AC (Patch H): if `reset_derived_data` fails mid-wipe (e.g. a target table
/// is missing because the schema is corrupt), the application layer surfaces
/// `RebuildError::Internal` (the HTTP layer maps this to a 500 `internal`
/// envelope). The wipe transaction rolls back, so the index is unchanged.
#[test]
fn rebuild_returns_internal_when_wipe_fails() {
    let tmp = tempdir().expect("tempdir");
    let root = make_memories(tmp.path());
    fs::write(root.join("MEMORY.md"), "body").expect("fixture");

    let conn = fresh_db();
    let source = confirm_codex(&conn, &root);
    let registry = SourceRegistry::new(&conn);
    let _ = application::scan_source(&registry, &conn, &source.source_id).expect("scan");

    // Pre-snapshot the rows so we can prove the wipe transaction rolled back.
    let pre_runs: i64 = count(&conn, "SELECT COUNT(*) FROM scan_runs");
    let pre_memory: i64 = count(&conn, "SELECT COUNT(*) FROM memory_records");
    assert!(pre_runs > 0);
    assert!(pre_memory > 0);

    // Sabotage: drop `scan_diagnostics` so the wipe's DELETE fails. The whole
    // wipe transaction must roll back (no half-applied wipe).
    conn.execute("DROP TABLE scan_diagnostics", [])
        .expect("drop scan_diagnostics");

    let err = application::rebuild_index(&conn).expect_err("wipe must fail");
    assert!(
        matches!(err, application::RebuildError::Internal),
        "expected Internal, got {err:?}"
    );

    // The wipe transaction rolled back: scan_runs + memory_records are
    // unchanged (the four DELETEs are one transaction; one failure rolls all
    // of them back).
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM scan_runs"),
        pre_runs,
        "wipe rolled back: scan_runs unchanged"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM memory_records"),
        pre_memory,
        "wipe rolled back: memory_records unchanged"
    );
}

/// AC (Patch H, pairs with A): when `begin_run` (the per-source reservation
/// inside `http::start_rebuild`) fails partway through the dispatch loop,
/// `start_rebuild` returns a 500 envelope AND every previously-reserved
/// `queued` row is cleaned up via `fail_run` — so the next rebuild is NOT
/// locked out by orphan in-flight runs (Patch A's cleanup). The race guard
/// `any_in_flight_run` returns false after the failed dispatch.
///
/// This test calls `http::start_rebuild` directly so it does not need a live
/// HTTP server. The SQLite trigger `tessera_patch_a_fail_after_first_insert`
/// fires on the SECOND INSERT into `scan_runs` (the first reservation
/// succeeds and is the orphan Patch A cleans up; the second triggers the
/// abort). Without cleanup, the first reservation's `queued` row would wedge
/// every subsequent rebuild.
#[test]
fn start_rebuild_cleans_up_orphan_reservations_on_begin_run_failure() {
    let tmp = tempdir().expect("tempdir");
    let root_a = make_memories(&tmp.path().join("a"));
    let root_b = make_memories(&tmp.path().join("b"));
    fs::write(root_a.join("MEMORY.md"), "body a").expect("a");
    fs::write(root_b.join("MEMORY.md"), "body b").expect("b");

    // Build a real IndexState (not in-memory) so start_rebuild's worker
    // threads can open their own connections to the same db_path.
    let data_dir = tempdir().expect("data dir");
    let state = std::sync::Arc::new(tessera_lib::boot(data_dir.path()).expect("boot"));
    let confirmed_ids: Vec<tessera_lib::domain::source::SourceId> = {
        let conn = state.conn.lock().expect("conn lock");
        let registry = SourceRegistry::new(&conn);
        let a = application::confirm_source(&registry, &codex_candidate(&root_a)).expect("confirm a");
        let b = application::confirm_source(&registry, &codex_candidate(&root_b)).expect("confirm b");
        let _ = application::scan_source(&registry, &conn, &a.source_id).expect("scan a");
        let _ = application::scan_source(&registry, &conn, &b.source_id).expect("scan b");
        vec![a.source_id, b.source_id]
    };

    // Install a trigger that fires on the SECOND INSERT into scan_runs. The
    // first begin_run succeeds (count(*) before insert = 0 after the wipe);
    // the second begin_run sees count(*) = 1 and aborts. This produces
    // exactly the catastrophic state Patch A prevents: one orphan queued row
    // + a failing second reservation.
    {
        let conn = state.conn.lock().expect("conn lock");
        conn.execute(
            "CREATE TRIGGER tessera_patch_a_fail_after_first_insert \
             BEFORE INSERT ON scan_runs \
             WHEN (SELECT COUNT(*) FROM scan_runs) >= 1 \
             BEGIN \
                 SELECT RAISE(ABORT, 'patch_a_trigger'); \
             END",
            [],
        )
        .expect("create trigger");
    }

    let result = tessera_lib::start_rebuild(&state);
    let err = result.expect_err("rebuild must surface 500 on begin_run failure");
    // The HTTP layer maps `RebuildError::Internal` to a 500 envelope with
    // code `internal` (see start_rebuild's map_err). We pinned the HTTP-layer
    // mapping in start_rebuild's body; the start_rebuild function returns
    // the ErrorEnvelope directly.
    assert_eq!(
        err.code, "internal",
        "begin_run failure → 500 internal envelope (got code={:?})",
        err.code
    );
    assert_eq!(err.phase, "rebuild");

    // Patch A's cleanup: every queued reservation this dispatch produced has
    // been fail_run'd. No orphan `queued/running/staging/committing` rows
    // remain in scan_runs. Without the cleanup, the first reservation would
    // stay `queued` forever and the next rebuild would 409.
    {
        let conn = state.conn.lock().expect("conn lock");
        let store = ScanStore::new(&conn);
        assert!(
            !store.any_in_flight_run().expect("any_in_flight"),
            "Patch A: no orphan queued runs wedge the next rebuild"
        );
        // The fail_run'd rows are visible as `failed` (not `queued`): the
        // cleanup used the canonical fail_run transition, not a raw DELETE.
        let failed_after: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM scan_runs WHERE state = 'failed'",
                [],
                |row| row.get(0),
            )
            .expect("count failed");
        assert!(
            failed_after >= 1,
            "Patch A: at least one reservation fail_run'd (found {failed_after})"
        );
    }

    // Drop the trigger — a subsequent rebuild MUST succeed now (Patch A's
    // cleanup unblocked it). This is the load-bearing claim: a transient
    // begin_run failure must NOT lock the user out of rebuilding until
    // reboot.
    {
        let conn = state.conn.lock().expect("conn lock");
        conn.execute("DROP TRIGGER tessera_patch_a_fail_after_first_insert", [])
            .expect("drop trigger");
    }
    // Boot recovery is what would normally flip the orphan `failed` rows out
    // of `failed` (they're already terminal — `failed` counts as terminal in
    // any_in_flight_run). The rebuild race guard treats only non-terminal
    // states as in-flight, so we do not need to clear the `failed` rows here.
    let second = tessera_lib::start_rebuild(&state);
    assert!(
        second.is_ok(),
        "Patch A: subsequent rebuild unblocked after cleanup (got {second:?})"
    );
    let _ = confirmed_ids;
}

// ===========================================================================
// boot + start_rebuild smoke (rebuild index under a real IndexState)
// ===========================================================================

/// Boot a real `IndexState` on a scratch dir, confirm a Codex source, scan
/// it, then call `application::rebuild_index` through the shared connection.
/// Smoke-tests that the rebuild works against a real file-backed DB (not just
/// in-memory) and that the synchronous core composes cleanly with `boot`'s
/// migration + recovery setup.
#[test]
fn rebuild_index_works_against_a_booted_file_backed_state() {
    let dir = tempdir().expect("scratch app-data");
    let source_root = tempfile::tempdir().expect("source root");
    std::fs::write(source_root.path().join("MEMORY.md"), "# memory\nbody").expect("memory");

    let state = Arc::new(tessera_lib::boot(dir.path()).expect("boot"));
    let source_id = {
        let conn = state.conn.lock().expect("conn lock");
        let registry = SourceRegistry::new(&conn);
        let candidate = CandidateSource {
            provider: "codex".into(),
            root_path: source_root.path().to_string_lossy().into_owned(),
            basis: DiscoveryBasis::CodexHomeEnv,
            coverage_level: CoverageLevel::Full,
            native_project: None,
        };
        let source = application::confirm_source(&registry, &candidate).expect("confirm");
        application::scan_source(&registry, &conn, &source.source_id).expect("initial scan");
        source.source_id
    };

    // Rebuild via the shared connection — wipe + return Confirmed ids.
    let confirmed = {
        let conn = state.conn.lock().expect("conn lock");
        application::rebuild_index(&conn).expect("rebuild")
    };
    assert_eq!(confirmed.len(), 1);
    assert_eq!(confirmed[0], source_id);

    // Post-wipe: no scan_runs, no memory_records, no active_generation meta.
    let conn = state.conn.lock().expect("conn lock");
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM scan_runs"),
        0,
        "scan_runs wiped"
    );
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM memory_records"),
        0,
        "memory_records wiped"
    );
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'"
        ),
        0,
        "active_generation:* meta wiped"
    );
    // Tempdirs + `state` (and its `conn` borrow) drop in reverse declaration
    // order at end of scope — `conn` before `state` — so no explicit drops here.
}

// ---------------------------------------------------------------------------
// Story 5.2 — Reset Index preserves `project_mapping_revision` (AD-29).
//
// The reset wipe keys on the `active_generation:` prefix only, so the
// `project_mapping_revision` key (a Story 5.2 scalar seeded by migration id 8)
// survives a rebuild. Mappings and their revision MUST survive rebuild so the
// user's authored state is not silently undone by a rebuild.
// ---------------------------------------------------------------------------

/// Story 5.2 (AD-29) — a rebuild MUST NOT touch `project_mapping_revision` or
/// any row of `tessera_projects` / `project_mappings`. The wipe keys on the
/// `active_generation:` prefix only; the mapping revision + mappings
/// themselves are user-authored state the rebuild is contractually forbidden
/// to undo. Pins the AC: "Reset Index is run, `project_mapping_revision` is
/// read after, then it is unchanged".
#[test]
fn rebuild_preserves_project_mapping_revision_and_mappings() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    use tessera_lib::index::ProjectStore;

    let tmp = tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(tmp.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("conn lock");
        let project_store = ProjectStore::new(&conn);
        // Create a project + add two mappings. Each successful add bumps the
        // revision (0 → 1 → 2).
        let project = create_project(
            &project_store,
            &CreateProjectRequest { name: "Federation".to_string() },
        )
        .expect("create project");
        add_mapping(
            &project_store,
            &MappingRequest {
                project_id: project.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .expect("add codex mapping");
        add_mapping(
            &project_store,
            &MappingRequest {
                project_id: project.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some("proj-a".to_string()),
            },
        )
        .expect("add claude mapping");
        assert_eq!(
            project_store.project_mapping_revision().unwrap(),
            2,
            "two add-mappings bump the revision to 2"
        );
    }

    // Run the rebuild. No confirmed sources means the wipe runs and no
    // rescans dispatch — exactly what we want to test the wipe's scope.
    let confirmed = {
        let conn = state.conn.lock().expect("conn lock");
        application::rebuild_index(&conn).expect("rebuild succeeds with no confirmed sources")
    };
    assert!(confirmed.is_empty(), "no confirmed sources → empty rescan list");

    // Post-rebuild: the project + its mappings + the revision survive.
    {
        let conn = state.conn.lock().expect("conn lock");
        let project_store = ProjectStore::new(&conn);
        assert_eq!(
            project_store.project_mapping_revision().unwrap(),
            2,
            "AD-29: project_mapping_revision survives rebuild"
        );
        let projects = tessera_lib::application::list_projects(&project_store).unwrap();
        assert_eq!(projects.len(), 1, "the project row survives rebuild");
        assert_eq!(projects[0].mappings.len(), 2, "both mappings survive rebuild");
        // Active-generation markers ARE wiped (the rebuild's job); confirm the
        // wipe ran by asserting no active_generation keys remain.
        let active: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 0, "active_generation markers wiped by rebuild");
    }
}
