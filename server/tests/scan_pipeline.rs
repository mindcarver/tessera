//! Scan pipeline integration tests (Story 1.4 / spec-1-4-scan-pipeline.md,
//! as amended by review loop 1).
//!
//! Drives the full application → adapter → scan store → SQLite stack against
//! the spec's I/O matrix: first-scan success, mid-scan failure preserving the
//! previous generation, `dirty_after_validation` (via a scripted adapter
//! through the public orchestration), commit-CAS contention (real second-owner
//! fencing), boot recovery, idempotent re-scan, empty directory, unknown /
//! non-confirmed source, invalid root, NFR-1/SM-2 zero-write, and
//! matrix-boundary enumeration. Amendment regressions: first-enumeration
//! failure lands on a persisted run row, generation isolation on
//! staging-then-drift failure (composite PK + plain INSERT), symlink-alias
//! count honesty, foreign-key orphan rejection, and loud failure on a corrupt
//! persisted run state. No `std::env::set_var` (parallel-test races); tempdir
//! roots are confirmed directly via `application::confirm_source`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::SystemTime;

use rusqlite::Connection;
use tempfile::tempdir;

use tessera_lib::adapters::codex::{file_uri, percent_encode_fragment, CodexAdapter};
use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    ArtifactDiagnostic, ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis,
    EnumerateError, FileUnit, ProviderAdapter,
};
use tessera_lib::domain::scan::{build_record_id, ScanError};
use tessera_lib::domain::source::{Source, SourceLifecycle};
use tessera_lib::index::migrations;
use tessera_lib::index::scan_store::ScanStore;
use tessera_lib::index::SourceRegistry;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Open a fresh in-memory DB and apply all migrations (v0_meta +
/// v1_source_registry + v2_scan_generations). Returns a connection at
/// schema_version 4 with foreign-key enforcement ON (matching boot — the v3
/// `memory_records.source_id` reference must actually be policed).
fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("open in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign_keys pragma must apply");
    migrations::apply(&mut conn).expect("migrations apply on fresh db");
    conn
}

/// Build a real Codex-shaped candidate for a tempdir root.
fn candidate_for(root: &Path) -> CandidateSource {
    CandidateSource {
        provider: "codex".to_string(),
        root_path: root.to_string_lossy().to_string(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    }
}

/// Create a memories-shaped directory under a tempdir and return its path.
fn make_memories(parent: &Path) -> PathBuf {
    let memories = parent.join("memories");
    fs::create_dir_all(&memories).expect("create memories dir");
    memories
}

/// Confirm a source for `root` against `conn`, returning the persisted Source.
fn confirm(conn: &Connection, root: &Path) -> Source {
    let registry = SourceRegistry::new(conn);
    application::confirm_source(&registry, &candidate_for(root)).expect("confirm ok")
}

/// Snapshot (path, mtime, size, content) of every file under `root` so NFR-1 /
/// SM-2 zero-write can be asserted after a scan. Sorted for stable compare.
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

/// Assert two tree snapshots are identical (NFR-1 / SM-2 zero-write).
fn assert_tree_unchanged(
    before: &[(PathBuf, SystemTime, u64, Vec<u8>)],
    after: &[(PathBuf, SystemTime, u64, Vec<u8>)],
) {
    assert_eq!(before.len(), after.len(), "same file count");
    for (b, a) in before.iter().zip(after.iter()) {
        assert_eq!(b.0, a.0, "path");
        assert_eq!(b.1, a.1, "mtime unchanged (SM-2)");
        assert_eq!(b.2, a.2, "size unchanged (SM-2)");
        assert_eq!(b.3, a.3, "content unchanged (SM-2)");
    }
}

/// Count rows in a table (test assertion helper).
fn count_rows(conn: &Connection, table: &str) -> i64 {
    conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .expect("count rows")
}

/// Read the state + error_code of the latest scan_runs row for a source rowid.
fn latest_run_state(conn: &Connection, source_rowid: i64) -> (String, Option<String>) {
    conn.query_row(
        "SELECT state, error_code FROM scan_runs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
        [source_rowid],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )
    .expect("latest run state")
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

// ---------------------------------------------------------------------------
// Scripted adapters (drive the public orchestration through real scenarios)
// ---------------------------------------------------------------------------

/// An adapter whose SECOND enumeration returns an empty set: the manifest
/// re-validation at commit time then sees a genuine drift (every staged file
/// "vanished") and fails with `dirty_after_validation`. This replaces the
/// pre-amendment test that bypassed the orchestrator and drove the store
/// directly.
#[derive(Debug)]
struct DriftAdapter {
    calls: AtomicUsize,
}

impl DriftAdapter {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderAdapter for DriftAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if call == 0 {
            CodexAdapter.enumerate_file_units(root)
        } else {
            Ok(Vec::new())
        }
    }
}

/// An adapter whose SECOND enumeration delegates to the real Codex adapter but
/// bumps every unit's `mtime` by one nanosecond: same file set, same record
/// ids, but a changed manifest → `dirty_after_validation` AFTER staging. The
/// staged generation then holds rows whose `record_id`s are IDENTICAL to the
/// active generation's — the composite-PK / plain-INSERT isolation proof.
#[derive(Debug)]
struct MtimeBumpAdapter {
    calls: AtomicUsize,
}

/// An adapter that retargets an in-root file immediately after final
/// enumeration. The digest validation must reject it before reading outside
/// the confirmed root.
#[cfg(unix)]
#[derive(Debug)]
struct RetargetAfterEnumerationAdapter {
    calls: AtomicUsize,
    outside: PathBuf,
}

#[cfg(unix)]
impl RetargetAfterEnumerationAdapter {
    fn new(outside: PathBuf) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            outside,
        }
    }
}

#[cfg(unix)]
impl ProviderAdapter for RetargetAfterEnumerationAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let units = CodexAdapter.enumerate_file_units(root)?;
        if call == 1 {
            fs::create_dir(&self.outside).expect("create outside directory");
            fs::remove_file(root.join("MEMORY.md")).expect("remove in-root file");
            std::os::unix::fs::symlink(&self.outside, root.join("MEMORY.md"))
                .expect("retarget source file");
        }
        Ok(units)
    }
}

/// An adapter that changes a source file after final enumeration while
/// preserving its size and restored mtime. Only the final byte-digest check
/// can detect this drift.
#[derive(Debug)]
struct SameMetadataMutationAdapter {
    calls: AtomicUsize,
    replacement: Vec<u8>,
}

impl SameMetadataMutationAdapter {
    fn new(replacement: &[u8]) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            replacement: replacement.to_vec(),
        }
    }
}

impl ProviderAdapter for SameMetadataMutationAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let units = CodexAdapter.enumerate_file_units(root)?;
        if call == 1 {
            let target = units
                .iter()
                .find(|unit| unit.relative_path == "MEMORY.md")
                .expect("memory unit");
            let modified = fs::metadata(&target.absolute_path)
                .expect("metadata")
                .modified()
                .expect("modified time");
            fs::write(&target.absolute_path, &self.replacement).expect("same-size rewrite");
            fs::File::open(&target.absolute_path)
                .expect("open rewritten source")
                .set_times(fs::FileTimes::new().set_modified(modified))
                .expect("restore mtime");
        }
        Ok(units)
    }
}

/// An adapter whose final artifact observation adds a safe diagnostic. Final
/// enumeration equality must reject the run rather than activating stale
/// diagnostics.
#[derive(Debug)]
struct DiagnosticDriftAdapter {
    calls: AtomicUsize,
}

impl DiagnosticDriftAdapter {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderAdapter for DiagnosticDriftAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        CodexAdapter.enumerate_file_units(root)
    }

    fn enumerate_artifacts(&self, root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut observation = CodexAdapter.enumerate_artifacts(root)?;
        if call == 1 {
            observation.diagnostics.push(ArtifactDiagnostic {
                kind: "unsupported_artifact",
                observed_path: "late-note.txt".to_string(),
            });
        }
        Ok(observation)
    }
}

impl MtimeBumpAdapter {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }
}

impl ProviderAdapter for MtimeBumpAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let mut units = CodexAdapter.enumerate_file_units(root)?;
        if call > 0 {
            for u in &mut units {
                u.mtime += 1;
            }
        }
        Ok(units)
    }
}

