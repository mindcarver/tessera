//! Read-side orchestration for confirmed Sources and their current active index.

use std::collections::HashSet;

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::ports::provider_adapter::ProviderMemoryType;
use crate::domain::ports::query_store::{BrowseCursorKey, QueryStore, SearchCursorKey};
use crate::domain::project::ProjectId;
use crate::domain::query::{
    BrowseEmptyState, BrowsePage, BrowseRequest, SearchEmptyState, SearchPage, SearchRequest,
    SourceQueryStatus, SourceQueryStatusKind, MAX_CURSOR_BYTES,
};
use crate::domain::scan::ScanRunState;
use crate::domain::source::{HealthState, SourceId, SourceLifecycle};
use crate::index::scan_store::ScanStore;
use crate::index::SourceRegistry;

#[derive(Debug)]
pub enum QueryError { BadRequest, CursorStale, Internal }

/// Versioned cursor payload. Version 4 (Story 5.2) additionally binds the
/// `tessera_project` filter so a project-filter change mid-pagination
/// invalidates an in-flight cursor (mirrors the v3 filter-binding pattern at
/// the project-projection boundary). Version 3 (Story 2.4) bound the active
/// cross-provider filters (`provider`, `memory_type`, `native_project`,
/// `since`) so a filter change mid-pagination invalidates an in-flight cursor.
/// Version 2 (Story 2.3) carried the full relevance sort key of the last record
/// on the previous page so the next-page predicate can perform a correct
/// "strictly-after" comparison across all four ORDER BY keys — a `record_id`-
/// only cursor would silently skip records whose id sorts below the cursor but
/// whose relevance rank is worse. The hex-encoded envelope format
/// (`v4.<hex>`) is unchanged; only the JSON payload and the version byte
/// change.
#[derive(Debug, Serialize, Deserialize)]
struct Cursor {
    version: u8,
    query: String,
    revision: String,
    last_record_id: String,
    /// Relevance sort-key components (added in cursor v2). For a v1 cursor
    /// supplied by an older client, `search` rejects it as `CursorStale`
    /// (HTTP 409 `cursor_stale`) so the UI's existing recovery path re-issues
    /// the first page under v4 — there is no persistent cursor storage, so
    /// this is transparent to the user.
    last_title_match: bool,
    last_observed_at: i64,
    last_coverage_full: bool,
    /// Story 2.4 — active filters bound into the cursor (added in v3). For a
    /// v2 cursor supplied by an older client, `search` rejects it as
    /// `CursorStale` (mirroring the v1→v2 path). `memory_type` is stored as
    /// the wire string (`ProviderMemoryType::as_str`) so a future variant
    /// addition does not silently break cursor decode; round-tripped through
    /// `ProviderMemoryType::parse_str` on the comparison path. `source` is the
    /// `src_<n>` handle string (round-trips through `SourceId` on the
    /// comparison path).
    provider: Option<String>,
    source: Option<String>,
    memory_type: Option<String>,
    native_project: Option<String>,
    since: Option<i64>,
    /// Story 5.2 — Tessera-project filter bound into the cursor (added in
    /// v4). For a v3 cursor supplied by an older client, `search` rejects it
    /// as `CursorStale` (mirroring the v1/v2→v3 path). Stored as the
    /// `proj_<n>` wire string so the cursor round-trips the same handle shape
    /// the request carries; compared as a raw string (the SQL layer normalizes
    /// through `ProjectId::to_rowid` at the predicate boundary, but a
    /// mid-pagination project change is a string mismatch either way).
    tessera_project: Option<String>,
}

const CURSOR_VERSION: u8 = 4;

pub fn search(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    request: SearchRequest,
) -> Result<SearchPage, QueryError> {
    let store = ScanStore::new(conn);
    let revision = store.current_index_revision().map_err(|_| QueryError::Internal)?;
    let cursor = match request.cursor() {
        Some(raw) => {
            // A `v1.<hex>`, `v2.<hex>`, or `v3.<hex>` cursor comes from a
            // pre-5.2 client (record_id-only / relevance-without-filters /
            // filters-without-tessera-project sort key). The cursor shape
            // changed in 5.2 (tessera_project bound in), so treat the older
            // envelope as `CursorStale` (HTTP 409 `cursor_stale`) rather than
            // `BadRequest`. The existing UI recovery path for `cursor_stale`
            // re-runs the first page, which is the correct outcome; a generic
            // contract error would surface an opaque `bad_request` instead.
            // v4 decode logic is unchanged below.
            if raw.starts_with("v1.") || raw.starts_with("v2.") || raw.starts_with("v3.") {
                return Err(QueryError::CursorStale);
            }
            let cursor = decode_cursor(raw).ok_or(QueryError::BadRequest)?;
            if cursor.version != CURSOR_VERSION || cursor.query != request.query() {
                return Err(QueryError::BadRequest);
            }
            if cursor.revision != revision { return Err(QueryError::CursorStale); }
            // Story 2.4 — a filter mismatch vs the request means the user
            // changed a filter mid-pagination. The UI clears its local cursor
            // on every filter change so this path is never hit in normal flow;
            // a stale filtered cursor from an older client is rejected as
            // `CursorStale` (per the I/O matrix) so the existing recovery path
            // re-runs page 1 under the new filter set, rather than paging
            // through a result set that no longer matches the request.
            if !cursor_filters_match(&cursor, &request) {
                return Err(QueryError::CursorStale);
            }
            Some(cursor)
        }
        None => None,
    };
    let after_key = cursor.as_ref().map(|item| SearchCursorKey {
        title_match: item.last_title_match,
        observed_at: item.last_observed_at,
        coverage_full: item.last_coverage_full,
        record_id: item.last_record_id.clone(),
    });
    let mut results = store
        .search_records(&request, after_key.as_ref())
        .map_err(|_| QueryError::Internal)?;
    let has_more = results.len() > request.limit();
    results.truncate(request.limit());
    let next_cursor = if has_more {
        results.last().map(|last| encode_cursor(&Cursor {
            version: CURSOR_VERSION,
            query: request.query().to_string(),
            revision,
            last_record_id: last.record_id().to_string(),
            last_title_match: last.title_match(),
            last_observed_at: last.observed_at(),
            last_coverage_full: last.coverage_level() == "full",
            provider: request.provider().map(str::to_string),
            source: request.source().map(|id| id.0.clone()),
            memory_type: request.memory_type().map(ProviderMemoryType::as_str).map(str::to_string),
            native_project: request.native_project().map(str::to_string),
            since: request.since(),
            // Story 5.2 — bind the active tessera_project so a project-filter
            // change mid-pagination invalidates the cursor (mirrors the v3
            // filter-binding pattern).
            tessera_project: request.tessera_project().map(str::to_string),
        }))
    } else { None };
    let sources = source_status_sidecar(registry, &store, request.tessera_project())?;
    let empty_state = if results.is_empty() && cursor.is_none() {
        empty_state(registry, &store)?
    } else { None };
    Ok(SearchPage::new(results, next_cursor, empty_state, sources))
}

