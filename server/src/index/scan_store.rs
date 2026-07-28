//! `index::scan_store` — persistence for scan runs and memory-record staging
//! generations (Story 1.4).
//!
//! This is the persistence layer over the `scan_runs` and `memory_records`
//! SQLite tables (created by migration id `3`). It owns the fencing-token
//! allocation, the state-machine transitions, the staging-generation writes,
//! and the single-transaction CAS commit that makes a generation visible
//! (AD-5/AD-16/AD-28/AD-32).
//!
//! Architecture invariants honoured here:
//! - **Fencing token (AD-28/AD-32):** `fencing_token = MAX(fencing_token)+1`
//!   per `source_id`, computed inside [`ScanStore::begin_run`]'s INSERT
//!   transaction. `UNIQUE(source_id, fencing_token)` enforces monotonicity at
//!   the storage layer; SQLite's single-writer model guarantees the MAX+1 read
//!   and the INSERT are serialized.
//! - **Generation (Design Notes):** `generation = gen_<scan_run_id>` — both
//!   come from the same AUTOINCREMENT sequence, no clock/rand dependency.
//! - **Atomic CAS commit (AD-32):** [`ScanStore::commit_cas`] runs the CAS
//!   UPDATE against the per-source `MAX(fencing_token)` (the holder must be
//!   the LATEST owner — comparing only against its own row is no fence at
//!   all), the `tessera_meta.active_generation` write, the old-generation
//!   cleanup, and the `succeeded` mark in ONE transaction. A 0-row CAS rolls
//!   the whole transaction back — the active generation never moves.
//! - **Generation isolation is physical (NFR-9):** `memory_records` primary
//!   key is the composite `(record_id, generation)` and staging uses plain
//!   `INSERT` (never `INSERT OR REPLACE`). A `record_id` recurs across
//!   generations (locator-based, AD-15), so single-field PK + REPLACE would
//!   let staging overwrite ACTIVE generation rows.
//! - **Boot recovery (AD-16):** [`ScanStore::recover_stale_runs`] flips stale
//!   in-flight runs to `failed` with `error_code='stale_recovered'` and
//!   deletes all non-active generation records. A search continuation is
//!   rejected as stale after the active index changes.
//! - **Corruption is loud:** [`ScanStore::latest_run`] returns an error on an
//!   unparseable persisted state string — it does NOT silently map an unknown
//!   state to `failed` (spec: unknown state = data corruption, surfaced as
//!   `Internal`).

use rusqlite::{params, Connection};

use crate::domain::open::OpenTarget;
use crate::domain::ports::provider_adapter::ProviderMemoryType;
use crate::domain::ports::query_store::{BrowseCursorKey, QueryStore, SearchCursorKey};
use crate::domain::query::{BrowseRequest, SearchRequest, SearchResult};
use crate::domain::scan::{Generation, ScanRunState};
use crate::domain::source::{HealthState, SourceId};

/// The `tessera_meta` key prefix for the active-generation marker. The full
/// key is `active_generation:<source_rowid>` (Design Notes — active
/// generation 存哪). The meta table is the reserved home for Tessera app-data
/// scalars like this.
const ACTIVE_GENERATION_KEY_PREFIX: &str = "active_generation:";

/// The stable `error_code` written by boot recovery when it flips a stale
/// in-flight run to `failed` (spec Design Notes — "error_code 稳定词汇").
/// This is the one vocabulary value that is NOT a `ScanError` variant — it is
/// written only here.
const ERROR_CODE_STALE_RECOVERED: &str = "stale_recovered";

/// A staged canonical record to insert into a staging generation. Carries
/// everything `memory_records` persists except the generation (supplied by
/// the caller) — the application layer builds these from enumerated file
/// units + content hashes.
#[derive(Debug, Clone)]
pub struct StagedRecord {
    pub record_id: String,
    pub source_rowid: i64,
    pub provider: String,
    pub unit_kind: String,
    pub native_unit_id: String,
    pub native_locator: String,
    pub content_hash: String,
    pub parser_version: String,
    pub title: String,
    pub body: String,
    pub native_project: Option<String>,
    pub provider_memory_type: String,
    pub coverage_level: String,
    pub observed_at: i64,
    pub source_revision: String,
    pub display_locator: String,
}

/// A persisted source-scoped diagnostic for an unsupported lexical artifact.
/// It is intentionally detached from canonical content records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedDiagnostic {
    pub source_rowid: i64,
    pub kind: String,
    pub observed_path: String,
}

/// A Knowledge record row read back for Browse (Story 6.9). Parallel to
/// `SearchResult` but for the Knowledge domain: carries the Vault-relative
/// path, derived excerpt, and provenance without Agent-Memory-specific fields
/// (`native_project`, `provider_memory_type`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeRecordRow {
    pub record_id: String,
    /// The note's derived title (first # heading or filename stem).
    pub title: String,
    /// First ~320 chars of the note body (derived presentation).
    pub excerpt: String,
    pub provider: String,
    pub source_id: SourceId,
    /// Vault-relative path (the user-visible provenance + identity input).
    pub vault_relative_path: String,
    /// Same as vault_relative_path for Knowledge (no line-range refinement).
    pub display_locator: String,
    pub observed_at: i64,
    pub coverage_level: String,
    /// RFC 3339 source modification time when available.
    pub modified_time: Option<String>,
    pub health_state: HealthState,
}

/// Truncate note body to a browse-friendly excerpt (mirrors the Agent-Memory
/// `excerpt` helper but operates on title+body for Knowledge notes).
fn knowledge_excerpt(title: &str, body: &str) -> String {
    let combined = if title.is_empty() {
        body.to_string()
    } else {
        format!("{title}\n\n{body}")
    };
    let chars: Vec<char> = combined.chars().collect();
    if chars.len() <= 320 {
        combined
    } else {
        let mut s: String = chars[..320].iter().collect();
        s.push('…');
        s
    }
}

/// A staged Knowledge record pending atomic generation activation (Story 6.5
/// follow-up / Phase C.0). Mirrors [`StagedRecord`] but for the independent
/// `knowledge_records` table (AD-19/AD-38): `krec_` identity, file-level
/// `note` units, Vault-relative locators, Knowledge parser version, and no
/// `native_project`/`provider_memory_type` (Obsidian Vaults have no
/// Agent-Memory project concept). Built by the Knowledge scan pipeline from
/// `enumerate_notes` output + per-note content hashes.
#[derive(Debug, Clone)]
pub struct StagedKnowledgeRecord {
    pub record_id: String,
    pub source_rowid: i64,
    pub provider: String,
    pub unit_kind: String,
    pub native_unit_id: String,
    pub native_locator: String,
    pub content_hash: String,
    pub parser_version: String,
    pub modified_time: Option<String>,
    /// Story 6.9 — derived presentation columns for Browse/Search.
    pub title: String,
    pub body: String,
    pub display_locator: String,
    pub observed_at: i64,
    pub coverage_level: String,
}

/// A single row read back from `scan_runs` for status reporting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanRunRow {
    pub scan_id: i64,
    pub state: ScanRunState,
    pub generation: Generation,
    pub error_code: Option<String>,
    pub finished_at: Option<i64>,
}

/// The Scan Store. Borrows the Derived Index connection for its lifetime; the
/// borrow is bounded by the IPC command's hold of the `IndexState` mutex. All
/// methods are synchronous — the 1.4 commands are synchronous and the std
/// Mutex serializes them (single owner per Source, AD-5).
#[derive(Debug)]
pub struct ScanStore<'a> {
    conn: &'a Connection,
}

