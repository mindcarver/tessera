use std::fs;
use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::application;
use tessera_lib::application::query::QueryError;
use tessera_lib::domain::ports::provider_adapter::{CoverageLevel, DiscoveryBasis, ProviderMemoryType};
use tessera_lib::domain::query::{SearchEmptyState, SearchFilters, SearchRequest, SourceQueryStatusKind, KNOWN_PROVIDER_IDS};
use tessera_lib::domain::source::SourceId;
use tessera_lib::index::{migrations, SourceRegistry};
use tessera_lib::domain::CandidateSource;

fn db() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/fixture', 'fixture', NULL)", []).unwrap();
    conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')", []).unwrap();
    conn.execute("INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')", []).unwrap();
    for (id, title, body) in [("rec_a", "中文记忆", "本地优先的设计"), ("rec_b", "另一条", "中文关键词") ] {
        conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES (?1, 1, 'gen_1', 'codex', 'section', ?1, 'file:///fixture#x', 'hash', 'v1', ?2, ?3, NULL, 'memory', 'full', 1, 'revision', 'file:///fixture#L1-L2')", params![id, title, body]).unwrap();
    }
    conn
}

#[test]
fn literal_two_and_three_character_cjk_queries_have_recall_and_page_stably() {
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, Some(1)).unwrap()).unwrap();
    assert_eq!(page.results().len(), 1);
    assert!(page.next_cursor().is_some());
    assert_eq!(page.results()[0].provider(), "codex");
    assert!(page.results()[0].excerpt().contains("中文"));
    let next = application::search(&registry, &conn, SearchRequest::new("中文".into(), page.next_cursor().map(str::to_string), Some(1)).unwrap()).unwrap();
    assert_eq!(next.results().len(), 1);
    assert_ne!(page.results()[0].record_id(), next.results()[0].record_id());
    assert!(
        !application::search(
            &registry,
            &conn,
            SearchRequest::new("本地优先".into(), None, None).unwrap(),
        )
        .unwrap()
        .results()
        .is_empty()
    );
}

#[test]
fn no_match_and_invalid_request_are_truthful() {
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("\" OR *".into(), None, None).unwrap()).unwrap();
    assert!(page.results().is_empty());
    assert_eq!(page.empty_state(), Some(SearchEmptyState::NoMatch));
    assert!(SearchRequest::new("   ".into(), None, None).is_err());
    assert!(SearchRequest::new("x".repeat(1025), None, None).is_err());
    assert!(SearchRequest::new("x".into(), Some("x".repeat(16 * 1024 + 1)), None).is_err());
}

#[test]
fn search_excludes_disabled_and_rejected_sources_even_when_they_have_active_records() {
    let conn = db();
    for (id, lifecycle, record_id) in [(2, "disabled", "rec_disabled"), (3, "rejected", "rec_rejected")] {
        conn.execute("INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES (?1, 'codex', 'agent_memory', ?2, 'unknown', 'full', ?3, ?3, NULL)", params![id, lifecycle, format!("/fixture-{id}")]).unwrap();
        conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (?1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')", params![id]).unwrap();
        conn.execute("INSERT INTO tessera_meta(key, value) VALUES (?1, 'gen_1')", params![format!("active_generation:{id}")]).unwrap();
        conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES (?1, ?2, 'gen_1', 'codex', 'section', ?1, 'file:///fixture#x', 'hash', 'v1', '中文隐藏记录', 'hidden', NULL, 'memory', 'full', 1, 'revision', 'file:///fixture#L1-L2')", params![record_id, id]).unwrap();
    }
    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, Some(20)).unwrap()).unwrap();
    assert_eq!(page.results().len(), 2);
    assert!(page.results().iter().all(|result| !matches!(result.record_id(), "rec_disabled" | "rec_rejected")));
}

#[test]
fn cursor_becomes_stale_after_a_new_generation_activates() {
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let first = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, Some(1)).unwrap()).unwrap();
    let cursor = first.next_cursor().expect("first page cursor").to_string();

    conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_2', 'succeeded', 2, 'gen_2', 'fixture')", []).unwrap();
    conn.execute("UPDATE tessera_meta SET value = 'gen_2' WHERE key = 'active_generation:1'", []).unwrap();
    conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES ('rec_c', 1, 'gen_2', 'codex', 'section', 'rec_c', 'file:///fixture#new', 'hash', 'v1', '中文新记录', 'new', NULL, 'memory', 'full', 2, 'revision', 'file:///fixture#L3-L4')", []).unwrap();

    let continuation = application::search(&registry, &conn, SearchRequest::new("中文".into(), Some(cursor), Some(1)).unwrap()).unwrap_err();
    assert!(matches!(continuation, QueryError::CursorStale));

    let fresh = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, Some(20)).unwrap()).unwrap();
    assert_eq!(fresh.results().len(), 1);
    assert_eq!(fresh.results()[0].record_id(), "rec_c");
}

#[test]
fn cursor_is_bound_to_its_query_and_empty_states_remain_distinct() {
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    let first = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, Some(1)).unwrap()).unwrap();
    let err = application::search(&registry, &conn, SearchRequest::new("本地优先".into(), first.next_cursor().map(str::to_string), Some(1)).unwrap()).unwrap_err();
    assert!(matches!(err, QueryError::BadRequest));

    conn.execute("DELETE FROM memory_records", []).unwrap();
    conn.execute("DELETE FROM tessera_meta", []).unwrap();
    let not_indexed = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, None).unwrap()).unwrap();
    assert_eq!(not_indexed.empty_state(), Some(SearchEmptyState::SourceNotIndexed));

    conn.execute("UPDATE scan_runs SET state = 'failed' WHERE source_id = 1", []).unwrap();
    let unavailable = application::search(&registry, &conn, SearchRequest::new("中文".into(), None, None).unwrap()).unwrap();
    assert_eq!(unavailable.empty_state(), Some(SearchEmptyState::SourceUnavailable));
}

