//! `index::migrations` — versioned, atomic migration runner.
//!
//! Architecture invariants honoured here (Phase 0 establishes the framework):
//!
//! - AD-29 / A-7: migration is atomic. Every pending migration runs inside a
//!   single SQLite transaction. If any migration fails, the transaction is
//!   rolled back and the previously-applied schema (the "last usable index")
//!   is preserved. The function returns an error and never leaves a
//!   half-applied schema.
//! - `tessera_meta(schema_version)` records the highest applied migration id,
//!   starting at `0` when no migration has run yet. On restart the runner
//!   skips already-applied entries and resumes from the next.
//! - `tessera_migrations_applied` is the audit log; each applied migration
//!   records one row. The runner never silently re-applies or skips.
//! - v0 (migration id `1`, named `v0_meta`) creates the meta tables and seeds
//!   the schema version. The full business schema (Source Registry, canonical
//!   records, FTS5 search, scan_runs state machine, project mapping) is added
//!   by later migrations in Stories 1.4/1.5/1.6 — each as its own atomic
//!   migration entry.
//!
//! Adding a migration: append a `(id, "name", sql_fn)` triple to
//! [`MIGRATIONS`]. `id`s must be strictly increasing and never reuse a value,
//! and must be `>= 1` because `schema_version = 0` is the "no migrations
//! applied yet" sentinel on a fresh database.

use rusqlite::Connection;

/// A single migration. `id` must be strictly monotonic and `>= 1`; `name` is
/// recorded in `tessera_migrations_applied` for human audit.
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub id: u32,
    pub name: &'static str,
    pub apply: fn(&Connection) -> rusqlite::Result<()>,
}

/// The ordered, append-only list of Tessera index migrations. Phase 0 shipped
/// the v0 bootstrap entry; Story 1.3 appends `v1_source_registry`. Future
/// migrations append to this list with strictly increasing ids; never reorder
/// or remove an entry.
pub static MIGRATIONS: &[Migration] = &[
    Migration {
        id: 1,
        name: "v0_meta",
        apply: v0_meta,
    },
    Migration {
        id: 2,
        name: "v1_source_registry",
        apply: v1_source_registry,
    },
    Migration {
        id: 3,
        name: "v2_scan_generations",
        apply: v2_scan_generations,
    },
    Migration {
        id: 4,
        name: "v3_canonical_memory_records",
        apply: v3_canonical_memory_records,
    },
    Migration {
        id: 5,
        name: "v4_rescan_cancellation",
        apply: v4_rescan_cancellation,
    },
    Migration {
        id: 6,
        name: "v5_source_health_cause",
        apply: v5_source_health_cause,
    },
    // Story 5.1 — Tessera Project mapping layer (local-only explicit
    // association of provider-native projects into a cross-Agent view). Lives
    // entirely in Tessera's own SQLite; provider directories/files are never
    // read-for-write or written.
    Migration {
        id: 7,
        name: "v6_tessera_projects",
        apply: v6_tessera_projects,
    },
    // Story 5.2 — project_mapping_revision scalar (seeds 0). Bumped inside the
    // existing `ProjectStore::with_transaction` whenever a Tessera Project's
    // mapped scope set changes (insert_mapping new / remove_mapping row
    // deleted / delete project with mappings). Folded into
    // `current_index_revision` so any mapping change invalidates every
    // outstanding search AND browse cursor (AD-26/AD-31). Survives Reset Index
    // (AD-29): the reset wipe keys on the `active_generation:` prefix only, so
    // this `tessera_meta` key is untouched.
    Migration {
        id: 8,
        name: "v7_project_mapping_revision",
        apply: v7_project_mapping_revision,
    },
];

/// Ensure the meta tables exist on a fresh DB so [`apply`] can read
/// `schema_version` before any migration has run. Idempotent: safe to call
/// on every boot.
///
/// This step does NOT bump `schema_version` and does NOT record an audit row;
/// it only brings the meta scaffolding into existence. The v0 migration
/// itself (id `1`) records its own audit row and seeds `schema_version = 1`
/// when it runs inside the apply transaction.
fn ensure_meta_tables(conn: &Connection) -> rusqlite::Result<()> {
    // Meta: single-row key/value store for schema version and other Tessera
    // app-data scalars (e.g. active generation markers, projection revisions
    // — those keys land in later Stories).
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tessera_meta (
            key   TEXT PRIMARY KEY,
            value TEXT NOT NULL
        ) STRICT;

        CREATE TABLE IF NOT EXISTS tessera_migrations_applied (
            id         INTEGER PRIMARY KEY,
            name       TEXT    NOT NULL,
            applied_at TEXT    NOT NULL
        ) STRICT;
        "#,
    )?;
    Ok(())
}

