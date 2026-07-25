//! Read-side orchestration for confirmed Sources and their current active index.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::domain::ports::query_store::{QueryStore, SearchCursorKey};
use crate::domain::query::{
    SearchEmptyState, SearchPage, SearchRequest, SourceQueryStatus, SourceQueryStatusKind,
    MAX_CURSOR_BYTES,
};
use crate::domain::scan::ScanRunState;
use crate::domain::source::{HealthState, SourceLifecycle};
use crate::index::scan_store::ScanStore;
use crate::index::SourceRegistry;

#[derive(Debug)]
pub enum QueryError { BadRequest, CursorStale, Internal }

/// Versioned cursor payload. Version 2 (Story 2.3) carries the full relevance
/// sort key of the last record on the previous page so the next-page predicate
/// can perform a correct "strictly-after" comparison across all four ORDER BY
/// keys — a `record_id`-only cursor would silently skip records whose id sorts
/// below the cursor but whose relevance rank is worse. The hex-encoded
/// envelope format (`v2.<hex>`) is unchanged; only the JSON payload and the
/// version byte change.
#[derive(Debug, Serialize, Deserialize)]
struct Cursor {
    version: u8,
    query: String,
    revision: String,
    last_record_id: String,
    /// Relevance sort-key components (added in cursor v2). For a v1 cursor
    /// supplied by an older client, `search` rejects it as `CursorStale`
    /// (HTTP 409 `cursor_stale`) so the UI's existing recovery path re-issues
    /// the first page under v2 — there is no persistent cursor storage, so
    /// this is transparent to the user.
    last_title_match: bool,
    last_observed_at: i64,
    last_coverage_full: bool,
}

const CURSOR_VERSION: u8 = 2;

pub fn search(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    request: SearchRequest,
) -> Result<SearchPage, QueryError> {
    let store = ScanStore::new(conn);
    let revision = store.current_index_revision().map_err(|_| QueryError::Internal)?;
    let cursor = match request.cursor() {
        Some(raw) => {
            // A `v1.<hex>` cursor comes from a pre-2.3 client (record_id-only
            // sort key). The relevance sort key changed in 2.3, so the index
            // shape is incompatible — treat it as `CursorStale` (HTTP 409
            // `cursor_stale`) rather than `BadRequest`. The existing UI
            // recovery path for `cursor_stale` re-runs the first page, which is
            // the correct outcome; a generic contract error would surface an
            // opaque `bad_request` instead. v2 decode logic is unchanged below.
            if raw.starts_with("v1.") {
                return Err(QueryError::CursorStale);
            }
            let cursor = decode_cursor(raw).ok_or(QueryError::BadRequest)?;
            if cursor.version != CURSOR_VERSION || cursor.query != request.query() {
                return Err(QueryError::BadRequest);
            }
            if cursor.revision != revision { return Err(QueryError::CursorStale); }
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
        }))
    } else { None };
    let sources = source_status_sidecar(registry, &store)?;
    let empty_state = if results.is_empty() && cursor.is_none() {
        empty_state(registry, &store)?
    } else { None };
    Ok(SearchPage::new(results, next_cursor, empty_state, sources))
}

/// Build the FR-14 per-query availability sidecar: one row per **confirmed**
/// source describing its availability for this query. Derived from
/// `health_state` + active-generation presence + `latest_run` state. A down
/// source's already-indexed records (if any) are NOT suppressed by this
/// sidecar — the flag is informational (Design Notes: "the sidecar flags; it
/// does not hide").
///
/// FR-14 best-effort: a per-source status lookup failure (e.g. a corrupt
/// `scan_runs.state` row or an unreadable active-generation marker) is logged
/// and falls back to a conservative status for THAT source only — it NEVER
/// propagates [`QueryError::Internal`]. "One unavailable source never breaks
/// the query" extends to the sidecar, not just the result rows.
fn source_status_sidecar(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
) -> Result<Vec<SourceQueryStatus>, QueryError> {
    let sources = registry.list().map_err(|_| QueryError::Internal)?;
    let mut out = Vec::new();
    for source in sources
        .into_iter()
        .filter(|source| source.lifecycle_state == SourceLifecycle::Confirmed)
    {
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
    encoded.push_str("v2.");
    for byte in json { encoded.push_str(&format!("{byte:02x}")); }
    encoded
}

fn decode_cursor(raw: &str) -> Option<Cursor> {
    let hex = raw.strip_prefix("v2.")?;
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
        };
        let encoded = encode_cursor(&cursor);
        assert!(!encoded.contains("记忆"));
        assert!(encoded.starts_with("v2."));
        let back = decode_cursor(&encoded).unwrap();
        assert_eq!(back.query, "记忆");
        assert_eq!(back.last_record_id, "rec_2");
        assert!(back.last_title_match);
        assert_eq!(back.last_observed_at, 99);
        assert!(back.last_coverage_full);
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
}