#[test]
fn generic_e2e_fixture_keeps_short_cjk_recall_without_a_performance_gate() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("memories");
    fs::create_dir(&root).unwrap();
    fs::copy(
        "../tests/fixtures/e2e-codex-home/memories/MEMORY.md",
        root.join("MEMORY.md"),
    )
    .unwrap();
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &CandidateSource {
        provider: "codex".into(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::CodexHomeEnv,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    }).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();

    for query in ["中文", "检索样"] {
        let page = application::search(&registry, &conn, SearchRequest::new(query.into(), None, None).unwrap()).unwrap();
        assert!(!page.results().is_empty(), "short-CJK recall must be non-zero");
    }
}

/// Helper: insert a confirmed source with an active generation and return its
/// rowid. Used by the Story 2.3 multi-provider and FR-14 fixtures.
fn insert_confirmed_source(
    conn: &Connection,
    rowid: i64,
    provider: &str,
    health: &str,
    native_project: Option<&str>,
    active: bool,
) {
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (?1, ?2, 'agent_memory', 'confirmed', ?3, 'full', ?4, ?4, ?5)",
        params![rowid, provider, health, format!("/fixture-{rowid}"), native_project],
    ).unwrap();
    if active {
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (?1, ?2, 'succeeded', 1, ?2, 'fixture')",
            params![rowid, format!("gen_{rowid}")],
        ).unwrap();
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES (?1, ?2)",
            params![format!("active_generation:{rowid}"), format!("gen_{rowid}")],
        ).unwrap();
    }
}

/// Helper: insert a memory record under a source's active generation.
#[allow(clippy::too_many_arguments)]
fn insert_record(
    conn: &Connection,
    record_id: &str,
    source_rowid: i64,
    provider: &str,
    title: &str,
    body: &str,
    observed_at: i64,
    coverage_level: &str,
    native_project: Option<&str>,
) {
    insert_record_typed(
        conn,
        record_id,
        source_rowid,
        provider,
        title,
        body,
        observed_at,
        coverage_level,
        native_project,
        "memory",
    );
}

/// Story 2.4 helper — like [`insert_record`] but also sets
/// `provider_memory_type`, needed by the memory-type filter tests. The default
/// helper keeps `'memory'` so 2.3 callers are unchanged.
#[allow(clippy::too_many_arguments)]
fn insert_record_typed(
    conn: &Connection,
    record_id: &str,
    source_rowid: i64,
    provider: &str,
    title: &str,
    body: &str,
    observed_at: i64,
    coverage_level: &str,
    native_project: Option<&str>,
    memory_type: &str,
) {
    let generation = format!("gen_{source_rowid}");
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
         VALUES (?1, ?2, ?3, ?4, 'section', ?1, ?5, 'hash', 'v1', ?6, ?7, ?8, ?9, ?10, ?11, 'revision', ?5)",
        params![
            record_id,
            source_rowid,
            generation,
            provider,
            format!("file:///fixture#{record_id}"),
            title,
            body,
            native_project,
            memory_type,
            coverage_level,
            observed_at,
        ],
    ).unwrap();
}

/// Story 2.3 AC — Codex + Claude Code records sharing a keyword both return,
/// each card tagged with its `provider`, ordered by the relevance key (title-
/// match before body-only), and the sidecar marks both sources `available`.
#[test]
fn multi_provider_search_returns_both_providers_in_relevance_order() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    // Source 1: Codex, healthy, indexed.
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Source 2: Claude Code, healthy, indexed.
    insert_confirmed_source(&conn, 2, "claude_code", "healthy", Some("proj-claude"), true);

    // Codex record: title match for "federation".
    insert_record(&conn, "rec_codex_title", 1, "codex", "federation patterns", "body text", 100, "full", None);
    // Codex record: body-only match (lower relevance).
    insert_record(&conn, "rec_codex_body", 1, "codex", "Other topic", "discusses federation briefly", 200, "full", None);
    // Claude record: title match for "federation", more recent than Codex title match.
    insert_record(&conn, "rec_claude_title", 2, "claude_code", "federation across agents", "claude body", 300, "full", Some("proj-claude"));

    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("federation".into(), None, Some(20)).unwrap()).unwrap();

    // Both providers must appear in the results.
    let providers: std::collections::HashSet<&str> = page.results().iter().map(|r| r.provider()).collect();
    assert!(providers.contains("codex"), "codex results must return: {:?}", providers);
    assert!(providers.contains("claude_code"), "claude_code results must return: {:?}", providers);

    // Relevance: title-match records must sort before body-only. Both title
    // matches sort before the body-only record, regardless of provider/recency.
    let ids: Vec<&str> = page.results().iter().map(|r| r.record_id()).collect();
    let body_idx = ids.iter().position(|id| *id == "rec_codex_body").expect("body-only record present");
    let title_a_idx = ids.iter().position(|id| *id == "rec_codex_title").expect("codex title record present");
    let title_b_idx = ids.iter().position(|id| *id == "rec_claude_title").expect("claude title record present");
    assert!(title_a_idx < body_idx, "codex title-match must outrank its body-only record: {:?}", ids);
    assert!(title_b_idx < body_idx, "claude title-match must outrank codex body-only record: {:?}", ids);
    // Among same match-tier, more recent first: claude title (observed_at=300) before codex title (100).
    assert!(title_b_idx < title_a_idx, "more-recent title-match must sort first: {:?}", ids);

    // The sidecar marks both confirmed sources available.
    let statuses: std::collections::HashMap<&str, SourceQueryStatusKind> = page.sources().iter().map(|s| (s.provider.as_str(), s.status)).collect();
    assert_eq!(statuses.get("codex"), Some(&SourceQueryStatusKind::Available), "codex must be available: {:?}", statuses);
    assert_eq!(statuses.get("claude_code"), Some(&SourceQueryStatusKind::Available), "claude must be available: {:?}", statuses);

    // Sidecar carries native_project for Claude, None for Codex.
    let claude_status = page.sources().iter().find(|s| s.provider == "claude_code").expect("claude in sidecar");
    assert_eq!(claude_status.native_project.as_deref(), Some("proj-claude"));
    let codex_status = page.sources().iter().find(|s| s.provider == "codex").expect("codex in sidecar");
    assert!(codex_status.native_project.is_none());
}