// ---------------------------------------------------------------------------
// Story 3.1 — query-less Browse entry. Extended in 3.2 with the in-source
// `memory_type` filter.
//
// `browse()` mirrors `search()`'s contract mechanics (revision-bound cursor,
// limit+1 truncation, per-confirmed-source sidecar, three-state empty
// derivation) but is query-less and scoped to a single confirmed source. The
// cursor binds to the SAME index revision (FNV-1a over confirmed sources) so
// any generation change → `cursor_stale`. Cross-type / cross-version recovery
// is handled by the ENVELOPE PREFIX GATE in `browse()`: a search `v3.<hex>`
// cursor, a 3.1-era `b3.<hex>` cursor, or a future `b5.<hex>` cursor is
// rejected as `CursorStale` before decode runs (the inner `version` field is
// only a same-prefix integrity backstop — see `decode_browse_cursor`). This
// mirrors the existing v1/v2 rejection path's choice of `CursorStale` for
// forward-compatible recovery (the UI's `cursor_stale` recovery path re-runs
// page 1, which is the correct outcome under a cross-type/cross-version
// cursor swap).
//
// The empty-state derivation reads ONLY the browsed source's scan facts
// (active generation + latest run state), reusing the per-source facts
// already aggregated by `list_inventory`/`ScanStore` (Design Notes /
// Boundaries — "derived from the browsed source's scan facts"). The three
// states are distinct from Search's three: browse is query-less so
// `no_match` is meaningless, and it needs `no_indexable_memory` (scanned OK,
// zero records) which search lacks.
// ---------------------------------------------------------------------------

/// Browse cursor envelope payload. Version 4 (Story 3.2) binds the in-effect
/// `memory_type` filter so a filter change mid-pagination invalidates an
/// in-flight cursor (mirrors Search 2.4's "resolve filter once on page 1"
/// invariant at the browse layer). Version 3 (Story 3.1) carried the three
/// ORDER BY keys plus the source only.
///
/// Carries the three ORDER BY keys (browse drops the `title_match` rank from
/// search's four-key cursor) plus the source the cursor was issued under, so a
/// cursor bound to `src_N` cannot be replayed against `src_M` (the application
/// layer rejects a cross-source cursor as `CursorStale`, mirroring search's
/// filter-mismatch path).
#[derive(Debug, Serialize, Deserialize)]
struct BrowseCursor {
    version: u8,
    /// `src_<n>` handle the cursor was issued under. Stored as a string so
    /// decoding round-trips through `to_rowid()` on the comparison path,
    /// mirroring search's `source` slot.
    source: String,
    revision: String,
    last_record_id: String,
    last_observed_at: i64,
    last_coverage_full: bool,
    /// Story 3.2 — the in-effect memory-type filter, stored as the wire string
    /// (`ProviderMemoryType::as_str`) so a future variant addition does not
    /// silently break cursor decode. On the comparison path in `browse()`,
    /// both sides are normalized through `ProviderMemoryType::parse_str`
    /// before comparing (so a future variant with two accepted spellings
    /// cannot spuriously mismatch on the raw string). `None` means "no filter
    /// was active when the cursor was issued" (3.1's only state). Decode does
    /// NOT vocabulary-validate this field; an unknown stored value funnels to
    /// `CursorStale` via the comparison path (see `decode_browse_cursor`).
    memory_type: Option<String>,
}

const BROWSE_CURSOR_VERSION: u8 = 4;

