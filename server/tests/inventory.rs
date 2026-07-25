use rusqlite::{params, Connection};

use tessera_lib::application;
use tessera_lib::domain::scan::ScanRunState;
use tessera_lib::domain::query::SearchRequest;
use tessera_lib::domain::source::{HealthState, SourceId};
use tessera_lib::index::scan_store::ScanStore;
use tessera_lib::index::{migrations, SourceRegistry};

fn db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("in-memory db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    migrations::apply(&mut conn).expect("migrations");
    conn
}

#[test]
fn inventory_keeps_last_success_and_omits_limited_coverage_count() {
    let conn = db();
    conn.execute_batch(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES
         (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/safe/root', 'one', NULL),
         (2, 'codex', 'agent_memory', 'confirmed', 'degraded', 'search_only', '/limited/root', 'two', 'project');
         INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at) VALUES
         (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture', 10),
         (1, 'gen_2', 'failed', 2, 'gen_2', 'fixture', 20);
         INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1');
         INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES
         ('rec_1', 1, 'gen_1', 'codex', 'section', 'unit', 'file:///safe', 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 1, 'revision', 'file:///safe#L1');",
    ).expect("fixture rows");
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_inventory(&registry, &conn).expect("inventory");
    let full = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_1".into()))
        .expect("full item");
    assert_eq!(full.health_state, HealthState::Healthy);
    assert_eq!(full.last_successful_scan, Some(10));
    assert_eq!(full.complete_record_count, Some(1));
    assert_eq!(
        full.latest_error.as_deref(),
        Some("Tessera could not complete the last rescan.")
    );
    let limited = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_2".into()))
        .expect("limited item");
    assert_eq!(limited.complete_record_count, None);
    assert_eq!(limited.native_project.as_deref(), Some("project"));
}

#[test]
fn health_updates_do_not_change_confirmation() {
    let conn = db();
    conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/safe/root', 'one', NULL)", []).expect("source");
    let registry = SourceRegistry::new(&conn);
    let source = registry
        .set_health(&SourceId("src_1".into()), HealthState::Degraded)
        .expect("health update")
        .expect("source");
    assert_eq!(
        source.lifecycle_state,
        tessera_lib::domain::source::SourceLifecycle::Confirmed
    );
    assert_eq!(source.health_state, HealthState::Degraded);
    let _: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM source_registry WHERE lifecycle_state = 'confirmed'",
            params![],
            |row| row.get(0),
        )
        .expect("query");
}

#[test]
fn cancellation_fence_prevents_staged_generation_activation() {
    let conn = db();
    conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/safe/root', 'one', NULL)", []).expect("source");
    let store = ScanStore::new(&conn);
    let (scan_id, token, generation) = store.begin_run(1, "fixture").expect("begin");
    store
        .set_state(scan_id, ScanRunState::Committing)
        .expect("committing");
    assert!(store.cancel_latest_run(1).expect("cancel"));
    assert!(!store
        .commit_cas(scan_id, token, &generation, 1)
        .expect("cas"));
    assert_eq!(store.active_generation(1).expect("active"), None);
}

#[test]
fn immediate_reserved_cancellation_stays_cancelled_and_is_inventory_safe() {
    let temp = tempfile::tempdir().expect("source root");
    std::fs::write(temp.path().join("MEMORY.md"), "# memory\nbody").expect("fixture");
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &tessera_lib::domain::CandidateSource {
        provider: "codex".into(), root_path: temp.path().to_string_lossy().into_owned(),
        basis: tessera_lib::domain::DiscoveryBasis::CodexHomeEnv,
        coverage_level: tessera_lib::domain::CoverageLevel::Full, native_project: None,
    }).expect("confirm");
    let store = ScanStore::new(&conn);
    let (scan_id, token, generation) = store.begin_run(1, "pending").expect("reserve");
    assert!(store.cancel_run(scan_id, 1).expect("immediate cancel"));
    let error = application::scan_reserved_source(&registry, &conn, &source.source_id, scan_id, token, generation).expect_err("cancelled run never scans");
    assert!(matches!(error, tessera_lib::domain::scan::ScanError::Cancelled));
    let run: (String, String) = conn.query_row("SELECT state, error_code FROM scan_runs WHERE id = ?1", [scan_id], |row| Ok((row.get(0)?, row.get(1)?))).expect("run");
    assert_eq!(run, ("failed".into(), "cancelled".into()));
    let inventory = application::list_inventory(&registry, &conn).expect("inventory");
    assert_eq!(inventory[0].latest_error.as_deref(), Some("The last rescan was cancelled."));
    assert_eq!(store.active_generation(1).expect("active"), None);
}