/// Story 2.3 AC — relevance ordering with stable pagination: a title-match
/// record whose `record_id` sorts AFTER a body-only record still appears first,
/// and pagination via the relevance-bound cursor reaches the body-only record
/// on the next page without skipping it.
#[test]
fn relevance_ordering_with_pagination_does_not_skip_lower_record_ids() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Body-only record with a LOW record_id (would be skipped by a naive
    // `record_id > cursor` predicate because the cursor is a high-id title
    // match record).
    insert_record(&conn, "rec_a_body_only", 1, "codex", "Other topic", "discusses federation briefly", 100, "full", None);
    // Title-match record with a HIGH record_id (the cursor after page 1).
    insert_record(&conn, "rec_z_title_match", 1, "codex", "federation patterns", "body text", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // Page 1 (limit 1): the title-match record (high id) returns first.
    let page1 = application::search(&registry, &conn, SearchRequest::new("federation".into(), None, Some(1)).unwrap()).unwrap();
    assert_eq!(page1.results().len(), 1);
    assert_eq!(page1.results()[0].record_id(), "rec_z_title_match");
    assert!(page1.results()[0].title_match(), "title-match flag must be set");

    let cursor = page1.next_cursor().expect("has more results").to_string();
    // Page 2: the body-only record (low id) must NOT be skipped.
    let page2 = application::search(&registry, &conn, SearchRequest::new("federation".into(), Some(cursor), Some(1)).unwrap()).unwrap();
    assert_eq!(page2.results().len(), 1, "body-only record must not be skipped by relevance pagination");
    assert_eq!(page2.results()[0].record_id(), "rec_a_body_only");
    assert!(!page2.results()[0].title_match(), "body-only record must not have title_match");
}

/// Story 2.3 AC / FR-14 prototype — one confirmed source is unavailable
/// (failed scan, no active generation) while another is healthy and matches.
/// The healthy source's results return normally and the unavailable source is
/// flagged in the sidecar — the query does not fail.
#[test]
fn fr14_one_source_unavailable_does_not_break_query_and_is_flagged_in_sidecar() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    // Source 1: healthy, indexed, has matching records.
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_codex", 1, "codex", "federation patterns", "body", 100, "full", None);
    // Source 2: confirmed but Failed scan, no active generation.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'error', 'full', '/fixture-2', '/fixture-2', 'proj-down')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, error_code)
         VALUES (2, 'gen_2', 'failed', 1, 'gen_2', 'fixture', 'enumeration_failed')",
        [],
    ).unwrap();

    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("federation".into(), None, Some(20)).unwrap()).unwrap();
    // The healthy source's results return.
    assert_eq!(page.results().len(), 1, "healthy source results must return while another is down");
    assert_eq!(page.results()[0].provider(), "codex");
    // The unavailable source is flagged in the sidecar.
    let claude_status = page.sources().iter().find(|s| s.provider == "claude_code").expect("claude in sidecar");
    assert_eq!(claude_status.status, SourceQueryStatusKind::Unavailable, "failed source must be flagged unavailable");
    let codex_status = page.sources().iter().find(|s| s.provider == "codex").expect("codex in sidecar");
    assert_eq!(codex_status.status, SourceQueryStatusKind::Available, "healthy source must be available");
    // The query did NOT fail — no empty_state.
    assert!(page.empty_state().is_none(), "partial unavailability must not produce an empty_state");
}

/// Story 2.3 I/O matrix — a degraded source with prior active records keeps
/// returning its records; the sidecar marks it `degraded` (not hidden).
#[test]
fn degraded_source_keeps_records_and_is_flagged_degraded_in_sidecar() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    insert_confirmed_source(&conn, 1, "codex", "degraded", None, true);
    insert_record(&conn, "rec_codex", 1, "codex", "federation patterns", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("federation".into(), None, Some(20)).unwrap()).unwrap();
    // The degraded source's records still return.
    assert_eq!(page.results().len(), 1, "degraded source records must not be suppressed");
    // The sidecar flags it degraded.
    let codex_status = page.sources().iter().find(|s| s.provider == "codex").expect("codex in sidecar");
    assert_eq!(codex_status.status, SourceQueryStatusKind::Degraded);
}

/// Story 2.3 I/O matrix — an Error source that STILL HAS an active generation
/// keeps returning its records; the sidecar marks it `degraded` (records
/// answer, flag for attention), NOT `unavailable`. `Unavailable` is reserved
/// for an Error source with no active generation (pins patch 1).
#[test]
fn error_source_with_active_generation_is_flagged_degraded_not_unavailable() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    // Error health, but an active generation is still serving records.
    insert_confirmed_source(&conn, 1, "codex", "error", None, true);
    insert_record(&conn, "rec_codex", 1, "codex", "federation patterns", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, SearchRequest::new("federation".into(), None, Some(20)).unwrap()).unwrap();
    // The Error source's prior-generation records still return (the search
    // JOINs on active_generation).
    assert_eq!(page.results().len(), 1, "error source's active records must still answer");
    // Sidecar flags it Degraded — not Unavailable — so the UI does not claim
    // the visible records are absent.
    let codex_status = page.sources().iter().find(|s| s.provider == "codex").expect("codex in sidecar");
    assert_eq!(codex_status.status, SourceQueryStatusKind::Degraded, "error+active must be degraded, not unavailable");
}

