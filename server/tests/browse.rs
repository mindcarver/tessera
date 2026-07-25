//! Story 3.1 / 3.2 — `application::browse` contract tests.
//!
//! Mirrors `server/tests/search.rs`'s structure: an in-memory fixture builder,
//! a confirmed-source-only read path, cursor stability + staleness, the three
//! distinct empty states, and the lifecycle-exclusion boundary (raw chat /
//! transcript / human-instruction files / unconfirmed sources never appear).
//! Story 3.2 adds the memory-type filter contract: filtered happy path,
//! filter-narrows-to-zero, stale cursor on filter change, and the legacy
//! `b3.` cursor rejection (envelope prefix bumped `b3.` → `b4.` so the
//! filter could bind into the cursor). Wire-level (HTTP) coverage lives in
//! `server/tests/http_api.rs`.

use rusqlite::{params, Connection};

use tessera_lib::application;
use tessera_lib::application::query::QueryError;
use tessera_lib::domain::ports::provider_adapter::ProviderMemoryType;
use tessera_lib::domain::ports::query_store::QueryStore;
use tessera_lib::domain::query::{BrowseEmptyState, BrowseRequest};
use tessera_lib::domain::source::SourceId;
use tessera_lib::index::{migrations, SourceRegistry};

/// Build an in-memory DB with migrations applied. The caller seeds sources and
/// records specific to each test.
fn empty_db() -> Connection {
    let mut conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
    migrations::apply(&mut conn).unwrap();
    conn
}

/// Insert a confirmed source row at `rowid`, optionally with an active
/// generation. Mirrors `search.rs::insert_confirmed_source` so the fixtures
/// stay consistent across the two test files.
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
    )
    .unwrap();
    if active {
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (?1, ?2, 'succeeded', 1, ?2, 'fixture')",
            params![rowid, format!("gen_{rowid}")],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES (?1, ?2)",
            params![format!("active_generation:{rowid}"), format!("gen_{rowid}")],
        )
        .unwrap();
    }
}

/// Insert a memory record under a source's active generation. Mirrors
/// `search.rs::insert_record` so the two files produce identical row shapes.
/// `memory_type` defaults to `"memory"` (the most common shape) so existing
/// callers keep their pre-3.2 fixtures unchanged.
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
    insert_record_with_memory_type(
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

/// Story 3.2 — insert a memory record with an explicit `provider_memory_type`
/// so the memory-type filter has a discriminating fixture. Defaults live in
/// `insert_record`; this helper exists only for tests that need a non-default
/// type.
#[allow(clippy::too_many_arguments)]
fn insert_record_with_memory_type(
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
    )
    .unwrap();
}

// ---------------------------------------------------------------------------
// Happy path + pagination
// ---------------------------------------------------------------------------

/// Story 3.1 AC — a confirmed, successfully-scanned source with records
/// returns a paginated list from `browse(source_id)`. Deterministic order is
/// `observed_at DESC → coverage_full → record_id ASC` (drops search's
/// `title_match`). The cursor carries the three sort keys so pagination is
/// stable across recency/coverage tiers.
#[test]
fn browse_returns_paginated_results_in_deterministic_order() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Three records: distinct observed_at so the recency tier alone orders
    // them; coverage and record_id are tiebreakers exercised by later tests.
    insert_record(&conn, "rec_old", 1, "codex", "older title", "body a", 100, "full", None);
    insert_record(&conn, "rec_new", 1, "codex", "newest title", "body b", 300, "full", None);
    insert_record(&conn, "rec_mid", 1, "codex", "middle title", "body c", 200, "full", None);

    let registry = SourceRegistry::new(&conn);
    // Page 1 (limit 1): newest observed_at first.
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(page1.results().len(), 1);
    assert_eq!(page1.results()[0].record_id(), "rec_new");
    assert!(page1.empty_state().is_none(), "happy path has no empty_state");
    assert!(page1.next_cursor().is_some());

    // The sidecar carries one row per confirmed source.
    assert_eq!(page1.sources().len(), 1);
    assert_eq!(page1.sources()[0].source_id.0, "src_1");

    // Page 2 (limit 1, same cursor): middle observed_at.
    let cursor = page1.next_cursor().unwrap().to_string();
    let page2 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(page2.results()[0].record_id(), "rec_mid");
    assert!(page2.next_cursor().is_some());

    // Page 3 (limit 1, same cursor): oldest observed_at. No more after.
    let cursor = page2.next_cursor().unwrap().to_string();
    let page3 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(page3.results()[0].record_id(), "rec_old");
    assert!(page3.next_cursor().is_none());
}