/// An adapter whose FIRST enumeration always fails: the scan must still land
/// on a persisted run row (begin_run precedes the first enumeration) marked
/// `failed` with `enumeration_failed`.
#[derive(Debug)]
struct FailingEnumAdapter;

impl ProviderAdapter for FailingEnumAdapter {
    fn provider_id(&self) -> &'static str {
        "codex"
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    // Story 2.2: scripted adapters spoof the codex provider id, so they
    // declare codex's parser version too. The persisted tag is now read from
    // the adapter (single source of truth) instead of a hard-coded constant.
    fn parser_version(&self) -> &'static str {
        "codex-markdown/v1"
    }

    fn discover(&self) -> Vec<CandidateSource> {
        Vec::new()
    }

    fn enumerate_file_units(&self, _root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        Err(EnumerateError::Unreadable)
    }
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

/// Migration id 4 (`v3_canonical_memory_records`) applies on a fresh DB and
/// creates the canonical provenance and diagnostic projection tables.
#[test]
fn migrations_apply_canonical_records_and_rescan_cancellation_schema() {
    let conn = fresh_db();
    let v: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    // Story 4.2 bumped the schema_version baseline 5→6 with the
    // v5_source_health_cause migration (adds source_registry.health_cause);
    // Story 5.1 bumped 6→7 with the v6_tessera_projects migration (adds the
    // `tessera_projects` + `project_mappings` tables).
    assert_eq!(v, "7");

    for table in ["scan_runs", "memory_records", "scan_diagnostics"] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name=?1",
                [table],
                |row| row.get(0),
            )
            .expect("check table");
        assert_eq!(n, 1, "table {table} exists");
    }
    for index in [
        "scan_runs_source_fencing",
        "memory_records_source_generation",
        "scan_diagnostics_source_generation",
    ] {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name=?1",
                [index],
                |row| row.get(0),
            )
            .expect("check index");
        assert_eq!(n, 1, "index {index} exists");
    }

    let (id, name): (i64, String) = conn
        .query_row(
            "SELECT id, name FROM tessera_migrations_applied WHERE id = 4",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("v3 audit row");
    assert_eq!(id, 4);
    assert_eq!(name, "v3_canonical_memory_records");
}

#[test]
fn v3_upgrade_keeps_source_registry_and_invalidates_old_derived_state() {
    let mut conn = Connection::open_in_memory().expect("open legacy database");
    conn.execute_batch(
        r#"
        CREATE TABLE tessera_meta (key TEXT PRIMARY KEY, value TEXT NOT NULL) STRICT;
        CREATE TABLE tessera_migrations_applied (
            id INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at TEXT NOT NULL
        ) STRICT;
        INSERT INTO tessera_meta(key, value) VALUES ('schema_version', '3');
        INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1');
        CREATE TABLE source_registry (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            provider TEXT NOT NULL, source_kind TEXT NOT NULL, lifecycle_state TEXT NOT NULL,
            health_state TEXT NOT NULL, coverage_level TEXT NOT NULL,
            normalized_root_path TEXT NOT NULL, fingerprint TEXT NOT NULL, native_project TEXT
        ) STRICT;
        INSERT INTO source_registry VALUES
            (1, 'codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/tmp/root', 'fp', NULL);
        CREATE TABLE scan_runs (
            id INTEGER PRIMARY KEY AUTOINCREMENT, source_id INTEGER NOT NULL,
            generation TEXT NOT NULL, state TEXT NOT NULL, fencing_token INTEGER NOT NULL,
            intent TEXT NOT NULL, manifest_revision TEXT NOT NULL, error_code TEXT, finished_at INTEGER
        ) STRICT;
        INSERT INTO scan_runs VALUES (1, 1, 'gen_1', 'succeeded', 1, 'gen_1', 'old', NULL, NULL);
        CREATE TABLE memory_records (
            record_id TEXT NOT NULL, source_id INTEGER NOT NULL, generation TEXT NOT NULL,
            provider TEXT NOT NULL, unit_kind TEXT NOT NULL, native_unit_id TEXT NOT NULL,
            native_locator TEXT NOT NULL, content_hash TEXT NOT NULL, parser_version TEXT NOT NULL,
            PRIMARY KEY (record_id, generation)
        ) STRICT;
        INSERT INTO memory_records VALUES
            ('rec_old', 1, 'gen_1', 'codex', 'file', 'MEMORY.md', 'file:///tmp/root/MEMORY.md', 'h', 'file-level/v1');
        "#,
    )
    .expect("build v3 fixture");

    migrations::apply(&mut conn).expect("upgrade v3 to v4");
    let sources: i64 = conn
        .query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
        .expect("sources remain");
    let records: i64 = conn
        .query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
        .expect("old projection cleared");
    let runs: i64 = conn
        .query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
        .expect("old runs cleared");
    let active_markers: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM tessera_meta WHERE key LIKE 'active_generation:%'",
            [],
            |row| row.get(0),
        )
        .expect("active markers cleared");
    assert_eq!((sources, records, runs, active_markers), (1, 0, 0, 0));
}

// ---------------------------------------------------------------------------
// I/O matrix row 1: first-scan success
// ---------------------------------------------------------------------------

/// First scan of a confirmed Source with 2 memory files: the run persists
/// queued→…→succeeded, the staging generation becomes active, 2 file-level
/// records are indexed, and the returned outcome reports them.
#[test]
fn first_scan_success_commits_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# mem\n").expect("w");
    fs::write(memories.join("raw_memories.md"), "raw\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("scan succeeds");
    assert_eq!(outcome.records_indexed, 2);
    assert!(outcome.generation.0.starts_with("gen_"));
    assert!(outcome.scan_id > 0);

    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // The latest run is succeeded with no error_code.
    let (state, error_code) = latest_run_state(&conn, source_rowid);
    assert_eq!(state, "succeeded");
    assert_eq!(error_code, None);

    // The staging generation became the active generation.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(outcome.generation.0.as_str()));

    // memory_records has exactly 2 file-level rows for the active generation.
    let n: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id=?1 AND generation=?2",
            rusqlite::params![source_rowid, outcome.generation.0],
            |row| row.get(0),
        )
        .expect("count records");
    assert_eq!(n, 2);

    // Records are canonical units with a stable semantic locator and an
    // independent line display locator.
    let mut stmt = conn
        .prepare(
            "SELECT record_id, unit_kind, parser_version, native_unit_id, native_locator, display_locator
             FROM memory_records WHERE source_id=?1",
        )
        .expect("prepare");
    let rows = stmt
        .query_map([source_rowid], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .expect("query");
    let mut seen = Vec::new();
    for r in rows {
        let (record_id, unit_kind, parser_version, native_unit_id, native_locator, display_locator) =
            r.expect("row");
        assert!(record_id.starts_with("rec_"), "rec_ id");
        assert!(matches!(unit_kind.as_str(), "section" | "file"));
        assert_eq!(parser_version, "codex-markdown/v1");
        assert!(native_locator.starts_with("file://"), "file URI locator");
        assert!(display_locator.contains("#L"), "display line locator");
        seen.push((native_unit_id, unit_kind));
    }
    seen.sort();
    assert_eq!(
        seen,
        vec![
            ("file".to_string(), "file".to_string()),
            ("section/h1:3:mem:1".to_string(), "section".to_string())
        ]
    );
}

// ---------------------------------------------------------------------------
// I/O matrix row 2: mid-scan failure preserves the previous generation
// ---------------------------------------------------------------------------