/// FR-14 best-effort sidecar (pins patch 2) — one confirmed source has a
/// corrupt `scan_runs.state` so its `latest_run` lookup errors. The search must
/// still return the healthy source's results; the sidecar must NOT propagate
/// `QueryError::Internal` — "one unavailable source never breaks the query"
/// extends to the sidecar.
#[test]
fn sidecar_status_lookup_failure_does_not_break_search() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    // Source 1: healthy, indexed, has matching records.
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_codex", 1, "codex", "federation patterns", "body", 100, "full", None);
    // Source 2: confirmed but its `scan_runs.state` is corrupt (unparseable),
    // so `latest_run` returns an error. It has no active-generation marker, so
    // it contributes no records — only its sidecar status lookup blows up.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-2', '/fixture-2', NULL)",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (2, 'gen_2', 'totally_bogus_state', 1, 'gen_2', 'fixture')",
        [],
    ).unwrap();

    let registry = SourceRegistry::new(&conn);
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new("federation".into(), None, Some(20)).unwrap(),
    ).expect("sidecar lookup failure must NOT fail the search");
    // The healthy source's results still return.
    assert_eq!(page.results().len(), 1, "healthy source results must return despite another source's corrupt status");
    assert_eq!(page.results()[0].provider(), "codex");
    // The healthy source is still marked available — its status lookup
    // succeeded normally.
    let codex_status = page.sources().iter().find(|s| s.provider == "codex").expect("codex in sidecar");
    assert_eq!(codex_status.status, SourceQueryStatusKind::Available);
    // The corrupt source still appears in the sidecar (conservative fallback)
    // rather than dropping the whole query.
    assert!(page.sources().iter().any(|s| s.provider == "claude_code"), "corrupt source still reported in sidecar");
}

/// Patch 5/6 — a pre-2.3 (`v1.<hex>`) cursor must be rejected as
/// `CursorStale`, NOT `BadRequest`. The index sort shape changed in 2.3, so the
/// existing UI `cursor_stale` recovery path (re-run the first page) is the
/// correct outcome; a generic contract error would surface an opaque
/// `bad_request`. Hand-encodes a v1 envelope so a future lenient fallback
/// fails loudly.
#[test]
fn v1_cursor_is_rejected_as_stale_not_bad_request() {
    let conn = db();
    let registry = SourceRegistry::new(&conn);
    // Hand-encode a v1 cursor envelope (pre-2.3 shape: version=1, no relevance
    // sort-key fields). The prefix check fires before any decode, but a
    // realistic payload keeps the test honest about the v1 wire shape.
    let payload = r#"{"version":1,"query":"中文","revision":"deadbeef","last_record_id":"rec_a"}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let v1_cursor = format!("v1.{hex}");
    let err = application::search(
        &registry,
        &conn,
        SearchRequest::new("中文".into(), Some(v1_cursor), Some(1)).unwrap(),
    ).unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "v1 cursor must map to CursorStale, got {err:?}");
}

// ---------------------------------------------------------------------------
// Story 2.4 — cross-provider combined filtering & range visibility
// ---------------------------------------------------------------------------

/// Build a multi-provider fixture for Story 2.4 filter tests: Codex (NULL
/// native_project) and Claude Code (proj-claude) both confirmed + indexed with
/// records carrying the shared keyword "federation". Records vary
/// `provider_memory_type` and `observed_at` so each filter dimension has a
/// discriminating fixture.
fn build_filter_fixture() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_confirmed_source(&conn, 2, "claude_code", "healthy", Some("proj-claude"), true);
    // Codex records: NULL native_project, type=memory.
    insert_record_typed(&conn, "rec_codex_old", 1, "codex", "federation early", "body", 100, "full", None, "memory");
    insert_record_typed(&conn, "rec_codex_summary", 1, "codex", "federation summary", "body", 500, "full", None, "memory_summary");
    // Claude records: proj-claude, mix of memory + topic_memory.
    insert_record_typed(&conn, "rec_claude_mem", 2, "claude_code", "federation memory", "body", 200, "full", Some("proj-claude"), "memory");
    insert_record_typed(&conn, "rec_claude_topic", 2, "claude_code", "federation topic", "body", 300, "full", Some("proj-claude"), "topic_memory");
    conn
}

/// Story 2.4 AC — provider filter narrows to one provider's records.
#[test]
fn provider_filter_narrows_to_one_provider() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { provider: Some("codex".into()), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let providers: std::collections::HashSet<&str> = page.results().iter().map(|r| r.provider()).collect();
    assert!(providers.contains("codex"), "codex must be present: {providers:?}");
    assert!(!providers.contains("claude_code"), "claude must be excluded by provider filter: {providers:?}");
    // The sidecar stays unfiltered (Design Notes: availability info, not
    // result info) — both confirmed sources are still listed.
    let sidecar_providers: std::collections::HashSet<&str> = page.sources().iter().map(|s| s.provider.as_str()).collect();
    assert!(sidecar_providers.contains("codex") && sidecar_providers.contains("claude_code"),
        "sidecar must stay unfiltered: {sidecar_providers:?}");
}

/// Story 2.4 AC (Spec Change Log 2026-07-25) — the per-source filter narrows to
/// one specific confirmed source's records. With the multi-source fixture,
/// `source=src_2` returns only Claude's records and `source=src_1` only Codex's.
#[test]
fn source_filter_narrows_to_one_specific_source() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    // src_2 → only the two Claude records.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { source: Some(SourceId("src_2".into())), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let providers: std::collections::HashSet<&str> = page.results().iter().map(|r| r.provider()).collect();
    assert!(!providers.contains("codex"), "codex (src_1) must be excluded by source=src_2: {providers:?}");
    assert!(providers.contains("claude_code"), "claude (src_2) must be present: {providers:?}");
    assert_eq!(page.results().len(), 2, "both src_2 records must match: {:?}", page.results().iter().map(|r| r.record_id()).collect::<Vec<_>>());

    // src_1 → only the two Codex records.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { source: Some(SourceId("src_1".into())), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let providers: std::collections::HashSet<&str> = page.results().iter().map(|r| r.provider()).collect();
    assert!(providers.contains("codex") && !providers.contains("claude_code"),
        "source=src_1 must narrow to codex only: {providers:?}");
    assert_eq!(page.results().len(), 2);
}

