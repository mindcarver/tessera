//! `index::project_store` — persistence for Tessera Projects and their explicit
//! `(provider, native_project)` mappings (Story 5.1).
//!
//! The project store is the persistence layer over the `tessera_projects` and
//! `project_mappings` SQLite tables (created by migration id `7`). It owns
//! row ↔ [`TesseraProject`] and row ↔ [`ProjectMapping`] mapping, plus the
//! `project_id` (`proj_<rowid>`) ↔ rowid translation. Application code never
//! sees the raw rowid; IPC never sees the raw INSERT/UPDATE SQL.
//!
//! Architecture invariants honoured (AD-2/AD-27/AD-29/AD-33):
//! - **AUTOINCREMENT rowid → stable handle.** `project_id = proj_<id>`; the
//!   id is never reused, so a deleted project's handle cannot be reattached
//!   to a different row by accident (Story 5.1 ships `delete_project`).
//! - **Explicit mappings cleanup on delete.** `project_mappings.tessera_project_id`
//!   carries `REFERENCES tessera_projects(id) ON DELETE CASCADE`, but the store
//!   deletes a project's mappings explicitly first (cascade-independent —
//!   correctness must not depend on `PRAGMA foreign_keys = ON` being set on the
//!   connection). The explicit delete also yields the actual `removed_mappings`
//!   count, reported atomically inside the application-layer transaction.
//! - **AD-27 storage backstop.** The `project_mappings_scope_unique` index
//!   enforces "at most one active project per `(provider, native_project)`"
//!   (NULL collapsed via `COALESCE`). The application layer pre-checks the
//!   scope inside the transaction to return `mapping_conflict` naming the
//!   owner; the index is the concurrency backstop if two writers race.
//! - **No content reads.** Every method here is pure SQL over the two
//!   project tables; the Source filesystem is never touched
//!   (NFR-1/NFR-5). Project operations never delete or modify canonical
//!   `memory_records` or `source_registry` rows.

use rusqlite::{params, Connection, Row};

use crate::domain::project::{
    ProjectId, ProjectMapping, TesseraProject, TesseraProjectView,
};
use crate::index::scan_store::unix_seconds_now_i64;

/// SQL columns for the `tessera_projects` table, in the order the row mapper
/// reads them. Centralized so every query stays in lock-step with the schema.
const PROJECT_SELECT_COLS: &str = "id, name, created_at, updated_at";

/// The Tessera Project store. Borrows the Derived Index connection for its
/// lifetime; the borrow is bounded by the IPC command's hold of the
/// `IndexState` mutex (mirrors [`crate::index::source_registry::SourceRegistry`]).
#[derive(Debug)]
pub struct ProjectStore<'a> {
    conn: &'a Connection,
}

impl<'a> ProjectStore<'a> {
    /// Construct a project-store view over a connection. The connection must
    /// have had migration `v6_tessera_projects` applied (the boot path in
    /// `lib.rs` guarantees this for the live app; tests use a fresh
    /// in-memory DB with [`crate::index::migrations::apply`]).
    pub fn new(conn: &'a Connection) -> Self {
        ProjectStore { conn }
    }