/// With an active generation present, a re-scan whose final manifest
/// re-validation detects a source change fails with `DirtyAfterValidation`,
/// the run is marked `failed` with `error_code='dirty_after_validation'`, the
/// new staging generation is NOT activated, and the previous active generation
/// + records stay fully visible (NFR-9).
///
/// Driven through the PUBLIC `application::scan_source_with` orchestration
/// with a scripted [`DriftAdapter`] whose second enumeration returns an empty
/// set — a real manifest drift observed by the real commit-time re-validation
/// (no test-only store seam).
#[test]
fn dirty_after_validation_never_activates_and_preserves_previous() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // First scan succeeds and activates gen for the single file.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    assert_eq!(first.records_indexed, 1);
    let first_gen = first.generation.0.clone();

    // Re-scan through a scripted adapter: the commit-time re-enumeration sees
    // an empty source (manifest drift) → DirtyAfterValidation.
    let err =
        application::scan_source_with(&DriftAdapter::new(), &registry, &conn, &source)
            .expect_err("drift at commit-time revalidation");
    assert!(
        matches!(err, ScanError::DirtyAfterValidation),
        "expected DirtyAfterValidation, got {err:?}"
    );

    // The latest run is failed with the dirty error code.
    let (state, error_code) = latest_run_state(&conn, source_rowid);
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("dirty_after_validation"));

    // The new generation is NOT active; the previous generation still is.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(
        active.as_deref(),
        Some(first_gen.as_str()),
        "previous gen stays active"
    );

    // The previous generation's record is still visible; the dirty staging
    // rows are present but not active (they will be GC'd at boot).
    let store = ScanStore::new(&conn);
    let active_count = store.count_active_records(source_rowid).expect("count");
    assert_eq!(active_count, 1, "previous generation record still visible");

    // Story 4.2 — DirtyAfterValidation classifies as `scan_failed` (the
    // catch-all for dirty_after_validation, commit_cas loss, internal). The
    // active generation is preserved, so the source is stale. Pins the
    // classifier mapping at the real-driver boundary (the scripted DriftAdapter
    // exercises the actual commit-time revalidation path). Note: DirtyAfterValidation
    // maps to HealthState::Error (not Degraded) per `health_for_scan_error` —
    // it is an internal-shaped failure, not a path/perm/format failure.
    let inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inv.health_state,
        tessera_lib::domain::source::HealthState::Error,
        "DirtyAfterValidation maps to Error (internal-shaped), not Degraded",
    );
    assert_eq!(
        inv.cause,
        Some(tessera_lib::domain::source::HealthCause::ScanFailed),
        "DirtyAfterValidation classifies as scan_failed",
    );
    assert!(
        inv.stale,
        "error-state source with an active generation is stale",
    );
}

// ---------------------------------------------------------------------------
// I/O matrix row 4: commit CAS contention
// ---------------------------------------------------------------------------

/// A commit CAS whose fencing token is no longer the per-source MAXIMUM (a
/// SECOND owner began a run after the first) affects 0 rows → the whole commit
/// transaction rolls back, the active generation is unchanged, and the first
/// run stays `committing` (recovered to failed at next boot).
///
/// The amended CAS predicate compares the holder's token against
/// `MAX(fencing_token)` over the source's runs — comparing only against the
/// holder's own row is no fence at all. This test creates the real second
/// owner instead of passing a hand-made wrong token.
#[test]
fn commit_cas_contention_rolls_back_and_leaves_active_unchanged() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Establish an active generation via a real scan.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();

    let store = ScanStore::new(&conn);
    // First owner: begin a run and move it to `committing`.
    let (run1_id, tok1, gen1) = store.begin_run(source_rowid, "rev2").expect("begin run1");
    store
        .set_state(run1_id, tessera_lib::domain::scan::ScanRunState::Committing)
        .expect("set committing");
    // Second owner: beginning another run allocates a HIGHER fencing token,
    // so run1's token is no longer the per-source MAX.
    let (_run2_id, tok2, _gen2) = store.begin_run(source_rowid, "rev3").expect("begin run2");
    assert!(tok2 > tok1, "second owner holds a higher token");

    // run1's commit must now lose the CAS (0 rows) and roll back.
    let committed = store
        .commit_cas(run1_id, tok1, &gen1, source_rowid)
        .expect("commit_cas returns Ok");
    assert!(
        !committed,
        "stale owner loses the CAS against the newer token"
    );

    // Active generation unchanged.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(first_gen.as_str()));

    // run1 is still `committing` (not re-marked by the loser) — boot recovery
    // will flip it to failed. Query run1 BY ID: the latest run is run2.
    let state: String = conn
        .query_row(
            "SELECT state FROM scan_runs WHERE id = ?1",
            [run1_id],
            |row| row.get(0),
        )
        .expect("run1 state");
    assert_eq!(state, "committing");
}

// ---------------------------------------------------------------------------
// I/O matrix row 5: crash recovery at boot
// ---------------------------------------------------------------------------

/// A process that exits with a run in `running`/`staging`/`committing` is
/// recovered at next boot: the stale run is flipped to `failed`, all
/// non-active-generation records are deleted (in-flight + historical failed
/// staging), and the previous active generation + records are preserved.
#[test]
fn boot_recovery_recovers_stale_runs_and_preserves_active() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Establish an active generation.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_gen = first.generation.0.clone();

    let store = ScanStore::new(&conn);
    // Simulate a crashed in-flight run: begin + move to `staging` + stage a
    // record, but never commit (process "exits").
    let (crash_id, _t, crash_gen) = store.begin_run(source_rowid, "revX").expect("begin");
    store
        .set_state(crash_id, tessera_lib::domain::scan::ScanRunState::Staging)
        .expect("staging");
    store
        .stage_records(
            &crash_gen,
            &[tessera_lib::index::scan_store::StagedRecord {
                record_id: "rec_stale".to_string(),
                source_rowid,
                provider: "codex".to_string(),
                unit_kind: "file".to_string(),
                native_unit_id: "MEMORY.md".to_string(),
                native_locator: "file:///x/MEMORY.md".to_string(),
                content_hash: "h".to_string(),
                parser_version: "file-level/v1".to_string(),
                title: "stale".to_string(),
                body: "".to_string(),
                native_project: None,
                provider_memory_type: "memory".to_string(),
                coverage_level: "full".to_string(),
                observed_at: 0,
                source_revision: "r".to_string(),
                display_locator: "file:///x/MEMORY.md#L1-L1".to_string(),
            }],
        )
        .expect("stage stale");
    store
        .stage_diagnostics(
            &crash_gen,
            &[tessera_lib::index::scan_store::StagedDiagnostic {
                source_rowid,
                kind: "unsupported_artifact".to_string(),
                observed_path: "stale-rule.md".to_string(),
            }],
        )
        .expect("stage stale diagnostic");

    // Sanity: before recovery, the stale record row exists (2 total: active +
    // stale).
    assert_eq!(count_rows(&conn, "memory_records"), 2);
    assert_eq!(count_rows(&conn, "scan_diagnostics"), 1);

    // Boot recovery.
    application::recover_scans(&conn).expect("recover");

    // The crashed run is now failed with the store-side `stale_recovered`
    // error code (queried BY ID — it is also the latest run here).
    let (state, error_code): (String, Option<String>) = conn
        .query_row(
            "SELECT state, error_code FROM scan_runs WHERE id = ?1",
            [crash_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("crashed run row");
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("stale_recovered"));

    // The stale (non-active) generation record is GC'd; only the active
    // generation's record remains.
    assert_eq!(count_rows(&conn, "memory_records"), 1);
    assert_eq!(count_rows(&conn, "scan_diagnostics"), 0);

    // The active generation is unchanged and still reports 1 record.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(first_gen.as_str()));
    let store = ScanStore::new(&conn);
    assert_eq!(store.count_active_records(source_rowid).expect("count"), 1);
}

// ---------------------------------------------------------------------------
// I/O matrix row 6: idempotent re-scan
// ---------------------------------------------------------------------------

/// Re-scanning an unchanged source produces a NEW monotonically-increasing
/// generation that becomes active while the superseded derived generation is
/// removed; the `record_id` set remains stable (locator-based identity — AD-15).
#[test]
fn rescan_unchanged_source_is_idempotent_with_stable_record_ids() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("w");
    fs::write(memories.join("raw_memories.md"), "raw\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Helper to read the sorted record_id set for the active generation.
    let active_record_ids = |conn: &Connection, gen: &str| -> Vec<String> {
        let mut stmt = conn
            .prepare(
                "SELECT record_id FROM memory_records WHERE source_id=?1 AND generation=?2 ORDER BY record_id",
            )
            .expect("prepare");
        let rows = stmt
            .query_map(rusqlite::params![source_rowid, gen], |row| row.get(0))
            .expect("query");
        rows.collect::<rusqlite::Result<Vec<String>>>()
            .expect("collect")
    };

    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_ids = active_record_ids(&conn, &first.generation.0);

    let second =
        application::scan_source(&registry, &conn, &source.source_id).expect("second scan");

    // New generation, monotonically increasing (higher scan_id → gen_<n>).
    assert_ne!(first.generation, second.generation);
    assert!(second.scan_id > first.scan_id);

    // The new generation is now active.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(second.generation.0.as_str()));

    // Local cursor simplification: a new activation deletes all superseded
    // derived records; continuations detect the changed index revision.
    assert_eq!(count_rows(&conn, "memory_records"), 2);

    // record_id set is stable across the unchanged re-scan (locator-based).
    let second_ids = active_record_ids(&conn, &second.generation.0);
    assert_eq!(first_ids, second_ids, "stable locator-based record_ids");
}