/// Story 2.4 AC (Spec Change Log) — the source filter is DISTINCT from the
/// coarser provider filter: when one provider owns several confirmed sources,
/// `source=src_<n>` narrows to just that source's records. Two Claude sources
/// each carry a record; `source=src_3` returns only src_3's record.
#[test]
fn source_filter_is_distinct_from_provider_filter() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    // Two Claude sources under one provider.
    insert_confirmed_source(&conn, 2, "claude_code", "healthy", Some("proj-a"), true);
    insert_confirmed_source(&conn, 3, "claude_code", "healthy", Some("proj-b"), true);
    insert_record(&conn, "rec_src2", 2, "claude_code", "federation a", "body", 100, "full", Some("proj-a"));
    insert_record(&conn, "rec_src3", 3, "claude_code", "federation b", "body", 200, "full", Some("proj-b"));

    let registry = SourceRegistry::new(&conn);
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { source: Some(SourceId("src_3".into())), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let ids: Vec<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids, vec!["rec_src3"], "source=src_3 must narrow to only src_3's record: {ids:?}");

    // AND-combines with another filter: source=src_2 AND memory_type.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters {
                source: Some(SourceId("src_2".into())),
                memory_type: Some(ProviderMemoryType::Memory),
                ..Default::default()
            },
        ).unwrap(),
    ).unwrap();
    let ids: Vec<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids, vec!["rec_src2"], "source AND memory_type must narrow to src_2's memory record: {ids:?}");
}

/// Story 2.4 — a malformed `source` handle is rejected at request construction
/// (the HTTP layer maps this to 400 `bad_request`).
#[test]
fn source_filter_rejects_malformed_handle() {
    assert!(SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { source: Some(SourceId("not-a-source".into())), ..Default::default() },
    ).is_err());
    // A bare `src_` with no rowid is also malformed.
    assert!(SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { source: Some(SourceId("src_".into())), ..Default::default() },
    ).is_err());
}

/// Story 2.4 — native_project is trimmed at request construction so stray
/// whitespace does not silently fail to match.
#[test]
fn native_project_filter_is_trimmed() {
    let request = SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { native_project: Some("  proj-claude  ".into()), ..Default::default() },
    ).unwrap();
    assert_eq!(request.native_project(), Some("proj-claude"));
    // All-whitespace becomes None (no predicate).
    let request = SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { native_project: Some("   ".into()), ..Default::default() },
    ).unwrap();
    assert_eq!(request.native_project(), None);
}

/// Story 2.4 AC — memory-type filter narrows to the matching
/// `provider_memory_type` across providers.
#[test]
fn memory_type_filter_narrows_across_providers() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { memory_type: Some(ProviderMemoryType::Memory), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    // Only type=memory records match (rec_codex_old + rec_claude_mem); the
    // memory_summary and topic_memory records are excluded.
    let ids: std::collections::HashSet<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert!(ids.contains("rec_codex_old"), "codex memory must match: {ids:?}");
    assert!(ids.contains("rec_claude_mem"), "claude memory must match: {ids:?}");
    assert!(!ids.contains("rec_codex_summary"), "memory_summary must be excluded: {ids:?}");
    assert!(!ids.contains("rec_claude_topic"), "topic_memory must be excluded: {ids:?}");
}

/// Story 2.4 AC — native-project filter matches across providers, and Codex's
/// NULL `native_project` honestly does NOT match (SQL `NULL = 'x'` is NULL).
#[test]
fn native_project_filter_matches_across_providers_and_excludes_null() {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_confirmed_source(&conn, 2, "claude_code", "healthy", Some("proj-claude"), true);
    // Add a second Claude source carrying the same project so the cross-
    // provider aspect is exercised within the Claude provider too.
    insert_confirmed_source(&conn, 3, "claude_code", "healthy", Some("proj-claude"), true);
    insert_record(&conn, "rec_codex_null_proj", 1, "codex", "federation global", "body", 100, "full", None);
    insert_record(&conn, "rec_claude_a", 2, "claude_code", "federation proj a", "body", 200, "full", Some("proj-claude"));
    insert_record(&conn, "rec_claude_b", 3, "claude_code", "federation proj b", "body", 300, "full", Some("proj-claude"));

    let registry = SourceRegistry::new(&conn);
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { native_project: Some("proj-claude".into()), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let ids: std::collections::HashSet<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert!(ids.contains("rec_claude_a") && ids.contains("rec_claude_b"),
        "both proj-claude records must match across sources: {ids:?}");
    assert!(!ids.contains("rec_codex_null_proj"),
        "Codex NULL native_project must NOT match a project filter: {ids:?}");
}

/// Story 2.4 AC — time filter (`since`) narrows by `observed_at >= since`.
#[test]
fn time_filter_narrows_by_observed_at() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    // Only records with observed_at >= 250 should match.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { since: Some(250), ..Default::default() },
        ).unwrap(),
    ).unwrap();
    let observed: Vec<i64> = page.results().iter().map(|r| r.observed_at()).collect();
    assert!(observed.iter().all(|&value| value >= 250), "all results must satisfy observed_at >= 250: {observed:?}");
    assert!(observed.contains(&500), "the 500 record must be present: {observed:?}");
    assert!(observed.contains(&300), "the 300 record must be present: {observed:?}");
    assert!(!observed.contains(&100) && !observed.contains(&200),
        "the 100/200 records must be excluded by since=250: {observed:?}");
}

/// Story 2.4 AC — combined filters narrow with AND.
#[test]
fn combined_filters_narrow_with_and() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    // provider=codex AND memory_type=memory AND since=50 → only rec_codex_old
    // (codex, memory, observed_at=100). rec_codex_summary is memory_summary;
    // both Claude records are excluded by provider.
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters {
                provider: Some("codex".into()),
                memory_type: Some(ProviderMemoryType::Memory),
                since: Some(50),
                ..Default::default()
            },
        ).unwrap(),
    ).unwrap();
    let ids: Vec<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids, vec!["rec_codex_old"], "AND combination must narrow to exactly the codex memory record: {ids:?}");
}

/// Story 2.4 AC — no filters (Default) is the 2.3 default scope: all confirmed
/// sources, relevance-ordered. This is the Clear-filters equivalent.
#[test]
fn no_filters_returns_default_confirmed_source_scope() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let unfiltered = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters::default(),
        ).unwrap(),
    ).unwrap();
    let providers: std::collections::HashSet<&str> = unfiltered.results().iter().map(|r| r.provider()).collect();
    assert!(providers.contains("codex") && providers.contains("claude_code"),
        "default scope must include both providers: {providers:?}");
    assert_eq!(unfiltered.results().len(), 4, "all four fixture records must match under no filters");
}