    /// Insert a new Tessera Project row, returning the materialized row with
    /// its allocated `project_id`. Caller supplies all domain fields. The
    /// store stamps `created_at` and `updated_at` with the SAME Unix-seconds
    /// value (the I/O matrix row "Create project" requires equal
    /// `created_at`/`updated_at` on the response); the first rename advances
    /// `updated_at`.
    pub fn create(&self, name: &str) -> rusqlite::Result<TesseraProject> {
        let now = unix_seconds_now_i64();
        self.conn.execute(
            "INSERT INTO tessera_projects (name, created_at, updated_at) VALUES (?1, ?2, ?3)",
            params![name, now, now],
        )?;
        let rowid = self.conn.last_insert_rowid();
        // Re-read so the returned project is exactly what is persisted.
        self.get_by_rowid(rowid)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    /// List every Tessera Project row, ordered by id for stable UI ordering
    /// (mirrors `SourceRegistry::list`).
    pub fn list(&self) -> rusqlite::Result<Vec<TesseraProject>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PROJECT_SELECT_COLS} FROM tessera_projects ORDER BY id ASC"
        ))?;
        let rows = stmt.query_map([], |row| Ok(row_to_project(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Fetch a Tessera Project by its `project_id`. Returns `Ok(None)` when
    /// the id does not match any row (e.g. unknown id passed to `rename` /
    /// `delete` / `add_mapping`).
    pub fn get(&self, project_id: &ProjectId) -> rusqlite::Result<Option<TesseraProject>> {
        match project_id.to_rowid() {
            Some(rowid) => self.get_by_rowid(rowid),
            None => Ok(None),
        }
    }

    /// Rename a Tessera Project. Returns `Ok(Some(updated))` on success,
    /// `Ok(None)` when the `project_id` does not match any row. Advances
    /// `updated_at` to the current Unix seconds (the I/O matrix row "Rename
    /// project" requires `updated_at` strictly greater than `created_at`
    /// after a rename — `unix_seconds_now_i64()` returns 0 on a broken RTC,
    /// which is the only way `updated_at` could fail to advance past a
    /// previously-stamped `created_at`, and a broken RTC is the operator's
    /// problem, not a contract break).
    pub fn rename(
        &self,
        project_id: &ProjectId,
        new_name: &str,
    ) -> rusqlite::Result<Option<TesseraProject>> {
        let Some(rowid) = project_id.to_rowid() else {
            return Ok(None);
        };
        let now = unix_seconds_now_i64();
        if self.conn.execute(
            "UPDATE tessera_projects SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_name, now, rowid],
        )? == 0
        {
            return Ok(None);
        }
        self.get_by_rowid(rowid)
    }

    /// Delete a Tessera Project. Returns `Ok(Some(count))` where `count` is
    /// the actual number of mappings removed, or `Ok(None)` when the
    /// `project_id` does not match any row. Mappings are deleted EXPLICITLY
    /// first (cascade-independent — correctness must not depend on
    /// `PRAGMA foreign_keys = ON` being set on the connection; the
    /// `ON DELETE CASCADE` foreign key remains as a belt-and-suspenders
    /// backstop). Because the mappings are deleted while the project row still
    /// exists, `count` is the real number of rows deleted, not a pre-count
    /// that could diverge from reality. The application layer wraps this in
    /// [`Self::with_transaction`] so the mappings delete + the project delete
    /// commit atomically.
    pub fn delete(&self, project_id: &ProjectId) -> rusqlite::Result<Option<u32>> {
        let Some(rowid) = project_id.to_rowid() else {
            return Ok(None);
        };
        // Confirm the project exists first so we can distinguish "deleted N
        // mappings" from "no such project" (a row that doesn't exist yields
        // Some(0) otherwise, which the I/O matrix treats as 404, not 200).
        if self.get_by_rowid(rowid)?.is_none() {
            return Ok(None);
        }
        // Remove this project's mappings explicitly, THEN the project row.
        // Order matters: deleting mappings while the project row still exists
        // means `removed` is the actual count and is independent of whether
        // the FK cascade fires. The subsequent project DELETE is a no-op for
        // mappings (already gone); the cascade is only a backstop.
        let removed = self.conn.execute(
            "DELETE FROM project_mappings WHERE tessera_project_id = ?1",
            params![rowid],
        )?;
        let touched = self.conn.execute(
            "DELETE FROM tessera_projects WHERE id = ?1",
            params![rowid],
        )?;
        if touched == 0 {
            // Raced with a concurrent delete between the get_by_rowid and the
            // DELETE — treat as not found so the caller surfaces 404.
            return Ok(None);
        }
        // `removed` is bounded by the number of mappings that existed, which
        // is at most a few hundred per project in realistic use; the u32 cast
        // cannot overflow.
        Ok(Some(u32::try_from(removed).unwrap_or(u32::MAX)))
    }

    /// Add an explicit `(provider, native_project)` mapping to a project.
    /// Returns `Ok(Some(idempotent_existing_project_id))` if the scope is
    /// already owned (by this project or another). The application layer
    /// interprets that:
    /// - same project → idempotent success (return the unchanged view);
    /// - different project → `mapping_conflict` naming the owner.
    ///
    /// Returning the owning project id from the store keeps the SQL +
    /// rowid→handle translation in one place; the application layer never
    /// sees a raw rowid. The unique index is the storage backstop for the
    /// rare race where two writers pass the pre-check simultaneously.
    pub fn find_mapping_owner(
        &self,
        provider: &str,
        native_project: Option<&str>,
    ) -> rusqlite::Result<Option<i64>> {
        // COALESCE collapses NULL (Codex global) to '' so the lookup matches
        // the uniqueness index's NULL handling exactly. The application
        // layer has already validated `native_project` shape before calling.
        // `query_row` returns `QueryReturnedNoRows` when the scope is free —
        // collapse that to `Ok(None)` so the caller can branch on
        // "free vs owned by this project vs owned by another project"
        // without catching errors.
        match self.conn.query_row(
            "SELECT tessera_project_id FROM project_mappings \
             WHERE provider = ?1 AND COALESCE(native_project, '') = COALESCE(?2, '')",
            params![provider, native_project],
            |row| row.get(0),
        ) {
            Ok(owner_rowid) => Ok(Some(owner_rowid)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Insert a mapping row. Assumes the application layer has already
    /// pre-checked the scope ([`Self::find_mapping_owner`]) and judged it
    /// either free or idempotently owned by THIS project. Returns the
    /// inserted mapping's autoincrement id (used by tests; the application
    /// layer re-reads the project view rather than trusting the raw id).
    pub fn insert_mapping(
        &self,
        project_rowid: i64,
        provider: &str,
        native_project: Option<&str>,
    ) -> rusqlite::Result<i64> {
        let now = unix_seconds_now_i64();
        self.conn.execute(
            "INSERT INTO project_mappings (tessera_project_id, provider, native_project, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![project_rowid, provider, native_project, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Remove a mapping row. Returns:
    /// - `Ok(Some(true))` — exactly one row removed (the I/O matrix "Remove
    ///   mapping" happy path returns the updated view).
    /// - `Ok(Some(false))` — the project exists but no mapping matched
    ///   `(provider, native_project)` for this project (the I/O matrix "Remove
    ///   a non-existent mapping" path returns 404 `mapping_not_found`).
    /// - `Ok(None)` — the `project_id` does not match any row (404
    ///   `project_not_found`).
    pub fn remove_mapping(
        &self,
        project_id: &ProjectId,
        provider: &str,
        native_project: Option<&str>,
    ) -> rusqlite::Result<Option<bool>> {
        let Some(rowid) = project_id.to_rowid() else {
            return Ok(None);
        };
        if self.get_by_rowid(rowid)?.is_none() {
            return Ok(None);
        }
        let touched = self.conn.execute(
            "DELETE FROM project_mappings \
             WHERE tessera_project_id = ?1 AND provider = ?2 \
             AND COALESCE(native_project, '') = COALESCE(?3, '')",
            params![rowid, provider, native_project],
        )?;
        Ok(Some(touched > 0))
    }

    /// List a project's mappings, ordered by `id` ascending (stable UI
    /// ordering). Returns an empty vec for a project with no mappings (the
    /// I/O matrix row "Create project" requires an empty `mappings` array).
    pub fn list_mappings(&self, project_id: &ProjectId) -> rusqlite::Result<Vec<ProjectMapping>> {
        let Some(rowid) = project_id.to_rowid() else {
            return Ok(Vec::new());
        };
        let mut stmt = self.conn.prepare(
            "SELECT id, tessera_project_id, provider, native_project, created_at \
             FROM project_mappings WHERE tessera_project_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![rowid], |row| Ok(row_to_mapping(row)))?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Assemble a [`TesseraProjectView`] from a project row + its mappings.
    /// Single round-trip per project so the UI gets a project + its mappings
    /// in one envelope. Mappings are projected onto [`NativeProjectRef`]
    /// (drops the rowid / `created_at` the UI never needs).
    pub fn view_for(&self, project: &TesseraProject) -> rusqlite::Result<TesseraProjectView> {
        let mappings = self.list_mappings(&project.project_id)?;
        Ok(TesseraProjectView {
            project_id: project.project_id.clone(),
            name: project.name.clone(),
            created_at: project.created_at,
            updated_at: project.updated_at,
            mappings: mappings.iter().map(ProjectMapping::to_ref).collect(),
        })
    }

    /// Story 5.2 — read the `project_mapping_revision` scalar from
    /// `tessera_meta`. Returns `0` when the key is absent (a fresh DB before
    /// migration id `8` runs, or any caller that has not yet applied the
    /// Story 5.2 migration). The value is parsed as `i64`; a non-numeric
    /// value (manual edit / corruption) collapses to `0` rather than
    /// surfacing an error so a corrupt key cannot break the read path — the
    /// next scope-set-changing op re-writes a numeric value via
    /// [`Self::bump_project_mapping_revision`].
    pub fn project_mapping_revision(&self) -> rusqlite::Result<i64> {
        match self.conn.query_row(
            "SELECT value FROM tessera_meta WHERE key = 'project_mapping_revision'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            Ok(value) => Ok(value.parse::<i64>().unwrap_or(0)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(error) => Err(error),
        }
    }

    /// Story 5.2 — bump the `project_mapping_revision` scalar (+1) inside
    /// the existing `ProjectStore::with_transaction` whenever a Tessera
    /// Project's mapped scope set changes (successful new `insert_mapping`,
    /// `remove_mapping` that deleted a row, `delete` that removed mappings).
    /// The application layer calls this only for scope-set-changing ops —
    /// `create` / `rename` / idempotent re-add do NOT bump because they leave
    /// the projection's result set unchanged (a renamed project still maps
    /// the same Native Projects, so every outstanding cursor's snapshot is
    /// still accurate).
    ///
    /// `CAST(COALESCE(CAST(value AS INTEGER), 0) + 1 AS TEXT)` performs the
    /// +1 in SQLite (no round-trip through the application layer) and stores
    /// the result back as TEXT, matching the `tessera_meta.value` column's
    /// TEXT affinity. `COALESCE(..., 0)` is NULL-safe: a missing or
    /// text-corrupt `value` collapses to `0` rather than propagating NULL
    /// back into the column (the unguarded `CAST(NULL AS INTEGER) + 1` is
    /// NULL, which would otherwise permanently zero the read path via the
    /// `parse::<i64>().unwrap_or(0)` self-masking in
    /// [`Self::project_mapping_revision`]).
    ///
    /// Affected-rows check: if the seed row is absent (a fixture that skipped
    /// migration id `8`, or any caller that has not yet applied Story 5.2's
    /// seed), the UPDATE matches 0 rows and — without the seed-then-retry
    /// below — every subsequent bump would silently no-op, leaving
    /// [`Self::project_mapping_revision`] reading `0` forever and breaking
    /// AD-31's cursor invalidation without a peep. Inserting the seed row
    /// then re-running the UPDATE guarantees the bump is never a silent
    /// no-op. Idempotent under the existing
    /// [`Self::with_transaction`] seam: an unsuccessful call surfaces as
    /// `rusqlite::Error` and the transaction rolls back, so the bump and
    /// the scope-set change commit atomically — or neither does.
    pub fn bump_project_mapping_revision(&self) -> rusqlite::Result<()> {
        let touched = self.conn.execute(
            "UPDATE tessera_meta SET value = CAST(COALESCE(CAST(value AS INTEGER), 0) + 1 AS TEXT) \
             WHERE key = 'project_mapping_revision'",
            [],
        )?;
        if touched == 0 {
            // Seed row absent. INSERT OR IGNORE keeps this concurrent-safe
            // (two racing bumps that both see 0 rows both INSERT OR IGNORE
            // the seed; exactly one wins, the other is a no-op) and the
            // second UPDATE always finds the row — guaranteeing the bump
            // commits +1 rather than silently disappearing.
            self.conn.execute(
                "INSERT OR IGNORE INTO tessera_meta(key, value) \
                 VALUES ('project_mapping_revision', '0')",
                [],
            )?;
            self.conn.execute(
                "UPDATE tessera_meta SET value = CAST(COALESCE(CAST(value AS INTEGER), 0) + 1 AS TEXT) \
                 WHERE key = 'project_mapping_revision'",
                [],
            )?;
        }
        Ok(())
    }

    fn get_by_rowid(&self, rowid: i64) -> rusqlite::Result<Option<TesseraProject>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {PROJECT_SELECT_COLS} FROM tessera_projects WHERE id = ?1"
        ))?;
        let mut rows = stmt.query(params![rowid])?;
        match rows.next()? {
            Some(row) => Ok(Some(row_to_project(row))),
            None => Ok(None),
        }
    }

    /// Story 5.1 — transactional seam, mirroring
    /// [`crate::index::source_registry::SourceRegistry::with_transaction`].
    /// Runs `body` against a temporary [`ProjectStore`] view bound to a
    /// single SQLite transaction. On `Ok` the transaction commits; on `Err`
    /// it rolls back.
    ///
    /// The cardinality pre-check + insert for `add_mapping` runs inside this
    /// transaction so the pre-check's read and the insert's write cannot
    /// observe a different state between them. `delete` also runs inside it
    /// so the `removed_mappings` count and the `DELETE` commit atomically.
    ///
    /// Reuses `Connection::unchecked_transaction` (the same primitive
    /// [`SourceRegistry`] uses): exclusivity is guaranteed by the caller
    /// holding the `IndexState` mutex for the whole command.
    pub fn with_transaction<T, E, F>(&self, body: F) -> Result<T, E>
    where
        E: From<rusqlite::Error>,
        F: FnOnce(&ProjectStore<'_>) -> Result<T, E>,
    {
        let tx = self.conn.unchecked_transaction().map_err(E::from)?;
        let view = ProjectStore::new(&tx);
        let result = body(&view);
        match result {
            Ok(value) => {
                tx.commit().map_err(E::from)?;
                Ok(value)
            }
            Err(err) => {
                if let Err(rollback_err) = tx.rollback() {
                    eprintln!(
                        "tessera: project_store transaction rollback failed: {rollback_err:?}; \
                         connection may be in BEGINNED state — next caller re-acquires the IndexState mutex sequentially"
                    );
                }
                Err(err)
            }
        }
    }
}

/// Map a `rusqlite::Row` into a [`TesseraProject`]. Field order MUST match
/// [`PROJECT_SELECT_COLS`].
fn row_to_project(row: &Row<'_>) -> TesseraProject {
    let rowid: i64 = row.get_unwrap(0);
    let name: String = row.get_unwrap(1);
    let created_at: i64 = row.get_unwrap(2);
    let updated_at: i64 = row.get_unwrap(3);
    TesseraProject {
        project_id: ProjectId::from_rowid(rowid),
        name,
        created_at,
        updated_at,
    }
}

/// Map a `rusqlite::Row` into a [`ProjectMapping`]. Field order matches the
/// `SELECT id, tessera_project_id, provider, native_project, created_at`
/// statement in [`ProjectStore::list_mappings`].
fn row_to_mapping(row: &Row<'_>) -> ProjectMapping {
    ProjectMapping {
        id: row.get_unwrap(0),
        tessera_project_id: row.get_unwrap(1),
        provider: row.get_unwrap(2),
        native_project: row.get_unwrap(3),
        created_at: row.get_unwrap(4),
    }
}