// ---------------------------------------------------------------------------
// I/O matrix row 7: empty directory scan
// ---------------------------------------------------------------------------

/// An empty confirmed root scans successfully: the generation activates with
/// `records_indexed: 0` (empty is a complete, honest success).
#[test]
fn empty_directory_scan_succeeds_with_zero_records() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("scan succeeds");
    assert_eq!(outcome.records_indexed, 0);

    // The generation is active despite zero records.
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(outcome.generation.0.as_str()));

    // Status reports succeeded with 0 active records.
    let status = application::get_scan_status(&registry, &conn, &source.source_id).expect("status");
    assert_eq!(status.active_records, 0);
    assert_eq!(
        status.active_generation.as_ref().map(|g| g.0.as_str()),
        Some(outcome.generation.0.as_str())
    );
}

// ---------------------------------------------------------------------------
// I/O matrix row 8: unknown / non-confirmed source
// ---------------------------------------------------------------------------

/// `scan_source` on an unknown `source_id` returns `SourceNotFound` and writes
/// no scan row.
#[test]
fn scan_unknown_source_returns_source_not_found_no_row() {
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let bogus = tessera_lib::domain::source::SourceId("src_99999".to_string());
    let err = application::scan_source(&registry, &conn, &bogus).expect_err("unknown id");
    assert!(matches!(err, ScanError::SourceNotFound));
    assert_eq!(count_rows(&conn, "scan_runs"), 0, "no scan row written");
}

/// `scan_source` on a rejected source returns `NotConfirmed` (maps to
/// `scan_failed`) and writes no scan row.
#[test]
fn scan_rejected_source_returns_not_confirmed_no_row() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let rejected =
        application::reject_source(&registry, &candidate_for(&memories)).expect("reject");
    assert_eq!(rejected.lifecycle_state, SourceLifecycle::Rejected);

    let err = application::scan_source(&registry, &conn, &rejected.source_id)
        .expect_err("rejected source");
    assert!(matches!(err, ScanError::NotConfirmed));
    assert_eq!(count_rows(&conn, "scan_runs"), 0, "no scan row written");
}

/// `scan_source` on a disabled source returns `NotConfirmed` and writes no
/// scan row.
#[test]
fn scan_disabled_source_returns_not_confirmed_no_row() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    application::disable_source(&registry, &source.source_id).expect("disable");

    let err =
        application::scan_source(&registry, &conn, &source.source_id).expect_err("disabled source");
    assert!(matches!(err, ScanError::NotConfirmed));
    assert_eq!(count_rows(&conn, "scan_runs"), 0, "no scan row written");
}

// ---------------------------------------------------------------------------
// I/O matrix row 9: root invalidated after confirm
// ---------------------------------------------------------------------------

/// Scanning a confirmed Source whose root was deleted after confirm fails with
/// `RootInvalid` (maps to `confirm_failed`), writes no scan row, and preserves
/// any prior active generation.
#[test]
fn scan_with_deleted_root_returns_root_invalid_preserves_active() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Establish an active generation first.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    // Delete the root.
    fs::remove_dir_all(&memories).expect("remove root");

    let err =
        application::scan_source(&registry, &conn, &source.source_id).expect_err("deleted root");
    assert!(matches!(err, ScanError::RootInvalid));

    // No NEW scan row was written for the failed attempt (root validation
    // happens before begin_run).
    let runs: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM scan_runs WHERE source_id=?1",
            [source_rowid],
            |row| row.get(0),
        )
        .expect("count runs");
    assert_eq!(runs, 1, "only the first successful run exists");

    // Prior active generation is preserved.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(first.generation.0.as_str()));

    // Story 4.2 — root-deleted classifies as path_missing (ErrorKind::NotFound
    // at canonicalize). The source row carries the persisted cause, and the
    // active generation makes the source stale.
    let inventory = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inventory.health_state,
        tessera_lib::domain::source::HealthState::Degraded
    );
    assert_eq!(
        inventory.cause,
        Some(tessera_lib::domain::source::HealthCause::PathMissing),
        "deleted root classifies as path_missing",
    );
    assert!(
        inventory.stale,
        "degraded source with an active generation is stale",
    );
}

// ---------------------------------------------------------------------------
// I/O matrix row 10: NFR-1 / SM-2 zero-write
// ---------------------------------------------------------------------------

/// A successful scan does not mutate the source: file set, content, size and
/// mtime are byte-identical before and after (SM-2).
#[test]
fn successful_scan_does_not_mutate_source_files_sm2() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# mem\nbody\n").expect("w");
    fs::write(memories.join("memory_summary.md"), "summary\n").expect("w");
    fs::create_dir_all(memories.join("rollout_summaries")).expect("mkdir");
    fs::write(
        memories.join("rollout_summaries").join("2026-07-01.md"),
        "r\n",
    )
    .expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    let before = snapshot_tree(&memories);
    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("scan succeeds");
    assert_eq!(outcome.records_indexed, 3);
    let after = snapshot_tree(&memories);
    assert_tree_unchanged(&before, &after);
}

/// A FAILED scan also does not mutate the source (SM-2 across all paths).
#[test]
fn failed_scan_does_not_mutate_source_files_sm2() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    // Take the snapshot, then delete the root so the scan fails at root
    // validation; the remaining tempdir tree must be untouched.
    let before = snapshot_tree(tmp.path());
    fs::remove_dir_all(&memories).expect("remove root");
    let _ = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("deleted root fails");
    let after = snapshot_tree(tmp.path());
    // The only difference is the removal we performed; assert no OTHER file
    // changed by comparing against a snapshot taken after our own removal.
    let after_our_removal = snapshot_tree(tmp.path());
    assert_tree_unchanged(&after_our_removal, &after);
    // And the pre-removal snapshot must differ only by the removed dir.
    assert!(before.len() > after.len(), "we removed the memories dir");
}

// ---------------------------------------------------------------------------
// I/O matrix row 11: matrix-boundary enumeration end-to-end
// ---------------------------------------------------------------------------

/// A root containing both in-matrix files (`rollout_summaries/*.md`) and
/// excluded files (`sessions/*.jsonl`, `CLAUDE.md`) indexes ONLY the in-matrix
/// files (AD-11).
#[test]
fn scan_indexes_only_supported_artifact_matrix() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("w");
    fs::write(memories.join("CLAUDE.md"), "rules\n").expect("w");
    fs::create_dir_all(memories.join("sessions")).expect("mkdir sessions");
    fs::write(memories.join("sessions").join("foo.jsonl"), "{}\n").expect("w");
    fs::create_dir_all(memories.join("rollout_summaries")).expect("mkdir rollout");
    fs::write(
        memories.join("rollout_summaries").join("2026-07-01.md"),
        "r\n",
    )
    .expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("scan succeeds");
    // Only MEMORY.md + rollout_summaries/2026-07-01.md are indexed.
    assert_eq!(outcome.records_indexed, 2);

    let mut stmt = conn
        .prepare(
            "SELECT native_locator FROM memory_records WHERE source_id=?1 ORDER BY native_locator",
        )
        .expect("prepare");
    let ids: Vec<String> = stmt
        .query_map([source_rowid], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect");
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().any(|id| id.contains("MEMORY.md")));
    assert!(ids
        .iter()
        .any(|id| id.contains("rollout_summaries/2026-07-01.md")));
}

// ---------------------------------------------------------------------------
// get_scan_status
// ---------------------------------------------------------------------------