/// Story 2.4 I/O matrix — a filter change mid-pagination is rejected as
/// `CursorStale`. The cursor binds the active filters (v3); a cursor whose
/// filters differ from the request cannot page through a stale result set.
/// The UI's existing `cursor_stale` recovery path re-runs page 1 under the new
/// filters.
#[test]
fn cursor_stale_on_filter_change_mid_pagination() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    // Page 1 with no filters (limit 1 → has more).
    let first = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(1),
            SearchFilters::default(),
        ).unwrap(),
    ).unwrap();
    let cursor = first.next_cursor().expect("first page cursor").to_string();
    // Continue with the cursor but ADD a provider filter — the cursor's bound
    // filters (none) differ from the request's filters (provider=codex), so
    // the server must reject it as CursorStale.
    let err = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            Some(cursor),
            Some(1),
            SearchFilters { provider: Some("codex".into()), ..Default::default() },
        ).unwrap(),
    ).unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "filter-change cursor must be CursorStale, got {err:?}");
}

/// Story 2.4 — a cursor issued UNDER a filter set must be accepted when the
/// request repeats the SAME filter set (pagination continues). This pins the
/// positive path so the stale-rejection test above is not vacuous.
#[test]
fn cursor_is_accepted_when_filters_match_the_request() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let filters = SearchFilters { provider: Some("codex".into()), ..Default::default() };
    let first = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters("federation".into(), None, Some(1), filters.clone()).unwrap(),
    ).unwrap();
    let cursor = first.next_cursor().expect("first page cursor").to_string();
    let second = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters("federation".into(), Some(cursor), Some(1), filters).unwrap(),
    ).unwrap();
    // Page 2 under the same filter set must return the next codex record,
    // not error.
    assert_eq!(second.results().len(), 1, "page 2 must return the next codex record");
    assert_eq!(second.results()[0].provider(), "codex");
    assert_ne!(second.results()[0].record_id(), first.results()[0].record_id());
}

/// Story 2.4 (Spec Change Log) — a cursor issued UNDER a per-source filter
/// round-trips the `source` binding: pagination continues when the same source
/// filter repeats, and is rejected as stale when the source filter changes.
#[test]
fn source_filter_binds_into_cursor_and_round_trips() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let filters = SearchFilters { source: Some(SourceId("src_1".into())), ..Default::default() };
    // Page 1 (limit 1): one codex record, has more.
    let first = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters("federation".into(), None, Some(1), filters.clone()).unwrap(),
    ).unwrap();
    let cursor = first.next_cursor().expect("first page cursor").to_string();
    assert_eq!(first.results()[0].provider(), "codex");

    // Same source filter → page 2 accepts the cursor and returns the next codex record.
    let second = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters("federation".into(), Some(cursor.clone()), Some(1), filters).unwrap(),
    ).unwrap();
    assert_eq!(second.results()[0].provider(), "codex");
    assert_ne!(second.results()[0].record_id(), first.results()[0].record_id());

    // Different source filter (src_2) with the src_1 cursor → CursorStale.
    let err = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            Some(cursor),
            Some(1),
            SearchFilters { source: Some(SourceId("src_2".into())), ..Default::default() },
        ).unwrap(),
    ).unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "source filter change must be CursorStale, got {err:?}");
}

/// Story 5.2 — `tessera_project` (was reserved in 2.4) now narrows the result
/// set to records whose `(provider, native_project)` is in the project's
/// mapping scope set. Replaces the 2.4 "accepted but ignored" test: the slot
/// is now live. Uses the `build_filter_fixture` Codex (NULL native_project) +
/// Claude (proj-claude) sources, creates a project mapping the Codex global
/// scope, and asserts only Codex records return.
#[test]
fn tessera_project_filter_narrows_to_mapped_scope() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let project_store = tessera_lib::index::ProjectStore::new(&conn);
    // Create a project and map the Codex global scope to it.
    let project = create_project(
        &project_store,
        &CreateProjectRequest { name: "Codex-only".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project.project_id.clone(),
            provider: "codex".to_string(),
            native_project: None,
        },
    )
    .unwrap();

    // Search by the project → only Codex records match (the Codex global
    // scope maps via COALESCE(native_project, '') = '').
    let project_filter = SearchFilters {
        tessera_project: Some(project.project_id.0.clone()),
        ..Default::default()
    };
    let narrowed = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters("federation".into(), None, Some(20), project_filter).unwrap(),
    )
    .unwrap();
    assert_eq!(
        narrowed.results().len(),
        2,
        "tessera_project must narrow to the 2 Codex records"
    );
    let providers: std::collections::HashSet<&str> =
        narrowed.results().iter().map(|r| r.provider()).collect();
    assert!(providers.contains("codex"), "only Codex records match: {providers:?}");
    assert!(!providers.contains("claude_code"), "Claude records must be excluded");

    // Unknown project id → empty results (treated as a filter that matches
    // nothing, NOT an error).
    let unknown = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters { tessera_project: Some("proj_999".into()), ..Default::default() },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(unknown.results().is_empty(), "unknown project id must yield no rows");

    // The sidecar narrows to the project's mapped sources (only Codex here).
    assert_eq!(narrowed.sources().len(), 1, "sidecar must narrow to the 1 mapped source");
    assert_eq!(narrowed.sources()[0].provider, "codex");
}