impl<'a> ScanStore<'a> {
    /// Construct a scan-store view over a connection that has had migration
    /// `v2_scan_generations` applied (boot guarantees this for the live app;
    /// tests use a fresh in-memory DB with [`crate::index::migrations::apply`]).
    pub fn new(conn: &'a Connection) -> Self {
        ScanStore { conn }
    }

    /// Begin a new scan run for `source_id`: allocate the next fencing token
    /// (MAX+1 per source), insert a `queued` row, and return
    /// `(scan_id, fencing_token, generation)`. The whole operation is one
    /// transaction so the token read and the INSERT are atomic.
    ///
    /// `manifest_revision` is captured at scan start (AD-34) and stored on the
    /// row; `intent` is the generation intent committed under CAS (in 1.4 the
    /// same string as `generation`).
    pub fn begin_run(
        &self,
        source_rowid: i64,
        manifest_revision: &str,
    ) -> rusqlite::Result<(i64, i64, Generation)> {
        // `unchecked_transaction` borrows `&Connection` (the store holds a
        // shared ref); exclusivity is guaranteed by the caller holding the
        // `IndexState` mutex for the whole command, and each store method runs
        // a single transaction at a time.
        let tx = self.conn.unchecked_transaction()?;
        // Next monotonic fencing token for this source. UNIQUE(source_id,
        // fencing_token) + SQLite single-writer make this race-free.
        let next_token: i64 = tx.query_row(
            "SELECT COALESCE(MAX(fencing_token), 0) + 1 FROM scan_runs WHERE source_id = ?1",
            params![source_rowid],
            |row| row.get(0),
        )?;
        // Insert with a placeholder generation first so we can read the
        // AUTOINCREMENT id, then set generation = gen_<id> and intent = same.
        tx.execute(
            "INSERT INTO scan_runs
                (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (?1, '', ?2, ?3, '', ?4)",
            params![
                source_rowid,
                ScanRunState::Queued.as_str(),
                next_token,
                manifest_revision
            ],
        )?;
        let scan_id = tx.last_insert_rowid();
        let generation = Generation::from_rowid(scan_id);
        tx.execute(
            "UPDATE scan_runs SET generation = ?1, intent = ?1 WHERE id = ?2",
            params![generation.0, scan_id],
        )?;
        tx.commit()?;
        Ok((scan_id, next_token, generation))
    }

    /// Move a run to a new state. Used for the `queued → running → staging →
    /// committing` progression. Returns the number of rows affected (1 on
    /// success). Does NOT set `finished_at` — that is written only by
    /// `fail_run` / `commit_cas`.
    pub fn set_state(&self, scan_id: i64, state: ScanRunState) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE scan_runs SET state = ?1 WHERE id = ?2
             AND state != ?3 AND cancel_requested = 0",
            params![state.as_str(), scan_id, ScanRunState::Failed.as_str()],
        )
    }

    /// Replace the placeholder manifest revision captured at `begin_run` with
    /// the real one once the first enumeration has produced the manifest
    /// (AD-34; the run must exist BEFORE the first enumeration so a crash
    /// during enumeration leaves a recoverable row). Returns rows affected.
    pub fn set_manifest_revision(
        &self,
        scan_id: i64,
        manifest_revision: &str,
    ) -> rusqlite::Result<usize> {
        self.conn.execute(
            "UPDATE scan_runs SET manifest_revision = ?1 WHERE id = ?2",
            params![manifest_revision, scan_id],
        )
    }

    /// Insert a batch of staged records into a staging generation. Each record
    /// is tagged with `generation`; only a generation whose CAS commit
    /// succeeds becomes visible. Batched in one transaction.
    ///
    /// **Plain `INSERT`, never `INSERT OR REPLACE` (NFR-9 / spec Design Notes
    /// — "generation 隔离是物理的").** The composite `(record_id, generation)`
    /// primary key means the same `record_id` staged under a NEW generation is
    /// a distinct row from the ACTIVE generation's row — staging can never
    /// overwrite the active index. A duplicate `(record_id, generation)` pair
    /// within one staging batch is a genuine conflict and surfaces as an
    /// error (the enumerator dedups by relative path, so this should not
    /// occur in practice).
    pub fn stage_records(
        &self,
        generation: &Generation,
        records: &[StagedRecord],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO memory_records
                    (record_id, source_id, generation, provider, unit_kind,
                     native_unit_id, native_locator, content_hash, parser_version,
                     title, body, native_project, provider_memory_type,
                     coverage_level, observed_at, source_revision, display_locator)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            )?;
            for r in records {
                stmt.execute(params![
                    r.record_id,
                    r.source_rowid,
                    generation.0,
                    r.provider,
                    r.unit_kind,
                    r.native_unit_id,
                    r.native_locator,
                    r.content_hash,
                    r.parser_version,
                    r.title,
                    r.body,
                    r.native_project,
                    r.provider_memory_type,
                    r.coverage_level,
                    r.observed_at,
                    r.source_revision,
                    r.display_locator,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Persist diagnostics alongside a staging generation. There is no body
    /// column here, so unknown artifacts can be explained without indexing or
    /// copying their source content.
    pub fn stage_diagnostics(
        &self,
        generation: &Generation,
        diagnostics: &[StagedDiagnostic],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO scan_diagnostics (source_id, generation, kind, observed_path)
                 VALUES (?1, ?2, ?3, ?4)",
            )?;
            for diagnostic in diagnostics {
                stmt.execute(params![
                    diagnostic.source_rowid,
                    generation.0,
                    diagnostic.kind,
                    diagnostic.observed_path,
                ])?;
            }
        }
        tx.commit()
    }

    /// Stage Knowledge records into the independent `knowledge_records` table
    /// (Story 6.5 follow-up / AD-38). Mirrors [`stage_records`] but writes the
    /// Knowledge canonical table, not `memory_records`. The composite
    /// `(record_id, generation)` PK means the same `krec_` staged under a NEW
    /// generation is distinct from the active generation's row — staging can
    /// never overwrite the active Knowledge index.
    pub fn stage_knowledge_records(
        &self,
        generation: &Generation,
        records: &[StagedKnowledgeRecord],
    ) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO knowledge_records
                    (record_id, source_id, generation, provider, unit_kind,
                     native_unit_id, native_locator, content_hash, parser_version,
                     modified_time, title, body, display_locator, observed_at, coverage_level)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
            )?;
            for r in records {
                stmt.execute(params![
                    r.record_id,
                    r.source_rowid,
                    generation.0,
                    r.provider,
                    r.unit_kind,
                    r.native_unit_id,
                    r.native_locator,
                    r.content_hash,
                    r.parser_version,
                    r.modified_time,
                    r.title,
                    r.body,
                    r.display_locator,
                    r.observed_at,
                    r.coverage_level,
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }


    ///
    /// Single transaction:
    /// 1. **CAS UPDATE** — `SET state='succeeded', finished_at=? WHERE id=?
    ///    AND state='committing' AND fencing_token = (SELECT MAX(fencing_token)
    ///    FROM scan_runs WHERE source_id=?)`. The token is compared against
    ///    the per-source CURRENT MAXIMUM, not merely the holder's own row:
    ///    comparing only against its own `begin_run` row is no fence at all
    ///    (a holder always matches itself). Once a SECOND owner begins a run
    ///    (allocating a higher token), the FIRST owner's commit must affect 0
    ///    rows and lose. A 0-row result rolls the WHOLE transaction back and
    ///    returns `Ok(false)` — `active_generation` never moves.
    /// 2. On a 1-row CAS, write `tessera_meta.active_generation:<source_rowid>`.
    /// 3. Delete every OTHER generation's `memory_records` rows for this
    ///    source (AD-2: old derived data is rebuildable).
    /// 4. Commit.
    ///
    /// Returns `Ok(true)` when the CAS succeeded and the generation is now
    /// active; `Ok(false)` on a lost CAS.
    pub fn commit_cas(
        &self,
        scan_id: i64,
        fencing_token: i64,
        generation: &Generation,
        source_rowid: i64,
    ) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let cas_rows = tx.execute(
            "UPDATE scan_runs SET state = ?1, finished_at = ?2
             WHERE id = ?3 AND state = ?4
               AND fencing_token = (
                   SELECT MAX(fencing_token) FROM scan_runs WHERE source_id = ?5
               )",
            params![
                ScanRunState::Succeeded.as_str(),
                unix_seconds_now_i64(),
                scan_id,
                ScanRunState::Committing.as_str(),
                source_rowid,
            ],
        )?;
        // `fencing_token` is retained in the signature for API stability and
        // debuggability (the holder's own token is still a useful correlation
        // id in logs), even though the CAS predicate now keys on the per-source
        // MAX. Debug-assert the holder's belief matches its row.
        debug_assert_eq!(
            fencing_token,
            tx.query_row::<i64, _, _>(
                "SELECT fencing_token FROM scan_runs WHERE id = ?1",
                params![scan_id],
                |row| row.get(0)
            )
            .unwrap_or(fencing_token),
            "holder token must match its own scan_runs row"
        );
        if cas_rows == 0 {
            // Lost the CAS: this run is not the current (latest) owner. Roll
            // back the whole transaction — the active generation marker is NOT
            // written. The run stays in `committing` for the next boot to
            // recover (spec Design Notes — CAS 失败不留半态的唯一例外).
            drop(tx);
            return Ok(false);
        }
        // CAS succeeded in this same transaction: flip the visible generation.
        let key = format!("{ACTIVE_GENERATION_KEY_PREFIX}{source_rowid}");
        tx.execute(
            "INSERT OR REPLACE INTO tessera_meta(key, value) VALUES (?1, ?2)",
            params![key, generation.0],
        )?;
        // A local search cursor never pins historical generations. Once the
        // marker moves, the old derived records are rebuildable and must be
        // removed in this same activation transaction.
        tx.execute(
            "DELETE FROM memory_records WHERE source_id = ?1 AND generation != ?2",
            params![source_rowid, generation.0],
        )?;
        tx.execute(
            "DELETE FROM scan_diagnostics WHERE source_id = ?1 AND generation != ?2",
            params![source_rowid, generation.0],
        )?;
        // Story 6.5 follow-up: a Knowledge Source's records live in the
        // independent `knowledge_records` table. Old generations are rebuildable
        // derived data and must be GC'd identically (AD-2/AD-38). For an Agent
        // Source this DELETE is a 0-row no-op (no knowledge_records rows exist
        // for that source), and vice versa — the table partition is clean.
        tx.execute(
            "DELETE FROM knowledge_records WHERE source_id = ?1 AND generation != ?2",
            params![source_rowid, generation.0],
        )?;
        tx.commit()?;
        Ok(true)
    }

    /// Finish a diagnostic-only observation without replacing an existing
    /// active generation. The diagnostics are still committed as a durable,
    /// source-scoped projection for this completed run.
    pub fn complete_without_activation(
        &self,
        scan_id: i64,
        fencing_token: i64,
        generation: &Generation,
        source_rowid: i64,
    ) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE scan_runs SET state = ?1, finished_at = ?2
             WHERE id = ?3 AND state = ?4
               AND fencing_token = (
                   SELECT MAX(fencing_token) FROM scan_runs WHERE source_id = ?5
               )",
            params![
                ScanRunState::Succeeded.as_str(),
                unix_seconds_now_i64(),
                scan_id,
                ScanRunState::Committing.as_str(),
                source_rowid,
            ],
        )?;
        if changed == 0 {
            drop(tx);
            return Ok(false);
        }
        tx.execute(
            "DELETE FROM scan_diagnostics WHERE source_id = ?1 AND generation != ?2",
            params![source_rowid, generation.0],
        )?;
        debug_assert!(fencing_token > 0);
        tx.commit()?;
        Ok(true)
    }

    /// Mark a run `failed` with a MANDATORY stable `error_code` from the
    /// vocabulary (spec Design Notes — "error_code 稳定词汇":
    /// `dirty_after_validation` / `read_failed` / `enumeration_failed` /
    /// `stale_recovered` / `internal`). Sets `finished_at`. Used by the
    /// orchestrator on every failure path EXCEPT a lost commit CAS (which is
    /// not re-marked — the run is left for boot recovery).
    pub fn fail_run(&self, scan_id: i64, error_code: &str) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE scan_runs SET state = ?1, error_code = ?2, finished_at = ?3
             WHERE id = ?4 AND state IN (?5, ?6, ?7, ?8)",
            params![
                ScanRunState::Failed.as_str(),
                error_code,
                unix_seconds_now_i64(),
                scan_id,
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
        )?;
        tx.execute(
            "DELETE FROM scan_diagnostics
             WHERE source_id = (SELECT source_id FROM scan_runs WHERE id = ?1)
               AND generation = (SELECT generation FROM scan_runs WHERE id = ?1)",
            params![scan_id],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Atomically cancel the newest in-flight run for one Source. The state
    /// transition is the cancellation fence: `commit_cas` only accepts
    /// `committing`, so a cancelled owner cannot later activate its staging
    /// generation. Returns false when no cancellable run exists (already
    /// finished is not reported as cancelled).
    pub fn cancel_latest_run(&self, source_rowid: i64) -> rusqlite::Result<bool> {
        let tx = self.conn.unchecked_transaction()?;
        let changed = tx.execute(
            "UPDATE scan_runs SET state = ?1, error_code = 'cancelled', cancel_requested = 1, finished_at = ?2
             WHERE id = (
                 SELECT id FROM scan_runs
                 WHERE source_id = ?3 AND state IN (?4, ?5, ?6, ?7)
                 ORDER BY id DESC LIMIT 1
             )",
            params![
                ScanRunState::Failed.as_str(),
                unix_seconds_now_i64(),
                source_rowid,
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
        )?;
        tx.commit()?;
        Ok(changed == 1)
    }

    /// True iff at least one run for `source_rowid` is in a non-terminal state
    /// (`queued`/`running`/`staging`/`committing`). Used by the reconcile
    /// reservation path ([`crate::application::reserve_run`]) to enforce the
    /// AD-5/16/28/32 "single fenced owner per source" invariant: when an
    /// in-flight run exists, a new reservation returns `AlreadyRunning` instead
    /// of allocating a second owner. This is the chokepoint both the HTTP
    /// rescan path and the watcher reconcile path pass through, so a runaway
    /// owner (e.g. a long-running rescan) blocks new reconciles for the same
    /// source until it finishes — exactly the single-owner contract.
    pub fn has_in_flight_run(&self, source_rowid: i64) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scan_runs
             WHERE source_id = ?1 AND state IN (?2, ?3, ?4, ?5)",
            params![
                source_rowid,
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// True iff at least one run ACROSS ALL sources is currently in a non-
    /// terminal state (`queued`/`running`/`staging`/`committing`). Story 4.4's
    /// rebuild uses this as its primary race guard: a rebuild is rejected with
    /// `rebuild_failed` (409) when any source has an in-flight run, preventing
    /// a wipe mid-pipeline (a scan that has already staged data would otherwise
    /// find its `scan_runs` row deleted and its staged rows reclaimed). Sibling
    /// to [`Self::has_in_flight_run`]: that enforces the per-source single-
    /// owner gate; this enforces the global "no scan is mid-flight" gate.
    pub fn any_in_flight_run(&self) -> rusqlite::Result<bool> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM scan_runs
             WHERE state IN (?1, ?2, ?3, ?4)",
            params![
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Story 4.4 — wipe exactly the Tessera-derived tables in ONE transaction
    /// (the AD-29 reset boundary promoted to a repeatable runtime operation).
    /// Mirrors the v3 schema-migration wipe at `migrations::v3_canonical_memory_records`
    /// (`migrations.rs:270-272`), but ADDS `scan_diagnostics` (which v3
    /// predates) and is callable at runtime (no migration id, no audit row).
    ///
    /// Deletes EXACTLY:
    /// - every `memory_records` row,
    /// - every `scan_runs` row,
    /// - every `scan_diagnostics` row,
    /// - every `tessera_meta` row whose key matches `active_generation:%`.
    ///
    /// Preserves (MUST NOT touch):
    /// - `source_registry` (Confirmed / Disabled / Rejected rows carry the
    ///   user's confirmation decisions and must survive a rebuild),
    /// - `tessera_meta.schema_version` (the migration state — touching it
    ///   would re-run every migration),
    /// - `tessera_migrations_applied` (the migration audit log),
    /// - any other `tessera_meta` key (e.g. `schema_version`, and any future
    ///   Tessera Project mapping revision).
    ///
    /// `tessera_meta` is a MIXED table (it carries the schema version AND
    /// active-generation pointers AND future mapping revisions); a blanket
    /// `DELETE FROM tessera_meta` would destroy the schema version, so the
    /// WHERE clause is keyed on the `active_generation:` prefix.
    ///
    /// Patch D: the prefix match uses `substr(key, 1, ?) = ?` bound to the
    /// literal `ACTIVE_GENERATION_KEY_PREFIX` (length + string). A `LIKE
    /// 'active_generation:%'` would treat `_` as a single-char wildcard, so a
    /// hypothetical `activeXgeneration:1` key would be wrongly deleted.
    /// substr-based prefix equality matches the prefix LITERALLY (the spec's
    /// "EXACTLY `active_generation:%`"). Stays one statement inside the same
    /// transaction.
    ///
    /// Returns `Ok(())` on a successful wipe. A SQLite error (e.g. disk-full)
    /// surfaces as `Err(_)` and rolls the whole transaction back — the index
    /// is unchanged.
    pub fn reset_derived_data(&self) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM memory_records", [])?;
        tx.execute("DELETE FROM scan_runs", [])?;
        tx.execute("DELETE FROM scan_diagnostics", [])?;
        // Patch D — literal prefix match. The bound pair `(prefix_len,
        // prefix_str)` makes SQLite compare the first N characters of `key`
        // against the prefix string with NO wildcard interpretation. `_`
        // (underscore) is no longer special-cased.
        tx.execute(
            "DELETE FROM tessera_meta WHERE substr(key, 1, ?1) = ?2",
            params![
                i64::try_from(ACTIVE_GENERATION_KEY_PREFIX.len()).unwrap_or(i64::MAX),
                ACTIVE_GENERATION_KEY_PREFIX
            ],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Cancel this exact reserved run. Job cancellation must never target a
    /// later rescan for the same Source, even if a stale UI action arrives.
    pub fn cancel_run(&self, scan_id: i64, source_rowid: i64) -> rusqlite::Result<bool> {
        let changed = self.conn.execute(
            "UPDATE scan_runs SET state = ?1, error_code = 'cancelled', cancel_requested = 1, finished_at = ?2
             WHERE id = ?3 AND source_id = ?4
               AND state IN (?5, ?6, ?7, ?8)",
            params![
                ScanRunState::Failed.as_str(),
                unix_seconds_now_i64(),
                scan_id,
                source_rowid,
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
        )?;
        Ok(changed == 1)
    }

    /// True if cancellation (or another owner/state transition) has revoked
    /// this run's right to proceed. It is intentionally checked at each
    /// pipeline boundary and before activation.
    pub fn is_cancelled(&self, scan_id: i64) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT state = ?1 OR cancel_requested != 0 FROM scan_runs WHERE id = ?2",
            params![ScanRunState::Failed.as_str(), scan_id],
            |row| row.get(0),
        )
    }

    /// Boot recovery (AD-16). Flip every stale in-flight run
    /// (`queued/running/staging/committing`) to `failed` with
    /// `error_code='stale_recovered'`, then delete every non-active record.
    /// Search cursors are revision-bound and never retain historical data.
    pub fn recover_stale_runs(&self) -> rusqlite::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute(
            "UPDATE scan_runs SET state = ?1, error_code = ?2, finished_at = ?3
             WHERE state IN (?4, ?5, ?6, ?7)",
            params![
                ScanRunState::Failed.as_str(),
                ERROR_CODE_STALE_RECOVERED,
                unix_seconds_now_i64(),
                ScanRunState::Queued.as_str(),
                ScanRunState::Running.as_str(),
                ScanRunState::Staging.as_str(),
                ScanRunState::Committing.as_str(),
            ],
        )?;
        // Reclaim every non-active derived generation, including legacy
        // successful generations from before local-cursor simplification.
        tx.execute(
            "DELETE FROM memory_records
             WHERE generation != COALESCE(
                 (SELECT value FROM tessera_meta
                   WHERE key = ?1 || memory_records.source_id),
                 ''
             )",
            params![ACTIVE_GENERATION_KEY_PREFIX],
        )?;
        tx.execute(
            "DELETE FROM scan_diagnostics
             WHERE NOT EXISTS (
                 SELECT 1 FROM scan_runs
                 WHERE scan_runs.source_id = scan_diagnostics.source_id
                   AND scan_runs.generation = scan_diagnostics.generation
                   AND scan_runs.state = ?1
             )",
            params![ScanRunState::Succeeded.as_str()],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The active generation for a source, or `None` when no generation has
    /// committed successfully yet.
    pub fn active_generation(&self, source_rowid: i64) -> rusqlite::Result<Option<Generation>> {
        let key = format!("{ACTIVE_GENERATION_KEY_PREFIX}{source_rowid}");
        match self.conn.query_row(
            "SELECT value FROM tessera_meta WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        ) {
            Ok(v) => Ok(Some(Generation(v))),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// The most recent run for a source (highest `scan_id`), or `None` when
    /// the source has never been scanned. Used by `get_scan_status`.
    ///
    /// Returns an `Err` when the persisted `state` string does not parse into a
    /// known [`ScanRunState`] — an unknown state is DATA CORRUPTION and must
    /// surface (the application layer maps it to `Internal`), NOT be silently
    /// coerced to `failed` (spec: no `unwrap_or(Failed)`).
    pub fn latest_run(&self, source_rowid: i64) -> rusqlite::Result<Option<ScanRunRow>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, state, generation, error_code, finished_at FROM scan_runs
             WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
        )?;
        let mut rows = stmt.query(params![source_rowid])?;
        match rows.next()? {
            Some(row) => {
                let state_str: String = row.get(1)?;
                let generation_str: String = row.get(2)?;
                let state = ScanRunState::parse_str(&state_str).ok_or_else(|| {
                    rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("scan_runs.state is corrupt: unparseable value {state_str:?}"),
                    )))
                })?;
                Ok(Some(ScanRunRow {
                    scan_id: row.get(0)?,
                    state,
                    generation: Generation(generation_str),
                    error_code: row.get(3)?,
                    finished_at: row.get(4)?,
                }))
            }
            None => Ok(None),
        }
    }

    /// Count the records in a SPECIFIC generation for a source. Used by the
    /// orchestrator to report `records_indexed` as the post-commit actual row
    /// count of the committed generation (spec Design Notes — "计数诚实"),
    /// which is guaranteed to match the staged vec length because the
    /// enumerator dedups by relative path.
    pub fn count_generation_records(
        &self,
        source_rowid: i64,
        generation: &Generation,
    ) -> rusqlite::Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id = ?1 AND generation = ?2",
            params![source_rowid, generation.0],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Count Knowledge records in a specific (staging) generation (Story 6.5
    /// follow-up). Mirrors [`count_generation_records`] for the independent
    /// `knowledge_records` table. Used by the Knowledge pipeline to report
    /// `records_indexed` before the CAS commit.
    pub fn count_generation_knowledge_records(
        &self,
        source_rowid: i64,
        generation: &Generation,
    ) -> rusqlite::Result<u64> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM knowledge_records WHERE source_id = ?1 AND generation = ?2",
            params![source_rowid, generation.0],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Count the records in the currently-active generation for a source
    /// (`0` when there is no active generation). Used by `get_scan_status`.
    pub fn count_active_records(&self, source_rowid: i64) -> rusqlite::Result<u64> {
        let active = self.active_generation(source_rowid)?;
        let Some(gen) = active else {
            return Ok(0);
        };
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM memory_records WHERE source_id = ?1 AND generation = ?2",
            params![source_rowid, gen.0],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Count active Knowledge records for a Source (Story 6.6). Mirrors
    /// [`count_active_records`] but reads the independent `knowledge_records`
    /// table (AD-19/AD-38), not `memory_records`. Returns 0 when the Source
    /// has no active generation yet (never scanned).
    pub fn count_active_knowledge_records(&self, source_rowid: i64) -> rusqlite::Result<u64> {
        let active = self.active_generation(source_rowid)?;
        let Some(gen) = active else {
            return Ok(0);
        };
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM knowledge_records WHERE source_id = ?1 AND generation = ?2",
            params![source_rowid, gen.0],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// Browse Knowledge records for a single confirmed Vault (Story 6.9).
    /// Mirrors the Agent-Memory `browse_records` shape but reads the independent
    /// `knowledge_records` table. Single-source, query-less, no memory_type
    /// filter (Obsidian Vaults have no Agent-Memory project/type concept).
    /// Cursor is a simple `record_id` lexicographic "strictly-after" (the
    /// natural ordering is by Vault-relative path, which is the display order
    /// the user expects when browsing a Vault's notes).
    ///
    /// Returns `(page, has_more)` where `page` is up to `limit` records and
    /// `has_more` indicates a next page exists (caller encodes the cursor from
    /// the last record's `record_id`).
    pub fn browse_knowledge_records(
        &self,
        source_rowid: i64,
        limit: u32,
        after_record_id: Option<&str>,
    ) -> rusqlite::Result<(Vec<KnowledgeRecordRow>, bool)> {
        let page_size = i64::try_from(limit + 1).expect("limit is bounded");
        let cursor_present: i64 = if after_record_id.is_some() { 1 } else { 0 };
        let cursor_id: Option<&str> = after_record_id;
        let mut stmt = self.conn.prepare(
            "SELECT k.record_id, k.title, k.body, k.provider, k.source_id,
                    k.native_locator, k.display_locator, k.observed_at,
                    k.coverage_level, k.modified_time, s.health_state
             FROM knowledge_records k
             JOIN source_registry s ON s.id = k.source_id
             JOIN tessera_meta active ON active.key = ('active_generation:' || k.source_id)
                                       AND active.value = k.generation
             WHERE s.lifecycle_state = 'confirmed'
               AND k.source_id = ?1
               AND (?2 = 0 OR k.record_id > ?3)
             ORDER BY k.native_locator ASC, k.record_id ASC
             LIMIT ?4",
        )?;
        let rows = stmt.query_map(
            params![source_rowid, cursor_present, cursor_id, page_size],
            |row| {
                let health: String = row.get(10)?;
                let health_state = HealthState::parse_str(&health)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                let title: String = row.get(1)?;
                let body: String = row.get(2)?;
                Ok(KnowledgeRecordRow {
                    record_id: row.get(0)?,
                    title: title.clone(),
                    excerpt: knowledge_excerpt(&title, &body),
                    provider: row.get(3)?,
                    source_id: SourceId::from_rowid(row.get(4)?),
                    vault_relative_path: row.get(5)?,
                    display_locator: row.get(6)?,
                    observed_at: row.get(7)?,
                    coverage_level: row.get(8)?,
                    modified_time: row.get(9)?,
                    health_state,
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        let has_more = out.len() as u32 > limit;
        if has_more {
            out.truncate(limit as usize);
        }
        Ok((out, has_more))
    }

    /// Search Knowledge records across ALL confirmed Knowledge sources with a
    /// usable active generation (Story 6.9). Mirrors the Agent-Memory
    /// `search_records` SQL pattern but reads `knowledge_records` and uses
    /// simpler Knowledge-domain filters (source, folder-prefix, since).
    ///
    /// Keyword matching is `instr(title || char(10) || body, ?query) > 0`
    /// (substring, no FTS5). Results are ordered by title-match rank first,
    /// then Vault-relative path, then record_id. The cursor is a record_id
    /// lexicographic strictly-after within the same title-match tier.
    ///
    /// `source_filter`: `Some(rowid)` narrows to one Vault; `None` searches all.
    /// `folder_prefix`: `Some("Notes/sub")` narrows to notes whose
    /// `native_locator` starts with the prefix. `since`: `Some(epoch_sec)`
    /// filters by observed_at >= threshold.
    pub fn search_knowledge_records(
        &self,
        query: &str,
        limit: u32,
        after_record_id: Option<&str>,
        after_title_match: Option<bool>,
        after_locator: Option<&str>,
        source_filter: Option<i64>,
        folder_prefix: Option<&str>,
        since: Option<i64>,
    ) -> rusqlite::Result<(Vec<KnowledgeRecordRow>, bool)> {
        let page_size = i64::try_from(limit + 1).expect("limit is bounded");
        // Cursor: present flag + title_match rank + record_id. The cursor
        // encodes the last result's (title_match, record_id) so the next page
        // continues within the correct tier. A title-match=true row sorts
        // before title-match=false; within a tier, record_id ASC.
        let cursor_present: i64 = if after_record_id.is_some() { 1 } else { 0 };
        let cursor_title_rank: i64 = match after_title_match {
            Some(true) => 0,
            Some(false) => 1,
            None => 0,
        };
        let cursor_locator: Option<&str> = after_locator;
        let cursor_id: Option<&str> = after_record_id;
        let source_present: i64 = if source_filter.is_some() { 1 } else { 0 };
        let source_value: Option<i64> = source_filter;
        let folder_present: i64 = if folder_prefix.is_some() { 1 } else { 0 };
        let folder_value: Option<&str> = folder_prefix;
        let since_present: i64 = if since.is_some() { 1 } else { 0 };
        let since_value: Option<i64> = since;
        let mut stmt = self.conn.prepare(
            "SELECT k.record_id, k.title, k.body, k.provider, k.source_id,
                    k.native_locator, k.display_locator, k.observed_at,
                    k.coverage_level, k.modified_time, s.health_state
             FROM knowledge_records k
             JOIN source_registry s ON s.id = k.source_id
             JOIN tessera_meta active ON active.key = ('active_generation:' || k.source_id)
                                       AND active.value = k.generation
             WHERE s.lifecycle_state = 'confirmed'
               AND instr(k.title || char(10) || k.body, ?1) > 0
               AND (
                   ?2 = 0
                   OR (CASE WHEN instr(k.title, ?1) > 0 THEN 0 ELSE 1 END) > ?3
                   OR ((CASE WHEN instr(k.title, ?1) > 0 THEN 0 ELSE 1 END) = ?3
                       AND k.native_locator > ?4)
                   OR ((CASE WHEN instr(k.title, ?1) > 0 THEN 0 ELSE 1 END) = ?3
                       AND k.native_locator = ?4
                       AND k.record_id > ?5)
               )
               AND (?6 = 0 OR k.source_id = ?7)
               AND (?8 = 0 OR k.native_locator LIKE ?9 || '%')
               AND (?10 = 0 OR k.observed_at >= ?11)
             ORDER BY
               (CASE WHEN instr(k.title, ?1) > 0 THEN 0 ELSE 1 END) ASC,
               k.native_locator ASC,
               k.record_id ASC
             LIMIT ?12",
        )?;
        let rows = stmt.query_map(
            params![
                query,
                cursor_present,
                cursor_title_rank,
                cursor_locator,
                cursor_id,
                source_present,
                source_value,
                folder_present,
                folder_value,
                since_present,
                since_value,
                page_size,
            ],
            |row| {
                let health: String = row.get(10)?;
                let health_state = HealthState::parse_str(&health)
                    .ok_or(rusqlite::Error::InvalidQuery)?;
                let title: String = row.get(1)?;
                let body: String = row.get(2)?;
                Ok(KnowledgeRecordRow {
                    record_id: row.get(0)?,
                    title: title.clone(),
                    excerpt: knowledge_excerpt(&title, &body),
                    provider: row.get(3)?,
                    source_id: SourceId::from_rowid(row.get(4)?),
                    vault_relative_path: row.get(5)?,
                    display_locator: row.get(6)?,
                    observed_at: row.get(7)?,
                    coverage_level: row.get(8)?,
                    modified_time: row.get(9)?,
                    health_state,
                })
            },
        )?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        let has_more = out.len() as u32 > limit;
        if has_more {
            out.truncate(limit as usize);
        }
        Ok((out, has_more))
    }


    pub fn last_successful_finished_at(&self, source_rowid: i64) -> rusqlite::Result<Option<i64>> {
        self.conn
            .query_row(
                "SELECT finished_at FROM scan_runs WHERE source_id = ?1 AND state = ?2
             ORDER BY id DESC LIMIT 1",
                params![source_rowid, ScanRunState::Succeeded.as_str()],
                |row| row.get::<_, Option<i64>>(0),
            )
            .or_else(|error| match error {
                rusqlite::Error::QueryReturnedNoRows => Ok(None),
                other => Err(other),
            })
    }

    /// Resolve a `SourceId` handle to its registry rowid. Returns `None` for a
    /// malformed id. (Rowid existence is the caller's check — this is a pure
    /// handle translation.)
    pub fn source_rowid(source_id: &SourceId) -> Option<i64> {
        source_id.to_rowid()
    }
    pub fn open_target_for_record(&self, record_id: &str) -> rusqlite::Result<Option<OpenTarget>> {
        let mut stmt = self.conn.prepare(
            "SELECT m.record_id, m.source_id, m.native_locator, s.normalized_root_path
             FROM memory_records m
             JOIN source_registry s ON s.id = m.source_id
             JOIN tessera_meta active ON active.key = ('active_generation:' || m.source_id)
                                       AND active.value = m.generation
             WHERE m.record_id = ?1
               AND s.lifecycle_state = 'confirmed'
             ORDER BY m.source_id ASC
             LIMIT 1",
        )?;
        let mut rows = stmt.query(params![record_id])?;
        match rows.next()? {
            Some(row) => Ok(Some(OpenTarget {
                record_id: row.get(0)?,
                source_id: SourceId::from_rowid(row.get(1)?),
                native_locator: row.get(2)?,
                normalized_root_path: row.get(3)?,
            })),
            None => Ok(None),
        }
    }

    /// Story 5.2 — read one Tessera Project's mapping scope set as a vec of
    /// `(provider, native_project)` pairs. Used by the search sidecar
    /// narrowing (Q3=A): when a `tessera_project` filter is set, the sidecar
    /// lists only confirmed sources whose `(provider,
    /// COALESCE(native_project, ''))` is in this set. The pairs are returned
    /// with `native_project` exactly as persisted (`None` for Codex global);
    /// the caller normalizes through `COALESCE` semantics when comparing.
    ///
    /// Returns an empty vec for an unknown project rowid (no mappings ⇒ no
    /// sources in the sidecar — matches the I/O matrix's "unknown project ⇒
    /// empty results, not an error"). Read-only; never mutates canonical
    /// rows or the project tables.
    pub(crate) fn project_mapping_scope_set(
        &self,
        project_rowid: i64,
    ) -> rusqlite::Result<Vec<(String, Option<String>)>> {
        let mut stmt = self.conn.prepare(
            "SELECT provider, native_project FROM project_mappings
             WHERE tessera_project_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![project_rowid], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

impl QueryStore for ScanStore<'_> {
    /// Search the current confirmed/active scope on every page, ordered by the
    /// Story 2.3 relevance key: **title-match first, then most-recently-
    /// observed, then `coverage_level='full'`, then `record_id` as a stable
    /// tiebreak** — a pure-SQL `ORDER BY` computed from columns already
    /// selected (no FTS5 / `bm25` / schema change; spec Block If).
    ///
    /// `instr` deliberately performs literal substring matching: no FTS grammar
    /// is parsed and two-character CJK terms remain searchable. The cursor
    /// predicate mirrors the ORDER BY exactly (see [`SearchCursorKey`]) so
    /// pagination is stable across relevance tiers — a `record_id`-only cursor
    /// would silently drop records whose id sorts below the cursor but whose
    /// relevance rank is worse.
    fn search_records(
        &self,
        request: &SearchRequest,
        after: Option<&SearchCursorKey>,
    ) -> rusqlite::Result<Vec<SearchResult>> {
        // The cursor predicate is a lexicographic "strictly-after" comparison
        // over the SAME four keys the ORDER BY uses, with each rank encoded so
        // ASC comparison yields the correct "comes later" verdict even for the
        // DESC `observed_at` and the boolean match/coverage flags:
        //   title_match_rank:   0 when title matches (sorts first), 1 otherwise
        //   observed_at:        compared DESC, so "after" = strictly smaller
        //   coverage_rank:      0 when coverage='full' (sorts first), 1 otherwise
        //   record_id:          final ASC tiebreak
        //
        // Story 2.4 cross-provider filters append one conditional AND predicate
        // each after the cursor predicate. The pattern mirrors the cursor
        // predicate: a `present` flag (0/1) short-circuits the predicate to
        // true when the filter is `None`, so a no-filter request runs the same
        // SQL shape as before (the flag is the first operand of an `OR`, so
        // SQLite stops evaluating at `0 = 0`). `native_project = ?` honestly
        // excludes rows whose `native_project` is NULL (Codex's global store):
        // SQL `NULL = 'x'` is NULL, not true, so a NULL row never matches a
        // project filter — the spec calls this the honest behavior, not a bug.
        //
        // Story 5.2 — `tessera_project` filter (was reserved in 2.4). The
        // predicate joins `memory_records` to `project_mappings` via an
        // `EXISTS` subquery on `(provider, COALESCE(native_project, ''))`. The
        // `COALESCE` collapse mirrors the Story 5.1 uniqueness index, so a
        // Codex global (`native_project NULL`) maps correctly. Reuses the
        // existing "presence-flag OR predicate" idiom so the no-filter path is
        // unchanged. No copy of canonical rows (AD-2): the join is read-only.
        let mut stmt = self.conn.prepare(
            "SELECT m.record_id, m.title, m.body, m.provider, m.source_id,
                    m.native_project, m.native_locator, m.display_locator,
                    m.observed_at, m.coverage_level, s.health_state
             FROM memory_records m
             JOIN source_registry s ON s.id = m.source_id
             JOIN tessera_meta active ON active.key = ('active_generation:' || m.source_id)
                                       AND active.value = m.generation
             WHERE s.lifecycle_state = 'confirmed'
               AND instr(m.title || char(10) || m.body, ?1) > 0
               AND (
                   ?2 = 0
                   OR (CASE WHEN instr(m.title, ?1) > 0 THEN 0 ELSE 1 END) > ?3
                   OR ((CASE WHEN instr(m.title, ?1) > 0 THEN 0 ELSE 1 END) = ?3
                       AND m.observed_at < ?4)
                   OR ((CASE WHEN instr(m.title, ?1) > 0 THEN 0 ELSE 1 END) = ?3
                       AND m.observed_at = ?4
                       AND (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) > ?5)
                   OR ((CASE WHEN instr(m.title, ?1) > 0 THEN 0 ELSE 1 END) = ?3
                       AND m.observed_at = ?4
                       AND (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) = ?5
                       AND m.record_id > ?6)
               )
               AND (?7 = 0 OR m.provider = ?8)
               AND (?9 = 0 OR m.source_id = ?10)
               AND (?11 = 0 OR m.provider_memory_type = ?12)
               AND (?13 = 0 OR m.native_project = ?14)
               AND (?15 = 0 OR m.observed_at >= ?16)
               AND (?17 = 0 OR EXISTS (
                   SELECT 1 FROM project_mappings pm
                   WHERE pm.tessera_project_id = ?18
                     AND pm.provider = m.provider
                     AND COALESCE(pm.native_project, '') = COALESCE(m.native_project, '')
               ))
             ORDER BY
               (CASE WHEN instr(m.title, ?1) > 0 THEN 0 ELSE 1 END) ASC,
               m.observed_at DESC,
               (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) ASC,
               m.record_id ASC
             LIMIT ?19",
        )?;
        let page_size = i64::try_from(request.limit() + 1).expect("search limit is bounded");
        let cursor_present: i64 = if after.is_some() { 1 } else { 0 };
        let cursor_title_rank: i64 = match after {
            Some(key) => i64::from(!key.title_match),
            None => 0,
        };
        let cursor_observed_at: i64 = after.map(|key| key.observed_at).unwrap_or(0);
        let cursor_coverage_rank: i64 = match after {
            Some(key) => i64::from(!key.coverage_full),
            None => 0,
        };
        let cursor_record_id: Option<&str> = after.map(|key| key.record_id.as_str());
        // Story 2.4 filter bindings. Each `present` flag is 1 when the filter
        // is `Some` (the OR evaluates the column predicate) and 0 when `None`
        // (the OR short-circuits to true). The value is bound as `Option`,
        // which rusqlite renders as NULL when `None`; the flag prevents the
        // NULL from ever being compared.
        let provider_present: i64 = request.provider().map_or(0, |_| 1);
        let provider_value: Option<&str> = request.provider();
        // Per-source filter (Spec Change Log 2026-07-25): narrows to one
        // confirmed source's rowid. `m.source_id` is the registry rowid; the
        // `SourceId` handle is translated to that rowid here. The confirmed-
        // source check is the JOIN on `lifecycle_state` above, so a non-
        // confirmed/non-existent id honestly yields no rows.
        let source_present: i64 = request.source().map_or(0, |_| 1);
        let source_value: Option<i64> = request.source().and_then(|id| id.to_rowid());
        let memory_type_present: i64 = request.memory_type().map_or(0, |_| 1);
        let memory_type_value: Option<&str> =
            request.memory_type().map(ProviderMemoryType::as_str);
        let native_project_present: i64 = request.native_project().map_or(0, |_| 1);
        let native_project_value: Option<&str> = request.native_project();
        let since_present: i64 = request.since().map_or(0, |_| 1);
        let since_value: Option<i64> = request.since();
        // Story 5.2 — Tessera-project projection filter. Resolve `proj_<n>`
        // to its rowid at this SQL-binding boundary (the wire/DTO shape is
        // unchanged). `None` here covers BOTH "filter absent" (no
        // `tessera_project` on the request) AND "malformed handle" (the
        // `to_rowid` parse failed); either way the EXISTS predicate's
        // `pm.tessera_project_id = NULL` is NULL (never true), so a malformed
        // id honestly yields no rows (treated as a filter that matches
        // nothing, NOT an error — I/O matrix). The presence flag stays 0 for
        // both so the OR short-circuits to true on the no-filter path.
        let tessera_project_present: i64 = request.tessera_project().map_or(0, |_| 1);
        let tessera_project_rowid: Option<i64> = request
            .tessera_project()
            .and_then(|p| crate::domain::project::ProjectId(p.to_string()).to_rowid());
        let rows = stmt.query_map(
            params![
                request.query(),
                cursor_present,
                cursor_title_rank,
                cursor_observed_at,
                cursor_coverage_rank,
                cursor_record_id,
                provider_present,
                provider_value,
                source_present,
                source_value,
                memory_type_present,
                memory_type_value,
                native_project_present,
                native_project_value,
                since_present,
                since_value,
                tessera_project_present,
                tessera_project_rowid,
                page_size,
            ],
            |row| {
                let health: String = row.get(10)?;
                let health_state =
                    HealthState::parse_str(&health).ok_or(rusqlite::Error::InvalidQuery)?;
                let title: String = row.get(1)?;
                let body: String = row.get(2)?;
                let observed_at: i64 = row.get(8)?;
                let coverage_level: String = row.get(9)?;
                let title_match = title.contains(request.query());
                Ok(SearchResult::new(
                    row.get(0)?,
                    excerpt(&title, &body),
                    row.get(3)?,
                    SourceId::from_rowid(row.get(4)?),
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    observed_at,
                    coverage_level,
                    health_state,
                    title_match,
                ))
            },
        )?;
        rows.collect()
    }

    /// Story 3.1 — Browse a single confirmed source's active generation,
    /// ordered by the query-less form of search's relevance key: `observed_at
    /// DESC → coverage_full → record_id ASC`. Drops the `instr` predicate and
    /// the `title_match` rank (browse is query-less). The cursor predicate
    /// mirrors the ORDER BY exactly (a lexicographic "strictly-after"
    /// comparison over the same three keys) so pagination stays stable across
    /// the recency/coverage tiers — a `record_id`-only cursor would silently
    /// drop records whose id sorts below the cursor but whose recency rank is
    /// worse.
    ///
    /// Story 3.2 — the one in-source filter dimension (`memory_type`) appends a
    /// present-flag predicate of the SAME shape search uses
    /// (`(?F = 0 OR m.provider_memory_type = ?G)`), so a no-filter request runs
    /// the same SQL shape as before (the flag short-circuits the OR). The
    /// filter is applied here in the SQL layer, not in the cursor key: the
    /// cursor body separately binds it so a filter change mid-pagination
    /// surfaces as `cursor_stale`.
    ///
    /// Same JOIN shape as `search_records`: `source_registry` JOIN +
    /// `lifecycle_state = 'confirmed'` + active-generation JOIN. The
    /// confirmed-source guarantee is therefore the SQL layer's, not the
    /// caller's: a non-confirmed/non-existent `source_id` honestly yields no
    /// rows.
    fn browse_records(
        &self,
        request: &BrowseRequest,
        after: Option<&BrowseCursorKey>,
    ) -> rusqlite::Result<Vec<SearchResult>> {
        // The cursor predicate is the same lexicographic "strictly-after"
        // comparison `search_records` uses, minus the `title_match` rank:
        //   observed_at:        compared DESC, so "after" = strictly smaller
        //   coverage_rank:      0 when coverage='full' (sorts first), 1 otherwise
        //   record_id:          final ASC tiebreak
        let mut stmt = self.conn.prepare(
            "SELECT m.record_id, m.title, m.body, m.provider, m.source_id,
                    m.native_project, m.native_locator, m.display_locator,
                    m.observed_at, m.coverage_level, s.health_state
             FROM memory_records m
             JOIN source_registry s ON s.id = m.source_id
             JOIN tessera_meta active ON active.key = ('active_generation:' || m.source_id)
                                       AND active.value = m.generation
             WHERE s.lifecycle_state = 'confirmed'
               AND m.source_id = ?1
               AND (
                   ?2 = 0
                   OR m.observed_at < ?3
                   OR (m.observed_at = ?3
                       AND (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) > ?4)
                   OR (m.observed_at = ?3
                       AND (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) = ?4
                       AND m.record_id > ?5)
               )
               AND (?6 = 0 OR m.provider_memory_type = ?7)
             ORDER BY
               m.observed_at DESC,
               (CASE WHEN m.coverage_level = 'full' THEN 0 ELSE 1 END) ASC,
               m.record_id ASC
             LIMIT ?8",
        )?;
        let page_size = i64::try_from(request.limit() + 1).expect("browse limit is bounded");
        let source_rowid: i64 = match request.source().to_rowid() {
            Some(rowid) => rowid,
            // The application layer validates the handle upstream; defense-in-
            // depth returns an empty page here rather than panicking.
            None => return Ok(Vec::new()),
        };
        let cursor_present: i64 = if after.is_some() { 1 } else { 0 };
        let cursor_observed_at: i64 = after.map(|key| key.observed_at).unwrap_or(0);
        let cursor_coverage_rank: i64 = match after {
            Some(key) => i64::from(!key.coverage_full),
            None => 0,
        };
        let cursor_record_id: Option<&str> = after.map(|key| key.record_id.as_str());
        // Story 3.2 — memory_type present-flag predicate (mirrors search's
        // `(?N = 0 OR m.provider_memory_type = ?M)` shape). The flag is 1 when
        // the filter is `Some`; 0 when `None` (the OR short-circuits to true so
        // a no-filter request runs the same SQL shape).
        let memory_type_present: i64 = request.memory_type().map_or(0, |_| 1);
        let memory_type_value: Option<&str> =
            request.memory_type().map(ProviderMemoryType::as_str);
        let rows = stmt.query_map(
            params![
                source_rowid,
                cursor_present,
                cursor_observed_at,
                cursor_coverage_rank,
                cursor_record_id,
                memory_type_present,
                memory_type_value,
                page_size,
            ],
            |row| {
                let health: String = row.get(10)?;
                let health_state =
                    HealthState::parse_str(&health).ok_or(rusqlite::Error::InvalidQuery)?;
                let title: String = row.get(1)?;
                let body: String = row.get(2)?;
                Ok(SearchResult::new(
                    row.get(0)?,
                    excerpt(&title, &body),
                    row.get(3)?,
                    SourceId::from_rowid(row.get(4)?),
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                    row.get(8)?,
                    row.get(9)?,
                    health_state,
                    // Browse is query-less, so `title_match` is meaningless;
                    // `false` keeps the SearchResult DTO happy without
                    // inventing a query. The field is `#[serde(skip)]` so it
                    // never crosses the wire.
                    false,
                ))
            },
        )?;
        rows.collect()
    }

    fn current_index_revision(&self) -> rusqlite::Result<String> {
        let mut stmt = self.conn.prepare(
            "SELECT s.id, active.value
             FROM source_registry s
             JOIN tessera_meta active ON active.key = ('active_generation:' || s.id)
             WHERE s.lifecycle_state = 'confirmed'
             ORDER BY s.id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut hash = 0xcbf29ce484222325u64;
        let mut any = false;
        for row in rows {
            let (id, generation) = row?;
            any = true;
            for byte in format!("{id}:{generation};").bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x100000001b3);
            }
        }
        // Story 5.2 — fold the `project_mapping_revision` scalar into the
        // index revision so ANY change to a Tessera Project's mapped scope set
        // invalidates every outstanding search AND browse cursor (AD-26 /
        // AD-31). Read via `tessera_meta`; absent ⇒ `0` (pre-Story-5.2 fresh
        // DB before migration id 8 runs, or a corrupt key — collapse to `0`
        // rather than surfacing an error so a corrupt key cannot break the
        // read path; the next scope-set-changing op re-writes a numeric
        // value). Bound into BOTH search and browse cursors via this single
        // revision, so the existing `revision != cursor.revision` gate in
        // `search`/`browse` surfaces `cursor_stale` on a mapping change
        // without further cursor plumbing.
        let mapping_revision: i64 = match self.conn.query_row(
            "SELECT value FROM tessera_meta WHERE key = 'project_mapping_revision'",
            [],
            |row| row.get::<_, String>(0),
        ) {
            Ok(value) => value.parse::<i64>().unwrap_or(0),
            Err(rusqlite::Error::QueryReturnedNoRows) => 0,
            Err(error) => return Err(error),
        };
        for byte in format!("pmr:{mapping_revision};").bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        Ok(if any {
            format!("{hash:016x}")
        } else {
            String::new()
        })
    }
}

/// A plain-text stored-content excerpt. It never interprets Markdown/HTML;
/// React receives this as text and therefore cannot execute source content.
fn excerpt(title: &str, body: &str) -> String {
    let combined = if body.is_empty() {
        title.to_string()
    } else {
        format!("{title}\n{body}")
    };
    combined.chars().take(320).collect()
}

/// Unix epoch seconds as an `i64`, or `0` if the system clock is before the
/// epoch (broken RTC). Mirrors the `migrations::unix_seconds_now` style — the
/// audit column is for human inspection; correctness never depends on it.
pub(crate) fn unix_seconds_now_i64() -> i64 {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64,
        Err(_) => 0,
    }
}
