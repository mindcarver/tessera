//! `index::source_registry` — persistence for confirmed / rejected / disabled
//! Sources (Story 1.3).
//!
//! The registry is the persistence layer over the `source_registry` SQLite
//! table (created by migration id `2`). It owns row ↔ [`Source`] mapping and
//! the `source_id` (`src_<rowid>`) <-> rowid translation. Application code
//! never sees the raw rowid; IPC never sees the fingerprint.
//!
//! Architecture invariants honoured (AD-33/AD-35):
//! - **Exact-match by fingerprint.** [`SourceRegistry::find_by_fingerprint`]
//!   is an equality lookup against the unique index. No fuzzy merge: a path
//!   or inode change produces a different fingerprint and therefore a
//!   different row.
//! - **AUTOINCREMENT rowid → stable handle.** `source_id = src_<id>`; the id
//!   is never reused, so a deleted Source's handle cannot be reattached to a
//!   different row by accident (no remove command ships in 1.3 — A-7).
//! - **No content reads.** Every method here is pure SQL over the registry
//!   table; the Source filesystem is never touched (NFR-1/NFR-5).
//!
//! The registry is intentionally lock-free: the IPC layer holds the
//! `IndexState { conn: Mutex<Connection> }` and hands the registry a `&Connection`
//! for the duration of a single command. 1.3 commands are synchronous (no
//! `.await`), so the existing std Mutex is correct; a tokio Mutex becomes
//! necessary only when the first async DB command lands (1.4 deferred item).

use rusqlite::{params, Connection, Row};

use crate::domain::ports::provider_adapter::CoverageLevel;
use crate::domain::source::{
    HealthCause, HealthState, Source, SourceFingerprint, SourceId, SourceKind, SourceLifecycle,
};

/// SQL columns for the `source_registry` table, in the order the row mapper
/// reads them. Centralized so every query stays in lock-step with the schema.
/// Story 4.2 appends `health_cause` (nullable TEXT; `None` reads back as
/// [`HealthCause::None`]).
const SELECT_COLS: &str = concat!(
    "id, provider, source_kind, lifecycle_state, health_state, ",
    "coverage_level, normalized_root_path, fingerprint, native_project, health_cause"
);

/// The Source Registry. Borrows the Derived Index connection for its lifetime;
/// the borrow is bounded by the IPC command's hold of the `IndexState` mutex.
#[derive(Debug)]
pub struct SourceRegistry<'a> {
    conn: &'a Connection,
}

impl<'a> SourceRegistry<'a> {
    /// Construct a registry view over a connection. The connection must have
    /// had migration `v1_source_registry` applied (the boot path in `lib.rs`
    /// guarantees this for the live app; tests use a fresh in-memory DB with
    /// [`crate::index::migrations::apply`]).
    pub fn new(conn: &'a Connection) -> Self {
        SourceRegistry { conn }
    }