pub fn browse(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    request: BrowseRequest,
) -> Result<BrowsePage, QueryError> {
    let store = ScanStore::new(conn);
    let revision = store.current_index_revision().map_err(|_| QueryError::Internal)?;
    // Story 3.1 I/O matrix — a non-confirmed/disabled/rejected/unknown source
    // must surface as `400 bad_request` (phase `browse`). The SQL layer's
    // `lifecycle_state = 'confirmed'` JOIN honestly yields zero rows for a
    // non-confirmed id, so without this check a disabled source would render
    // as `not_yet_scanned`/`no_indexable_memory`, hiding the real lifecycle
    // state. Resolve the registry row and explicitly reject anything that is
    // NOT `confirmed`.
    let source = registry
        .get(request.source())
        .map_err(|_| QueryError::Internal)?
        .ok_or(QueryError::BadRequest)?;
    if source.lifecycle_state != SourceLifecycle::Confirmed {
        return Err(QueryError::BadRequest);
    }
    let cursor = match request.cursor() {
        Some(raw) => {
            // CROSS-VERSION / CROSS-TYPE GATE (the real forward-compat
            // boundary). The envelope prefix is the single discriminator a
            // real client produces for its contract: a search cursor is
            // `v3.<hex>`, a 3.1-era browse cursor is `b3.<hex>`, the current
            // browse cursor is `b4.<hex>`, and a future bump would be
            // `b5.<hex>` (etc.). Anything that does not match THIS contract's
            // prefix is rejected as `CursorStale` so the UI's existing
            // recovery path re-runs page 1 — `BadRequest` would surface an
            // opaque error and break the recovery contract. This mirrors
            // search's v1/v2 rejection choice and 3.1's cross-type rejection
            // of a `v3.` search cursor.
            if !raw.starts_with("b4.") {
                return Err(QueryError::CursorStale);
            }
            let cursor = decode_browse_cursor(raw).ok_or(QueryError::BadRequest)?;
            // SAME-PREFIX INTEGRITY BACKSTOP. `decode_browse_cursor` does NOT
            // validate the inner `version` (a future-version `b5.` cursor is
            // rejected by the prefix gate above; a same-prefix cursor with a
            // tampered inner `version` reaches this check). This field
            // therefore only ever sees `version == 4` in practice; a value of
            // e.g. `99` here means a hand-edited same-prefix cursor, and
            // surfacing it as `CursorStale` (rather than `BadRequest`) keeps
            // the recovery path uniform.
            if cursor.version != BROWSE_CURSOR_VERSION {
                return Err(QueryError::CursorStale);
            }
            // Cross-source / cross-revision cursors invalidate pagination.
            // Compare the source via normalized rowid so equivalent handles
            // (`src_2` vs `src_02`) match (mirrors search's
            // `cursor_source_rowid` comparison).
            let cursor_source_rowid = SourceId(cursor.source.clone()).to_rowid();
            if cursor_source_rowid != request.source().to_rowid()
                || cursor.revision != revision
            {
                return Err(QueryError::CursorStale);
            }
            // Story 3.2 — a memory_type mismatch vs the request funnels to
            // `CursorStale`. This covers EVERY "cursor filter ≠ request"
            // shape uniformly:
            //   - the user changed the filter mid-pagination (the normal
            //     path; the UI clears its local cursor on every filter change
            //     so this is rare in flow),
            //   - a stale filtered cursor from an older client,
            //   - a hand-edited cursor carrying an UNKNOWN `memory_type`
            //     string (the cursor's stored value is normalized through
            //     `parse_str` below; an unknown value becomes `None`, which
            //     cannot equal a valid request filter unless the request is
            //     ALSO unfiltered — in which case the cursor simply had no
            //     filter, which is a legal state and matches).
            // Routing all three through `CursorStale` keeps the recovery UX
            // uniform (re-run page 1 under the new filter), rather than
            // splitting them across `CursorStale` and `BadRequest`. The
            // decode path performs only structural/length validation (no
            // vocabulary check) so it cannot preempt this funnel. Mirrors
            // search's `cursor_filters_match` recovery.
            //
            // Normalize both sides through `ProviderMemoryType::parse_str`
            // before comparing (P4): a future variant with two accepted
            // spellings must not spuriously mismatch on the raw stored string.
            let cursor_memory_type = cursor
                .memory_type
                .as_deref()
                .and_then(ProviderMemoryType::parse_str);
            if cursor_memory_type != request.memory_type() {
                return Err(QueryError::CursorStale);
            }
            Some(cursor)
        }
        None => None,
    };
    let after_key = cursor.as_ref().map(|item| BrowseCursorKey {
        observed_at: item.last_observed_at,
        coverage_full: item.last_coverage_full,
        record_id: item.last_record_id.clone(),
    });
    let mut results = store
        .browse_records(&request, after_key.as_ref())
        .map_err(|_| QueryError::Internal)?;
    let has_more = results.len() > request.limit();
    results.truncate(request.limit());
    let next_cursor = if has_more {
        results.last().map(|last| encode_browse_cursor(&BrowseCursor {
            version: BROWSE_CURSOR_VERSION,
            source: request.source().0.clone(),
            revision,
            last_record_id: last.record_id().to_string(),
            last_observed_at: last.observed_at(),
            last_coverage_full: last.coverage_level() == "full",
            // Story 3.2 — bind the in-effect memory_type so a filter change
            // invalidates the cursor (mirrors search's filter-bound cursor).
            memory_type: request
                .memory_type()
                .map(ProviderMemoryType::as_str)
                .map(str::to_string),
        }))
    } else { None };
    let sources = source_status_sidecar(registry, &store, None)?;
    let empty_state = if results.is_empty() && cursor.is_none() {
        Some(browse_empty_state(&store, request.source())?)
    } else { None };
    Ok(BrowsePage::new(results, next_cursor, empty_state, sources))
}