/// v0 — create the meta tables and seed `schema_version`. Runs as migration
/// id `1` inside the apply transaction, so its audit row is recorded exactly
/// once on a fresh database and never re-recorded on idempotent re-runs.
fn v0_meta(conn: &Connection) -> rusqlite::Result<()> {
    ensure_meta_tables(conn)?;
    // Seed schema_version to this migration's id. `INSERT OR IGNORE` keeps it
    // idempotent: the apply runner also issues an UPDATE, but the seed guards
    // against any future caller that runs v0_meta standalone in a fixture.
    conn.execute(
        "INSERT OR IGNORE INTO tessera_meta(key, value) VALUES ('schema_version', '1')",
        [],
    )?;
    Ok(())
}

/// v1 (migration id `2`) — Source Registry (Story 1.3).
///
/// Persists confirmed / rejected / disabled Sources and their fingerprints.
/// Design (AD-33/AD-35 / A-19):
/// - `id INTEGER PRIMARY KEY AUTOINCREMENT` — never reused, even after a row
///   is deleted. The external `source_id` is `src_<id>` (Design Notes —
///   "source_id 方案"). AUTOINCREMENT (not just INTEGER PRIMARY KEY) is what
///   guarantees non-reuse; the slight btree cost is irrelevant for an
///   MVP-sized registry.
/// - `provider`, `source_kind` (MVP only `agent_memory` — A-19/AD-10),
///   `lifecycle_state`, `health_state`, `coverage_level`,
///   `normalized_root_path` are all `NOT NULL`: every Source row always
///   carries the full domain state.
/// - `fingerprint` is the versioned match key (`root-fingerprint/v1|...`).
///   The `UNIQUE INDEX` enforces AD-35 "no fuzzy merge" at the storage layer:
///   two rows can never share a fingerprint.
/// - `native_project` is `NULL`-able because Codex memories are a global
///   store with no discoverable native project.
/// - No timestamps (Design Notes — "为何不加时间戳"): the architecture SOURCE
///   ER entity has none; last-scan / last-error times belong to Story 1.8.
fn v1_source_registry(conn: &Connection) -> rusqlite::Result<()> {
    // STRICT so every column has a concrete affinity and a value of the wrong
    // type is rejected at insert time (matches the Phase 0 STRICT convention).
    conn.execute_batch(
        r#"
        CREATE TABLE source_registry (
            id                   INTEGER PRIMARY KEY AUTOINCREMENT,
            provider             TEXT    NOT NULL,
            source_kind          TEXT    NOT NULL,
            lifecycle_state      TEXT    NOT NULL,
            health_state         TEXT    NOT NULL,
            coverage_level       TEXT    NOT NULL,
            normalized_root_path TEXT    NOT NULL,
            fingerprint          TEXT    NOT NULL,
            native_project       TEXT
        ) STRICT;

        CREATE UNIQUE INDEX source_registry_fingerprint
            ON source_registry(fingerprint);
        "#,
    )?;
    Ok(())
}

