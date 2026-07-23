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
    confirm_source, disable_source, discover_sources, get_scan_status, list_sources, ping,
    reject_source, scan_source,
};

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
#[derive(Debug)]
pub struct IndexState {
    pub conn: Mutex<Connection>,
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

    let mut conn = Connection::open(&db_path)
        .map_err(std::io::Error::other)?;

    // Enforce foreign keys on every connection (SQLite default is OFF;
    // the pragma is per-connection, not persisted). Required so the v2
    // `memory_records.source_id → source_registry(id)` reference is
    // actually policed.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(std::io::Error::other)?;

    // Apply migrations atomically; on failure the previous usable index
    // remains (AD-29). v0 only seeds meta.
    index::migrations::apply(&mut conn)
        .map_err(std::io::Error::other)?;

    // Boot scan recovery (AD-16): flip stale in-flight runs to failed
    // and GC non-active-generation records, preserving the last active
    // generation. Runs after migrations so the v2 tables exist.
    // Log-and-continue: a recovery failure must not wedge the app; the
    // next boot retries (stale rows are still stale then).
    if let Err(e) = application::recover_scans(&conn) {
        eprintln!("tessera: boot scan recovery failed (will retry next boot): {e:?}");
    }

    Ok(IndexState {
        conn: Mutex::new(conn),
    })
}