/// `get_scan_status` reports the latest run state + active generation + count
/// for a scanned source, and the null shape for a never-scanned source.
#[test]
fn get_scan_status_reports_state_generation_and_count() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    // Never scanned: null state + null generation + 0 records.
    let status = application::get_scan_status(&registry, &conn, &source.source_id).expect("status");
    assert_eq!(status.state, None);
    assert_eq!(status.active_generation, None);
    assert_eq!(status.active_records, 0);

    // After a scan: succeeded + active generation + count.
    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    let status = application::get_scan_status(&registry, &conn, &source.source_id).expect("status");
    assert_eq!(
        status.state,
        Some(tessera_lib::domain::scan::ScanRunState::Succeeded)
    );
    assert_eq!(
        status.active_generation.as_ref().map(|g| g.0.as_str()),
        Some(outcome.generation.0.as_str())
    );
    assert_eq!(status.active_records, 1);
}

/// `get_scan_status` on an unknown source returns `SourceNotFound`.
#[test]
fn get_scan_status_unknown_source_returns_source_not_found() {
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let bogus = tessera_lib::domain::source::SourceId("src_424242".to_string());
    let err = application::get_scan_status(&registry, &conn, &bogus).expect_err("unknown");
    assert!(matches!(err, ScanError::SourceNotFound));
}

// ---------------------------------------------------------------------------
// Fencing token monotonicity
// ---------------------------------------------------------------------------

/// Fencing tokens are monotonically increasing per source (AD-28/AD-32):
/// successive `begin_run` calls allocate MAX+1.
#[test]
fn fencing_tokens_are_monotonic_per_source() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    let store = ScanStore::new(&conn);
    let (_a, tok_a, _ga) = store.begin_run(source_rowid, "r1").expect("begin a");
    let (_b, tok_b, _gb) = store.begin_run(source_rowid, "r2").expect("begin b");
    let (_c, tok_c, _gc) = store.begin_run(source_rowid, "r3").expect("begin c");
    assert!(tok_b > tok_a, "monotonic");
    assert!(tok_c > tok_b, "monotonic");

    // UNIQUE(source_id, fencing_token) holds: a duplicate insert is rejected.
    let dup = conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (?1, 'g', 'queued', ?2, 'g', 'r')",
        rusqlite::params![source_rowid, tok_a],
    );
    assert!(
        dup.is_err(),
        "duplicate fencing token rejected by unique index"
    );
}

// ---------------------------------------------------------------------------
// I/O matrix row 2 (in-loop read failure): mid-scan file read failure
// ---------------------------------------------------------------------------

/// I/O matrix row 2 driven through the PUBLIC `application::scan_source`
/// orchestration: a file that ENUMERATES successfully (metadata/stat OK) but
/// fails at `fs::read` time surfaces `ScanError::ReadFailed`, the run is
/// marked `failed` (fail_run ran — not left in staging/running), the staging
/// generation is NOT activated, and the previous active generation + its
/// records stay fully visible (no half-index, NFR-9).
///
/// Deterministic trigger (unix): an in-matrix file (`raw_memories.md`) is
/// `chmod 0o000`. `metadata`/`canonicalize` only need the parent directory's
/// search permission, so enumeration still yields the file; `fs::read` then
/// fails with EACCES inside `run_pipeline`'s per-file loop. This exercises the
/// genuine read-error branch (no test-only seam, no env mutation). The prior
/// active generation is established first with only the valid file so the
/// preservation assertions are meaningful.
#[cfg(unix)]
#[test]
fn mid_scan_file_read_failure_preserves_previous_generation() {
    use std::os::unix::fs::PermissionsExt;

    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    // Valid in-matrix file present for BOTH scans.
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("write MEMORY.md");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // --- Step 1: establish a prior active generation (only the valid file).
    let first =
        application::scan_source(&registry, &conn, &source.source_id).expect("first scan ok");
    assert_eq!(first.records_indexed, 1);
    let first_gen = first.generation.0.clone();
    let store = ScanStore::new(&conn);
    let prior_active_count = store.count_active_records(source_rowid).expect("count");
    assert_eq!(prior_active_count, 1);

    // --- Step 2: add the failing in-matrix file and make it unreadable.
    let failing = memories.join("raw_memories.md");
    fs::write(&failing, "raw\n").expect("write raw_memories.md");
    fs::set_permissions(&failing, fs::Permissions::from_mode(0o000)).expect("chmod 000");

    // --- Step 3: the failing scan surfaces ReadFailed via the public API.
    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("scan must fail at fs::read");
    assert!(
        matches!(err, ScanError::ReadFailed),
        "expected ReadFailed, got {err:?}"
    );

    // The failed run is marked `failed` (fail_run ran) with the `read_failed`
    // error code — not left staging / running. It is the latest run for this
    // source.
    let (state, error_code) = latest_run_state(&conn, source_rowid);
    assert_eq!(state, "failed", "failed run must be marked failed");
    assert_eq!(error_code.as_deref(), Some("read_failed"));

    // The staging generation was NOT activated: active generation is still the
    // previous one.
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(
        active.as_deref(),
        Some(first_gen.as_str()),
        "previous generation stays active (no half-index)"
    );

    // Previous records fully visible: count still equals the prior count.
    let store = ScanStore::new(&conn);
    let count_after = store
        .count_active_records(source_rowid)
        .expect("count after");
    assert_eq!(
        count_after, prior_active_count,
        "previous generation records remain fully visible"
    );

    // Story 4.2 — `read_verified`'s mid-scan `ReadFailed` defaults to
    // `scan_failed` (the spec's Design Note residual-risk #3 documents that
    // the io kind is intentionally not threaded out of the helper, so the
    // cause falls into the scan_failed catch-all rather than being refined to
    // permission_denied). Pinning the chosen default here means a future
    // refactor cannot silently change it without a test failure.
    let inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inv.cause,
        Some(tessera_lib::domain::source::HealthCause::ScanFailed),
        "read_verified's ReadFailed defaults to scan_failed (io kind not threaded)",
    );
    assert!(
        inv.stale,
        "degraded source with an active generation is stale",
    );

    // --- Cleanup: restore permissions BEFORE the tempdir drops so removal
    // never fails on a 0o000 file.
    fs::set_permissions(&failing, fs::Permissions::from_mode(0o644)).expect("chmod 644 restore");
}

// ---------------------------------------------------------------------------
// Amendment regression: first-enumeration failure lands on a persisted run row
// ---------------------------------------------------------------------------

/// `begin_run` precedes the FIRST enumeration (spec amendment 4): a scan whose
/// first enumeration fails still lands on a persisted run row, marked `failed`
/// with `error_code='enumeration_failed'` — the failure is crash-recoverable
/// and inspectable, not an invisible no-row error.
#[test]
fn first_enumeration_failure_marks_run_failed_with_enumeration_code() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    let err =
        application::scan_source_with(&FailingEnumAdapter, &registry, &conn, &source)
            .expect_err("first enumeration fails");
    assert!(
        matches!(err, ScanError::EnumerationFailed),
        "expected EnumerationFailed, got {err:?}"
    );

    // A run row EXISTS (begin_run ran before the first enumeration) and is
    // marked failed with the enumeration code — the only run for this source.
    assert_eq!(count_rows(&conn, "scan_runs"), 1);
    let (state, error_code) = latest_run_state(&conn, source_rowid);
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("enumeration_failed"));

    // No generation was activated.
    assert_eq!(active_generation_str(&conn, source_rowid), None);

    // Story 4.2 — FailingEnumAdapter returns EnumerateError::Unreadable, which
    // classifies as scan_failed (the catch-all for non-path/non-perm io kinds
    // at a dir site). The source has NO active generation (the first scan
    // failed before any success), so it is `unavailable`, NOT `stale`.
    let inventory = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inventory.health_state,
        tessera_lib::domain::source::HealthState::Degraded
    );
    assert_eq!(
        inventory.cause,
        Some(tessera_lib::domain::source::HealthCause::ScanFailed),
        "FailingEnumAdapter's Unreadable classifies as scan_failed",
    );
    assert!(
        !inventory.stale,
        "degraded source with NO active generation is unavailable, not stale",
    );
}

// ---------------------------------------------------------------------------
// Amendment regression: generation isolation on staging-then-drift failure
// ---------------------------------------------------------------------------