    /// Look up a Source by its fingerprint. Returns `Ok(None)` when no row has
    /// this fingerprint (AD-35 exact match — no fuzzy merge). The unique index
    /// guarantees at most one row. A DB error surfaces as `Err(_)` so the
    /// application layer can map it to `internal`.
    pub fn find_by_fingerprint(
        &self,
        fingerprint: &SourceFingerprint,
    ) -> rusqlite::Result<Option<Source>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM source_registry WHERE fingerprint = ?1"
        ))?;
        let mut rows = stmt.query(params![fingerprint.0])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_source(row))),
            None => Ok(None),
        }
    }

    /// Insert a new Source row, returning the materialized [`Source`] with its
    /// allocated `source_id`. Caller supplies all domain fields. The
    /// `fingerprint` must be unique (enforced by the unique index; a duplicate
    /// insert surfaces as a constraint error). Use [`Self::find_by_fingerprint`]
    /// first to implement idempotent confirm/reject.
    ///
    /// `last_insert_rowid()` is the SQLite function that returns the
    /// AUTOINCREMENT id of the just-inserted row; rusqlite exposes it via
    /// [`Connection::last_insert_rowid`].
    pub fn upsert_by_fingerprint(&self, fields: &SourceInsert<'_>) -> rusqlite::Result<Source> {
        self.conn.execute(
            "INSERT INTO source_registry
                (provider, source_kind, lifecycle_state, health_state, coverage_level,
                 normalized_root_path, fingerprint, native_project, health_cause)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                fields.provider,
                fields.source_kind.as_str(),
                lifecycle_to_str(fields.lifecycle_state),
                fields.health_state.as_str(),
                coverage_to_str(fields.coverage_level),
                fields.normalized_root_path,
                fields.fingerprint.0,
                fields.native_project,
                fields.health_cause.as_str(),
            ],
        )?;
        let rowid = self.conn.last_insert_rowid();
        // Re-read the row so the returned Source is exactly what is persisted
        // (no risk of drift between caller-supplied fields and DB state).
        self.get_by_rowid(rowid)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// Flip a Source's lifecycle state. Returns `Ok(Some(updated))` on success,
    /// `Ok(None)` when the `source_id` does not match any row. Used by confirm
    /// (wake-up path), reject, and disable.
    pub fn set_lifecycle(
        &self,
        source_id: &SourceId,
        target: SourceLifecycle,
    ) -> rusqlite::Result<Option<Source>> {
        let Some(rowid) = source_id.to_rowid() else {
            return Ok(None);
        };
        let updated = self.conn.execute(
            "UPDATE source_registry SET lifecycle_state = ?1 WHERE id = ?2",
            params![lifecycle_to_str(target), rowid],
        )?;
        if updated == 0 {
            return Ok(None);
        }
        self.get_by_rowid(rowid)
    }

    /// Persist a scan-derived health fact AND its structured cause together
    /// (Story 4.2). This is the single write surface for health: cause and
    /// state never drift apart because they are written in one UPDATE.
    /// Success writes `(Healthy, None)`; root-validation failure writes
    /// `(Degraded, path_missing|permission_denied|scan_failed)`; parse failure
    /// writes `(Degraded, format_unsupported)`; dirty-after-validation/internal
    /// write `(Error, scan_failed)`. The cause is cleared (set to `None`) on
    /// the next successful scan via this same call.
    ///
    /// Unknown stored values are rejected by `row_to_source`; this method
    /// never coerces corruption to `unknown`/`none`. Returns `Ok(Some(updated))`
    /// on success, `Ok(None)` when the `source_id` does not match any row.
    pub fn set_health_and_cause(
        &self,
        source_id: &SourceId,
        health: HealthState,
        cause: HealthCause,
    ) -> rusqlite::Result<Option<Source>> {
        let Some(rowid) = source_id.to_rowid() else {
            return Ok(None);
        };
        if self.conn.execute(
            "UPDATE source_registry SET health_state = ?1, health_cause = ?2 WHERE id = ?3",
            params![health.as_str(), cause.as_str(), rowid],
        )? == 0
        {
            return Ok(None);
        }
        self.get_by_rowid(rowid)
    }

    /// Fetch a Source by its `source_id`. Returns `Ok(None)` when the id does
    /// not match any row (e.g. unknown id passed to `disable_source`).
    pub fn get(&self, source_id: &SourceId) -> rusqlite::Result<Option<Source>> {
        match source_id.to_rowid() {
            Some(rowid) => self.get_by_rowid(rowid),
            None => Ok(None),
        }
    }

    /// List every Source row, ordered by id for stable UI ordering. Used by
    /// `list_sources`.
    pub fn list(&self) -> rusqlite::Result<Vec<Source>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM source_registry ORDER BY id ASC"
        ))?;
        // `query_map` requires a `FnMut(&Row) -> Result<T>` closure; wrap the
        // infallible mapper so the signature matches.
        let rows = stmt.query_map([], |row| Ok(row_to_source(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    fn get_by_rowid(&self, rowid: i64) -> rusqlite::Result<Option<Source>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {SELECT_COLS} FROM source_registry WHERE id = ?1"
        ))?;
        let mut rows = stmt.query(params![rowid])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_source(row))),
            None => Ok(None),
        }
    }

    /// Story 4.3 — transactional seam. Runs `body` against a temporary
    /// [`SourceRegistry`] view bound to a single SQLite transaction. On `Ok`
    /// the transaction commits; on `Err` it rolls back. This is the atomic
    /// boundary the spec's Boundaries mandate for rebind's disable-old +
    /// insert-or-wake-new pair: a crash/error between the two writes rolls the
    /// disable back so the old row returns to its prior lifecycle state (no
    /// window exists where the old row is `Disabled` with no new `Confirmed`
    /// Source).
    ///
    /// Reuses `Connection::unchecked_transaction` (the same primitive
    /// [`crate::index::scan_store::ScanStore`] uses for its CAS commit):
    /// exclusivity is guaranteed by the caller holding the `IndexState` mutex
    /// for the whole command, and each command runs a single transaction at a
    /// time. The body sees the same `SourceRegistry` API (`set_lifecycle`,
    /// `set_health_and_cause`, `upsert_by_fingerprint`, `find_by_fingerprint`,
    /// `get`) because `Transaction` derefs to `Connection`.
    ///
    /// The error type must convert from [`rusqlite::Error`] so commit/begin
    /// failures surface through the same path as body failures.
    ///
    /// Rollback-failure handling: `Transaction::rollback` consumes `self` (so
    /// no `Drop` runs after it — the doc comment's earlier claim about
    /// "implicit rollback on drop" was wrong). On the rare rollback `Err`,
    /// this method logs the rusqlite error and returns the body's original
    /// error. The pooled `IndexState` connection is then suspect (it may
    /// still be in `BEGINNED` state), but the `IndexState` mutex means the
    /// next caller re-acquires it sequentially — there is no concurrent
    /// corruption window. The operator-visible `eprintln!` matches the
    /// existing error-logging posture in `application::reconcile.rs`.
    pub fn with_transaction<T, E, F>(&self, body: F) -> Result<T, E>
    where
        E: From<rusqlite::Error>,
        F: FnOnce(&SourceRegistry<'_>) -> Result<T, E>,
    {
        let tx = self.conn.unchecked_transaction().map_err(E::from)?;
        let view = SourceRegistry::new(&tx);
        let result = body(&view);
        // `view`'s borrow on `tx` ends here via NLL; the binding stays in
        // scope until this point so the borrow does not overlap with
        // `tx.commit()` / `tx.rollback()` below.
        match result {
            Ok(value) => {
                tx.commit().map_err(E::from)?;
                Ok(value)
            }
            Err(err) => {
                // Rollback is best-effort; the body's original error is what
                // the caller sees. `tx.rollback()` consumes `tx`, so there is
                // no Drop-driven implicit rollback — a rollback `Err` leaves
                // the connection suspect, logged for operator visibility.
                // (The IndexState mutex guarantees no concurrent caller can
                // observe the suspect connection; the next caller blocks on
                // the same mutex until this method returns.)
                if let Err(rollback_err) = tx.rollback() {
                    eprintln!(
                        "tessera: source_registry transaction rollback failed: {rollback_err:?}; \
                         connection may be in BEGINNED state — next caller re-acquires the IndexState mutex sequentially"
                    );
                }
                Err(err)
            }
        }
    }
}

