//! Tessera — Rust core (local web app, Phase 0 scaffold).
//!
//! Architecture spine summary:
//! - AD-1: Rust core is the sole application boundary. All file access, provider
//!   parsing, index writes, project mapping and query coordination must go
//!   through Rust core application services. The browser UI only calls the
//!   versioned HTTP API.
//! - AD-2: Sources own truth; Tessera owns only its derived index/projection.
//! - AD-9/AD-17: the transport is a loopback-only HTTP server embedded in this
//!   process; API contracts are versioned and bounded.
//! - AD-12/AD-20: Local-only enforced default — the server binds 127.0.0.1
//!   only, there is no outbound network path, no auto-update, no telemetry,
//!   and logs omit body/query/credentials.
//!
//! Phase 0 only establishes the scaffold, the `ping` contract sample, and the
//! migration framework v0. No business logic (discovery, scan, parse, search,
//! open) is implemented in this Story; it lands in 1.2 – 1.6.

#![forbid(unsafe_code)]
#![warn(missing_debug_implementations, rust_2018_idioms)]

pub mod adapters;
pub mod application;
pub mod domain;
pub mod http;
pub mod index;
pub mod policy;
pub mod state;

pub use http::envelope::{Envelope, ErrorEnvelope, Pong, API_VERSION};
pub use http::{
    add_mapping, browse, cancel_rescan_request, confirm_source, create_project, delete_project,
    disable_source, discover_sources, get_scan_status, list_projects, list_sources,
    open_original_location, ping, reject_source, remove_mapping, rename_project,
    scan_source, search, source_inventory, start_rebuild, start_rescan,
};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

/// Shared handle holding the Derived Index connection.
///
/// The Derived Index (AD-2) is Tessera-owned app-data: it can be deleted and
/// rebuilt from Confirmed Sources and is never written back to Sources. Phase 0
/// only owns the migration framework and the meta row; business schema is the
/// responsibility of later Stories (1.4/1.5). The HTTP server hands an `Arc` of
/// this state to every connection thread (tiny_http is one-thread-per-
/// connection), so handlers see the same `Mutex<Connection>` the Tauri-managed
/// state previously provided.
///
/// Story 4.1 adds the optional reconcile supervisor handle. The supervisor
/// itself borrows the shared `Arc<IndexState>`, so it is installed AFTER the
/// `Arc` is constructed (via [`boot_with_reconcile`] / [`install_reconcile`])
/// and stored under a `Mutex<Option<…>>` so its lifetime is bound to the state.
/// Dropping the `Arc<IndexState>` drops the supervisor, whose `Drop` stops the
/// watcher threads cleanly.
#[derive(Debug)]
pub struct IndexState {
    pub conn: Mutex<Connection>,
    /// Jobs are ephemeral transport observations; the durable cancellation
    /// fence remains in `scan_runs`.
    pub rescan_jobs: Mutex<HashMap<String, RescanJob>>,
    pub db_path: PathBuf,
    /// Story 4.1 — watcher/reconcile supervisor. `None` until
    /// [`install_reconcile`] wires it in (or permanently `None` in tests that
    /// do not exercise watcher/reconcile). Stored under a `Mutex` so it can be
    /// installed after the `Arc<IndexState>` exists, since the supervisor
    /// borrows that `Arc` for its whole lifetime.
    pub reconcile_supervisor: Mutex<Option<application::ReconcileSupervisor>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RescanEvent {
    pub api_version: &'static str,
    pub job_id: String,
    pub source_id: String,
    pub sequence: u64,
    pub state: String,
    pub message: String,
}

/// Bounded, source-scoped transport observation for one reserved scan run.
/// The persistent `scan_runs` row is the authority for fencing/cancellation;
/// this only lets an SSE client observe that job.
#[derive(Debug, Clone)]
pub struct RescanJob {
    pub scan_id: i64,
    pub job_id: String,
    pub events: Vec<RescanEvent>,
    pub terminal: bool,
}

/// Resolve the OS-managed Tessera app-data directory (AD-20).
///
/// `$HOME/Library/Application Support/tessera` on macOS; honors
/// `TESSERA_DATA_DIR` so tests and verification runs can redirect app data to
/// a scratch directory instead of the real user location. Replaces the
/// Tauri-resolved `app_data_dir`.
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("TESSERA_DATA_DIR") {
        return PathBuf::from(dir);
    }
    dirs::data_dir()
        .expect("OS must resolve an application data directory")
        .join("tessera")
}