/// v2 (migration id `3`) — scan_runs state machine + memory_records staging
/// generations (Story 1.4).
///
/// Design (AD-5/AD-16/AD-28/AD-32):
/// - `scan_runs` persists the scan state machine (`queued → running → staging
///   → committing → succeeded | failed`) and the persistent monotonic fencing
///   token. `fencing_token = MAX(fencing_token)+1` per `source_id`, computed
///   inside `begin_run`'s INSERT transaction; `UNIQUE(source_id, fencing_token)`
///   enforces monotonicity at the storage layer (Design Notes — fencing token
///   方案).
/// - `generation` is `gen_<scan_run_id>` — both come from AUTOINCREMENT, no
///   clock/rand dependency (same precedent as `src_<rowid>` in 1.3).
/// - `intent` stores the generation intent committed under CAS (AD-28); in 1.4
///   it is the same `generation` string (a single scan commits one generation).
/// - `manifest_revision` is the FNV-1a hash of the sorted source manifest
///   (relative path + size + mtime) captured at scan start and re-validated
///   before commit (AD-34 `snapshot-at-validation`). A drift marks the run
///   `error_code='dirty_after_validation'` (AD-36) — the persistent flag slot.
/// - `finished_at` is Unix seconds (INTEGER) written by `fail_run` /
///   `commit_cas`; `NULL` while the run is in-flight. Reuses the
///   `migrations::unix_seconds_now` style — no `chrono` (spec Never).
/// - `memory_records` holds file-level staging rows marked by `generation`.
///   Only the generation whose CAS commit succeeded becomes `active` (via
///   `tessera_meta.active_generation:<source_rowid>`); all non-active rows are
///   GC'd at boot (AD-16) or replaced at next successful commit (AD-2 derived
///   data is rebuildable).
/// - `record_id` is `rec_<fnv1a(source_id|provider|native_locator|unit_kind)>` —
///   locator-based identity, stable across re-scans of unchanged files (AD-15/
///   AD-30). Because the SAME `record_id` recurs across generations, the
///   primary key is the COMPOSITE `(record_id, generation)` and staging uses
///   plain `INSERT` — a single-field `record_id` PK plus any `REPLACE`
///   semantics would let a staging write overwrite ACTIVE generation rows
///   (NFR-9 broken; spec Design Notes — "generation 隔离是物理的").
/// - `content_hash` (FNV-1a over bytes) is for change detection only.
/// - Both tables declare `REFERENCES source_registry(id)`; the app enforces it
///   by opening every connection with `PRAGMA foreign_keys = ON` (spec Design
///   Notes — "FK 必须实际强制").
/// - `parser_version` is the constant `file-level/v1` in 1.4 (spec Never: no
///   section identity, no FTS5, no body).
fn v2_scan_generations(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE scan_runs (
            id                INTEGER PRIMARY KEY AUTOINCREMENT,
            source_id         INTEGER NOT NULL REFERENCES source_registry(id),
            generation        TEXT    NOT NULL,
            state             TEXT    NOT NULL,
            fencing_token     INTEGER NOT NULL,
            intent            TEXT    NOT NULL,
            manifest_revision TEXT    NOT NULL,
            error_code        TEXT,
            finished_at       INTEGER
        ) STRICT;

        CREATE UNIQUE INDEX scan_runs_source_fencing
            ON scan_runs(source_id, fencing_token);

        CREATE TABLE memory_records (
            record_id      TEXT    NOT NULL,
            source_id      INTEGER NOT NULL REFERENCES source_registry(id),
            generation     TEXT    NOT NULL,
            provider       TEXT    NOT NULL,
            unit_kind      TEXT    NOT NULL,
            native_unit_id TEXT    NOT NULL,
            native_locator TEXT    NOT NULL,
            content_hash   TEXT    NOT NULL,
            parser_version TEXT    NOT NULL,
            PRIMARY KEY (record_id, generation)
        ) STRICT;

        CREATE INDEX memory_records_source_generation
            ON memory_records(source_id, generation);
        "#,
    )?;
    Ok(())
}

/// v3 (migration id `4`) — canonical record provenance and source-scoped
/// diagnostics (Story 1.5).
///
/// The previous file-level records cannot truthfully populate section title,
/// body, independent display locations, or source-file revisions. The
/// migration is additive at schema level, then atomically invalidates that
/// derived state: Source Registry rows remain, while old records, scan runs,
/// active markers, and their diagnostics are removed for a clean rebuild.
fn v3_canonical_memory_records(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        ALTER TABLE memory_records ADD COLUMN title TEXT NOT NULL DEFAULT '';
        ALTER TABLE memory_records ADD COLUMN body TEXT NOT NULL DEFAULT '';
        ALTER TABLE memory_records ADD COLUMN native_project TEXT;
        ALTER TABLE memory_records ADD COLUMN provider_memory_type TEXT NOT NULL DEFAULT '';
        ALTER TABLE memory_records ADD COLUMN coverage_level TEXT NOT NULL DEFAULT '';
        ALTER TABLE memory_records ADD COLUMN observed_at INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE memory_records ADD COLUMN source_revision TEXT NOT NULL DEFAULT '';
        ALTER TABLE memory_records ADD COLUMN display_locator TEXT NOT NULL DEFAULT '';

        CREATE TABLE scan_diagnostics (
            source_id     INTEGER NOT NULL REFERENCES source_registry(id),
            generation    TEXT    NOT NULL,
            kind          TEXT    NOT NULL,
            observed_path TEXT    NOT NULL,
            PRIMARY KEY (source_id, generation, kind, observed_path)
        ) STRICT;

        CREATE INDEX scan_diagnostics_source_generation
            ON scan_diagnostics(source_id, generation);

        DELETE FROM memory_records;
        DELETE FROM scan_runs;
        DELETE FROM tessera_meta WHERE key LIKE 'active_generation:%';
        "#,
    )?;
    Ok(())
}