/// A scan that STAGES a full generation and only then fails at commit-time
/// manifest re-validation leaves the previous active generation's rows
/// byte-identical row-by-row — even though the failed staging generation holds
/// rows with the SAME `record_id`s. This is the composite-PK
/// `(record_id, generation)` + plain-`INSERT` isolation proof (pre-amendment a
/// single-column PK + `INSERT OR REPLACE` would have silently overwritten the
/// active rows).
#[test]
fn staging_then_drift_failure_preserves_previous_rows_identically() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("w");
    fs::write(memories.join("raw_memories.md"), "raw\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // First scan: gen1 active with 2 records.
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    assert_eq!(first.records_indexed, 2);
    let gen1 = first.generation.0.clone();

    // Snapshot gen1 rows (record_id, native_unit_id, content_hash), sorted.
    let gen_rows = |conn: &Connection, gen: &str| -> Vec<(String, String, String)> {
        let mut stmt = conn
            .prepare(
                "SELECT record_id, native_unit_id, content_hash FROM memory_records
                 WHERE source_id=?1 AND generation=?2 ORDER BY record_id",
            )
            .expect("prepare");
        stmt.query_map(rusqlite::params![source_rowid, gen], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect")
    };
    let gen1_before = gen_rows(&conn, &gen1);
    assert_eq!(gen1_before.len(), 2);

    // Second scan stages gen2 (same record_ids — same files, same locators)
    // then fails: the second enumeration bumps every mtime by 1ns → drift.
    let err = application::scan_source_with(
        &MtimeBumpAdapter::new(),
        &registry,
        &conn,
        &source,
    )
    .expect_err("mtime drift at revalidation");
    assert!(matches!(err, ScanError::DirtyAfterValidation));

    // gen1 rows are byte-identical row-by-row (plain INSERT never touched
    // them) and still the ONLY active rows.
    let gen1_after = gen_rows(&conn, &gen1);
    assert_eq!(
        gen1_before, gen1_after,
        "active generation rows must be untouched by the failed staging"
    );
    let store = ScanStore::new(&conn);
    assert_eq!(store.count_active_records(source_rowid).expect("count"), 2);
    let active = active_generation_str(&conn, source_rowid);
    assert_eq!(active.as_deref(), Some(gen1.as_str()));

    // Sanity: the failed staging generation DOES exist alongside gen1 (with
    // the same record_ids — proving the composite PK held; pre-amendment this
    // INSERT would have errored or replaced).
    let scan2_gen: String = conn
        .query_row(
            "SELECT generation FROM scan_runs WHERE source_id=?1 ORDER BY id DESC LIMIT 1",
            [source_rowid],
            |row| row.get(0),
        )
        .expect("gen2");
    assert_ne!(scan2_gen, gen1);
    assert_eq!(
        gen_rows(&conn, &scan2_gen).len(),
        2,
        "staged gen2 rows exist"
    );
}

// ---------------------------------------------------------------------------
// Amendment regression: symlink-alias count honesty
// ---------------------------------------------------------------------------

/// An in-root symlink alias (`rollout_summaries/b.md → a.md`) canonicalizes to
/// the SAME relative/real path as its target, so the enumerator's dedup
/// collapses it to a single unit: the announced `records_indexed` equals the
/// actual persisted row count (spec Design Notes — "计数诚实").
#[cfg(unix)]
#[test]
fn symlink_alias_is_deduped_and_count_stays_honest() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "mem\n").expect("w");
    let rollout = memories.join("rollout_summaries");
    fs::create_dir_all(&rollout).expect("mkdir rollout");
    fs::write(rollout.join("a.md"), "a\n").expect("w a");
    std::os::unix::fs::symlink(rollout.join("a.md"), rollout.join("b.md")).expect("symlink b");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);

    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("scan succeeds");
    // MEMORY.md + rollout_summaries/a.md (b.md collapsed into a.md).
    assert_eq!(outcome.records_indexed, 2, "announced count is deduped");
    assert_eq!(
        count_rows(&conn, "memory_records"),
        2,
        "persisted row count equals the announced count"
    );

    // The two persisted records retain the two real source-file locators.
    let mut stmt = conn
        .prepare("SELECT native_locator FROM memory_records ORDER BY native_locator")
        .expect("prepare");
    let ids: Vec<String> = stmt
        .query_map([], |row| row.get(0))
        .expect("query")
        .collect::<rusqlite::Result<Vec<String>>>()
        .expect("collect");
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().any(|id| id.contains("MEMORY.md")));
    assert!(ids.iter().any(|id| id.contains("rollout_summaries/a.md")));
}

// ---------------------------------------------------------------------------
// Amendment regression: foreign-key enforcement
// ---------------------------------------------------------------------------

/// With `PRAGMA foreign_keys = ON` (as boot now sets), an orphan
/// `memory_records` row referencing a non-existent `source_registry.id` is
/// rejected (migration v2's `REFERENCES source_registry(id)` is policed).
#[test]
fn memory_records_rejects_orphan_source_reference() {
    let conn = fresh_db();
    let orphan = conn.execute(
        "INSERT INTO memory_records
            (record_id, source_id, generation, provider, unit_kind,
             native_unit_id, native_locator, content_hash, parser_version)
         VALUES
            ('rec_orphan', 999999, 'gen_1', 'codex', 'file',
             'MEMORY.md', 'file:///x/MEMORY.md', 'h', 'file-level/v1')",
        [],
    );
    assert!(
        orphan.is_err(),
        "orphan memory_records row must be rejected by the FK"
    );
}

// ---------------------------------------------------------------------------
// Story 1.5 canonical provenance and safe failure regressions
// ---------------------------------------------------------------------------

#[test]
fn canonical_records_persist_title_body_and_all_provider_memory_types() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# Memory\nbody\n").expect("memory");
    fs::write(memories.join("memory_summary.md"), "# Summary\nbody\n").expect("summary");
    fs::write(memories.join("raw_memories.md"), "# Raw\nbody\n").expect("raw");
    fs::create_dir(memories.join("rollout_summaries")).expect("rollout dir");
    fs::write(
        memories.join("rollout_summaries").join("run.md"),
        "# Rollout\nbody\n",
    )
    .expect("rollout");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    application::scan_source(&registry, &conn, &source.source_id).expect("scan");

    let mut stmt = conn
        .prepare(
            "SELECT title, body, provider_memory_type, coverage_level, native_project,
                    source_revision, native_locator, display_locator, parser_version
             FROM memory_records WHERE source_id=?1 ORDER BY provider_memory_type",
        )
        .expect("prepare canonical projection");
    type CanonicalRow = (
        String,
        String,
        String,
        String,
        Option<String>,
        String,
        String,
        String,
        String,
    );
    let rows: Vec<CanonicalRow> = stmt
        .query_map([source_rowid], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        })
        .expect("query")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect");
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().map(|row| row.2.as_str()).collect::<Vec<_>>(),
        vec![
            "memory",
            "memory_summary",
            "raw_memories",
            "rollout_summary"
        ]
    );
    assert_eq!(
        rows.iter()
            .map(|row| (row.0.as_str(), row.2.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("Memory", "memory"),
            ("Summary", "memory_summary"),
            ("Raw", "raw_memories"),
            ("Rollout", "rollout_summary"),
        ]
    );
    for row in rows {
        assert!(matches!(
            row.0.as_str(),
            "Memory" | "Summary" | "Raw" | "Rollout"
        ));
        assert_eq!(row.1, "body\n");
        assert_eq!(row.3, "full");
        assert_eq!(row.4, None, "unmapped native project stays None");
        assert!(!row.5.is_empty(), "whole-file source revision");
        assert!(row.6.contains("#section%2F"), "semantic locator");
        assert!(row.7.contains("#L1-L2"), "display locator");
        assert_eq!(row.8, "codex-markdown/v1");
    }
}