/// Story 3.1 — pagination via the cursor stays stable across recency/coverage
/// tiers. A record whose `record_id` sorts BEFORE the cursor but whose
/// `observed_at` is worse must NOT be skipped (mirrors search's
/// `relevance_ordering_with_pagination_does_not_skip_lower_record_ids`).
#[test]
fn browse_pagination_does_not_skip_records_with_lower_record_ids() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // A "newer" record with a low id; the cursor after page 1 (the high-id
    // record) would skip this under a naive `record_id > cursor` predicate.
    insert_record(&conn, "rec_a", 1, "codex", "older but low id", "body", 100, "full", None);
    // The "newer" record with a high id sorts first by observed_at DESC.
    insert_record(&conn, "rec_z", 1, "codex", "newer but high id", "body", 200, "full", None);

    let registry = SourceRegistry::new(&conn);
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(page1.results()[0].record_id(), "rec_z");

    let cursor = page1.next_cursor().unwrap().to_string();
    let page2 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap();
    // Must NOT be skipped despite rec_a's id sorting BEFORE the cursor.
    assert_eq!(page2.results()[0].record_id(), "rec_a");
}

// ---------------------------------------------------------------------------
// The three empty states
// ---------------------------------------------------------------------------

/// Story 3.1 AC — confirmed source with no active generation and no successful
/// scan returns `empty_state = NotYetScanned` on page 1.
#[test]
fn browse_empty_not_yet_scanned_for_confirmed_unscanned_source() {
    let conn = empty_db();
    // Confirmed but never scanned — no scan_runs row, no active generation.
    insert_confirmed_source(&conn, 1, "codex", "unknown", None, false);

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, None).unwrap(),
    )
    .unwrap();
    assert!(page.results().is_empty());
    assert_eq!(page.empty_state(), Some(BrowseEmptyState::NotYetScanned));
}

/// Story 3.1 AC — confirmed source that scanned successfully (active
/// generation present) but indexed ZERO records returns
/// `empty_state = NoIndexableMemory`.
#[test]
fn browse_empty_no_indexable_memory_for_successfully_scanned_empty_source() {
    let conn = empty_db();
    // Active generation committed, but no memory_records rows under it.
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, None).unwrap(),
    )
    .unwrap();
    assert!(page.results().is_empty());
    assert_eq!(page.empty_state(), Some(BrowseEmptyState::NoIndexableMemory));
}

/// Story 3.1 AC — confirmed source whose latest run is Failed with no usable
/// active generation returns `empty_state = SourceUnavailable`.
#[test]
fn browse_empty_source_unavailable_for_failed_scan_without_active_generation() {
    let conn = empty_db();
    // Confirmed but the latest (and only) run failed with no active generation
    // committed.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (1, 'codex', 'agent_memory', 'confirmed', 'error', 'full', '/fixture-1', '/fixture-1', NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, error_code)
         VALUES (1, 'gen_1', 'failed', 1, 'gen_1', 'fixture', 'enumeration_failed')",
        [],
    )
    .unwrap();

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, None).unwrap(),
    )
    .unwrap();
    assert!(page.results().is_empty());
    assert_eq!(page.empty_state(), Some(BrowseEmptyState::SourceUnavailable));
}

/// Story 3.1 — empty_state is computed ONLY on page 1 (no cursor). A
/// continuation page that returns zero rows (the source's last record was on
/// the previous page) carries no `empty_state`.
#[test]
fn browse_empty_state_is_absent_when_results_present() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // Page 1 returns the only record → results present → no empty_state, even
    // though it IS page 1 (the empty-state computation only fires when the
    // initial page has zero results).
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(20)).unwrap(),
    )
    .unwrap();
    assert_eq!(page1.results().len(), 1);
    assert!(page1.next_cursor().is_none());
    assert!(page1.empty_state().is_none());
}

// ---------------------------------------------------------------------------
// Cursor staleness + cross-type rejection
// ---------------------------------------------------------------------------

/// Story 3.1 AC — pagination mid-flight and the browsed source's generation
/// changes → `409 cursor_stale`. The cursor binds to the index revision
/// (FNV-1a over confirmed sources' active generations), so any generation
/// change invalidates it.
#[test]
fn browse_cursor_becomes_stale_after_a_new_generation_activates() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);
    insert_record(&conn, "rec_b", 1, "codex", "title2", "body2", 200, "full", None);

    let registry = SourceRegistry::new(&conn);
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    let cursor = page1.next_cursor().expect("first page cursor").to_string();

    // Activate a new generation under src_1: the revision digest changes.
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (1, 'gen_99', 'succeeded', 2, 'gen_99', 'fixture')",
        [],
    )
    .unwrap();
    conn.execute(
        "UPDATE tessera_meta SET value = 'gen_99' WHERE key = 'active_generation:1'",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
         VALUES ('rec_new', 1, 'gen_99', 'codex', 'section', 'rec_new', 'file:///fixture#rec_new', 'hash', 'v1', 'new title', 'new body', NULL, 'memory', 'full', 300, 'revision', 'file:///fixture#L1-L2')",
        [],
    )
    .unwrap();

    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale));

    // A fresh page-1 request under the new generation returns the new record.
    let fresh = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(20)).unwrap(),
    )
    .unwrap();
    assert_eq!(fresh.results().len(), 1);
    assert_eq!(fresh.results()[0].record_id(), "rec_new");
}