/// v4 (Story 1.8) makes manual cancellation durable. A cancellation changes
/// the run out of its in-flight state, so the existing `commit_cas` predicate
/// can never activate that generation after the cancellation wins the race.
fn v4_rescan_cancellation(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "ALTER TABLE scan_runs ADD COLUMN cancel_requested INTEGER NOT NULL DEFAULT 0;",
    )
}

/// v5 (Story 4.2) — persist the structured health-cause taxonomy on each
/// Source row.
///
/// `source_registry.health_cause TEXT` is nullable: a `NULL` (the default for
/// pre-existing rows) reads back as [`HealthCause::None`] via the registry's
/// row mapper. The cause is written at every `set_health_and_cause` site —
/// success writes `(Healthy, none)`, root-validation failure writes
/// `(Degraded, path_missing|permission_denied|scan_failed)`, parse failure
/// writes `(Degraded, format_unsupported)`, dirty-after-validation/internal
/// write `(Error, scan_failed)`. The cause is cleared (set to `none`) on the
/// next successful scan.
///
/// **Why persist rather than derive.** The single most important Connector
/// failure — root deleted — fails at root validation BEFORE `begin_run`, so
/// it writes NO `scan_runs` row. Deriving cause from `scan_runs.error_code`
/// would therefore return `None` for exactly the failure that matters most.
/// Persisting `health_cause` on the `source_registry` row — written at the
/// same `set_health` call sites — makes every failure category recoverable
/// regardless of whether a run row exists. This pays the debt the 1.3 design
/// note (`domain/source.rs`) explicitly deferred: "Source rows carry no
/// timestamps … last-scan / last-error times belong to Story 1.8 / 4.x."
/// `health_cause` is a categorical code (not a timestamp), so it does not
/// violate the "no timestamps / no `chrono`" rule.
fn v5_source_health_cause(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch("ALTER TABLE source_registry ADD COLUMN health_cause TEXT;")
}

/// v6 (Story 5.1) — Tessera Project mapping layer.
///
/// Two new local-only tables record the user-authored explicit association of
/// provider-native projects into a cross-Agent Tessera Project view:
/// - `tessera_projects` — the user-created project (`name`, `created_at`,
///   `updated_at`, all NOT NULL). `id` is AUTOINCREMENT so a deleted project's
///   `proj_<n>` handle is never reused.
/// - `project_mappings` — explicit `(provider, native_project)` pairs mapped to
///   a project. `native_project` is `NULL` for Codex's global store (mirroring
///   the Source Registry's `native_project` column). The mapping key is the
///   same native identity already carried on Sources and canonical records, so
///   projection (Story 5.2) can filter records with a direct
///   `(provider, native_project) IN (...)` predicate — no copy of canonical
///   rows, no native-identity change (AD-2).
///
/// AD-27 cardinality backstop: the `project_mappings_scope_unique` index
/// collapses `NULL` (Codex global) to `''` via `COALESCE` so NULL scopes are
/// unique too, and enforces "within one mapping scope `(provider,
/// native_project)`, a Native Project belongs to at most one active Tessera
/// Project" at the storage layer. The application layer pre-checks the scope
/// inside the transaction to return `mapping_conflict` naming the owning
/// project (rather than surfacing a raw constraint violation); the index is the
/// concurrency backstop. Same-project idempotent re-add is the same index: a
/// duplicate `(provider, COALESCE(native_project,''))` row is the same scope,
/// so the pre-check returns the existing project's id and the application layer
/// returns the unchanged view without INSERTing.
///
/// Both tables are `STRICT` with snake_case columns, matching the Phase 0
/// convention. `project_mappings.tessera_project_id` carries
/// `ON DELETE CASCADE` so deleting a project atomically removes its mappings
/// (the project store counts the removed mappings inside the same transaction
/// to satisfy the `delete` I/O matrix row's `removed_mappings` response field).
/// `PRAGMA foreign_keys = ON` is set on every connection at boot, so the
/// cascade actually fires.
fn v6_tessera_projects(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        CREATE TABLE tessera_projects (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            name       TEXT    NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        ) STRICT;

        CREATE TABLE project_mappings (
            id                 INTEGER PRIMARY KEY AUTOINCREMENT,
            tessera_project_id INTEGER NOT NULL REFERENCES tessera_projects(id) ON DELETE CASCADE,
            provider           TEXT    NOT NULL,
            native_project     TEXT,
            created_at         INTEGER NOT NULL
        ) STRICT;

        -- AD-27: at most one active project per (provider, native_project).
        -- COALESCE collapses NULL (Codex global) to '' so NULL scopes are
        -- unique too. Same-project idempotent re-add is the same index (same
        -- scope row → application pre-check returns the existing view).
        CREATE UNIQUE INDEX project_mappings_scope_unique
            ON project_mappings (provider, COALESCE(native_project, ''));
        "#,
    )
}