/// Derive the browsed source's three-state empty value from its OWN scan facts
/// (active generation + latest run state). Reuses the same `ScanStore`
/// aggregations that back `list_inventory` (Design Notes — "reusing the
/// per-source facts already aggregated by `list_inventory`/`ScanStore`").
///
/// Decision table (Boundaries/I/O matrix), exhaustive over `ScanRunState`:
/// - Active generation present but zero records → `NoIndexableMemory`.
/// - No active generation:
///   - No run at all (never scanned) → `NotYetScanned`.
///   - Latest run `succeeded` but no generation activated (diagnostic-only
///     `complete_without_activation`) → `NoIndexableMemory`. The scan ran and
///     succeeded; it simply has no activatable Agent Memory, so "not yet
///     scanned" would be dishonest.
///   - Latest run `failed`/`retry` → `SourceUnavailable` (no prior generation).
///   - Latest run in flight (`queued`/`running`/`staging`/`committing`) →
///     `NotYetScanned` (no completed generation to show yet).
fn browse_empty_state(
    store: &ScanStore<'_>,
    source_id: &SourceId,
) -> Result<BrowseEmptyState, QueryError> {
    let Some(rowid) = source_id.to_rowid() else {
        // The application layer validated the handle upstream; treat an
        // invalid handle here as `Internal` rather than silently mapping to
        // an empty state.
        return Err(QueryError::Internal);
    };
    let active_generation = store
        .active_generation(rowid)
        .map_err(|_| QueryError::Internal)?;
    if active_generation.is_some() {
        // The source scanned successfully and activated a generation, but
        // browse returned zero rows → the generation is empty.
        return Ok(BrowseEmptyState::NoIndexableMemory);
    }
    // No active generation. Classify by the latest run's state so every
    // `ScanRunState` variant has an explicit verdict (no silent fall-through).
    let latest_run = store.latest_run(rowid).map_err(|_| QueryError::Internal)?;
    Ok(match latest_run.as_ref().map(|run| run.state) {
        None => BrowseEmptyState::NotYetScanned,
        Some(ScanRunState::Failed) | Some(ScanRunState::Retry) => BrowseEmptyState::SourceUnavailable,
        // A run can finish `succeeded` without activating a generation
        // (`complete_without_activation`); the scan ran, so "not yet scanned"
        // would be wrong — map to `NoIndexableMemory`.
        Some(ScanRunState::Succeeded) => BrowseEmptyState::NoIndexableMemory,
        // Scan in flight, no completed generation yet: "not yet scanned" is the
        // honest three-state fit.
        Some(ScanRunState::Queued)
        | Some(ScanRunState::Running)
        | Some(ScanRunState::Staging)
        | Some(ScanRunState::Committing) => BrowseEmptyState::NotYetScanned,
    })
}

fn encode_browse_cursor(cursor: &BrowseCursor) -> String {
    let json = serde_json::to_vec(cursor).expect("cursor DTO serialization is total");
    let mut encoded = String::with_capacity(3 + json.len() * 2);
    encoded.push_str("b4.");
    for byte in json { encoded.push_str(&format!("{byte:02x}")); }
    encoded
}

fn decode_browse_cursor(raw: &str) -> Option<BrowseCursor> {
    let hex = raw.strip_prefix("b4.")?;
    if hex.is_empty() || hex.len() % 2 != 0 || raw.len() > MAX_CURSOR_BYTES { return None; }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    let cursor: BrowseCursor = serde_json::from_slice(&bytes).ok()?;
    // Decode performs ONLY structural / length validation. It intentionally
    // does NOT validate the inner `version` OR the `memory_type` vocabulary:
    //   - The `version` field is a same-prefix integrity backstop (a future-
    //     version cursor is rejected by the prefix gate in `browse()` before
    //     decode is ever called; only a same-prefix hand-edited cursor reaches
    //     the inner version check). Validating it here would short-circuit
    //     `browse()`'s `CursorStale` mapping to an opaque `BadRequest`.
    //   - The `memory_type` vocabulary is normalized through `parse_str` on
    //     the comparison path in `browse()`, where an unknown stored value
    //     becomes `None` and is funneled to `CursorStale` (the same recovery
    //     UX as a filter change). Validating it here would split the failure
    //     mode between `BadRequest` (smuggled value) and `CursorStale`
    //     (legitimate filter change), which is the wrong shape — both mean
    //     "cursor filter ≠ request," and both should re-run page 1.
    if cursor.source.is_empty()
        || cursor.source.len() > crate::domain::query::MAX_FILTER_BYTES
        || SourceId(cursor.source.clone()).to_rowid().is_none()
        || cursor.revision.is_empty()
        || cursor.revision.len() > 64
        || cursor.last_record_id.is_empty()
        || cursor.last_record_id.len() > 512
        // Story 3.2 — every cursor string field has an explicit length cap
        // for defense-in-depth (mirrors `decode_cursor`'s sibling shape).
        // `memory_type` is bounded by `MAX_CURSOR_BYTES` overall, but capping
        // it here keeps the decode shape consistent with the other fields.
        // The vocabulary check is intentionally NOT here (see above).
        || cursor.memory_type.as_ref().is_some_and(|value| value.len() > crate::domain::query::MAX_FILTER_BYTES)
    {
        return None;
    }
    Some(cursor)
}

/// Compare the cursor's bound filters against the incoming request's filters.
/// A mismatch means the user changed a filter mid-pagination: the cursor's
/// result set no longer corresponds to the request, so the caller rejects it
/// as `CursorStale` (Story 2.4 I/O matrix). `memory_type` is compared via the
/// wire string so a cursor serialized before a future variant addition still
/// round-trips correctly. `source` is compared via its normalized rowid
/// (`to_rowid()`) rather than the raw handle string, because the SQL layer
/// normalizes through `to_rowid()` — `src_2` and `src_02` both map to rowid 2
/// and are equivalent, so a raw-string comparison would spuriously flag the
/// cursor as stale (Story 2.4 pass-2). Both sides are validated well-formed
/// (cursor at decode, request at construction), so each `to_rowid()` is
/// `Some`; comparing `Option<Option<i64>>` still treats `None` (no source
/// filter) as equal.
fn cursor_filters_match(cursor: &Cursor, request: &SearchRequest) -> bool {
    let cursor_source_rowid = cursor.source.as_ref().map(|s| SourceId(s.clone()).to_rowid());
    let request_source_rowid = request.source().map(|id| id.to_rowid());
    cursor.provider.as_deref() == request.provider()
        && cursor_source_rowid == request_source_rowid
        && cursor.native_project.as_deref() == request.native_project()
        && cursor.since == request.since()
        && cursor.memory_type.as_deref() == request.memory_type().map(ProviderMemoryType::as_str)
        && cursor.tessera_project.as_deref() == request.tessera_project()
}