/// Story 3.1 — a cross-source cursor (issued under src_1, replayed against
/// src_2) is rejected as `CursorStale`, mirroring search's filter-mismatch
/// path. Two confirmed sources with records; the cursor from src_1 must not
/// page through src_2's records.
#[test]
fn browse_cursor_is_bound_to_its_source() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_confirmed_source(&conn, 2, "claude_code", "healthy", Some("proj-a"), true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);
    insert_record(&conn, "rec_b", 1, "codex", "title2", "body2", 200, "full", None);
    insert_record(&conn, "rec_c", 2, "claude_code", "claude title", "body", 100, "full", Some("proj-a"));

    let registry = SourceRegistry::new(&conn);
    let page1_src1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    let cursor = page1_src1.next_cursor().unwrap().to_string();

    // Replaying src_1's cursor against src_2 → CursorStale.
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_2".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "cross-source cursor must be CursorStale, got {err:?}");
}

/// Story 3.1 — a search cursor (`v3.<hex>`) handed to the browse endpoint is
/// rejected as `CursorStale`, NOT `BadRequest`, mirroring search's v1/v2
/// rejection choice. The UI's existing `cursor_stale` recovery path re-runs
/// page 1, which is the correct outcome under a cross-type cursor swap.
#[test]
fn browse_rejects_search_cursor_as_stale_not_bad_request() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // Hand-craft a search-shaped cursor (v3 envelope). The browse endpoint
    // must reject it as CursorStale before any decode.
    let payload = r#"{"version":3,"query":"federation","revision":"deadbeef","last_record_id":"rec_a","last_title_match":false,"last_observed_at":0,"last_coverage_full":false,"provider":null,"source":null,"memory_type":null,"native_project":null,"since":null}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let search_cursor = format!("v3.{hex}");
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(search_cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "cross-type cursor must be CursorStale, got {err:?}");
}

// ---------------------------------------------------------------------------
// Lifecycle exclusion + non-confirmed-source rejection
// ---------------------------------------------------------------------------

/// Story 3.1 I/O matrix — a non-confirmed (disabled/rejected/unknown) source
/// surfaces as `BadRequest`, never as an empty state. Without this check the
/// SQL layer's `lifecycle_state = 'confirmed'` JOIN would yield zero rows
/// and the orchestrator would render `NotYetScanned`, hiding the real
/// lifecycle state.
#[test]
fn browse_rejects_non_confirmed_source_as_bad_request() {
    let conn = empty_db();
    // A disabled source with records — must NOT be browsable.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (1, 'codex', 'agent_memory', 'disabled', 'unknown', 'full', '/fixture-1', '/fixture-1', NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
         VALUES ('rec_disabled', 1, 'gen_1', 'codex', 'section', 'rec_disabled', 'file:///fixture#rec_disabled', 'hash', 'v1', 'hidden', 'hidden', NULL, 'memory', 'full', 100, 'revision', 'file:///fixture#L1-L2')",
        [],
    )
    .unwrap();
    // Also a rejected source.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (2, 'codex', 'agent_memory', 'rejected', 'unknown', 'full', '/fixture-2', '/fixture-2', NULL)",
        [],
    )
    .unwrap();

    let registry = SourceRegistry::new(&conn);
    // Disabled source → BadRequest.
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, None).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::BadRequest), "disabled source must be BadRequest, got {err:?}");

    // Rejected source → BadRequest.
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_2".into()), None, None).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::BadRequest), "rejected source must be BadRequest, got {err:?}");

    // Unknown source → BadRequest.
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_99".into()), None, None).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::BadRequest), "unknown source must be BadRequest, got {err:?}");
}