/// v7 (Story 5.2) — seed the `project_mapping_revision` scalar in `tessera_meta`.
///
/// This is the single monotonic integer bumped (+1) inside the existing
/// `ProjectStore::with_transaction` on every operation that changes a Tessera
/// Project's mapped scope set: a successful new `insert_mapping`, a
/// `remove_mapping` that deleted a row, and a `delete` that removed mappings.
/// It is NOT bumped by `create`, `rename`, or idempotent re-add (those leave
/// the scope set — and therefore every projection result — unchanged). The
/// revision is folded into `current_index_revision()` (Story 5.2) so any
/// mapping change makes every outstanding search AND browse cursor return
/// `cursor_stale` (HTTP 409); the caller restarts from page 1 under the new
/// snapshot (AD-26/AD-31).
///
/// `INSERT OR IGNORE` keeps the migration idempotent: the apply runner also
/// bumps the schema_version, but the seed guards against any caller that runs
/// `v7_project_mapping_revision` standalone in a fixture. The value `0` is the
/// pre-mapping baseline; the first mapping op raises it to `1`.
///
/// No new table — the key lives in the existing `tessera_meta` key/value store
/// (created by `v0_meta`). Strictly additive: a database that already has
/// mappings (impossible today, but defense-in-depth) keeps them and simply
/// starts the revision at `0`; the first scope-set-changing op bumps to `1`.
fn v7_project_mapping_revision(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR IGNORE INTO tessera_meta(key, value) VALUES ('project_mapping_revision', '0')",
        [],
    )?;
    Ok(())
}

/// Apply all pending migrations atomically (AD-29).
///
/// Semantics:
/// - Bring meta tables into existence so `schema_version` is readable on a
///   fresh DB (sentinel value `0`).
/// - Read the current `schema_version`; treat absent as `0`.
/// - Apply every migration whose `id > schema_version`, in ascending order,
///   each inside the same outer transaction.
/// - For each applied migration: record its audit row and bump
///   `schema_version` to its id, in the same transaction.
/// - Commit once at the end. Any error rolls back the entire batch and leaves
///   the schema at the prior version. On the next boot the runner resumes
///   from the recorded `schema_version`.
pub fn apply(conn: &mut Connection) -> rusqlite::Result<()> {
    ensure_meta_tables(conn)?;

    // Read the current schema version. Three distinct cases:
    //   - row absent (fresh DB, no migration applied yet) → sentinel 0;
    //   - row present and parses to u32 → use it;
    //   - row present but does NOT parse (corruption / manual edit) → error,
    //     NOT a silent reset to 0. Collapsing the last case into 0 would
    //     re-apply every migration and (once destructive DDL exists in a later
    //     Story) risk data loss — spec AD-29 / review finding.
    let current: u32 = match conn.query_row::<String, _, _>(
        "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
        [],
        |row| row.get(0),
    ) {
        Err(rusqlite::Error::QueryReturnedNoRows) => 0,
        Ok(v) => v.parse::<u32>().map_err(|_| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tessera_meta.schema_version is present but not a valid u32",
            )))
        })?,
        Err(e) => return Err(e),
    };

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.id > current)
        .collect::<Vec<_>>();

    if pending.is_empty() {
        return Ok(());
    }

    let tx = conn.transaction()?;
    for migration in pending {
        (migration.apply)(&tx)?;
        // Record audit row and bump schema_version in the same transaction.
        tx.execute(
            "INSERT OR REPLACE INTO tessera_migrations_applied(id, name, applied_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![
                migration.id,
                migration.name,
                unix_seconds_now(),
            ],
        )?;
        tx.execute(
            "UPDATE tessera_meta SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![migration.id.to_string()],
        )?;
    }
    tx.commit()?;
    Ok(())
}