/// Story 5.2 (P1 regression) — a MALFORMED `tessera_project` id
/// (`ProjectId::to_rowid()` returns None, e.g. "proj_x" / "garbage") must
/// collapse to an EMPTY scope set, NOT to "no narrowing". The SQL layer
/// already binds `tessera_project_id = NULL` (which makes the EXISTS
/// predicate always false → empty results); the sidecar MUST match that
/// posture or it would mis-list every confirmed source as in-scope for a
/// query whose result set is empty. Distinct from the `proj_999` case in
/// `tessera_project_filter_narrows_to_mapped_scope` (well-formed handle,
/// unknown rowid → SQL returns no mappings → empty set), this case covers
/// the malformed-handle path where `to_rowid()` itself returns None.
#[test]
fn malformed_tessera_project_id_yields_empty_results_and_empty_sidecar() {
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    // Two confirmed sources in the fixture (Codex global + Claude proj-claude);
    // a malformed project id must NOT list them in the sidecar.
    for malformed in ["proj_x", "garbage", "proj_", "proj_abc"] {
        let page = application::search(
            &registry,
            &conn,
            SearchRequest::new_with_filters(
                "federation".into(),
                None,
                Some(20),
                SearchFilters {
                    tessera_project: Some(malformed.into()),
                    ..Default::default()
                },
            )
            .unwrap(),
        )
        .unwrap();
        assert!(
            page.results().is_empty(),
            "malformed tessera_project {malformed:?} must yield zero results"
        );
        assert!(
            page.sources().is_empty(),
            "malformed tessera_project {malformed:?} must yield zero sidecar sources \
             (collapse to empty set, NOT no-narrowing)"
        );
    }
}