/// Story 3.1 — browse's confirmed-source scoping means a disabled source's
/// records NEVER appear in a browse list even when the disabled source has an
/// active generation. This pins the "raw chat / transcript / human-
/// instruction files / unconfirmed-source records never appear in browse"
/// boundary at the application layer (the SQL JOIN on lifecycle_state is the
/// load-bearing guarantee; this test pins its observable effect).
#[test]
fn browse_excludes_records_from_non_confirmed_sources() {
    let conn = empty_db();
    // A confirmed source (src_1) with one record.
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_visible", 1, "codex", "title", "body", 100, "full", None);
    // A disabled source (src_2) with an active generation and a record that
    // shares provider + similar content — must never appear under browse.
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (2, 'codex', 'agent_memory', 'disabled', 'unknown', 'full', '/fixture-2', '/fixture-2', NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (2, 'gen_2', 'succeeded', 1, 'gen_2', 'fixture')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:2', 'gen_2')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
         VALUES ('rec_hidden', 2, 'gen_2', 'codex', 'section', 'rec_hidden', 'file:///fixture#rec_hidden', 'hash', 'v1', 'hidden title', 'hidden body', NULL, 'memory', 'full', 100, 'revision', 'file:///fixture#L1-L2')",
        [],
    )
    .unwrap();

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(20)).unwrap(),
    )
    .unwrap();
    // Only src_1's record is visible.
    let ids: Vec<&str> = page.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids, vec!["rec_visible"]);
}

// ---------------------------------------------------------------------------
// Bad input
// ---------------------------------------------------------------------------

/// Story 3.1 I/O matrix — a malformed source handle, missing source, or
/// invalid limit is rejected at `BrowseRequest::new` (the HTTP layer maps
/// this to 400 `bad_request`).
#[test]
fn browse_rejects_invalid_input_at_request_construction() {
    // Malformed source handle.
    assert!(BrowseRequest::new(SourceId("not-a-source".into()), None, None).is_err());
    assert!(BrowseRequest::new(SourceId("src_".into()), None, None).is_err());
    // Zero/negative rowid.
    assert!(BrowseRequest::new(SourceId("src_0".into()), None, None).is_err());
    assert!(BrowseRequest::new(SourceId("src_-5".into()), None, None).is_err());
    // limit = 0.
    assert!(BrowseRequest::new(SourceId("src_1".into()), None, Some(0)).is_err());
    // limit > MAX_SEARCH_LIMIT.
    assert!(BrowseRequest::new(SourceId("src_1".into()), None, Some(10_000)).is_err());
    // Oversized cursor.
    let huge = "x".repeat(tessera_lib::domain::query::MAX_CURSOR_BYTES + 1);
    assert!(BrowseRequest::new(SourceId("src_1".into()), Some(huge), None).is_err());
}

// ---------------------------------------------------------------------------
// Empty-state derivation edge cases (review pass)
// ---------------------------------------------------------------------------

/// Story 3.1 — a confirmed source whose latest run is `succeeded` but which
/// activated NO generation (`complete_without_activation` path: state set to
/// `succeeded` without an `active_generation` meta key) maps to
/// `NoIndexableMemory`, NOT `NotYetScanned`. The scan ran and succeeded; it
/// simply has no activatable Agent Memory, so "not yet scanned" would be
/// dishonest.
#[test]
fn browse_empty_no_indexable_memory_for_succeeded_run_without_activation() {
    let conn = empty_db();
    conn.execute(
        "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
         VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-1', '/fixture-1', NULL)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
         VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
        [],
    )
    .unwrap();
    // Deliberately NO `active_generation:1` meta row (mirrors
    // `complete_without_activation`).

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, None).unwrap(),
    )
    .unwrap();
    assert!(page.results().is_empty());
    assert_eq!(
        page.empty_state(),
        Some(BrowseEmptyState::NoIndexableMemory),
        "succeeded run without activation must be NoIndexableMemory, not NotYetScanned"
    );
}

// ---------------------------------------------------------------------------
// Cursor version + decode edge cases (review pass)
// ---------------------------------------------------------------------------

/// Story 3.1 / 3.2 — same-prefix integrity backstop. A `b4.`-prefixed cursor
/// whose inner `version` is not the current browse cursor version is rejected
/// as `CursorStale`. This is NOT the cross-version forward-compat path (a
/// real future-version client emits a different envelope prefix, e.g. `b5.`,
/// and is rejected by the prefix gate before decode — see
/// `browse_rejects_future_prefix_cursor_as_stale`). The inner `version`
/// check only ever sees `version == 4` in practice; a value of 99 here means
/// a hand-edited same-prefix cursor, and surfacing it as `CursorStale` keeps
/// the recovery path uniform. (Story 3.2 bumped the envelope prefix `b3.` →
/// `b4.` and the inner version 3 → 4 together.)
#[test]
fn browse_rejects_future_version_cursor_as_stale() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // A well-formed b4. envelope whose inner version is 99 (a hand-edited
    // same-prefix cursor; a real future-version client would emit `b5.`).
    let payload = r#"{"version":99,"source":"src_1","revision":"deadbeef","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false,"memory_type":null}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let future_cursor = format!("b4.{hex}");
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(future_cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "future-version cursor must be CursorStale, got {err:?}");
}