/// Boot path shared by the binary and tests.
///
/// 1. Open or create the Tessera app-data SQLite database under the OS-managed
///    app-data dir (AD-20).
/// 2. Atomically apply migrations (AD-29): on failure the previous usable
///    index must be preserved. v0 only seeds the migration meta.
/// 3. Run boot scan recovery (AD-16), then hand the connection back as shared
///    state so registered handlers can share it.
///
/// No outbound network is performed at boot (AD-12/NFR-2).
pub fn boot(data_dir: &Path) -> std::io::Result<IndexState> {
    std::fs::create_dir_all(data_dir)?;
    let db_path = data_dir.join("tessera-index.db");

    let mut conn = Connection::open(&db_path).map_err(std::io::Error::other)?;

    // Enforce foreign keys on every connection (SQLite default is OFF;
    // the pragma is per-connection, not persisted). Required so the v2
    // `memory_records.source_id → source_registry(id)` reference is
    // actually policed.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(std::io::Error::other)?;

    // Apply migrations atomically; on failure the previous usable index
    // remains (AD-29). v0 only seeds meta.
    index::migrations::apply(&mut conn).map_err(std::io::Error::other)?;

    // Boot scan recovery (AD-16): flip stale in-flight runs to failed and
    // reclaim non-active derived records. Search cursors bind the current
    // index revision, so they never retain historical snapshots. Runs after migrations
    // so the v2 tables exist.
    // Log-and-continue: a recovery failure must not wedge the app; the
    // next boot retries (stale rows are still stale then).
    if let Err(e) = application::recover_scans(&conn) {
        eprintln!("tessera: boot scan recovery failed (will retry next boot): {e:?}");
    }

    Ok(IndexState {
        conn: Mutex::new(conn),
        rescan_jobs: Mutex::new(HashMap::new()),
        db_path,
        // Story 4.1 — the supervisor is installed AFTER the `Arc<IndexState>`
        // exists (the supervisor borrows the Arc for its whole lifetime). It
        // stays `None` until [`install_reconcile`] / [`boot_with_reconcile`]
        // wires it in.
        reconcile_supervisor: Mutex::new(None),
    })
}

/// Boot the index AND start the reconcile supervisor (Story 4.1). This is the
/// production entry point: `boot()` opens the DB and runs recovery; this wraps
/// the state in `Arc`, starts the supervisor (which starts watchers for every
/// confirmed source and kicks off the periodic reconcile loop), stores the
/// supervisor handle inside the Arc, and returns the Arc.
///
/// Equivalent to `boot()` + `Arc::new()` + [`install_reconcile`]; provided as
/// one call so the binary does not forget to install the supervisor.
///
/// A supervisor start failure is log-and-continue: the index is still usable
/// (manual rescan still works); periodic reconcile simply does not run. This
/// mirrors the spec I/O matrix row "Boot with confirmed sources" →
/// "Log-and-continue if a watcher fails to start".
pub fn boot_with_reconcile(
    data_dir: &Path,
    config: application::ReconcileConfig,
) -> std::io::Result<std::sync::Arc<IndexState>> {
    let state = std::sync::Arc::new(boot(data_dir)?);
    if let Err(e) = install_reconcile(&state, config) {
        eprintln!("tessera: reconcile supervisor failed to start (continuing without): {e:?}");
    }
    Ok(state)
}

/// Install (or replace) the reconcile supervisor on an existing
/// `Arc<IndexState>`. The supervisor borrows the Arc for its whole lifetime,
/// so it must be installed AFTER the Arc is constructed. Stored under the
/// state's `Mutex<Option<…>>`; dropping the Arc drops the supervisor, whose
/// `Drop` stops the watcher threads cleanly.
///
/// Returns `Err` if the supervisor could not start (e.g. the notify backend
/// failed to initialize). On `Err`, no supervisor is installed; the caller
/// may continue without reconcile (manual rescan still works).
pub fn install_reconcile(
    state: &std::sync::Arc<IndexState>,
    config: application::ReconcileConfig,
) -> std::io::Result<()> {
    let supervisor = application::ReconcileSupervisor::start(std::sync::Arc::clone(state), config)?;
    let mut slot = state
        .reconcile_supervisor
        .lock()
        .map_err(|_| std::io::Error::other("reconcile_supervisor mutex poisoned"))?;
    // Replacing a prior supervisor drops it, stopping its watcher threads.
    *slot = Some(supervisor);
    Ok(())
}
