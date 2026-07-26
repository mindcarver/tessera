//! `index` — Tessera Derived Index adapter (SQLite + FTS5) and migrations.
//!
//! The Derived Index is Tessera-owned app-data (AD-2). It can be deleted and
//! rebuilt from Confirmed Sources; it is never written back to Sources
//! (AD-29). FTS5 is enabled via `rusqlite`'s `bundled` feature; the
//! `fts5_available` test asserts the virtual table can be created on the
//! locked stack (see `server/tests/fts5_available.rs`).
//!
//! Phase 0 owns the migration framework (v0 meta) and the FTS5 availability
//! assertion. Story 1.3 adds the Source Registry (`source_registry` module +
//! migration id `2`) so confirmed / rejected / disabled Sources and their
//! fingerprints persist across restarts. Staging generations, scan_runs state
//! machine, canonical body table and FTS5 search schema land in Stories
//! 1.4/1.5. Story 5.1 adds the Tessera Project mapping layer
//! (`project_store` module + migration id `7`) so the user can explicitly
//! associate provider-native projects into a cross-Agent view (local-only;
//! provider directories are never modified). Story 5.2 appends migration id
//! `8` (`v7_project_mapping_revision`) seeding the `project_mapping_revision`
//! scalar that invalidates outstanding cursors on mapping changes.

pub mod migrations;
pub mod project_store;
pub mod scan_store;
pub mod source_registry;

/// Current Tessera schema version. Equals the highest applied migration id
/// in [`migrations::MIGRATIONS`]; bumped only by appending a new entry.
///
/// Phase 0 shipped migration id `1` (`v0_meta`). Story 1.3 appended migration
/// id `2` (`v1_source_registry`); Story 1.4 appended migration id `3`
/// (`v2_scan_generations`); Story 1.5 appended migration id `4`
/// (`v3_canonical_memory_records`); Story 1.8 appended migration id `5`
/// (`v4_rescan_cancellation`); Story 4.2 appended migration id `6`
/// (`v5_source_health_cause`); Story 5.1 appended migration id `7`
/// (`v6_tessera_projects`); Story 5.2 appended migration id `8`
/// (`v7_project_mapping_revision`), so the current schema version is `8`.
/// The value `0` is reserved as the pre-migration sentinel on a fresh
/// database and is never a valid `CURRENT_SCHEMA_VERSION`.
pub const CURRENT_SCHEMA_VERSION: u32 = 8;

/// Re-export the registries so application / IPC code can name them without a
/// long path.
pub use project_store::ProjectStore;
pub use source_registry::SourceRegistry;