/// Story 3.2 — the CROSS-VERSION / CROSS-TYPE forward-compat boundary is the
/// envelope PREFIX GATE, not the inner `version` field. A `b5.`-prefixed
/// cursor (any inner body — a real future-version client's envelope) is
/// rejected as `CursorStale` before decode runs, so the UI's existing
/// recovery path re-runs page 1. This is the load-bearing forward-compat
/// test (the same-prefix `version:99` case above only proves the backstop).
#[test]
fn browse_rejects_future_prefix_cursor_as_stale() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // A `b5.`-prefixed envelope with a syntactically valid v4 body. The
    // prefix gate rejects it as CursorStale before any decode is attempted,
    // proving the prefix — not the inner version — is the forward-compat
    // boundary a real future-version client would hit.
    let payload = r#"{"version":5,"source":"src_1","revision":"deadbeef","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false,"memory_type":null}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let future_cursor = format!("b5.{hex}");
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(future_cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "future-prefix b5. cursor must be CursorStale, got {err:?}");
}

/// Story 3.2 — a 3.1-era `b3.` cursor (any payload) is rejected as
/// `CursorStale` at the prefix check (the envelope moved to `b4.` so the
/// filter could bind in), mirroring search's v1/v2 rejection choice and the
/// cross-type rejection of a search `v3.` cursor. The UI's existing
/// `cursor_stale` recovery path re-runs page 1, which is the correct outcome
/// for a client whose cursor predates the contract change.
#[test]
fn browse_rejects_legacy_b3_cursor_as_stale() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // A well-formed b3. envelope from a 3.1 client (carries the v3 body, no
    // memory_type slot). The prefix check rejects it as CursorStale before
    // any decode is attempted.
    let payload = r#"{"version":3,"source":"src_1","revision":"deadbeef","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let legacy_cursor = format!("b3.{hex}");
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(legacy_cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "legacy b3. cursor must be CursorStale, got {err:?}");
}

/// Story 3.1 / 3.2 — a `b4.`-prefixed cursor that fails to decode (invalid
/// hex / empty payload) is rejected as `BadRequest`. A truncated or corrupted
/// cursor (URL editing, log paste) must surface the right error code.
#[test]
fn browse_rejects_malformed_cursor_as_bad_request() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    for malformed in ["b4.zzz", "b4.", "b4.zz"] {
        let err = application::browse(
            &registry,
            &conn,
            BrowseRequest::new(SourceId("src_1".into()), Some(malformed.into()), Some(1)).unwrap(),
        )
        .unwrap_err();
        assert!(matches!(err, QueryError::BadRequest), "malformed cursor {malformed:?} must be BadRequest, got {err:?}");
    }
}

// ---------------------------------------------------------------------------
// Coverage-tier tiebreak in ORDER BY (review pass)
// ---------------------------------------------------------------------------

/// Story 3.1 — the `coverage_full` tier in the ORDER BY (`full` before
/// `search_only` at equal `observed_at`) is load-bearing for stable
/// pagination. Two records sharing `observed_at` but differing coverage must
/// order `full` first, and the cursor must cross that boundary without
/// skipping or duplicating rows.
#[test]
fn browse_orders_full_coverage_ahead_of_search_only_at_equal_observed_at() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Same observed_at; one full, one search_only. `full` must sort first.
    insert_record(&conn, "rec_search_only", 1, "codex", "so title", "body", 200, "search_only", None);
    insert_record(&conn, "rec_full", 1, "codex", "full title", "body", 200, "full", None);

    let registry = SourceRegistry::new(&conn);
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        page1.results()[0].record_id(),
        "rec_full",
        "full coverage sorts ahead of search_only at equal observed_at"
    );

    let cursor = page1.next_cursor().expect("continuation cursor").to_string();
    let page2 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(cursor), Some(1)).unwrap(),
    )
    .unwrap();
    assert_eq!(
        page2.results()[0].record_id(),
        "rec_search_only",
        "search_only record follows across the coverage boundary without skip/dup"
    );
    assert!(page2.next_cursor().is_none());
}

// ---------------------------------------------------------------------------
// Story 3.2 — memory-type filter on the application::browse contract.
//
// The filter narrows the browse WHERE with `AND m.provider_memory_type = ?`,
// mirroring Search's predicate shape. The cursor binds the in-effect filter
// (bumping the envelope `b3.` → `b4.` so the filter could bind into the
// cursor), so a filter change mid-pagination surfaces `CursorStale`. A
// memory-type-filtered browse that narrows to zero results returns
// `BrowseEmptyState::NoIndexableMemory` on page 1 (the same state 3.1 uses
// for "scanned, zero records").
// ---------------------------------------------------------------------------

