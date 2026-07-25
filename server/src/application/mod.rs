//! `application` — application services that coordinate domain ports.
//!
//! This is the only layer allowed to orchestrate Source Registry, scan,
//! reconcile, parse, index and query (AD-1). The UI never touches adapters,
//! SQLite, or the filesystem directly.
//!
//! Phase 0 only declared the module. Concrete services land in 1.2 – 1.6:
//! - `source` (1.2/1.3): discover / confirm / reject / disable / list.
//! - `scan` (1.4/1.5): generation staging, fencing token, atomic commit.
//! - `reconcile` (1.4/1.5/1.8): watcher hint ingestion, size/mtime/hash
//!   checks, parser-version recovery.
//! - `query` (1.6/1.7): BrowsePage/SearchPage, cursor + limit, provenance.
//!
//! Story 1.2 inlined the stateless discover orchestrator here; Story 1.3
//! extracts the `source` submodule (Design Notes — "application 内联" Note
//! 兑现) and moves `discover_sources` into it, then adds the confirm /
//! reject / disable / list orchestrators.

pub mod open;
pub mod query;
pub mod scan;
pub mod source;

// Re-export so IPC and lib.rs can name `application::discover_sources` etc.
// without a path change from 1.2.
pub use source::{
    confirm_source, disable_source, discover_sources, list_sources, reject_source, SourceError,
};

// Story 1.4 scan orchestration (AD-1). Re-exported so IPC and the boot path
// can name them without a long path.
pub use open::{
    open_original_location, reset_open_path_for_tests, set_open_path_for_tests, OpenError,
};
pub use query::{browse, search};
pub use scan::{
    cancel_rescan, get_scan_status, list_inventory, recover_scans, scan_reserved_source,
    scan_source, scan_source_with,
};
