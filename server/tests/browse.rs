//! Story 3.1 — `application::browse` contract tests.
//!
//! Mirrors `server/tests/search.rs`'s structure: an in-memory fixture builder,
//! a confirmed-source-only read path, cursor stability + staleness, the three
//! distinct empty states, and the lifecycle-exclusion boundary (raw chat /
//! transcript / human-instruction files / unconfirmed sources never appear).
//! Wire-level (HTTP) coverage lives in `server/tests/http_api.rs`.

use rusqlite::{params, Connection};

use tessera_lib::application;
use tessera_lib::application::query::QueryError;
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

/// Story 3.1 — a `b3.`-prefixed cursor whose inner `version` is not the
/// current browse cursor version is rejected as `CursorStale` (forward-compat
/// recovery), not `BadRequest` — mirroring search's v1/v2 rejection choice.
#[test]
fn browse_rejects_future_version_cursor_as_stale() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    // A well-formed b3. envelope whose inner version is 99 (a future bump).
    let payload = r#"{"version":99,"source":"src_1","revision":"deadbeef","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let future_cursor = format!("b3.{hex}");
    let err = application::browse(
        &registry,
        &conn,
        BrowseRequest::new(SourceId("src_1".into()), Some(future_cursor), Some(1)).unwrap(),
    )
    .unwrap_err();
    assert!(matches!(err, QueryError::CursorStale), "future-version cursor must be CursorStale, got {err:?}");
}

/// Story 3.1 — a `b3.`-prefixed cursor that fails to decode (invalid hex /
/// empty payload) is rejected as `BadRequest`. A truncated or corrupted cursor
/// (URL editing, log paste) must surface the right error code.
#[test]
fn browse_rejects_malformed_cursor_as_bad_request() {
    let conn = empty_db();
    insert_confirmed_source(&conn, 1, "codex", "healthy", None, true);
    insert_record(&conn, "rec_a", 1, "codex", "title", "body", 100, "full", None);

    let registry = SourceRegistry::new(&conn);
    for malformed in ["b3.zzz", "b3.", "b3.zz"] {
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