/// Story 3.2 AC — `browse(memory_type = Some(...))` narrows the list to records
/// of that `provider_memory_type` only, the cursor binds the filter, and "Load
/// more" continues within the same filtered snapshot.
#[test]
fn browse_memory_type_filter_narrows_results_and_binds_cursor() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Two memory records + one topic_memory record. The topic_memory record is
    // the only one the filter must keep.
    insert_record_with_memory_type(&conn, "rec_mem_a", 1, "codex", "memory a", "body", 100, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_topic", 1, "codex", "topic title", "body", 200, "full", None, "topic_memory");
    insert_record_with_memory_type(&conn, "rec_mem_b", 1, "codex", "memory b", "body", 300, "full", None, "memory");

    let registry = SourceRegistry::new(&conn);
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(20),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap();
    // Only the topic_memory record matches; no empty_state (results present).
    assert_eq!(page1.results().len(), 1);
    assert_eq!(page1.results()[0].record_id(), "rec_topic");
    assert!(page1.empty_state().is_none());

    // No filter → all three records return (filter is purely additive).
    let unfiltered = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(20)).unwrap(),
    )
    .unwrap();
    assert_eq!(unfiltered.results().len(), 3);
}

/// Story 3.2 AC — a memory-type-filtered browse with zero matching records on
/// page 1 returns `BrowseEmptyState::NoIndexableMemory`. The contract cannot
/// distinguish "this source has no records of that type" from "this source has
/// no indexable memory" — both are a zero-row first page on a scanned-OK
/// source, so they reuse the same state (no fourth state is added).
#[test]
fn browse_memory_type_filter_narrows_to_zero_returns_no_indexable_memory() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Only memory-type records; filter for topic_memory yields zero.
    insert_record_with_memory_type(&conn, "rec_mem_a", 1, "codex", "memory a", "body", 100, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_mem_b", 1, "codex", "memory b", "body", 200, "full", None, "memory");

    let registry = SourceRegistry::new(&conn);
    let page = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(20),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(page.results().is_empty());
    assert_eq!(page.empty_state(), Some(BrowseEmptyState::NoIndexableMemory));
}

/// Story 3.2 AC — pagination under a memory-type filter stays within the same
/// filtered snapshot. The cursor binds the filter so "Load more" continues
/// past the first page, never leaking records of other types in.
#[test]
fn browse_memory_type_filter_paginates_within_filtered_snapshot() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Three topic_memory records (so the filter has >1 page under limit=2)
    // interleaved with memory records the filter must exclude on every page.
    insert_record_with_memory_type(&conn, "rec_topic_old", 1, "codex", "t1", "body", 100, "full", None, "topic_memory");
    insert_record_with_memory_type(&conn, "rec_mem_a", 1, "codex", "m1", "body", 150, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_topic_mid", 1, "codex", "t2", "body", 200, "full", None, "topic_memory");
    insert_record_with_memory_type(&conn, "rec_mem_b", 1, "codex", "m2", "body", 250, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_topic_new", 1, "codex", "t3", "body", 300, "full", None, "topic_memory");

    let registry = SourceRegistry::new(&conn);
    let page1 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(2),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap();
    // ORDER BY observed_at DESC → newest topic_memory records first.
    let ids: Vec<&str> = page1.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids, vec!["rec_topic_new", "rec_topic_mid"], "filter narrows AND ordering preserved: {ids:?}");

    // Continuation must stay inside the filtered snapshot (the cursor binds
    // memory_type so no memory record leaks in on page 2).
    let cursor = page1.next_cursor().expect("continuation cursor").to_string();
    let page2 = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(cursor),
            Some(2),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap();
    let ids2: Vec<&str> = page2.results().iter().map(|r| r.record_id()).collect();
    assert_eq!(ids2, vec!["rec_topic_old"], "page 2 stays in the filtered snapshot: {ids2:?}");
    assert!(page2.next_cursor().is_none(), "no third page");
}