/// Build the FR-14 per-query availability sidecar: one row per **confirmed**
/// source describing its availability for this query. Derived from
/// `health_state` + active-generation presence + `latest_run` state. A down
/// source's already-indexed records (if any) are NOT suppressed by this
/// sidecar — the flag is informational (Design Notes: "the sidecar flags; it
/// does not hide").
///
/// Story 5.2 (Q3=A) — when `tessera_project` is `Some`, the sidecar NARROWS
/// to only confirmed sources whose `(provider, COALESCE(native_project, ''))`
/// is in that project's mapping scope set. Without narrowing, a project-
/// filtered search would report Coverage/Health for sources whose records
/// the project filter excludes — misleading. With `tessera_project == None`
/// the sidecar is unchanged (all confirmed sources). An unknown / malformed
/// `proj_<n>` resolves to an empty scope set → empty sidecar, matching the
/// I/O matrix's "unknown project ⇒ empty results, not an error" posture.
///
/// FR-14 best-effort: a per-source status lookup failure (e.g. a corrupt
/// `scan_runs.state` row or an unreadable active-generation marker) is logged
/// and falls back to a conservative status for THAT source only — it NEVER
/// propagates [`QueryError::Internal`]. "One unavailable source never breaks
/// the query" extends to the sidecar, not just the result rows.
fn source_status_sidecar(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
    tessera_project: Option<&str>,
) -> Result<Vec<SourceQueryStatus>, QueryError> {
    let sources = registry.list().map_err(|_| QueryError::Internal)?;
    // Story 5.2 — resolve the project filter to its mapping scope set once.
    // The three states MUST stay distinct so the sidecar matches the SQL
    // layer's "unknown / malformed project ⇒ empty results, not an error"
    // posture:
    //   - `None`                       ⇒ no filter; do NOT narrow (all
    //                                  confirmed sources listed).
    //   - `Some(empty set)`            ⇒ filter present but resolves to no
    //                                  mappings (malformed `proj_x` handle,
    //                                  unknown id, or a project with zero
    //                                  mappings); narrow to nothing.
    //   - `Some({(provider, np), …})`  ⇒ narrow to the project's mapped
    //                                  sources.
    // Collapsing a malformed id to `None` (no narrowing) would list ALL
    // confirmed sources for a query whose result set is empty — misleading
    // and inconsistent with the SQL EXISTS predicate, which binds
    // `tessera_project_id = NULL` and is therefore always false. The
    // `COALESCE(native_project, '')` collapse on the value side mirrors the
    // Story 5.1 uniqueness index's NULL handling so a Codex global mapping
    // matches a confirmed Codex source (NULL `native_project`).
    let scope_set: Option<HashSet<(String, String)>> = match tessera_project {
        None => None,
        Some(id) => {
            let set = match ProjectId(id.to_string()).to_rowid() {
                Some(rowid) => store
                    .project_mapping_scope_set(rowid)
                    .map_err(|_| QueryError::Internal)?
                    .into_iter()
                    .map(|(provider, native_project)| {
                        (provider, native_project.unwrap_or_default())
                    })
                    .collect(),
                // Malformed handle (e.g. "proj_x", "garbage"): the filter is
                // present but matches nothing. Produce an EMPTY set (not None)
                // so the sidecar narrows to nothing rather than listing every
                // confirmed source for an empty result set.
                None => HashSet::new(),
            };
            Some(set)
        }
    };
    let mut out = Vec::new();
    for source in sources
        .into_iter()
        .filter(|source| source.lifecycle_state == SourceLifecycle::Confirmed)
    {
        // Story 5.2 — when narrowing, skip sources whose scope is not in the
        // project's mapping set. The collapse matches the scope set's
        // `COALESCE(..., '')` shape.
        if let Some(set) = &scope_set {
            let key = (source.provider.clone(), source.native_project.clone().unwrap_or_default());
            if !set.contains(&key) {
                continue;
            }
        }
        let Some(rowid) = source.source_id.to_rowid() else { continue };
        let status = match source_status(store, rowid, source.health_state) {
            Ok(status) => status,
            Err(error) => {
                eprintln!(
                    "tessera: source status sidecar lookup failed for {} (falling back to unavailable): {error:?}",
                    source.source_id.0
                );
                // Conservative fallback: we could not verify availability, so
                // flag for attention rather than fail the whole query.
                SourceQueryStatusKind::Unavailable
            }
        };
        out.push(SourceQueryStatus {
            source_id: source.source_id,
            provider: source.provider,
            native_project: source.native_project,
            status,
        });
    }
    Ok(out)
}

/// Derive one source's availability status from its persisted facts. Returns
/// an `Err` when the underlying status lookup is unreadable so the caller can
/// fall back best-effort instead of failing the whole query (FR-14).
fn source_status(
    store: &ScanStore<'_>,
    rowid: i64,
    health_state: HealthState,
) -> Result<SourceQueryStatusKind, rusqlite::Error> {
    let active_generation = store.active_generation(rowid)?;
    let latest_run = store.latest_run(rowid)?;
    Ok(derive_status(
        health_state,
        active_generation.is_some(),
        latest_run.as_ref().map(|run| run.state),
    ))
}

/// Map a source's persisted facts to the wire status enum.
///
/// - No active generation → [`Unavailable`]: the source contributes no records.
///   (`has_active_generation == false` short-circuits here regardless of
///   `health_state`, so the `Error` arm below is only reached WITH an active
///   generation.)
/// - `health_state = error` WITH an active generation → [`Degraded`]: the
///   latest scan failed at the source level, but the prior generation's
///   records still answer (the search JOINs on `active_generation`), so flag
///   for attention rather than reporting them absent. [`Unavailable`] is
///   reserved for an `Error` source with NO active generation.
/// - `health_state = degraded` → [`Degraded`].
/// - `health_state = healthy` / `unknown` with a failed/retry latest run →
///   [`Degraded`] (records from the prior generation still answer).
/// - Otherwise → [`Available`].
fn derive_status(
    health_state: HealthState,
    has_active_generation: bool,
    latest_run_state: Option<ScanRunState>,
) -> SourceQueryStatusKind {
    if !has_active_generation {
        return SourceQueryStatusKind::Unavailable;
    }
    match health_state {
        // An Error source still serves its active generation's records (the
        // search JOINs on `active_generation`), so flag Degraded — records
        // answer but need attention. Unavailable is reserved for the
        // no-active-generation early return above.
        HealthState::Error => SourceQueryStatusKind::Degraded,
        HealthState::Degraded => SourceQueryStatusKind::Degraded,
        HealthState::Healthy | HealthState::Unknown => {
            if matches!(latest_run_state, Some(ScanRunState::Failed | ScanRunState::Retry)) {
                SourceQueryStatusKind::Degraded
            } else {
                SourceQueryStatusKind::Available
            }
        }
    }
}

