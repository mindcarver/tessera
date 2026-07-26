//! `application` — application services that coordinate domain ports.
//!
//! This is the only layer allowed to orchestrate Source Registry, scan,
//! reconcile, parse, index and query (AD-1). The UI never touches adapters,
//! SQLite, or the filesystem directly.
//!
//! Concrete services:
//! - `source` (1.2/1.3): discover / confirm / reject / disable / list.
//! - `scan` (1.4/1.5): generation staging, fencing token, atomic commit.
//! - `reconcile` (4.1): per-Source `notify` watcher hint ingestion (HINTS
//!   ONLY — A-12: writes no canonical tables) + bounded reconcile that reuses
//!   `scan_reserved_source` through the same atomic generation switch as
//!   manual rescans (AD-5/AD-34/AD-36 — single mutation path). Periodic
//!   reconcile self-heals dropped/missed notify events (AD-8).
//! - `query` (1.6/1.7): BrowsePage/SearchPage, cursor + limit, provenance.
//!
//! Story 1.2 inlined the stateless discover orchestrator here; Story 1.3
//! extracts the `source` submodule (Design Notes — "application 内联" Note
//! 兑现) and moves `discover_sources` into it, then adds the confirm /
//! reject / disable / list orchestrators.

pub mod open;
pub mod project;
pub mod query;
pub mod reconcile;
pub mod rebuild;
pub mod scan;
pub mod source;

// Re-export so IPC and lib.rs can name `application::discover_sources` etc.
// without a path change from 1.2.
pub use source::{
    confirm_knowledge_source, confirm_source, disable_source, discover_obsidian_vaults,
    discover_sources, list_sources, native_project_for_root, rebind_source,
    rebind_source_with_opencode_identity_resolver, reject_knowledge_source, reject_source,
    request_existing_vault_picker, SourceError, VaultPickerOutcome,
};

// Story 5.1 — Tessera Project orchestration. Re-exported so IPC and tests
// can name `application::create_project` etc. without a long path.
pub use project::{
    add_mapping, create_project, delete_project, list_projects, remove_mapping, rename_project,
    ProjectError,
};

// Story 4.4 — synchronous rebuild core. Re-exported so IPC and tests can name
// `application::rebuild_index` / `application::RebuildError` without a long
// path. The HTTP layer wraps this with transport job tracking + per-source
// worker dispatch (see `http::start_rebuild`).
pub use rebuild::{rebuild_index, RebuildError};

// Story 1.4 scan orchestration (AD-1). Re-exported so IPC and the boot path
// can name them without a long path.
pub use open::{
    open_original_location, reset_open_path_for_tests, set_open_path_for_tests, OpenError,
};
pub use query::{browse, browse_knowledge, search, search_knowledge};
pub use reconcile::{
    reserve_run, trigger_reconcile, HintQueue, ReconcileConfig, ReconcileSupervisor, TriggerError,
    DEFAULT_DEBOUNCE, DEFAULT_PERIOD,
};
pub use scan::{
    cancel_rescan, get_scan_status, list_inventory, list_knowledge_inventory, recover_scans,
    scan_reserved_source, scan_source, scan_source_with,
};
