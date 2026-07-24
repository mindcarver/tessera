use std::fs;
use std::time::Instant;

use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::application;
use tessera_lib::application::query::QueryError;
use tessera_lib::domain::ports::provider_adapter::{CoverageLevel, DiscoveryBasis};
use tessera_lib::domain::query::{SearchEmptyState, SearchRequest};
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
fn committed_local_codex_fixture_has_short_cjk_recall_without_printing_source_text() {
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
        let started = Instant::now();
        let page = application::search(&registry, &conn, SearchRequest::new(query.into(), None, None).unwrap()).unwrap();
        assert!(!page.results().is_empty(), "short-CJK recall must be non-zero");
        let latency_us = started.elapsed().as_micros();
        assert!(latency_us > 0);
        eprintln!("search_fixture_query_bytes={} latency_us={latency_us}", query.len());
    }

    let benchmark: serde_json::Value = serde_json::from_str(include_str!("benchmarks/memory-index.json")).unwrap();
    assert_eq!(benchmark["thresholds"]["recall"], serde_json::Value::Null);
    assert_eq!(benchmark["thresholds"]["empty_result_rate"], serde_json::Value::Null);
    assert_eq!(benchmark["thresholds"]["latency_us"], serde_json::Value::Null);
}