/// Story 3.2 AC — a cursor bound to one `memory_type` is rejected as
/// `CursorStale` when the next-page request carries a different
/// `memory_type`. Mirrors Search's "resolve filter once on page 1" invariant:
/// the cursor's result set no longer corresponds to the request.
///
/// The test is SYMMETRIC across both replay directions (memory→topic_memory
/// AND topic_memory→memory), plus the unfiltered→filtered case. The fixture
/// has TWO records of each type so a `limit=1` page-1 under either filter
/// yields a continuation cursor (one record on page 1, one on page 2).
#[test]
fn browse_memory_type_filter_change_invalidates_cursor_as_stale() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    // Two memory records + two topic_memory records so EITHER filter under
    // `limit=1` yields a continuation cursor (page 1 returns one record,
    // page 2 returns the other, no third page).
    insert_record_with_memory_type(&conn, "rec_mem_a", 1, "codex", "m1", "body", 100, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_mem_b", 1, "codex", "m2", "body", 200, "full", None, "memory");
    insert_record_with_memory_type(&conn, "rec_topic_a", 1, "codex", "t1", "body", 300, "full", None, "topic_memory");
    insert_record_with_memory_type(&conn, "rec_topic_b", 1, "codex", "t2", "body", 400, "full", None, "topic_memory");

    let registry = SourceRegistry::new(&conn);

    // Direction 1: cursor issued under memory_type=memory, replayed under
    // memory_type=topic_memory → CursorStale.
    let page1_memory = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(1),
            Some(ProviderMemoryType::Memory),
        )
        .unwrap(),
    )
    .unwrap();
    let memory_cursor = page1_memory
        .next_cursor()
        .expect("memory-filtered continuation cursor")
        .to_string();
    // Sanity: the cursor really was issued under memory_type=memory (b4.
    // envelope; the body's memory_type slot is exercised by the smuggled-
    // value test below).
    assert!(memory_cursor.starts_with("b4."));
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(memory_cursor),
            Some(1),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "memory→topic_memory replay must invalidate cursor, got {err:?}",
    );

    // Direction 2 (the previously-abandoned one): cursor issued under
    // memory_type=topic_memory, replayed under memory_type=memory →
    // CursorStale. Now reachable because the fixture has two topic records,
    // so a topic-filtered page-1 with limit=1 yields a continuation cursor.
    let page1_topic = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(1),
            Some(ProviderMemoryType::TopicMemory),
        )
        .unwrap(),
    )
    .unwrap();
    let topic_cursor = page1_topic
        .next_cursor()
        .expect("topic-filtered continuation cursor")
        .to_string();
    assert!(topic_cursor.starts_with("b4."));
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(topic_cursor),
            Some(1),
            Some(ProviderMemoryType::Memory),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "topic_memory→memory replay must invalidate cursor, got {err:?}",
    );

    // Direction 3: cursor issued unfiltered (memory_type = null), replayed
    // under memory_type=memory → CursorStale (the cursor's `memory_type`
    // slot is `None`, which normalizes to `None` on the comparison path and
    // cannot equal the request's `Some(Memory)`).
    let page1_unfiltered = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), None, Some(1)).unwrap(),
    )
    .unwrap();
    let unfiltered_cursor = page1_unfiltered
        .next_cursor()
        .expect("unfiltered continuation cursor")
        .to_string();
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(unfiltered_cursor),
            Some(1),
            Some(ProviderMemoryType::Memory),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "unfiltered→filtered replay must invalidate cursor, got {err:?}",
    );
}

/// Story 3.2 — `BrowseRequest::new_with_memory_type` accepts any of the
/// vocabulary types (mirrors Search's `memory_type` acceptance); an unknown
/// value is rejected at the HTTP layer via `ProviderMemoryType::parse_str`
/// (covered in http_api). Application-layer pin: every variant produces a
/// well-formed request and the resulting SQL short-circuits correctly.
#[test]
fn browse_memory_type_filter_accepts_every_vocabulary_variant() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_mem", 1, "codex", "title", "body", 100, "full", None);

    for variant in [
        ProviderMemoryType::Memory,
        ProviderMemoryType::MemorySummary,
        ProviderMemoryType::RawMemories,
        ProviderMemoryType::RolloutSummary,
        ProviderMemoryType::TopicMemory,
    ] {
        let registry = SourceRegistry::new(&conn);
        let request = BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            None,
            Some(20),
            Some(variant),
        )
        .unwrap();
        // memory_type() round-trips the typed value back to the caller.
        assert_eq!(request.memory_type(), Some(variant));
        let page = application::browse(&registry, &conn, request).unwrap();
        // Only `Memory` matches the fixture's default record; every other
        // variant narrows to zero rows. No variant errors out.
        let expected_count = if matches!(variant, ProviderMemoryType::Memory) { 1 } else { 0 };
        assert_eq!(
            page.results().len(),
            expected_count,
            "variant {:?} narrowed to {expected_count} row(s)",
            variant,
        );
    }
}