#[test]
fn cancelled_rescan_keeps_existing_search_and_open_target_active() {
    let temp = tempfile::tempdir().expect("source root");
    std::fs::write(temp.path().join("MEMORY.md"), "# retained memory\nbody").expect("fixture");
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &tessera_lib::domain::CandidateSource {
        provider: "codex".into(), root_path: temp.path().to_string_lossy().into_owned(),
        basis: tessera_lib::domain::DiscoveryBasis::CodexHomeEnv,
        coverage_level: tessera_lib::domain::CoverageLevel::Full, native_project: None,
    }).expect("confirm");
    application::scan_source(&registry, &conn, &source.source_id).expect("initial success");
    let store = ScanStore::new(&conn);
    let previous_generation = store.active_generation(1).expect("active generation").expect("initial generation");
    let result = application::search(&registry, &conn, SearchRequest::new("retained".into(), None, None).expect("request")).expect("search");
    let record_id = result.results().first().expect("indexed result").record_id().to_string();

    let (scan_id, token, generation) = store.begin_run(1, "pending").expect("reserve");
    assert!(store.cancel_run(scan_id, 1).expect("cancel reserved run"));
    assert!(matches!(
        application::scan_reserved_source(&registry, &conn, &source.source_id, scan_id, token, generation),
        Err(tessera_lib::domain::scan::ScanError::Cancelled)
    ));

    assert_eq!(store.active_generation(1).expect("active generation"), Some(previous_generation));
    assert!(!application::search(&registry, &conn, SearchRequest::new("retained".into(), None, None).expect("request")).expect("search").results().is_empty());
    assert!(store.open_target_for_record(&record_id).expect("open target").is_some());
}

#[test]
fn inventory_preserves_lifecycle_and_limited_actions_can_be_gated_by_the_ui() {
    let conn = db();
    conn.execute_batch("INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES
      (1, 'codex', 'agent_memory', 'disabled', 'unknown', 'full', '/disabled', 'one', NULL),
      (2, 'codex', 'agent_memory', 'rejected', 'unknown', 'unsupported', '/rejected', 'two', NULL);").expect("sources");
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_inventory(&registry, &conn).expect("inventory");
    assert_eq!(inventory[0].lifecycle_state, tessera_lib::domain::source::SourceLifecycle::Disabled);
    assert_eq!(inventory[1].lifecycle_state, tessera_lib::domain::source::SourceLifecycle::Rejected);
    assert_eq!(inventory[1].complete_record_count, None);
}

#[test]
fn root_validation_failure_has_a_safe_inventory_reason_without_erasing_success() {
    let temp = tempfile::tempdir().expect("source root");
    let root = temp.path().join("memories");
    std::fs::create_dir(&root).expect("root");
    std::fs::write(root.join("MEMORY.md"), "# memory\nbody").expect("fixture");
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &tessera_lib::domain::CandidateSource {
        provider: "codex".into(), root_path: root.to_string_lossy().into_owned(),
        basis: tessera_lib::domain::DiscoveryBasis::CodexHomeEnv,
        coverage_level: tessera_lib::domain::CoverageLevel::Full, native_project: None,
    }).expect("confirm");
    application::scan_source(&registry, &conn, &source.source_id).expect("initial success");
    std::fs::remove_dir_all(&root).expect("remove root");
    assert!(matches!(application::scan_source(&registry, &conn, &source.source_id), Err(tessera_lib::domain::scan::ScanError::RootInvalid)));
    let item = application::list_inventory(&registry, &conn).expect("inventory").remove(0);
    assert_eq!(item.health_state, HealthState::Degraded);
    assert_eq!(item.complete_record_count, Some(1));
    assert_eq!(item.latest_error.as_deref(), Some("Tessera could not access this source."));
}