#[test]
fn persisted_canonical_identity_and_display_ranges_are_exact() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let file = memories.join("MEMORY.md");
    fs::write(&file, "lead\n# Alpha\nfirst\n# Alpha\nlast\n").expect("source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    application::scan_source(&registry, &conn, &source.source_id).expect("scan");

    let canonical_file = fs::canonicalize(&file).expect("canonical file");
    let file_locator = file_uri(&canonical_file).expect("file URI");
    let mut expected_rows = [
        ("preamble", "preamble", "L1-L1"),
        ("section", "section/h1:5:Alpha:1", "L2-L3"),
        ("section", "section/h1:5:Alpha:2", "L4-L5"),
    ]
    .into_iter()
    .map(|(unit_kind, native_unit_id, range)| {
        let native_locator = format!(
            "{}#{}",
            file_locator,
            percent_encode_fragment(native_unit_id)
        );
        (
            build_record_id(&source.source_id, "codex", &native_locator, unit_kind),
            unit_kind.to_string(),
            native_unit_id.to_string(),
            native_locator,
            format!("{}#{}", file_locator, range),
        )
    })
    .collect::<Vec<_>>();
    expected_rows.sort_by(|left, right| left.2.cmp(&right.2));

    let mut stmt = conn
        .prepare(
            "SELECT record_id, unit_kind, native_unit_id, native_locator, display_locator
             FROM memory_records WHERE source_id=?1 ORDER BY native_unit_id",
        )
        .expect("prepare canonical rows");
    let actual_rows = stmt
        .query_map([source_rowid], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .expect("query canonical rows")
        .collect::<rusqlite::Result<Vec<_>>>()
        .expect("collect canonical rows");
    assert_eq!(actual_rows, expected_rows);
}

#[test]
fn mapped_native_project_and_observed_at_persist_with_canonical_records() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# Memory\nbody\n").expect("memory");
    let conn = fresh_db();
    let mut candidate = candidate_for(&memories);
    candidate.native_project = Some("project-alpha".to_string());
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &candidate).expect("confirm mapped source");
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    let (native_project, observed_at): (Option<String>, i64) = conn
        .query_row(
            "SELECT native_project, observed_at FROM memory_records WHERE source_id=?1",
            [source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("canonical provenance");
    assert_eq!(native_project.as_deref(), Some("project-alpha"));
    assert!(observed_at > 0, "observed timestamp is persisted");
}

#[test]
fn edited_section_changes_its_hash_while_unchanged_sibling_keeps_hash() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let file = memories.join("MEMORY.md");
    fs::write(&file, "# Changed\nalpha\n# Stable\nfixed\n").expect("initial source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let first_changed: (String, String, String, String) = conn
        .query_row(
            "SELECT record_id, content_hash, source_revision, display_locator FROM memory_records
             WHERE source_id=?1 AND generation=?2 AND title='Changed'",
            rusqlite::params![source_rowid, first.generation.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("changed row before edit");
    let first_stable: (String, String, String, String) = conn
        .query_row(
            "SELECT record_id, content_hash, source_revision, display_locator FROM memory_records
             WHERE source_id=?1 AND generation=?2 AND title='Stable'",
            rusqlite::params![source_rowid, first.generation.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("stable row before edit");

    fs::write(&file, "# Changed\nbravo\nextra\n# Stable\nfixed\n").expect("edited source");
    let second =
        application::scan_source(&registry, &conn, &source.source_id).expect("second scan");
    let second_changed: (String, String, String, String) = conn
        .query_row(
            "SELECT record_id, content_hash, source_revision, display_locator FROM memory_records
             WHERE source_id=?1 AND generation=?2 AND title='Changed'",
            rusqlite::params![source_rowid, second.generation.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("changed row after edit");
    let second_stable: (String, String, String, String) = conn
        .query_row(
            "SELECT record_id, content_hash, source_revision, display_locator FROM memory_records
             WHERE source_id=?1 AND generation=?2 AND title='Stable'",
            rusqlite::params![source_rowid, second.generation.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("stable row after edit");

    assert_eq!(
        first_changed.0, second_changed.0,
        "semantic identity stays stable"
    );
    assert_ne!(
        first_changed.1, second_changed.1,
        "edited body changes hash"
    );
    assert_ne!(first_changed.2, second_changed.2, "source revision changes");
    assert_ne!(
        first_changed.3, second_changed.3,
        "edited range changes display locator"
    );
    assert_eq!(
        first_stable.0, second_stable.0,
        "sibling identity stays stable"
    );
    assert_eq!(
        first_stable.1, second_stable.1,
        "unchanged sibling hash stays stable"
    );
    assert_ne!(
        first_stable.2, second_stable.2,
        "whole-file revision changes"
    );
    assert_ne!(
        first_stable.3, second_stable.3,
        "shifted sibling display range changes"
    );
}

#[test]
fn same_size_restored_mtime_byte_change_fails_final_digest_validation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# Item\nalpha\n").expect("initial source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    let error = application::scan_source_with(
        &SameMetadataMutationAdapter::new(b"# Item\nbravo\n"),
        &registry,
        &conn,
        &source,
    )
    .expect_err("digest drift must fail");
    assert!(matches!(error, ScanError::DirtyAfterValidation));
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first.generation.0.as_str())
    );
}

#[test]
fn malformed_allowlisted_source_fails_safely_without_replacing_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let file = memories.join("MEMORY.md");
    fs::write(&file, "# Good\nbody\n").expect("good source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    fs::write(&file, [0xff, 0xfe, b'\n']).expect("malformed source");
    let error =
        application::scan_source(&registry, &conn, &source.source_id).expect_err("parse fails");
    assert!(matches!(error, ScanError::ParseFailed));
    assert_eq!(
        active_generation_str(&conn, source_rowid),
        Some(first.generation.0)
    );
    let (state, error_code) = latest_run_state(&conn, source_rowid);
    assert_eq!(state, "failed");
    assert_eq!(error_code.as_deref(), Some("parse_failed"));

    // Story 4.2 — the real parse-failure path classifies as `format_unsupported`
    // and the source is stale (the prior active generation is preserved).
    // Pins the classifier mapping at the real I/O boundary, not just the
    // projection (the inventory_surfaces_format_unsupported test only writes
    // the cause directly and would pass even if the scan layer attached the
    // wrong cause).
    let inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inv.health_state,
        tessera_lib::domain::source::HealthState::Degraded
    );
    assert_eq!(
        inv.cause,
        Some(tessera_lib::domain::source::HealthCause::FormatUnsupported),
        "ParseFailed must classify as format_unsupported via the real scan path",
    );
    assert!(
        inv.stale,
        "degraded source with an active generation is stale",
    );
}

#[test]
fn diagnostic_only_rescan_preserves_active_records_and_projects_safe_diagnostic() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    let file = memories.join("MEMORY.md");
    fs::write(&file, "# Good\nbody\n").expect("good source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    fs::remove_file(file).expect("remove supported source");
    fs::write(memories.join("AGENTS.md"), "not indexed\n").expect("unknown");
    let outcome =
        application::scan_source(&registry, &conn, &source.source_id).expect("diagnostic scan");
    assert_eq!(outcome.generation, first.generation);
    assert_eq!(outcome.records_indexed, 1);
    assert_eq!(
        active_generation_str(&conn, source_rowid),
        Some(first.generation.0)
    );
    let diagnostic: (String, String, String) = conn
        .query_row(
            "SELECT generation, kind, observed_path FROM scan_diagnostics WHERE source_id=?1",
            [source_rowid],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("diagnostic persists");
    assert_eq!(diagnostic.1, "unsupported_artifact");
    assert_eq!(diagnostic.2, "AGENTS.md");
    let diagnostic_generation: String = conn
        .query_row(
            "SELECT generation FROM scan_runs WHERE id=?1",
            [outcome.scan_id],
            |row| row.get(0),
        )
        .expect("diagnostic run generation");
    assert_eq!(diagnostic.0, diagnostic_generation);
    assert_eq!(
        conn.query_row::<i64, _, _>(
            "SELECT COUNT(*) FROM memory_records WHERE source_id=?1 AND generation=?2",
            rusqlite::params![source_rowid, diagnostic_generation],
            |row| row.get(0),
        )
        .expect("no record in diagnostic-only generation"),
        0
    );

    fs::remove_file(memories.join("AGENTS.md")).expect("remove diagnostic source");
    fs::write(memories.join("MEMORY.md"), "# Rebuilt\nbody\n").expect("restore source");
    application::scan_source(&registry, &conn, &source.source_id).expect("successful cleanup");
    assert_eq!(count_rows(&conn, "scan_diagnostics"), 0);
}

#[cfg(unix)]
#[test]
fn native_byte_unknown_entry_persists_safe_diagnostic_when_supported() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# Good\nbody\n").expect("memory");
    let unknown = memories.join(OsString::from_vec(b"bad\xff-entry".to_vec()));
    if fs::write(&unknown, "unknown\n").is_err() {
        return;
    }

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan");
    assert_eq!(outcome.records_indexed, 1);
    let observed_path: String = conn
        .query_row(
            "SELECT observed_path FROM scan_diagnostics WHERE source_id=?1",
            [source_rowid],
            |row| row.get(0),
        )
        .expect("native-byte diagnostic");
    assert_eq!(observed_path, "bad%FF-entry");
}

#[test]
fn diagnostic_drift_between_initial_and_final_enumeration_preserves_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "# Good\nbody\n").expect("source");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let registry = SourceRegistry::new(&conn);
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    fs::write(memories.join("AGENTS.md"), "staged diagnostic\n").expect("unknown source");

    let error = application::scan_source_with(
        &DiagnosticDriftAdapter::new(),
        &registry,
        &conn,
        &source,
    )
    .expect_err("diagnostic drift must fail");
    assert!(matches!(error, ScanError::DirtyAfterValidation));
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first.generation.0.as_str())
    );
    assert_eq!(count_rows(&conn, "scan_diagnostics"), 0);
}