/// Story 5.2 — Codex global (`native_project IS NULL`) records ARE returned
/// when the project maps `(codex, null)`. The COALESCE collapse in the EXISTS
/// predicate mirrors the Story 5.1 uniqueness index, so NULL matches NULL.
#[test]
fn tessera_project_codex_null_scope_matches_via_coalesce() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let project_store = tessera_lib::index::ProjectStore::new(&conn);
    let project = create_project(
        &project_store,
        &CreateProjectRequest { name: "Codex-global".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project.project_id.clone(),
            provider: "codex".to_string(),
            native_project: None,
        },
    )
    .unwrap();

    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters {
                tessera_project: Some(project.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    // Both Codex records (rec_codex_old, rec_codex_summary) have
    // native_project IS NULL — both MUST match via COALESCE.
    let ids: std::collections::HashSet<&str> =
        page.results().iter().map(|r| r.record_id()).collect();
    assert!(ids.contains("rec_codex_old"));
    assert!(ids.contains("rec_codex_summary"));
    // The Claude records have native_project = "proj-claude" — excluded.
    assert!(!ids.contains("rec_claude_mem"));
    assert!(!ids.contains("rec_claude_topic"));
}

/// Story 5.2 — empty `q=` + `tessera_project` returns every in-scope record
/// (browse-by-project via Search). The instr predicate matches every row when
/// the needle is empty. Without a tessera_project filter, an empty query is
/// still rejected (2.x contract). Also covers a newly-created project (zero
/// mappings) → empty results + `SearchEmptyState::NoMatch` on page 1.
#[test]
fn tessera_project_empty_query_returns_all_in_scope_records() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let project_store = tessera_lib::index::ProjectStore::new(&conn);

    // Empty query with NO project filter → still rejected (2.x contract).
    assert!(SearchRequest::new_with_filters(
        "".into(),
        None,
        Some(20),
        SearchFilters::default(),
    )
    .is_err());

    // A project with zero mappings (newly created) → empty results, NoMatch.
    let empty_project = create_project(
        &project_store,
        &CreateProjectRequest { name: "Empty".to_string() },
    )
    .unwrap();
    let empty_page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "".into(),
            None,
            Some(20),
            SearchFilters {
                tessera_project: Some(empty_project.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(empty_page.results().is_empty());
    assert_eq!(empty_page.empty_state(), Some(SearchEmptyState::NoMatch));

    // A project mapped to both Codex and Claude Code scopes → empty query
    // returns every in-scope record (4 in the fixture).
    let full_project = create_project(
        &project_store,
        &CreateProjectRequest { name: "Full".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: full_project.project_id.clone(),
            provider: "codex".to_string(),
            native_project: None,
        },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: full_project.project_id.clone(),
            provider: "claude_code".to_string(),
            native_project: Some("proj-claude".to_string()),
        },
    )
    .unwrap();
    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "".into(),
            None,
            Some(20),
            SearchFilters {
                tessera_project: Some(full_project.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    // The fixture's 4 records all match: 2 Codex (NULL) + 2 Claude (proj-claude).
    assert_eq!(page.results().len(), 4, "empty q + project returns every in-scope record");
    assert!(page.empty_state().is_none());
}

/// Story 5.2 — a search cursor binds `tessera_project`, so changing the
/// project filter mid-pagination surfaces `cursor_stale` (mirrors the v3
/// filter-binding pattern).
#[test]
fn tessera_project_filter_change_mid_pagination_is_cursor_stale() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let project_store = tessera_lib::index::ProjectStore::new(&conn);

    let project_a = create_project(
        &project_store,
        &CreateProjectRequest { name: "A".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project_a.project_id.clone(),
            provider: "codex".to_string(),
            native_project: None,
        },
    )
    .unwrap();
    let project_b = create_project(
        &project_store,
        &CreateProjectRequest { name: "B".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project_b.project_id.clone(),
            provider: "claude_code".to_string(),
            native_project: Some("proj-claude".to_string()),
        },
    )
    .unwrap();

    // Page 1 under project A.
    let first = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(1),
            SearchFilters {
                tessera_project: Some(project_a.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    let cursor = first.next_cursor().expect("page-1 cursor").to_string();

    // Continue with project B's filter → CursorStale (cursor bound project A).
    let err = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            Some(cursor),
            Some(1),
            SearchFilters {
                tessera_project: Some(project_b.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "mid-pagination project change must be CursorStale, got {err:?}"
    );
}

/// Story 5.2 — the AC's headline combination: a mapping-change mid-pagination
/// invalidates an outstanding cursor EVEN WHEN the request repeats the SAME
/// `tessera_project` filter. Distinct from
/// `tessera_project_filter_change_mid_pagination_is_cursor_stale` (which pins
/// the filter-change path: cursor bound project A, request project B): here
/// the cursor is bound to project A's pre-bump `current_index_revision`, and
/// the add-mapping bumps the shared `project_mapping_revision` (which folds
/// into `current_index_revision`), so the replayed cursor's bound revision
/// mismatches the live revision even though the project filter is unchanged.
/// This pins mapping-change × project-filter-set, the AC's load-bearing case
/// for AD-31 cursor invalidation under the Story 5.2 project projection.
#[test]
fn mapping_change_invalidates_cursor_with_same_project_filter() {
    use tessera_lib::application::{add_mapping, create_project};
    use tessera_lib::domain::project::{CreateProjectRequest, MappingRequest};
    let conn = build_filter_fixture();
    let registry = SourceRegistry::new(&conn);
    let project_store = tessera_lib::index::ProjectStore::new(&conn);

    // Create project A and map the Codex global scope (2 Codex records in
    // scope, so limit=1 leaves a page-2 cursor).
    let project_a = create_project(
        &project_store,
        &CreateProjectRequest { name: "A".to_string() },
    )
    .unwrap();
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project_a.project_id.clone(),
            provider: "codex".to_string(),
            native_project: None,
        },
    )
    .unwrap();

    // Page 1 under project A (limit 1 → has more, captures cursor).
    let first = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(1),
            SearchFilters {
                tessera_project: Some(project_a.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(first.results().len(), 1, "page 1 returns one in-scope Codex record");
    assert!(
        first.results()[0].provider() == "codex",
        "the in-scope record is a Codex record"
    );
    let cursor = first.next_cursor().expect("page-1 cursor").to_string();

    // Add a SECOND mapping to project A (the Codex global scope is already
    // mapped, so this is a genuine scope-set change, not an idempotent re-add).
    // The bump_project_mapping_revision call inside `add_mapping`'s
    // transaction bumps the shared revision that folds into
    // `current_index_revision`.
    add_mapping(
        &project_store,
        &MappingRequest {
            project_id: project_a.project_id.clone(),
            provider: "claude_code".to_string(),
            native_project: Some("proj-claude".to_string()),
        },
    )
    .unwrap();

    // Replay the cursor REPEATING THE SAME project A filter → CursorStale.
    // The cursor's bound revision predates the add-mapping bump, so the
    // revision mismatch surfaces — independent of the project filter, which
    // is unchanged. This is the AC's headline combination.
    let err = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            Some(cursor),
            Some(1),
            SearchFilters {
                tessera_project: Some(project_a.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "mapping-change mid-pagination with the SAME project filter must be CursorStale, got {err:?}"
    );

    // A fresh page-1 under the same project A filter still succeeds and now
    // returns the expanded scope (Codex + Claude = 4 records). This confirms
    // the cursor was rejected for a revision mismatch, not for any request
    // shape issue, and that the mapping change took effect on the live view.
    let fresh = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "federation".into(),
            None,
            Some(20),
            SearchFilters {
                tessera_project: Some(project_a.project_id.0.clone()),
                ..Default::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    let providers: std::collections::HashSet<&str> =
        fresh.results().iter().map(|r| r.provider()).collect();
    assert!(
        providers.contains("codex") && providers.contains("claude_code"),
        "fresh page-1 under project A must include both providers after the mapping add: {providers:?}"
    );
    assert_eq!(fresh.results().len(), 4, "expanded scope has all 4 records");
}

/// Story 2.4 I/O matrix — unknown provider / memory_type / negative `since`
/// are rejected at `SearchRequest::new_with_filters` (the HTTP layer maps this
/// to 400 `bad_request`).
#[test]
fn invalid_filter_values_are_rejected_at_request_construction() {
    // Unknown provider id.
    assert!(SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { provider: Some("bogus_provider".into()), ..Default::default() },
    ).is_err());
    // Negative `since`.
    assert!(SearchRequest::new_with_filters(
        "federation".into(), None, Some(20),
        SearchFilters { since: Some(-1), ..Default::default() },
    ).is_err());
    // `memory_type` is a typed enum, so the HTTP layer's `from_str` is the
    // rejection boundary — covered by http_api tests. Here we just confirm the
    // typed path does not itself invent a bogus variant.
    assert_eq!(ProviderMemoryType::parse_str("bogus_type"), None);
    assert_eq!(ProviderMemoryType::parse_str("memory"), Some(ProviderMemoryType::Memory));
}

/// Story 2.4 — `ProviderMemoryType::parse_str` is the exact reverse of `as_str`
/// for the whole vocabulary, so the filter contract has one source of truth.
#[test]
fn provider_memory_type_from_str_reverses_as_str_for_whole_vocabulary() {
    for variant in [
        ProviderMemoryType::Memory,
        ProviderMemoryType::MemorySummary,
        ProviderMemoryType::RawMemories,
        ProviderMemoryType::RolloutSummary,
        ProviderMemoryType::TopicMemory,
    ] {
        assert_eq!(
            ProviderMemoryType::parse_str(variant.as_str()),
            Some(variant),
            "from_str(as_str({:?})) must round-trip",
            variant,
        );
    }
    assert_eq!(ProviderMemoryType::parse_str(""), None);
    assert_eq!(ProviderMemoryType::parse_str("MEMORY"), None, "parse_str is case-sensitive");
}

/// Patch 11 — `KNOWN_PROVIDER_IDS` is an explicit allowlist in `domain`, which
/// cannot import the adapter registry without breaking the hexagonal rule
/// (`domain` depends only on its own ports; `adapters` implement them). The
/// allowlist can therefore drift from the adapters' `PROVIDER_ID` constants.
/// This test fails if the two desync: a missing adapter id would make a valid
/// provider filter reject as `bad_request`, and a stale allowlist entry would
/// let the UI offer a provider filter that always yields zero rows.
#[test]
fn known_provider_ids_match_registered_adapters() {
    use tessera_lib::adapters::claude_code::ClaudeCodeAdapter;
    use tessera_lib::adapters::codex::CodexAdapter;
    let adapter_ids = [CodexAdapter::PROVIDER_ID, ClaudeCodeAdapter::PROVIDER_ID];
    // Every registered adapter's id is in the allowlist.
    for id in adapter_ids.iter() {
        assert!(
            KNOWN_PROVIDER_IDS.contains(id),
            "adapter PROVIDER_ID {id:?} must be in KNOWN_PROVIDER_IDS"
        );
    }
    // The allowlist carries no id that no adapter claims.
    for id in KNOWN_PROVIDER_IDS.iter() {
        assert!(
            adapter_ids.contains(id),
            "KNOWN_PROVIDER_IDS entry {id:?} must map to a registered adapter"
        );
    }
}