fn empty_state(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
) -> Result<Option<SearchEmptyState>, QueryError> {
    if !store.current_index_revision().map_err(|_| QueryError::Internal)?.is_empty() { return Ok(Some(SearchEmptyState::NoMatch)); }
    let sources = registry.list().map_err(|_| QueryError::Internal)?;
    let mut unavailable = false;
    for source in sources.into_iter().filter(|source| source.lifecycle_state == SourceLifecycle::Confirmed) {
        let Some(rowid) = source.source_id.to_rowid() else { continue };
        if let Some(run) = store.latest_run(rowid).map_err(|_| QueryError::Internal)? {
            unavailable |= matches!(run.state, ScanRunState::Failed | ScanRunState::Retry);
        }
    }
    Ok(Some(if unavailable {
        SearchEmptyState::SourceUnavailable
    } else {
        SearchEmptyState::SourceNotIndexed
    }))
}

fn encode_cursor(cursor: &Cursor) -> String {
    let json = serde_json::to_vec(cursor).expect("cursor DTO serialization is total");
    let mut encoded = String::with_capacity(3 + json.len() * 2);
    encoded.push_str("v4.");
    for byte in json { encoded.push_str(&format!("{byte:02x}")); }
    encoded
}

fn decode_cursor(raw: &str) -> Option<Cursor> {
    let hex = raw.strip_prefix("v4.")?;
    if hex.is_empty() || hex.len() % 2 != 0 || raw.len() > MAX_CURSOR_BYTES { return None; }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for pair in hex.as_bytes().chunks_exact(2) {
        let hi = (pair[0] as char).to_digit(16)?;
        let lo = (pair[1] as char).to_digit(16)?;
        bytes.push(((hi << 4) | lo) as u8);
    }
    let cursor: Cursor = serde_json::from_slice(&bytes).ok()?;
    if cursor.version != CURSOR_VERSION
        || cursor.query.is_empty()
        || cursor.query.len() > crate::domain::query::MAX_QUERY_BYTES
        || cursor.revision.is_empty()
        || cursor.revision.len() > 64
        || cursor.last_record_id.is_empty()
        || cursor.last_record_id.len() > 512
    {
        return None;
    }
    // Story 2.4 — bound-filter shape sanity (defense-in-depth, mirroring the
    // request-time validation in `SearchRequest::new_with_filters`). A filter
    // string is bounded at request time by `MAX_FILTER_BYTES`; a cursor that
    // violates the same bound is rejected as malformed (the hex decode above
    // already succeeded, so a length violation here means the cursor was
    // tampered with or comes from a buggy client). Vocabulary/range checks that
    // the request constructor performs are repeated here so a hand-edited
    // cursor cannot smuggle an unknown value past the comparison path:
    //   - `provider` must be a known provider id (`KNOWN_PROVIDER_IDS`),
    //   - `memory_type` must round-trip through `ProviderMemoryType::parse_str`,
    //   - `source` must be a well-formed `src_<n>` handle (`to_rowid().is_some()`),
    //   - `since` must be in `[0, MAX_SINCE]`.
    // Story 5.2 — `tessera_project` is bounded by `MAX_FILTER_BYTES` (no
    // vocabulary check; an unknown / malformed `proj_<n>` is honestly compared
    // via `cursor_filters_match` and surfaces `CursorStale` if it differs from
    // the request, mirroring the SQL layer's "unknown ⇒ matches nothing"
    // posture).
    if cursor.provider.as_ref().is_some_and(|value| {
        value.len() > crate::domain::query::MAX_FILTER_BYTES
            || !crate::domain::query::KNOWN_PROVIDER_IDS.contains(&value.as_str())
    })
        || cursor.source.as_ref().is_some_and(|value| {
            value.len() > crate::domain::query::MAX_FILTER_BYTES
                || crate::domain::source::SourceId(value.clone()).to_rowid().is_none()
        })
        || cursor.native_project.as_ref().is_some_and(|value| value.len() > crate::domain::query::MAX_FILTER_BYTES)
        || cursor.memory_type.as_ref().is_some_and(|value| ProviderMemoryType::parse_str(value).is_none())
        || cursor.since.is_some_and(|value| !(0..=crate::domain::query::MAX_SINCE).contains(&value))
        || cursor.tessera_project.as_ref().is_some_and(|value| value.len() > crate::domain::query::MAX_FILTER_BYTES)
    {
        return None;
    }
    Some(cursor)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_round_trip_is_opaque_and_versioned() {
        let cursor = Cursor {
            version: CURSOR_VERSION,
            query: "记忆".into(),
            revision: "bc93e71f".into(),
            last_record_id: "rec_2".into(),
            last_title_match: true,
            last_observed_at: 99,
            last_coverage_full: true,
            provider: Some("codex".into()),
            source: Some("src_2".into()),
            memory_type: Some("memory".into()),
            native_project: None,
            since: Some(1_700_000_000),
            tessera_project: Some("proj_7".into()),
        };
        let encoded = encode_cursor(&cursor);
        assert!(!encoded.contains("记忆"));
        assert!(encoded.starts_with("v4."));
        let back = decode_cursor(&encoded).unwrap();
        assert_eq!(back.query, "记忆");
        assert_eq!(back.last_record_id, "rec_2");
        assert!(back.last_title_match);
        assert_eq!(back.last_observed_at, 99);
        assert!(back.last_coverage_full);
        assert_eq!(back.provider.as_deref(), Some("codex"));
        assert_eq!(back.source.as_deref(), Some("src_2"));
        assert_eq!(back.memory_type.as_deref(), Some("memory"));
        assert!(back.native_project.is_none());
        assert_eq!(back.since, Some(1_700_000_000));
        // Story 5.2 — tessera_project round-trips through the v4 envelope.
        assert_eq!(back.tessera_project.as_deref(), Some("proj_7"));
    }

    #[test]
    fn derive_status_maps_health_active_gen_and_latest_run() {
        use crate::domain::scan::ScanRunState;
        // No active generation → unavailable regardless of health.
        assert_eq!(
            derive_status(HealthState::Healthy, false, None),
            SourceQueryStatusKind::Unavailable
        );
        assert_eq!(
            derive_status(HealthState::Error, false, Some(ScanRunState::Failed)),
            SourceQueryStatusKind::Unavailable
        );
        // Active gen + healthy/unknown + clean latest run → available.
        assert_eq!(
            derive_status(HealthState::Healthy, true, Some(ScanRunState::Succeeded)),
            SourceQueryStatusKind::Available
        );
        assert_eq!(
            derive_status(HealthState::Unknown, true, None),
            SourceQueryStatusKind::Available
        );
        // Active gen + error → degraded (records still answer; flag for
        // attention). Unavailable is reserved for no active generation (the
        // early return above), which the next assertion pins.
        assert_eq!(
            derive_status(HealthState::Error, true, Some(ScanRunState::Failed)),
            SourceQueryStatusKind::Degraded
        );
        // Active gen + error + succeeded latest run → still degraded: the
        // source-level health flag dominates the per-run state.
        assert_eq!(
            derive_status(HealthState::Error, true, Some(ScanRunState::Succeeded)),
            SourceQueryStatusKind::Degraded
        );
        // Active gen + degraded → degraded.
        assert_eq!(
            derive_status(HealthState::Degraded, true, Some(ScanRunState::Succeeded)),
            SourceQueryStatusKind::Degraded
        );
        // Active gen + healthy but latest run failed → degraded.
        assert_eq!(
            derive_status(HealthState::Healthy, true, Some(ScanRunState::Failed)),
            SourceQueryStatusKind::Degraded
        );
    }

    /// Patch 5 — equivalent `src_<n>` handles (`src_2` vs `src_02`) normalize to
    /// the same rowid via `to_rowid()`, so a cursor bound to one and a request
    /// carrying the other must NOT be flagged stale. The SQL layer normalizes
    /// the same way, so they are the same predicate; a raw-string comparison
    /// would spuriously trigger `CursorStale`.
    #[test]
    fn cursor_source_comparison_normalizes_equivalent_handles() {
        let cursor = Cursor {
            version: CURSOR_VERSION,
            query: "q".into(),
            revision: "rev".into(),
            last_record_id: "rec_1".into(),
            last_title_match: false,
            last_observed_at: 0,
            last_coverage_full: false,
            provider: None,
            source: Some("src_2".into()),
            memory_type: None,
            native_project: None,
            since: None,
            tessera_project: None,
        };
        let equivalent = crate::domain::query::SearchRequest::new_with_filters(
            "q".into(),
            None,
            Some(20),
            crate::domain::query::SearchFilters {
                source: Some(crate::domain::source::SourceId("src_02".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            cursor_filters_match(&cursor, &equivalent),
            "src_2 (cursor) and src_02 (request) normalize to rowid 2 and must match"
        );

        // A genuinely different source still mismatches.
        let other = crate::domain::query::SearchRequest::new_with_filters(
            "q".into(),
            None,
            Some(20),
            crate::domain::query::SearchFilters {
                source: Some(crate::domain::source::SourceId("src_3".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            !cursor_filters_match(&cursor, &other),
            "src_2 vs src_3 must mismatch"
        );

        // Some vs None also mismatches (one side has a source predicate).
        let none_request = crate::domain::query::SearchRequest::new_with_filters(
            "q".into(),
            None,
            Some(20),
            crate::domain::query::SearchFilters::default(),
        )
        .unwrap();
        assert!(
            !cursor_filters_match(&cursor, &none_request),
            "Some(src_2) vs None source must mismatch"
        );
    }

    /// Patch 10 — `decode_cursor` performs defense-in-depth validation on the
    /// bound filters (mirroring `SearchRequest::new_with_filters`), so a
    /// hand-edited / buggy cursor cannot smuggle an unknown value past the
    /// comparison path. Each tampered body mutates exactly one bound-filter
    /// field; `decode_cursor` must reject it as `None`.
    #[test]
    fn decode_cursor_rejects_tampered_bound_filters() {
        // Hex-encode a JSON cursor body and wrap it in the v4 envelope. Building
        // the cursor by hand (not via encode_cursor) lets us inject values that
        // SearchRequest/encode_cursor would reject upstream.
        fn envelope(json: &str) -> String {
            let hex: String = json.bytes().map(|byte| format!("{byte:02x}")).collect();
            format!("v4.{hex}")
        }
        // Build a cursor JSON body with the bound filters parameterized; every
        // other field is a fixed valid baseline. `null` means "filter absent".
        // Story 5.2 — the body includes `tessera_project` (added in v4); the
        // rejection cases below mutate the OTHER bound filters, and the
        // baseline stays `tessera_project:null`.
        fn body(
            provider: Option<&str>,
            source: Option<&str>,
            memory_type: Option<&str>,
            since: Option<i64>,
        ) -> String {
            let str_field = |key: &str, value: Option<&str>| -> String {
                match value {
                    Some(v) => format!("\"{key}\":\"{v}\""),
                    None => format!("\"{key}\":null"),
                }
            };
            let since_field = match since {
                Some(v) => format!("\"since\":{v}"),
                None => "\"since\":null".to_string(),
            };
            format!(
                r#"{{"version":4,"query":"q","revision":"rev","last_record_id":"rec_1","last_title_match":false,"last_observed_at":0,"last_coverage_full":false,{provider},{source},{memory_type},"native_project":null,{since},"tessera_project":null}}"#,
                provider = str_field("provider", provider),
                source = str_field("source", source),
                memory_type = str_field("memory_type", memory_type),
                since = since_field,
            )
        }

        // Baseline: every bound filter absent → decodes cleanly. Guards against
        // the rejection cases below passing due to a typo in the helper.
        assert!(
            decode_cursor(&envelope(&body(None, None, None, None))).is_some(),
            "baseline cursor must decode"
        );

        // provider not in KNOWN_PROVIDER_IDS → rejected.
        assert!(
            decode_cursor(&envelope(&body(Some("bogus"), None, None, None))).is_none(),
            "unknown provider must be rejected"
        );
        // since beyond MAX_SINCE → rejected.
        assert!(
            decode_cursor(&envelope(&body(None, None, None, Some(crate::domain::query::MAX_SINCE + 1)))).is_none(),
            "since > MAX_SINCE must be rejected"
        );
        // source not a well-formed src_<n> handle → rejected.
        assert!(
            decode_cursor(&envelope(&body(None, Some("not-a-source"), None, None))).is_none(),
            "malformed source handle must be rejected"
        );
        // Patch 9 — negative rowid (src_-5) is rejected by to_rowid now.
        assert!(
            decode_cursor(&envelope(&body(None, Some("src_-5"), None, None))).is_none(),
            "src_-5 must be rejected"
        );
        // Patch 9 — zero rowid (src_0) is rejected by to_rowid now.
        assert!(
            decode_cursor(&envelope(&body(None, Some("src_0"), None, None))).is_none(),
            "src_0 must be rejected"
        );
        // memory_type not in the ProviderMemoryType vocabulary → rejected.
        assert!(
            decode_cursor(&envelope(&body(None, None, Some("bogus_type"), None))).is_none(),
            "unknown memory_type must be rejected"
        );

        // Equivalent source handles both decode (the rowid-normalized
        // comparison is exercised by cursor_source_comparison_normalizes_*
        // above); src_02 is well-formed and must be accepted.
        assert!(
            decode_cursor(&envelope(&body(None, Some("src_02"), None, None))).is_some(),
            "src_02 is well-formed (rowid 2) and must decode"
        );
        // A known-good provider / memory_type / since combination decodes.
        assert!(
            decode_cursor(&envelope(&body(Some("codex"), Some("src_1"), Some("memory"), Some(0)))).is_some(),
            "valid bound filters must decode"
        );

        // P7 — an over-length `tessera_project` (mirrors the provider/source
        // length-cap cases above). `body(...)` hardcodes
        // `"tessera_project":null`; splice in a 5000-char value to exceed
        // `MAX_FILTER_BYTES` and confirm `decode_cursor` rejects it as None.
        // The hex envelope grows proportionally but stays well under
        // `MAX_CURSOR_BYTES`, so the rejection must come from the
        // tessera_project length check, not the overall cursor cap.
        let overlong_tp = "x".repeat(5000);
        let overlong_tp_body = body(None, None, None, None)
            .replace("\"tessera_project\":null", &format!("\"tessera_project\":\"{overlong_tp}\""));
        assert!(
            decode_cursor(&envelope(&overlong_tp_body)).is_none(),
            "over-length tessera_project (5000 chars) must be rejected"
        );
    }

    /// P4 — the in-body `version` byte is load-bearing on decode. The envelope
    /// prefix (`v4.<hex>`) only gates the OUTER envelope shape; a hand-edited
    /// `v4.` envelope whose JSON body claims `"version":3` must still be
    /// rejected by `decode_cursor` (returning `None`), so a same-prefix tampered
    /// cursor cannot sneak past the version check. Pins the source-side check at
    /// the top of `decode_cursor` (`cursor.version != CURSOR_VERSION`) so a
    /// future refactor that moves / drops the check fails this test loudly.
    #[test]
    fn decode_cursor_rejects_tampered_version_byte() {
        fn envelope(json: &str) -> String {
            let hex: String = json.bytes().map(|byte| format!("{byte:02x}")).collect();
            format!("v4.{hex}")
        }
        // Baseline body shape mirroring `decode_cursor_rejects_tampered_bound_filters`,
        // with version=4 (CURSOR_VERSION) so the baseline decodes cleanly.
        fn body(version: u8) -> String {
            format!(
                r#"{{"version":{version},"query":"q","revision":"rev","last_record_id":"rec_1","last_title_match":false,"last_observed_at":0,"last_coverage_full":false,"provider":null,"source":null,"memory_type":null,"native_project":null,"since":null,"tessera_project":null}}"#
            )
        }
        // Baseline (version=4) decodes — guards against the rejection below
        // passing due to a typo in the helper.
        assert!(
            decode_cursor(&envelope(&body(CURSOR_VERSION))).is_some(),
            "baseline v4 cursor must decode"
        );
        // Tampered version byte (3) inside a `v4.` envelope → rejected. The
        // prefix gate only inspects the envelope, so the in-body byte is the
        // version-tampering backstop.
        assert!(
            decode_cursor(&envelope(&body(3))).is_none(),
            "tampered version byte (3) inside a v4 envelope must be rejected"
        );
        // Other tampered version bytes are also rejected (pins the check is
        // an equality compare, not a `> CURSOR_VERSION` or `< CURSOR_VERSION`
        // half-check that would let a lower version slip through).
        assert!(
            decode_cursor(&envelope(&body(99))).is_none(),
            "tampered version byte (99) must be rejected"
        );
        assert!(
            decode_cursor(&envelope(&body(0))).is_none(),
            "tampered version byte (0) must be rejected"
        );
    }
}
