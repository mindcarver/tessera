use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::adapters::codex::file_uri;
use tessera_lib::application;
use tessera_lib::application::OpenError;
use tessera_lib::domain::open::OpenRequest;
use tessera_lib::index::migrations;

static OPEN_TEST_LOCK: Mutex<()> = Mutex::new(());
static OPENED_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn capture_open(path: &Path) -> io::Result<()> {
    *OPENED_PATH.lock().expect("opened path lock") = Some(path.to_path_buf());
    Ok(())
}

fn fail_open(_: &Path) -> io::Result<()> {
    Err(io::Error::other("opener unavailable"))
}

fn reset_open_state() {
    *OPENED_PATH.lock().expect("opened path lock") = None;
    application::reset_open_path_for_tests();
}

fn db(root: &Path, locator: &str, lifecycle: &str) -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    let root = std::fs::canonicalize(root).unwrap();
    let root = root.to_string_lossy().into_owned();
    conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', ?1, 'unknown', 'full', ?2, 'fixture', NULL)", params![lifecycle, root]).unwrap();
    conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')", []).unwrap();
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
        [],
    )
    .unwrap();
    conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES ('rec_a', 1, 'gen_1', 'codex', 'section', 'rec_a', ?1, 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 1, 'revision', ?2)", params![locator, format!("{locator}#L1-L2")]).unwrap();
    conn
}

#[test]
fn open_active_confirmed_record_invokes_configured_opener() {
    let _guard = OPEN_TEST_LOCK.lock().expect("test lock");
    reset_open_state();
    application::set_open_path_for_tests(capture_open);
    let root = tempdir().unwrap();
    let memory = root.path().join("MEMORY.md");
    std::fs::write(&memory, "memory").unwrap();
    let locator = format!("{}#unit", file_uri(&memory).unwrap());
    let conn = db(root.path(), &locator, "confirmed");

    let result =
        application::open_original_location(&conn, OpenRequest::new("rec_a".into()).unwrap())
            .unwrap();

    assert_eq!(result.record_id(), "rec_a");
    assert_eq!(
        OPENED_PATH.lock().unwrap().as_ref().unwrap(),
        &std::fs::canonicalize(memory).unwrap()
    );
    reset_open_state();
}

#[test]
fn missing_and_non_confirmed_records_do_not_open() {
    let _guard = OPEN_TEST_LOCK.lock().expect("test lock");
    reset_open_state();
    application::set_open_path_for_tests(capture_open);
    let root = tempdir().unwrap();
    let memory = root.path().join("MEMORY.md");
    std::fs::write(&memory, "memory").unwrap();
    let locator = file_uri(&memory).unwrap();
    let conn = db(root.path(), &locator, "disabled");

    let err = application::open_original_location(&conn, OpenRequest::new("rec_a".into()).unwrap())
        .unwrap_err();
    assert!(matches!(err, OpenError::RecordNotFound));
    assert!(OPENED_PATH.lock().unwrap().is_none());
    assert!(OpenRequest::new("not-a-record".into()).is_err());
    reset_open_state();
}

#[test]
fn escaped_or_missing_target_returns_open_failed_without_opening() {
    let _guard = OPEN_TEST_LOCK.lock().expect("test lock");
    reset_open_state();
    application::set_open_path_for_tests(capture_open);
    let root = tempdir().unwrap();
    let outside = tempdir().unwrap();
    let outside_file = outside.path().join("MEMORY.md");
    std::fs::write(&outside_file, "outside").unwrap();
    let conn = db(root.path(), &file_uri(&outside_file).unwrap(), "confirmed");

    let err = application::open_original_location(&conn, OpenRequest::new("rec_a".into()).unwrap())
        .unwrap_err();

    assert!(matches!(err, OpenError::OpenFailed { .. }));
    assert!(OPENED_PATH.lock().unwrap().is_none());
    reset_open_state();
}

#[test]
fn opener_failure_maps_to_open_failed() {
    let _guard = OPEN_TEST_LOCK.lock().expect("test lock");
    reset_open_state();
    application::set_open_path_for_tests(fail_open);
    let root = tempdir().unwrap();
    let memory = root.path().join("MEMORY.md");
    std::fs::write(&memory, "memory").unwrap();
    let conn = db(root.path(), &file_uri(&memory).unwrap(), "confirmed");

    let err = application::open_original_location(&conn, OpenRequest::new("rec_a".into()).unwrap())
        .unwrap_err();

    assert!(matches!(err, OpenError::OpenFailed { .. }));
    reset_open_state();
}

/// `open_target_for_record` joins `tessera_meta active ON active.key=
/// ('active_generation:'||m.source_id) AND active.value=m.generation` to
/// confine opens to the current active index. Every other test in this file
/// uses `gen_1` as active, so the JOIN never actually rejects anything. This
/// test pins the rejection: a confirmed source with `gen_1` ACTIVE containing
/// `rec_active` and `gen_2` NON-active containing `rec_stale` must reject the
/// stale record with `RecordNotFound` and never invoke the opener. Mirrors
/// `missing_and_non_confirmed_records_do_not_open` in shape.
#[test]
fn inactive_generation_records_do_not_open() {
    let _guard = OPEN_TEST_LOCK.lock().expect("test lock");
    reset_open_state();
    application::set_open_path_for_tests(capture_open);
    let root = tempdir().unwrap();
    let memory = root.path().join("MEMORY.md");
    std::fs::write(&memory, "memory").unwrap();
    let locator = file_uri(&memory).unwrap();

    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    let root_canon = std::fs::canonicalize(root.path()).unwrap();
    let root_str = root_canon.to_string_lossy().into_owned();
    // Confirmed source — only its ACTIVE generation's records are openable.
    conn.execute(
        "INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', ?1, 'fixture', NULL)",
        params![root_str],
    )
    .unwrap();
    // gen_1 is the ACTIVE generation; gen_2 is a non-active generation whose
    // rows would normally have been deleted by `commit_cas`, but the open
    // query's active-generation JOIN must still exclude them if they ever
    // resurface (defense-in-depth pinned here).
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_2', 'succeeded', 2, 'gen_2', 'fixture')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES ('rec_active', 1, 'gen_1', 'codex', 'section', 'rec_active', ?1, 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 1, 'revision', ?2)",
        params![locator, format!("{locator}#L1-L2")],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES ('rec_stale', 1, 'gen_2', 'codex', 'section', 'rec_stale', ?1, 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 1, 'revision', ?2)",
        params![locator, format!("{locator}#L1-L2")],
    )
    .unwrap();

    // The stale record exists in the table but is NOT in the active
    // generation — the JOIN must reject it.
    let err =
        application::open_original_location(&conn, OpenRequest::new("rec_stale".into()).unwrap())
            .unwrap_err();
    assert!(matches!(err, OpenError::RecordNotFound));
    assert!(OPENED_PATH.lock().unwrap().is_none());
    // Sanity: the active record in the SAME table DOES open, so the rejection
    // is generation-scoped (not a broken fixture).
    let ok = application::open_original_location(
        &conn,
        OpenRequest::new("rec_active".into()).unwrap(),
    )
    .unwrap();
    assert_eq!(ok.record_id(), "rec_active");
    assert_eq!(
        OPENED_PATH.lock().unwrap().as_ref().unwrap(),
        &std::fs::canonicalize(memory).unwrap()
    );
    reset_open_state();
}
