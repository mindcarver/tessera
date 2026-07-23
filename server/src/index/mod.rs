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
//! 1.4/1.5.

pub mod migrations;
pub mod scan_store;
pub mod source_registry;

/// Current Tessera schema version. Equals the highest applied migration id
/// in [`migrations::MIGRATIONS`]; bumped only by appending a new entry.
///
/// Phase 0 shipped migration id `1` (`v0_meta`). Story 1.3 appended migration
/// id `2` (`v1_source_registry`); Story 1.4 appended migration id `3`
/// (`v2_scan_generations`), so the post-1.4 schema version is `3`. The value
/// `0` is reserved as the pre-migration sentinel on a fresh database and is
/// never a valid `CURRENT_SCHEMA_VERSION`.
pub const CURRENT_SCHEMA_VERSION: u32 = 3;

/// Re-export the registry so application / IPC code can name it without a long
/// path.
pub use source_registry::SourceRegistry;