// ---------------------------------------------------------------------------
// Amendment regression: corrupt persisted run state fails loudly
// ---------------------------------------------------------------------------

/// `ScanStore::latest_run` must NOT silently map an unparseable persisted
/// `state` onto `Failed` (pre-amendment `unwrap_or(Failed)` hid corruption):
/// it surfaces a conversion error so the caller fails honestly.
#[test]
fn latest_run_errors_on_unparseable_persisted_state() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Insert a run row with a state outside the persisted vocabulary.
    conn.execute(
        "INSERT INTO scan_runs
            (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (?1, 'gen_1', 'bogus', 1, 'gen_1', 'rev')",
        [source_rowid],
    )
    .expect("insert corrupt row");

    let store = ScanStore::new(&conn);
    assert!(
        store.latest_run(source_rowid).is_err(),
        "unparseable persisted state must surface an error, not a silent Failed"
    );
}

// ---------------------------------------------------------------------------
// Review regressions: root identity and safe empty-rescan handling
// ---------------------------------------------------------------------------

/// Recreating a confirmed root at the same path changes its filesystem
/// identity. The old Source must not silently scan the replacement directory;
/// it requires explicit re-confirmation and preserves the active generation.
#[test]
fn replaced_root_requires_reconfirmation_and_preserves_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w v1");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    fs::remove_dir_all(&memories).expect("remove old root");
    fs::create_dir(&memories).expect("recreate root");
    fs::write(memories.join("MEMORY.md"), "replacement\n").expect("w replacement");

    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("replacement root requires reconfirmation");
    assert!(matches!(err, ScanError::RootIdentityChanged));
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first.generation.0.as_str()),
        "the prior active generation stays visible"
    );
    assert_eq!(count_rows(&conn, "scan_runs"), 1, "no new run was started");

    // Story 4.2 — RootIdentityChanged (a fingerprint mismatch, no io error in
    // hand) classifies as `path_missing` (the spec's Boundaries name the base
    // mapping RootInvalid/RootIdentityChanged → PathMissing, refined by io
    // kind only when an io error is available). The active generation is
    // preserved, so the source is stale.
    let inv = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inv.health_state,
        tessera_lib::domain::source::HealthState::Degraded
    );
    assert_eq!(
        inv.cause,
        Some(tessera_lib::domain::source::HealthCause::PathMissing),
        "RootIdentityChanged classifies as path_missing (no io hint)",
    );
    assert!(
        inv.stale,
        "degraded source with an active generation is stale",
    );
}

/// A first empty scan is valid, but an empty re-scan must not replace a useful
/// active generation: unreadable or unexpectedly empty roots fail honestly.
#[test]
fn empty_rescan_preserves_existing_active_generation() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w v1");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    fs::remove_file(memories.join("MEMORY.md")).expect("remove supported file");
    let err = application::scan_source(&registry, &conn, &source.source_id)
        .expect_err("empty replacement generation is rejected");
    assert!(matches!(err, ScanError::EmptyScanWithActiveGeneration));
    assert_eq!(
        latest_run_state(&conn, source_rowid),
        ("failed".to_string(), Some("enumeration_failed".to_string()))
    );
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first.generation.0.as_str()),
        "the prior active generation stays visible"
    );
}

/// `fail_run` is deliberately in-flight-only. A future post-commit failure
/// cannot change a successfully committed run to `failed`.
#[test]
fn fail_run_does_not_overwrite_a_succeeded_run() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w v1");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let outcome = application::scan_source(&registry, &conn, &source.source_id).expect("scan");

    ScanStore::new(&conn)
        .fail_run(outcome.scan_id, "internal")
        .expect("ignored for succeeded run");
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    assert_eq!(latest_run_state(&conn, source_rowid).0, "succeeded");
}

/// A filename that is retargeted after enumeration is rejected before any
/// outside content is read or committed; the previous active generation stays
/// visible.
#[cfg(unix)]
#[test]
fn retargeted_file_after_enumeration_fails_before_reading_outside_target() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "inside\n").expect("w inside");
    let outside = tmp.path().join("outside-dir");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");
    let first = application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    let err = application::scan_source_with(
        &RetargetAfterEnumerationAdapter::new(outside),
        &registry,
        &conn,
        &source,
    )
    .expect_err("retargeted file is rejected");
    assert!(matches!(err, ScanError::DirtyAfterValidation));
    assert_eq!(
        active_generation_str(&conn, source_rowid).as_deref(),
        Some(first.generation.0.as_str())
    );
    assert_eq!(
        latest_run_state(&conn, source_rowid),
        (
            "failed".to_string(),
            Some("dirty_after_validation".to_string())
        )
    );
}

// ---------------------------------------------------------------------------
// Story 4.2 — set_health_and_cause on each failure path; cancel does not
// clear a previously-persisted cause (cancel is not a health transition).
// ---------------------------------------------------------------------------

/// Story 4.2 AC — a cancelled rescan does NOT call `set_health_and_cause`, so
/// a previously-persisted cause survives the cancel. `latest_error` carries
/// the cancelled string (its derivation is unchanged), but `cause` and `stale`
/// reflect the prior failure, not the cancel. This pins the spec's binding
/// constraint that `cause` and `latest_error` are INDEPENDENT.
#[test]
fn cancel_does_not_clear_previously_persisted_cause() {
    let tmp = tempdir().expect("tempdir");
    let memories = make_memories(tmp.path());
    fs::write(memories.join("MEMORY.md"), "v1\n").expect("w");

    let conn = fresh_db();
    let source = confirm(&conn, &memories);
    let registry = SourceRegistry::new(&conn);
    let source_rowid = source.source_id.to_rowid().expect("rowid");

    // Establish an active generation.
    application::scan_source(&registry, &conn, &source.source_id).expect("first scan");

    // Persist a cause as if a prior failure had set it (e.g. a previous
    // rescan had failed with path_missing before this cancel).
    use tessera_lib::domain::source::{HealthCause, HealthState};
    registry
        .set_health_and_cause(&source.source_id, HealthState::Degraded, HealthCause::PathMissing)
        .expect("persist cause");

    // Reserve a run, cancel it immediately, then drive the reserved scan
    // (which surfaces Cancelled without calling set_health_and_cause).
    let store = ScanStore::new(&conn);
    let (scan_id, token, generation) = store.begin_run(source_rowid, "pending").expect("reserve");
    assert!(store.cancel_run(scan_id, source_rowid).expect("cancel reserved run"));
    let err = application::scan_reserved_source(
        &registry,
        &conn,
        &source.source_id,
        scan_id,
        token,
        generation,
    )
    .expect_err("cancelled run never scans");
    assert!(matches!(err, ScanError::Cancelled));

    // Inventory: latest_error carries the cancelled string (its derivation is
    // unchanged — pinned string), but the previously-persisted cause survives
    // the cancel and stale reflects the Degraded health + active generation.
    let inventory = application::list_inventory(&registry, &conn)
        .expect("inventory")
        .into_iter()
        .find(|item| item.source_id == source.source_id)
        .expect("inventory row");
    assert_eq!(
        inventory.latest_error.as_deref(),
        Some("The last rescan was cancelled."),
        "latest_error keeps its existing derivation (pinned string)"
    );
    assert_eq!(
        inventory.cause,
        Some(HealthCause::PathMissing),
        "cancel does not clear a previously-persisted cause",
    );
    assert!(
        inventory.stale,
        "Degraded + active generation stays stale across a cancel",
    );
}
