use std::fs;
use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::application;
use tessera_lib::application::query::QueryError;
use tessera_lib::domain::ports::provider_adapter::{CoverageLevel, DiscoveryBasis};
use tessera_lib::domain::query::{SearchEmptyState, SearchRequest, SourceQueryStatusKind};
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
    let generation = format!("gen_{source_rowid}");
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
         VALUES (?1, ?2, ?3, ?4, 'section', ?1, ?5, 'hash', 'v1', ?6, ?7, ?8, 'memory', ?9, ?10, 'revision', ?5)",
        params![
            record_id,
            source_rowid,
            generation,
            provider,
            format!("file:///fixture#{record_id}"),
            title,
            body,
            native_project,
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