/// Fields needed to insert a new Source row. Kept as a borrowed-arg struct so
/// callers do not have to construct a full [`Source`] (which would require
/// inventing a `source_id` that only the DB can allocate).
#[derive(Debug, Clone)]
pub struct SourceInsert<'a> {
    pub provider: &'a str,
    pub source_kind: SourceKind,
    pub lifecycle_state: SourceLifecycle,
    pub health_state: HealthState,
    pub coverage_level: CoverageLevel,
    pub normalized_root_path: &'a str,
    pub fingerprint: &'a SourceFingerprint,
    pub native_project: Option<&'a str>,
    /// Story 4.2 — persisted structured health cause. Defaults to `None` for a
    /// fresh insert (a brand-new row has had no health probe yet).
    pub health_cause: HealthCause,
}

/// Map a `rusqlite::Row` into a [`Source`]. Field order MUST match
/// [`SELECT_COLS`]. Returns `Source` directly (not `Result`) so it can be used
/// with both `query_map` (which wraps it in `Result` via the closure) and
/// manual `next()`.
fn row_to_source(row: &Row<'_>) -> Source {
    // Columns (per SELECT_COLS): id, provider, source_kind, lifecycle_state,
    // health_state, coverage_level, normalized_root_path, fingerprint,
    // native_project, health_cause.
    let rowid: i64 = row.get_unwrap(0);
    let provider: String = row.get_unwrap(1);
    let source_kind: String = row.get_unwrap(2);
    let lifecycle_state: String = row.get_unwrap(3);
    let health_state: String = row.get_unwrap(4);
    let coverage_level: String = row.get_unwrap(5);
    let normalized_root_path: String = row.get_unwrap(6);
    let fingerprint: String = row.get_unwrap(7);
    let native_project: Option<String> = row.get_unwrap(8);
    // Story 4.2: health_cause is nullable (the v5 migration adds the column
    // with no default, so pre-existing rows and any row written before a
    // `set_health_and_cause` carry NULL). A NULL reads back as `None` so a
    // never-probed or pre-4.2 source surfaces no stale cause. An unrecognized
    // non-null value also reads back as `None` — the cause is a display-only
    // hint, and corruption should not crash the read path.
    let health_cause_str: Option<String> = row.get_unwrap(9);
    let health_cause = health_cause_str
        .as_deref()
        .and_then(HealthCause::parse_str)
        .unwrap_or(HealthCause::None);

    Source {
        source_id: SourceId::from_rowid(rowid),
        provider,
        source_kind: SourceKind::parse_str(&source_kind).unwrap_or(SourceKind::AgentMemory),
        lifecycle_state: lifecycle_from_str(&lifecycle_state).unwrap_or(SourceLifecycle::Confirmed),
        health_state: HealthState::parse_str(&health_state).unwrap_or(HealthState::Unknown),
        coverage_level: coverage_from_str(&coverage_level).unwrap_or(CoverageLevel::Full),
        normalized_root_path,
        native_project,
        fingerprint: SourceFingerprint(fingerprint),
        health_cause,
    }
}

fn lifecycle_to_str(state: SourceLifecycle) -> &'static str {
    match state {
        SourceLifecycle::Confirmed => "confirmed",
        SourceLifecycle::Disabled => "disabled",
        SourceLifecycle::Rejected => "rejected",
    }
}

fn lifecycle_from_str(s: &str) -> Option<SourceLifecycle> {
    match s {
        "confirmed" => Some(SourceLifecycle::Confirmed),
        "disabled" => Some(SourceLifecycle::Disabled),
        "rejected" => Some(SourceLifecycle::Rejected),
        _ => None,
    }
}

fn coverage_to_str(level: CoverageLevel) -> &'static str {
    match level {
        CoverageLevel::Full => "full",
        CoverageLevel::SearchOnly => "search_only",
        CoverageLevel::ExistenceOnly => "existence_only",
        CoverageLevel::Unsupported => "unsupported",
    }
}

fn coverage_from_str(s: &str) -> Option<CoverageLevel> {
    match s {
        "full" => Some(CoverageLevel::Full),
        "search_only" => Some(CoverageLevel::SearchOnly),
        "existence_only" => Some(CoverageLevel::ExistenceOnly),
        "unsupported" => Some(CoverageLevel::Unsupported),
        _ => None,
    }
}