/// Return Unix epoch seconds for audit rows, or `"unknown"` if the system
/// clock is before the Unix epoch (broken RTC / NTP not yet settled) — never
/// `"0"`, which would be indistinguishable from a real 1970-01-01 timestamp.
///
/// NOTE: this records Unix seconds, NOT RFC 3339. The audit column is for
/// human inspection only; migration ordering is governed by the monotonic
/// migration `id`, not by this value. Observation/source times elsewhere use
/// RFC 3339 when known.
fn unix_seconds_now() -> String {
    match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("{}", d.as_secs()),
        Err(_) => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_db() -> Connection {
        let mut conn = Connection::open_in_memory().expect("open in-memory db");
        apply(&mut conn).expect("v0 applies on fresh db");
        conn
    }

    #[test]
    fn v0_creates_meta_and_seeds_schema_version() {
        let conn = fresh_db();
        let v: String = conn
            .query_row(
                "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version row exists");
        // `fresh_db` applies ALL migrations in MIGRATIONS. Phase 0 shipped only
        // v0_meta (id 1) so schema_version was 1; Story 1.3 appended
        // v1_source_registry (id 2) and Story 1.4 appended v2_scan_generations
        // (id 3), Story 1.5 appended v3_canonical_memory_records (id 4),
        // Story 1.8 appended durable rescan cancellation (id 5), Story 4.2
        // appended the structured source health cause (id 6), Story 5.1
        // appended the Tessera Project mapping layer (id 7), and Story 5.2
        // appended the project_mapping_revision seed (id 8).
        // The `0` value remains reserved as the pre-migration sentinel.
        assert_eq!(v, "8");
    }

    #[test]
    fn v0_records_audit_row() {
        let conn = fresh_db();
        let (id, name): (i64, String) = conn
            .query_row(
                "SELECT id, name FROM tessera_migrations_applied WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .expect("v0 audit row exists");
        assert_eq!(id, 1);
        assert_eq!(name, "v0_meta");
    }

    #[test]
    fn apply_is_idempotent_when_no_new_migrations() {
        let mut conn = Connection::open_in_memory().expect("open db");
        apply(&mut conn).expect("first apply");
        // Second apply must succeed and record no new rows.
        apply(&mut conn).expect("second apply");
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tessera_migrations_applied",
                [],
                |row| row.get(0),
            )
            .expect("count");
        // Phase 0 shipped 1 migration (v0_meta); Story 1.3 appended
        // v1_source_registry and Story 1.4 appended v2_scan_generations, so the
        // v3_canonical_memory_records and v4_rescan_cancellation make the
        // idempotent baseline five audit rows; Story 4.2's v5_source_health_cause
        // brings it to six; Story 5.1's v6_tessera_projects brings it to seven;
        // Story 5.2's v7_project_mapping_revision brings it to eight.
        assert_eq!(count, 8, "exactly eight audit rows after idempotent re-run");
    }

    /// AD-29 / A-7: migration is atomic. If a later migration fails mid-batch,
    /// the entire batch rolls back and the previously-applied schema (and
    /// schema_version) is preserved. We simulate a failing follow-up migration
    /// by temporarily extending MIGRATIONS at test time. This is the spec's
    /// binding "atomic apply" invariant.
    ///
    /// The current shipping migrations are v0_meta (id 1), v1_source_registry
    /// (id 2), v2_scan_generations (id 3), v3_canonical_memory_records (id 4),
    /// v4_rescan_cancellation (id 5), v5_source_health_cause (id 6),
    /// v6_tessera_projects (id 7), and v7_project_mapping_revision (id 8).
    /// This test starts from a fully-migrated DB and simulates a failing
    /// migration id 9.
    #[test]
    fn failed_migration_batch_rolls_back_atomically() {
        let mut conn = Connection::open_in_memory().expect("open db");
        apply(&mut conn).expect("all shipping migrations apply on first boot");

        // After all shipping migrations: schema_version = 8, eight audit rows.
        let pre_version: String = conn
            .query_row(
                "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version readable");
        assert_eq!(pre_version, "8");

        // Simulate a failing follow-up migration. The failing migration
        // writes a sentinel table first, then errors; the atomic batch must
        // roll back both the sentinel table creation AND the schema_version
        // bump.
        fn partial_then_fail(conn: &Connection) -> rusqlite::Result<()> {
            conn.execute_batch("CREATE TABLE should_not_survive (x INTEGER);")?;
            Err(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1),
                Some("simulated migration failure".into()),
            ))
        }

        let failing_batch: &[Migration] = &[
            Migration {
                id: 1,
                name: "v0_meta",
                apply: v0_meta,
            },
            Migration {
                id: 2,
                name: "v1_source_registry",
                apply: v1_source_registry,
            },
            Migration {
                id: 3,
                name: "v2_scan_generations",
                apply: v2_scan_generations,
            },
            Migration {
                id: 4,
                name: "v3_canonical_memory_records",
                apply: v3_canonical_memory_records,
            },
            Migration {
                id: 5,
                name: "v4_rescan_cancellation",
                apply: v4_rescan_cancellation,
            },
            Migration {
                id: 6,
                name: "v5_source_health_cause",
                apply: v5_source_health_cause,
            },
            Migration {
                id: 7,
                name: "v6_tessera_projects",
                apply: v6_tessera_projects,
            },
            Migration {
                id: 8,
                name: "v7_project_mapping_revision",
                apply: v7_project_mapping_revision,
            },
            Migration {
                id: 9,
                name: "partial_then_fail",
                apply: partial_then_fail,
            },
        ];

        // Manually replay the apply loop with the extended batch so we can
        // observe the rollback without permanently shipping a failing
        // migration in MIGRATIONS.
        let current: u32 = conn
            .query_row(
                "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
                [],
                |row| {
                    row.get::<_, String>(0)
                        .map(|s| s.parse::<u32>().unwrap_or(0))
                },
            )
            .unwrap_or(0);
        let pending: Vec<&Migration> = failing_batch.iter().filter(|m| m.id > current).collect();
        let result = {
            let tx = conn.transaction().expect("begin tx");
            let outcome: rusqlite::Result<()> = (|| {
                for migration in &pending {
                    (migration.apply)(&tx)?;
                    tx.execute(
                        "INSERT OR REPLACE INTO tessera_migrations_applied(id, name, applied_at) VALUES (?1, ?2, ?3)",
                        rusqlite::params![migration.id, migration.name, unix_seconds_now()],
                    )?;
                    tx.execute(
                        "UPDATE tessera_meta SET value = ?1 WHERE key = 'schema_version'",
                        rusqlite::params![migration.id.to_string()],
                    )?;
                }
                Ok(())
            })();
            match outcome {
                Ok(()) => tx.commit(),
                Err(e) => {
                    // Drop the tx explicitly to roll back, then surface the
                    // original error so the caller observes the failure.
                    drop(tx);
                    Err(e)
                }
            }
        };
        assert!(result.is_err(), "failing migration must propagate error");

        // Post-condition (AD-29): schema is unchanged from before the batch.
        let post_version: String = conn
            .query_row(
                "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .expect("schema_version still readable after rollback");
        assert_eq!(
            post_version, "8",
            "schema_version must not advance on failure"
        );

        let audit_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM tessera_migrations_applied",
                [],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(
            audit_count, 8,
            "no audit row recorded for the failed migration"
        );

        // The sentinel table the failing migration created must NOT exist:
        // the entire batch — including DDL inside it — was rolled back.
        let table_exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'should_not_survive'",
                [],
                |row| row.get(0),
            )
            .expect("check sentinel table absence");
        assert_eq!(
            table_exists, 0,
            "DDL from failed migration must be rolled back"
        );
    }
}
