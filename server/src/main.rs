//! Tessera binary entry point — local web app bootstrap (AD-9/AD-20).
//!
//! Boots the Rust core (app-data DB, migrations, scan recovery), then embeds
//! the loopback-only HTTP server and opens the user's default browser at the
//! local UI address. The server serves the built React UI (`dist/`) and the
//! versioned `/api/*` surface on `127.0.0.1` only — there is no external
//! listener and no outbound network path (AD-12/NFR-2).
//!
//! Environment overrides (verification / development only):
//! - `TESSERA_PORT` — loopback port (default 1420)
//! - `TESSERA_STATIC_DIR` — UI static root (default `dist`)
//! - `TESSERA_DATA_DIR` — app-data dir (default OS-managed, see
//!   `tessera_lib::default_data_dir`)

#![forbid(unsafe_code)]

fn main() {
    let port: u16 = std::env::var("TESSERA_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(tessera_lib::http::server::DEFAULT_PORT);
    let static_root = std::env::var("TESSERA_STATIC_DIR").unwrap_or_else(|_| "dist".to_string());
    let data_dir = tessera_lib::default_data_dir();

    let state = tessera_lib::boot_with_reconcile(&data_dir, tessera_lib::application::ReconcileConfig::default()).unwrap_or_else(|e| {
        eprintln!("tessera: boot failed at {}: {e}", data_dir.display());
        std::process::exit(1);
    });

    let addr = format!("{}:{port}", tessera_lib::http::server::BIND_HOST);
    let url = format!("http://{addr}/");

    // The browser is the application shell (Story 1.1 rework). Automated
    // browser tests suppress the side effect with `TESSERA_NO_BROWSER=1`.
    // Failure to open a browser is non-fatal: the user can navigate to the
    // URL manually.
    if std::env::var("TESSERA_NO_BROWSER").as_deref() != Ok("1") {
        if let Err(e) = open::that(&url) {
            eprintln!("tessera: could not open default browser ({e}); open {url} manually");
        }
    }

    tessera_lib::http::server::serve(&addr, state, std::path::PathBuf::from(static_root));
}
