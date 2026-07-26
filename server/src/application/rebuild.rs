//! `application::rebuild` — Full Derived Index rebuild core (Story 4.4).
//!
//! The synchronous rebuild core: a pure application-layer callable that wipes
//! exactly the Tessera-derived tables and returns the list of Confirmed
//! Source ids the HTTP layer must re-scan. The HTTP layer (in `http::mod`)
//! layers transport job tracking + per-source worker dispatch on top.
//!
//! ## Intent (binding contract — see spec-4-4-rebuild-index.md)
//!
//! Rebuild is a repeatable RUNTIME operation, not a schema migration. The v3
//! migration (`migrations::v3_canonical_memory_records`) already proved the
//! "clear derived, preserve registry" SQL once; 4.4 promotes it to a user-
//! triggered action callable via `POST /api/index/rebuild`.
//!
//! Boundaries honored:
//! - **AD-29 reset boundary.** The wipe deletes EXACTLY `memory_records`,
//!   `scan_runs`, `scan_diagnostics`, and `tessera_meta` rows matching
//!   `active_generation:%` in ONE SQLite transaction. It MUST NOT touch
//!   `source_registry`, `tessera_meta.schema_version`,
//!   `tessera_migrations_applied`, or any other `tessera_meta` key.
//! - **Reject-if-in-flight race guard.** Returns [`RebuildError::InFlight`]
//!   when ANY source has a `queued/running/staging/committing` run. This
//!   prevents a wipe mid-pipeline (a scan that has already staged data would
//!   otherwise find its `scan_runs` row deleted and its staged rows reclaimed
//!   by the next recovery/wipe). The HTTP layer maps this to a 409
//!   `rebuild_failed` envelope so the UI can tell the user to wait or cancel.
//! - **Only Confirmed sources are re-scanned** (returned to the caller for
//!   dispatch via the existing scan pipeline). Disabled/Rejected rows are NOT
//!   in the rescan set, but their leaked derived records (from a prior
//!   confirm/scan) ARE cleared by the wipe — this is the first path that ever
//!   clears them.
//! - **Zero-source-mutation gate (NFR-1/NFR-10).** This function does only
//!   SQL + registry reads; the read-only scan pipeline (which the HTTP layer
//!   re-dispatches) is the one that touches source files, and it has its own
//!   zero-mutation contract verified by the existing 1.4/4.2 test suite.

use rusqlite::Connection;

use crate::domain::source::{SourceId, SourceLifecycle};
use crate::index::scan_store::ScanStore;
use crate::index::SourceRegistry;

/// The error raised by [`rebuild_index`]. The HTTP layer maps each variant
/// onto a stable API code (AD-13) — see `http::start_rebuild`.
///
/// - [`Self::InFlight`] → 409 `rebuild_failed` (the primary race guard: a
///   scan is already mid-flight; no wipe, no reservation).
/// - [`Self::Internal`] → 500 `internal` (wipe / DB failure).
#[derive(Debug)]
pub enum RebuildError {
    /// At least one scan run is currently `queued/running/staging/committing`
    /// across ANY source. The rebuild MUST NOT proceed: the wipe would race
    /// with the in-flight scan's commit. The HTTP layer surfaces this as a 409
    /// `rebuild_failed` envelope; the UI tells the user to wait or cancel.
    InFlight,
    /// The wipe itself failed (SQLite error). Surfaces as a 500 `internal`
    /// envelope; the index is unchanged because the wipe transaction rolls
    /// back on the error.
    Internal,
}

/// Run the synchronous rebuild core (Story 4.4).
///
/// Steps:
/// 1. **Race guard:** if any scan run is in-flight across ANY source, return
///    [`RebuildError::InFlight`] BEFORE any wipe or reservation. This is the
///    primary race guard: a wipe mid-pipeline would race with the in-flight
///    scan's commit.
/// 2. **Wipe:** [`ScanStore::reset_derived_data`] runs the four-target DELETE
///    in ONE transaction (AD-29 reset boundary). On failure the transaction
///    rolls back and the index is unchanged; the error surfaces as
///    [`RebuildError::Internal`].
/// 3. **Collect:** enumerate the registry and return the list of Confirmed
///    Source ids the caller must re-scan. Disabled/Rejected rows are NOT in
///    the rescan set, but their leaked derived records (if any) ARE cleared
///    by the wipe.
///
/// The caller (the HTTP layer) is responsible for dispatching one rescan per
/// returned Source id via the existing scan pipeline. The wipe +
/// per-source-run-reservation is performed UNDER the IndexState mutex (see
/// `http::start_rebuild`); workers then run on their own connections exactly
/// like `start_rescan` (NFR-12: queries stay available during the rebuild).
pub fn rebuild_index(conn: &Connection) -> Result<Vec<SourceId>, RebuildError> {
    let store = ScanStore::new(conn);
    // Race guard FIRST: never wipe while a scan is mid-flight. The wipe would
    // delete the in-flight scan's `scan_runs` row, leaving it unable to commit
    // (commit_cas returns Ok(false), generation never activates); rejecting
    // up-front gives the user a clear "wait/cancel and retry".
    if store.any_in_flight_run().map_err(|_| RebuildError::Internal)? {
        return Err(RebuildError::InFlight);
    }
    // Atomic four-target wipe. Rolls back on failure → index unchanged.
    store.reset_derived_data().map_err(|_| RebuildError::Internal)?;

    // Collect Confirmed Source ids for the caller to re-scan. Disabled /
    // Rejected rows are NOT in the set, but their leaked derived records (from
    // a prior confirm/scan) were already cleared by the wipe.
    let registry = SourceRegistry::new(conn);
    let sources = registry.list().map_err(|_| RebuildError::Internal)?;
    let confirmed: Vec<SourceId> = sources
        .into_iter()
        .filter(|source| source.lifecycle_state == SourceLifecycle::Confirmed)
        .map(|source| source.source_id)
        .collect();
    Ok(confirmed)
}