/// Story 3.2 (review pass) — a `b4.` cursor whose body carries an UNKNOWN
/// `memory_type` value (hand-edited or from a buggy client) is funneled to
/// `CursorStale`, NOT `BadRequest`. The decode path performs only
/// structural/length validation; the vocabulary check is normalized through
/// `ProviderMemoryType::parse_str` on the comparison path in `browse()`,
/// where an unknown stored value becomes `None`. `None` cannot equal a valid
/// request filter, so the cursor is rejected as `CursorStale` — the SAME
/// recovery UX as a legitimate filter change (re-run page 1).
///
/// Mirrors search's `decode_cursor_rejects_tampered_bound_filters` shape but
/// asserts the post-P2 funnel: both "smuggled unknown" and "legitimate
/// filter change" surface as `CursorStale`, not split across `BadRequest`
/// and `CursorStale`.
#[test]
fn browse_cursor_with_smuggled_memory_type_is_cursor_stale_not_bad_request() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);

    // To exercise the memory_type comparison path specifically (rather than
    // the source/revision check, which would mask it), the cursor's revision
    // must MATCH the live index revision. Read it from the store the same
    // way `browse()` does.
    use tessera_lib::index::scan_store::ScanStore;
    let store = ScanStore::new(&conn);
    let live_revision = store.current_index_revision().expect("revision");

    // Hex-encode a `b4.` cursor body by hand (not via `encode_browse_cursor`)
    // so we can inject values the request constructor would reject upstream.
    // The body's `memory_type` slot is parameterized; every other field is a
    // fixed valid baseline matching the request (src_1, live revision, rec_a).
    fn envelope(body: &str) -> String {
        let hex: String = body.bytes().map(|byte| format!("{byte:02x}")).collect();
        format!("b4.{hex}")
    }
    let body_with = |memory_type: Option<&str>| -> String {
        let slot = match memory_type {
            Some(v) => format!("\"memory_type\":\"{v}\""),
            None => "\"memory_type\":null".to_string(),
        };
        format!(
            r#"{{"version":4,"source":"src_1","revision":"{rev}","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false,{slot}}}"#,
            rev = live_revision,
        )
    };

    // Baseline sanity #1: a cursor with NO memory_type, replayed against an
    // unfiltered request, decodes and matches → no error (proves the helper
    // is well-formed so the rejection cases below are meaningful).
    let baseline_outcome = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(
            SourceId("src_1".into()),
            Some(envelope(&body_with(None))),
            Some(1),
        )
        .unwrap(),
    );
    assert!(
        baseline_outcome.is_ok(),
        "baseline (no filter, unfiltered request) must succeed; helper is malformed otherwise: {:?}",
        baseline_outcome.err(),
    );

    // Baseline sanity #2 (P6's known-good case): a cursor with
    // `"memory_type":"memory"` replayed against a `memory_type=memory`
    // request decodes and matches → no error.
    let memory_outcome = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(envelope(&body_with(Some("memory")))),
            Some(1),
            Some(ProviderMemoryType::Memory),
        )
        .unwrap(),
    );
    assert!(
        memory_outcome.is_ok(),
        "known-good memory_type=memory cursor must decode and match: {:?}",
        memory_outcome.err(),
    );

    // SMUGGLED UNKNOWN VALUE — the case P6 is about. The cursor body carries
    // `"memory_type":"bogus_type"`, which `ProviderMemoryType::parse_str`
    // rejects. P2 rerouted this from `BadRequest` to `CursorStale`: decode
    // performs only structural validation, the comparison path normalizes
    // the bogus value to `None` via `parse_str`.
    let smuggled_cursor = envelope(&body_with(Some("bogus_type")));
    // Case A — filtered request (`Some(Memory)`): `None != Some(Memory)` →
    // `CursorStale`. This is the load-bearing P2 assertion: a smuggled
    // unknown value against a filtered request funnels to `CursorStale`
    // (the same recovery UX as a filter change), NOT `BadRequest`.
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new_with_memory_type(
            SourceId("src_1".into()),
            Some(smuggled_cursor.clone()),
            Some(1),
            Some(ProviderMemoryType::Memory),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(
        matches!(err, QueryError::CursorStale),
        "smuggled memory_type vs filtered request must funnel to CursorStale (not BadRequest), got {err:?}",
    );

    // Case B — unfiltered request (`None`): per P2's documented contract,
    // the smuggled value normalizes to `None`, which EQUALS the unfiltered
    // request's `None` filter, so the cursor is accepted (the cursor simply
    // had no filter, which is a legal state). This is intentional: a
    // smuggled-unknown value carries no actionable filter information, and
    // an unfiltered request reads the whole active generation anyway. The
    // decode path's structural checks (length bound, etc.) still apply; only
    // the vocabulary check moved out.
    let unfiltered_outcome = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(
            SourceId("src_1".into()),
            Some(smuggled_cursor),
            Some(1),
        )
        .unwrap(),
    );
    assert!(
        unfiltered_outcome.is_ok(),
        "smuggled memory_type vs UNFILTERED request must match (None == None per P2), got: {:?}",
        unfiltered_outcome.err(),
    );
}