/// Story 2.5 AC — the inventory lists every confirmed source regardless of
/// provider. With one Codex + one Claude Code source confirmed and scanned,
/// `list_inventory` returns both rows — each carrying its own provider,
/// coverage, native project, and honest per-source record count. Pins the
/// multi-provider panorama at the inventory endpoint (the backend has been
/// multi-provider at the row level since 2.1; this test guards against a
/// future regression that filters by provider).
#[test]
fn inventory_lists_multiple_providers_together() {
    let conn = db();
    conn.execute_batch(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES
         (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/codex/root', 'fp-codex', NULL),
         (2, 'claude_code', 'agent_memory', 'confirmed', 'healthy', 'full', '/claude/root', 'fp-claude', 'proj-claude');
         INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at) VALUES
         (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture', 100),
         (2, 'gen_2', 'succeeded', 1, 'gen_2', 'fixture', 200);
         INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1'), ('active_generation:2', 'gen_2');
         INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES
         ('rec_codex', 1, 'gen_1', 'codex', 'section', 'rec_codex', 'file:///codex', 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 1, 'revision', 'file:///codex#L1'),
         ('rec_claude_a', 2, 'gen_2', 'claude_code', 'section', 'rec_claude_a', 'file:///claude#A', 'hash', 'v1', 'title a', 'body a', 'proj-claude', 'memory', 'full', 2, 'revision', 'file:///claude#L1'),
         ('rec_claude_b', 2, 'gen_2', 'claude_code', 'section', 'rec_claude_b', 'file:///claude#B', 'hash', 'v1', 'title b', 'body b', 'proj-claude', 'topic_memory', 'full', 3, 'revision', 'file:///claude#L2');",
    )
    .expect("fixture rows");
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_inventory(&registry, &conn).expect("inventory");
    assert_eq!(
        inventory.len(),
        2,
        "both providers' rows must appear in one inventory: {inventory:?}",
    );
    let codex = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_1".into()))
        .expect("codex row present");
    assert_eq!(codex.provider.as_str(), "codex");
    assert_eq!(codex.health_state, HealthState::Healthy);
    assert_eq!(codex.coverage_level.as_str(), "full");
    assert_eq!(
        codex.complete_record_count,
        Some(1),
        "codex count is its own, honest per-source",
    );
    assert_eq!(codex.last_successful_scan, Some(100));
    assert_eq!(codex.native_project.as_deref(), None);

    let claude = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_2".into()))
        .expect("claude row present");
    assert_eq!(claude.provider.as_str(), "claude_code");
    assert_eq!(claude.health_state, HealthState::Healthy);
    assert_eq!(claude.coverage_level.as_str(), "full");
    assert_eq!(
        claude.complete_record_count,
        Some(2),
        "claude count is its own, independent of codex",
    );
    assert_eq!(claude.last_successful_scan, Some(200));
    assert_eq!(claude.native_project.as_deref(), Some("proj-claude"));

    // The panorama reflects the real registry state: both providers, neither
    // hidden, in registry order.
    let providers: std::collections::HashSet<&str> =
        inventory.iter().map(|item| item.provider.as_str()).collect();
    assert!(providers.contains("codex"));
    assert!(providers.contains("claude_code"));
}

/// Story 2.5 AC — one source's scan failure / `error` health does not affect
/// another source's display or status. With source 1 in `error` (latest run
/// failed) and source 2 `healthy` (succeeded scan, active generation), both
/// rows return: the failed one carries its `latest_error`, the healthy one's
/// count and last-successful-scan are intact. Pins the per-source isolation
/// guarantee at the inventory endpoint (the scan/health layer has been
/// per-source-isolated since 1.x; this test guards the inventory projection).
#[test]
fn inventory_one_source_down_does_not_affect_others() {
    let conn = db();
    conn.execute_batch(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES
         (1, 'codex', 'agent_memory', 'confirmed', 'error', 'full', '/codex/down', 'fp-codex', NULL),
         (2, 'claude_code', 'agent_memory', 'confirmed', 'healthy', 'full', '/claude/up', 'fp-claude', 'proj-claude');
         INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at, error_code) VALUES
         (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture', 100, NULL),
         (1, 'gen_2', 'failed', 2, 'gen_2', 'fixture', 200, 'enumeration_failed'),
         (2, 'gen_3', 'succeeded', 1, 'gen_3', 'fixture', 300, NULL);
         INSERT INTO tessera_meta(key, value) VALUES ('active_generation:2', 'gen_3');
         INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES
         ('rec_claude', 2, 'gen_3', 'claude_code', 'section', 'rec_claude', 'file:///claude', 'hash', 'v1', 'title', 'body', 'proj-claude', 'memory', 'full', 3, 'revision', 'file:///claude#L1');",
    )
    .expect("fixture rows");
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_inventory(&registry, &conn).expect("inventory");
    assert_eq!(
        inventory.len(),
        2,
        "both rows return even though source 1 is down: {inventory:?}",
    );

    // Source 1: error health, latest run failed, latest_error carried.
    let codex = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_1".into()))
        .expect("codex row present despite failure");
    assert_eq!(codex.health_state, HealthState::Error);
    assert_eq!(
        codex.latest_error.as_deref(),
        Some("Tessera could not read this source."),
        "failed latest run surfaces its safe reason",
    );

    // Source 2: healthy, its own count + last-successful-scan intact —
    // unaffected by source 1's failure (per-source isolation at the inventory
    // layer). This is the load-bearing assertion of the test.
    let claude = inventory
        .iter()
        .find(|item| item.source_id == SourceId("src_2".into()))
        .expect("claude row present");
    assert_eq!(claude.health_state, HealthState::Healthy);
    assert_eq!(
        claude.complete_record_count,
        Some(1),
        "healthy source count is intact",
    );
    assert_eq!(
        claude.last_successful_scan,
        Some(300),
        "healthy source last-successful-scan is intact",
    );
    assert!(
        claude.latest_error.is_none(),
        "healthy source carries no latest_error",
    );
}
