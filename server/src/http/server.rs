//! `http::server` — loopback-only HTTP transport (revised AD-9, 2026-07-22).
//!
//! The delivery form is a local web app: this process embeds a tiny synchronous
//! HTTP server that serves the built React UI and the versioned `/api/*`
//! surface, and the user's default browser is the application shell.
//!
//! AD-9 hard rules implemented here:
//! - **Loopback-only binding.** The listener binds `127.0.0.1`; no external
//!   interface is ever opened (AD-12: local-only is an enforced default).
//! - **Host validation (anti DNS-rebinding).** Every request must carry a
//!   `Host` header naming the bound loopback authority; anything else (a
//!   foreign domain rebinding to 127.0.0.1) is rejected with 400.
//! - **Origin validation (anti cross-site calls).** When an `Origin` header is
//!   present (browsers attach it to cross-origin requests), it must name this
//!   server's own loopback origin. Same-origin fetches from the served UI may
//!   omit Origin; those pass.
//! - **Hardened response headers.** Every response carries a tight CSP,
//!   `X-Content-Type-Options: nosniff`, and `Referrer-Policy: no-referrer`;
//!   API responses additionally carry `Cache-Control: no-store`. The CSP keeps
//!   the Phase 0 `default-src 'self'` posture (see docs/phase-0-verification.md)
//!   with `connect-src 'self'` — the UI only talks back to this origin.
//! - **Synchronous handlers.** tiny_http is one-thread-per-connection, so
//!   handlers stay synchronous and the `std::sync::Mutex<Connection>` single-
//!   owner scan pattern (AD-5, Story 1.4 spec) holds unchanged. Handlers that
//!   run a scan hold the mutex for the whole request; no async, no tokio.
//!
//! The server never performs outbound network requests itself; the only
//! network surface is this loopback listener (NFR-2).

use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

use crate::domain::open::OpenRequest;
use crate::domain::query::{
    BrowseRequest, SearchFilters, SearchRequest, MAX_CURSOR_BYTES, MAX_FILTER_BYTES,
    MAX_QUERY_BYTES,
};
use crate::domain::ports::provider_adapter::ProviderMemoryType;
use crate::domain::source::SourceId;
use crate::domain::CandidateSource;
use crate::http::{
    add_mapping, browse, cancel_rescan_request, confirm_source, create_project, delete_project,
    disable_source, discover_sources, get_scan_status, list_projects, list_sources,
    open_original_location, ping, rebind_source, reject_source, remove_mapping, rename_project,
    rescan_events, scan_source, search, source_inventory, start_rebuild, start_rescan,
};
use crate::IndexState;

/// The only interface the server is allowed to bind (AD-9/AD-12).
pub const BIND_HOST: &str = "127.0.0.1";

/// Default loopback port for the local web app.
pub const DEFAULT_PORT: u16 = 1420;

/// Content-Security-Policy applied to every response (Story 1.1 rework: the
/// Tauri-conf CSP moves to an HTTP response header, same posture).
///
/// - `default-src 'self'` / `script-src 'self'`: no remote code of any kind.
/// - `connect-src 'self'`: the UI may only talk back to this loopback origin —
///   there is no remote endpoint in the product (AD-12/NFR-2).
/// - `style-src 'self' 'unsafe-inline'`: accepted only while Phase 0 renders
///   no untrusted content; the Story 1.5 Markdown sanitizer must strip
///   `style` attributes / `<style>` elements so this can be re-tightened
///   (docs/phase-0-verification.md §2).
/// - `object-src 'none'` / `frame-src 'none'` / `base-uri 'self'`: no raw
///   HTML execution entry points (NFR-7).
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-src 'none'";

/// Bind the server on `addr` (must be a 127.0.0.1 authority) and serve
/// requests until the process exits. `static_root` is the built UI directory
/// (`dist/`). This function never returns under normal operation.
pub fn serve(addr: &str, state: Arc<IndexState>, static_root: PathBuf) -> ! {
    let server = bind(addr);
    let bound_port = server.server_addr().to_ip().map(|ip| ip.port());
    eprintln!("tessera: serving local web UI at http://{addr}/ (loopback only)");
    serve_with(server, state, static_root, bound_port)
}

/// Bind the loopback listener. Split from [`serve`] so integration tests can
/// learn the ephemeral port (`127.0.0.1:0`) before entering the serve loop.
pub fn bind(addr: &str) -> Server {
    Server::http(addr).unwrap_or_else(|e| {
        eprintln!("tessera: failed to bind {addr} (loopback only): {e}");
        std::process::exit(1);
    })
}

/// The accept loop. Never returns; per-request and accept-level failures are
/// log-and-continue so one aborted connection cannot wedge the server (the
/// local web app has no supervisor).
pub fn serve_with(
    server: Server,
    state: Arc<IndexState>,
    static_root: PathBuf,
    bound_port: Option<u16>,
) -> ! {
    loop {
        match server.recv() {
            Ok(request) => {
                let response = route(request, &state, &static_root, bound_port);
                if let Err(e) = response {
                    eprintln!("tessera: failed to write response: {e}");
                }
            }
            Err(e) => eprintln!("tessera: failed to accept connection: {e}"),
        }
    }
}

fn route(
    mut request: Request,
    state: &Arc<IndexState>,
    static_root: &Path,
    bound_port: Option<u16>,
) -> std::io::Result<()> {
    // AD-9: Host header must name the bound loopback authority. Rejecting
    // foreign Host values blocks DNS-rebinding attacks where a malicious web
    // page points its domain at 127.0.0.1.
    if !host_header_is_loopback(&request, bound_port) {
        return request.respond(json_error(
            StatusCode(400),
            "forbidden_host",
            "Tessera only accepts requests addressed to its loopback host.",
        ));
    }
    // AD-9: an Origin header, when present, must be this server's own origin.
    // Browsers send Origin on cross-origin requests, so a malicious page on
    // another origin is rejected here even before CORS would apply.
    if !origin_header_is_allowed(&request, bound_port) {
        return request.respond(json_error(
            StatusCode(403),
            "forbidden_origin",
            "Tessera only accepts requests from its own local UI.",
        ));
    }

    let method = request.method().clone();
    let url = request.url().to_string();
    let (path, query) = split_path_query(&url);

    match (method, path) {
        (Method::Get, "/api/ping") => respond_ok(request, ping()),
        (Method::Get, "/api/sources/discover") => respond_ok(request, discover_sources()),
        (Method::Post, "/api/sources/confirm") => {
            let candidate = match read_json_body::<ConfirmRequest>(&mut request) {
                Ok(body) => body.candidate,
                Err(response) => return request.respond(response),
            };
            respond_result(request, confirm_source(&candidate, state))
        }
        (Method::Post, "/api/sources/reject") => {
            let candidate = match read_json_body::<ConfirmRequest>(&mut request) {
                Ok(body) => body.candidate,
                Err(response) => return request.respond(response),
            };
            respond_result(request, reject_source(&candidate, state))
        }
        (Method::Post, "/api/sources/disable") => {
            let source_id = match read_source_id_body(&mut request) {
                Ok(id) => id,
                Err(response) => return request.respond(response),
            };
            respond_result(request, disable_source(&source_id, state))
        }
        // Story 4.3 — explicit rebind: the recovery path for a Confirmed
        // Source whose root moved / lost permissions / changed filesystem
        // identity. Body is `{ source_id, root_path }`; the application layer
        // canonicalizes + fingerprints the new root and disables-old +
        // inserts-or-wakes-new inside ONE transaction.
        (Method::Post, "/api/sources/rebind") => {
            let rebind_request = match read_json_body::<crate::http::RebindRequest>(&mut request) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, rebind_source(rebind_request, state))
        }
        (Method::Get, "/api/sources") => respond_result(request, list_sources(state)),
        (Method::Get, "/api/sources/inventory") => respond_result(request, source_inventory(state)),
        // Story 4.4 — Derived Index rebuild. Atomically wipes
        // `memory_records` + `scan_runs` + `scan_diagnostics` +
        // `tessera_meta` rows matching `active_generation:%` (preserving
        // `source_registry`, `schema_version`, `tessera_migrations_applied`,
        // and any other `tessera_meta` key), then re-scans every Confirmed
        // Source by reusing the existing scan pipeline. Returns 409
        // `rebuild_failed` when a scan is currently in-flight across ANY
        // source (the primary race guard).
        (Method::Post, "/api/index/rebuild") => {
            respond_result(request, start_rebuild(state))
        }
        // Story 5.1 — Tessera Project mapping surface (six versioned,
        // loopback-only endpoints). All project endpoints are POST except
        // `GET /api/projects`; bodies are JSON request DTOs parsed the same
        // way `/api/sources/rebind` already parses its body (read_json_body
        // enforces the 1 MiB bound + bad_request stable code on shape drift).
        (Method::Post, "/api/projects/create") => {
            let body = match read_json_body::<crate::domain::project::CreateProjectRequest>(
                &mut request,
            ) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, create_project(body, state))
        }
        (Method::Get, "/api/projects") => respond_result(request, list_projects(state)),
        (Method::Post, "/api/projects/rename") => {
            let body = match read_json_body::<crate::domain::project::RenameProjectRequest>(
                &mut request,
            ) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, rename_project(body, state))
        }
        (Method::Post, "/api/projects/delete") => {
            let body = match read_json_body::<crate::domain::project::DeleteProjectRequest>(
                &mut request,
            ) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, delete_project(body, state))
        }
        (Method::Post, "/api/projects/mappings/add") => {
            let body = match read_json_body::<crate::domain::project::MappingRequest>(
                &mut request,
            ) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, add_mapping(body, state))
        }
        (Method::Post, "/api/projects/mappings/remove") => {
            let body = match read_json_body::<crate::domain::project::MappingRequest>(
                &mut request,
            ) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, remove_mapping(body, state))
        }
        (Method::Post, "/api/sources/rescan") => {
            let source_id = match read_source_id_body(&mut request) {
                Ok(id) => id,
                Err(response) => return request.respond(response),
            };
            respond_result(request, start_rescan(&source_id, state))
        }
        (Method::Post, "/api/sources/rescan/cancel") => {
            let source_id = match read_source_id_body(&mut request) {
                Ok(id) => id,
                Err(response) => return request.respond(response),
            };
            respond_result(request, cancel_rescan_request(&source_id, state))
        }
        (Method::Get, "/api/sources/rescan/events") => {
            let (source_id, job_id, after) = match parse_rescan_events_query(query) {
                Some(value) => value,
                None => {
                    return request.respond(json_error(
                        StatusCode(400),
                        "bad_request",
                        "missing or invalid source_id query parameter.",
                    ))
                }
            };
            match rescan_events(&source_id, &job_id, after, state) {
                Ok(events) => request.respond(sse_response(&events)),
                Err(error) => request.respond(json_response(StatusCode(500), &error)),
            }
        }
        (Method::Post, "/api/open") => {
            let open_request = match read_open_request_body(&mut request) {
                Ok(body) => body,
                Err(response) => return request.respond(response),
            };
            respond_result(request, open_original_location(open_request, state))
        }
        (Method::Get, "/api/search") => {
            let request_dto = match parse_search_query(query) {
                Ok(request_dto) => request_dto,
                Err(()) => {
                    return request.respond(json_error(
                        StatusCode(400),
                        "bad_request",
                        "The request did not match Tessera's search contract.",
                    ))
                }
            };
            respond_result(request, search(request_dto, state))
        }
        // Story 3.1 — query-less browse entry. Same envelope + cursor_stale →
        // 409 mapping as search; bad input → 400 (phase `browse`) per the I/O
        // matrix.
        (Method::Get, "/api/browse") => {
            let request_dto = match parse_browse_query(query) {
                Ok(request_dto) => request_dto,
                Err(()) => {
                    return request.respond(json_error(
                        StatusCode(400),
                        "bad_request",
                        "The request did not match Tessera's browse contract.",
                    ))
                }
            };
            respond_result(request, browse(request_dto, state))
        }
        (Method::Post, "/api/scan") => {
            let source_id = match read_source_id_body(&mut request) {
                Ok(id) => id,
                Err(response) => return request.respond(response),
            };
            respond_result(request, scan_source(&source_id, state))
        }
        (Method::Get, "/api/scan/status") => {
            let source_id = match parse_source_id_query(query) {
                Some(id) => id,
                None => {
                    return request.respond(json_error(
                        StatusCode(400),
                        "bad_request",
                        "missing or invalid source_id query parameter.",
                    ))
                }
            };
            respond_result(request, get_scan_status(&source_id, state))
        }
        (Method::Get, path) if !path.starts_with("/api/") => {
            serve_static(request, static_root, path)
        }
        _ => request.respond(json_error(
            StatusCode(404),
            "not_found",
            "Tessera has no such endpoint.",
        )),
    }
}

// ---------------------------------------------------------------------------
// Request DTOs (AD-4: confirm/reject are the only endpoints that accept a
// path, and only inside a CandidateSource; everything else takes a source_id)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct ConfirmRequest {
    candidate: CandidateSource,
}

#[derive(Debug, serde::Deserialize)]
struct SourceIdRequest {
    source_id: SourceId,
}

#[derive(Debug, serde::Deserialize)]
struct OpenRequestBody {
    record_id: String,
}

/// Read and deserialize a JSON request body. Enforces a 1 MiB bound (AD-17:
/// bounded contracts) and a `bad_request` stable code on any shape drift —
/// a malformed body is a client contract violation, never an internal error.
fn read_json_body<T: serde::de::DeserializeOwned>(
    request: &mut Request,
) -> Result<T, Response<std::io::Cursor<Vec<u8>>>> {
    const MAX_BODY_BYTES: usize = 1024 * 1024;
    let mut body = String::new();
    let mut limited = request.as_reader().take((MAX_BODY_BYTES + 1) as u64);
    if limited.read_to_string(&mut body).is_err() || body.len() > MAX_BODY_BYTES {
        return Err(json_error(
            StatusCode(400),
            "bad_request",
            "request body is unreadable or too large.",
        ));
    }
    serde_json::from_str(&body).map_err(|_| {
        json_error(
            StatusCode(400),
            "bad_request",
            "request body did not match the expected shape.",
        )
    })
}

fn read_source_id_body(
    request: &mut Request,
) -> Result<SourceId, Response<std::io::Cursor<Vec<u8>>>> {
    read_json_body::<SourceIdRequest>(request).map(|body| body.source_id)
}

fn read_open_request_body(
    request: &mut Request,
) -> Result<OpenRequest, Response<std::io::Cursor<Vec<u8>>>> {
    let body = read_json_body::<OpenRequestBody>(request)?;
    OpenRequest::new(body.record_id).map_err(|_| {
        json_response(
            StatusCode(400),
            &crate::http::ErrorEnvelope::bad_request("open"),
        )
    })
}

/// Parse `source_id=src_<n>` from a query string. Rejects anything that does
/// not carry exactly the expected parameter shape (AD-4: only `source_id`,
/// never an arbitrary path).
fn parse_source_id_query(query: &str) -> Option<SourceId> {
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        if key == "source_id" && value.starts_with("src_") && value.len() > 4 {
            return Some(SourceId(value.to_string()));
        }
    }
    None
}

/// Events are scoped to the queued job and can be resumed from a strict
/// sequence cursor. Unknown/duplicate fields are rejected rather than being
/// silently interpreted as a different job.
fn parse_rescan_events_query(query: &str) -> Option<(SourceId, String, u64)> {
    let mut source_id = None;
    let mut job_id = None;
    let mut after = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=')?;
        match key {
            "source_id" if source_id.is_none() && value.starts_with("src_") && value.len() > 4 => source_id = Some(SourceId(value.to_string())),
            "job_id" if job_id.is_none() && value.starts_with("job_") && value.len() > 4 => job_id = Some(value.to_string()),
            "after" if after.is_none() && value.len() <= 20 => after = value.parse::<u64>().ok(),
            _ => return None,
        }
    }
    Some((source_id?, job_id?, after?))
}

/// Parse exactly q/cursor/limit plus the Story 2.4 cross-provider filter
/// params (`provider`, `source`, `memory_type`, `native_project`, `since`,
/// reserved `tessera_project`). Text values are percent-decoded before the
/// request domain validates byte length and vocabulary; `since` is parsed as a
/// Unix-epoch `i64`; `source` is a `src_<n>` handle; unknown/duplicate keys are
/// rejected so API behavior stays bounded and unambiguous. Validation (known
/// provider id, valid memory type, `since >= 0`, well-formed `source`) lives
/// in [`SearchRequest::new_with_filters`].
fn parse_search_query(query: &str) -> Result<SearchRequest, ()> {
    let mut q = None;
    let mut cursor = None;
    let mut limit = None;
    let mut provider = None;
    let mut source = None;
    let mut memory_type = None;
    let mut native_project = None;
    let mut since = None;
    let mut tessera_project = None;
    if query.is_empty() {
        return Err(());
    }
    for pair in query.split('&') {
        let (key, raw_value) = pair.split_once('=').ok_or(())?;
        match key {
            "q" if q.is_none() => {
                q = Some(percent_decode_bounded(raw_value, MAX_QUERY_BYTES).ok_or(())?);
            }
            "cursor" if cursor.is_none() => {
                cursor = Some(percent_decode_bounded(raw_value, MAX_CURSOR_BYTES).ok_or(())?);
            }
            "limit" if limit.is_none() => {
                if raw_value.len() > 16 {
                    return Err(());
                }
                limit = Some(
                    percent_decode_bounded(raw_value, 16)
                        .ok_or(())?
                        .parse::<usize>()
                        .map_err(|_| ())?,
                );
            }
            // Story 2.4 — text filters are percent-decoded and bounded; the
            // request domain validates provider id membership and memory-type
            // vocabulary.
            "provider" if provider.is_none() => {
                provider = Some(percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?);
            }
            // Per-source filter (Spec Change Log 2026-07-25): `src_<n>` handle.
            // The shape is validated by `SearchRequest::new_with_filters`
            // (`to_rowid().is_some()`); the confirmed-source check is the SQL
            // JOIN on `lifecycle_state`.
            "source" if source.is_none() => {
                let value = percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?;
                source = Some(SourceId(value));
            }
            "memory_type" if memory_type.is_none() => {
                let value = percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?;
                memory_type = Some(ProviderMemoryType::parse_str(&value).ok_or(())?);
            }
            "native_project" if native_project.is_none() => {
                native_project = Some(percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?);
            }
            "since" if since.is_none() => {
                if raw_value.len() > 20 {
                    return Err(());
                }
                since = Some(
                    percent_decode_bounded(raw_value, 20)
                        .ok_or(())?
                        .parse::<i64>()
                        .map_err(|_| ())?,
                );
            }
            // Story 5.2 — Tessera-project projection filter (was reserved in
            // 2.4). Accepted on the wire as `proj_<n>` and forwarded to
            // SearchFilters; resolved to the project rowid at the SQL
            // boundary. An unknown / malformed id honestly matches nothing
            // (treated as a filter, NOT an error).
            "tessera_project" if tessera_project.is_none() => {
                tessera_project = Some(percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?);
            }
            _ => return Err(()),
        }
    }
    SearchRequest::new_with_filters(
        q.ok_or(())?,
        cursor,
        limit,
        SearchFilters {
            provider,
            source,
            memory_type,
            native_project,
            since,
            tessera_project,
        },
    )
    .map_err(|_| ())
}

/// Story 3.1 — parse exactly `source`, `cursor`, and `limit` for the
/// query-less browse endpoint. Text values are percent-decoded before the
/// request domain validates byte length and shape; `source` is a `src_<n>`
/// handle. Unknown/duplicate keys are rejected so API behavior stays bounded
/// and unambiguous (mirrors `parse_search_query`). Validation (`source`
/// well-formed, `limit` in `[1, MAX_SEARCH_LIMIT]`) lives in
/// [`BrowseRequest::new`]. The confirmed-source check is the SQL layer's
/// `lifecycle_state = 'confirmed'` JOIN plus the application layer's
/// explicit registry lookup.
///
/// Story 3.2 — also accept an optional `memory_type` query param, validated
/// against the `ProviderMemoryType` vocabulary (mirrors search's
/// `memory_type` handling). An invalid value returns `Err(())` → `400
/// bad_request` (phase `browse`), matching Search's invalid-memory-type
/// behavior so the two surfaces share one vocabulary.
fn parse_browse_query(query: &str) -> Result<BrowseRequest, ()> {
    let mut source = None;
    let mut cursor = None;
    let mut limit = None;
    let mut memory_type = None;
    if query.is_empty() {
        return Err(());
    }
    for pair in query.split('&') {
        let (key, raw_value) = pair.split_once('=').ok_or(())?;
        match key {
            // `src_<n>` handle. Shape validated by `BrowseRequest::new`
            // (`to_rowid().is_some()`); the confirmed-source check is the
            // SQL JOIN on `lifecycle_state`.
            "source" if source.is_none() => {
                let value = percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?;
                source = Some(SourceId(value));
            }
            "cursor" if cursor.is_none() => {
                cursor = Some(percent_decode_bounded(raw_value, MAX_CURSOR_BYTES).ok_or(())?);
            }
            "limit" if limit.is_none() => {
                if raw_value.len() > 16 {
                    return Err(());
                }
                limit = Some(
                    percent_decode_bounded(raw_value, 16)
                        .ok_or(())?
                        .parse::<usize>()
                        .map_err(|_| ())?,
                );
            }
            // Story 3.2 — memory_type filter. Vocabulary validated via
            // `ProviderMemoryType::parse_str` (single source of truth shared
            // with Search); an unknown value → 400 (phase `browse`).
            "memory_type" if memory_type.is_none() => {
                let value = percent_decode_bounded(raw_value, MAX_FILTER_BYTES).ok_or(())?;
                memory_type = Some(ProviderMemoryType::parse_str(&value).ok_or(())?);
            }
            _ => return Err(()),
        }
    }
    BrowseRequest::new_with_memory_type(source.ok_or(())?, cursor, limit, memory_type)
        .map_err(|_| ())
}

/// Reject oversized encoded values before allocating their decoded buffer.
/// Percent encoding can expand one decoded byte to three raw bytes.
fn percent_decode_bounded(value: &str, max_decoded_bytes: usize) -> Option<String> {
    if value.len() > max_decoded_bytes.checked_mul(3)? {
        return None;
    }
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'+' => {
                bytes.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < raw.len() => {
                let hi = (raw[index + 1] as char).to_digit(16)?;
                let lo = (raw[index + 2] as char).to_digit(16)?;
                bytes.push(((hi << 4) | lo) as u8);
                index += 3;
            }
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

fn split_path_query(url: &str) -> (&str, &str) {
    match url.split_once('?') {
        Some((path, query)) => (path, query),
        None => (url, ""),
    }
}

// ---------------------------------------------------------------------------
// Loopback validation (AD-9/AD-12)
// ---------------------------------------------------------------------------

/// Allowed Host authorities for this server: the bound loopback port on
/// 127.0.0.1 or localhost. When the bound port is unknown (tests), any
/// loopback authority with an explicit port is accepted.
fn host_header_is_loopback(request: &Request, bound_port: Option<u16>) -> bool {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv("host"))
        .map(|h| host_authority_allowed(h.value.as_str(), bound_port))
        .unwrap_or(false)
}

fn host_authority_allowed(authority: &str, bound_port: Option<u16>) -> bool {
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, port.parse::<u16>().ok()),
        None => (authority, None),
    };
    let host_ok = host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "[::1]";
    match (host_ok, bound_port, port) {
        (true, Some(expected), Some(actual)) => actual == expected,
        // With no known bound port, require an explicit port so bare-domain
        // rebinding ("evil.com" → 127.0.0.1) cannot pass as host-less HTTP/1.0.
        (true, None, Some(_)) => true,
        _ => false,
    }
}

/// An `Origin` header, when present, must be an HTTP origin on this server's
/// loopback authority. Absence is allowed: same-origin GET/fetch and non-
/// browser clients may omit it, and cross-origin requests are precisely the
/// ones browsers mark with Origin.
fn origin_header_is_allowed(request: &Request, bound_port: Option<u16>) -> bool {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv("origin"))
        .map(|h| origin_value_allowed(h.value.as_str(), bound_port))
        .unwrap_or(true)
}

fn origin_value_allowed(origin: &str, bound_port: Option<u16>) -> bool {
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .unwrap_or("");
    host_authority_allowed(authority, bound_port)
}

// ---------------------------------------------------------------------------
// Responses
// ---------------------------------------------------------------------------

fn respond_ok<T: serde::Serialize>(
    request: Request,
    envelope: crate::http::Envelope<T>,
) -> std::io::Result<()> {
    request.respond(json_response(StatusCode(200), &envelope))
}

fn respond_result<T: serde::Serialize>(
    request: Request,
    result: Result<crate::http::Envelope<T>, crate::http::ErrorEnvelope>,
) -> std::io::Result<()> {
    match result {
        Ok(envelope) => request.respond(json_response(StatusCode(200), &envelope)),
        Err(error) => {
            let status = match error.code.as_str() {
                "bad_request" => StatusCode(400),
                "source_not_found" | "record_not_found" | "project_not_found"
                | "mapping_not_found" => StatusCode(404),
                "confirm_failed" | "scan_failed" | "cursor_stale" | "open_failed"
                | "rebuild_failed" | "mapping_conflict" => StatusCode(409),
                _ => StatusCode(500),
            };
            request.respond(json_response(status, &error))
        }
    }
}

fn json_response<T: serde::Serialize>(
    status: StatusCode,
    value: &T,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::to_vec(value).unwrap_or_else(|_| {
        // Serialization of our own DTOs is total; this branch only exists so a
        // bug can never produce a bare HTTP error without the envelope shape.
        b"{\"code\":\"internal\",\"message\":\"Tessera hit an internal error.\",\"source_id\":null,\"phase\":\"transport\"}".to_vec()
    });
    with_security_headers(Response::from_data(body).with_status_code(status), true)
}

fn json_error(status: StatusCode, code: &str, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    json_response(
        status,
        &crate::http::ErrorEnvelope {
            code: code.to_string(),
            message: message.to_string(),
            source_id: None,
            phase: "transport".to_string(),
        },
    )
}

/// A finite SSE snapshot. Clients may reconnect while a job is running; each
/// event carries its own version and strictly increasing sequence, so a client
/// can reject malformed or replayed/out-of-order observations without gaining
/// any scan authority.
fn sse_response(events: &[crate::RescanEvent]) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut body = Vec::new();
    for event in events {
        body.extend_from_slice(b"event: progress\n");
        body.extend_from_slice(b"data: ");
        body.extend_from_slice(
            serde_json::to_string(event)
                .unwrap_or_else(|_| "{}".to_string())
                .as_bytes(),
        );
        body.extend_from_slice(b"\n\n");
    }
    with_security_headers(
        Response::from_data(body)
            .with_status_code(StatusCode(200))
            .with_header(
                Header::from_bytes("Content-Type", "text/event-stream; charset=utf-8").unwrap(),
            ),
        false,
    )
}

/// Attach the AD-9 security headers. `is_api` adds `Cache-Control: no-store`
/// so derived-index data is never cached by the browser or any intermediary.
fn with_security_headers<R: std::io::Read>(response: Response<R>, is_api: bool) -> Response<R> {
    let response = response
        .with_header(
            Header::from_bytes("Content-Security-Policy", CONTENT_SECURITY_POLICY).unwrap(),
        )
        .with_header(Header::from_bytes("X-Content-Type-Options", "nosniff").unwrap())
        .with_header(Header::from_bytes("Referrer-Policy", "no-referrer").unwrap());
    if is_api {
        response
            .with_header(
                Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap(),
            )
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap())
    } else {
        response
    }
}

// ---------------------------------------------------------------------------
// Static UI serving
// ---------------------------------------------------------------------------

/// Serve a file from the built UI directory. `/` maps to `index.html`. Any
/// path containing a parent-directory component or an absolute component is
/// rejected — the static root is a strict subtree (AD-4's allowlist mindset
/// applied to the one directory the server may expose).
fn serve_static(request: Request, static_root: &Path, path: &str) -> std::io::Result<()> {
    let relative = match sanitize_static_path(path) {
        Some(relative) => relative,
        None => {
            return request.respond(json_error(StatusCode(400), "bad_request", "invalid path."))
        }
    };
    let full_path = static_root.join(&relative);
    let body = match std::fs::read(&full_path) {
        Ok(body) => body,
        Err(_) => {
            return request.respond(json_error(
                StatusCode(404),
                "not_found",
                "no such UI asset.",
            ))
        }
    };
    let content_type = match full_path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    };
    let response = Response::from_data(body)
        .with_status_code(StatusCode(200))
        .with_header(Header::from_bytes("Content-Type", content_type).unwrap());
    request.respond(with_security_headers(response, false))
}

/// Turn a URL path into a safe relative path inside the static root.
fn sanitize_static_path(path: &str) -> Option<PathBuf> {
    let trimmed = path.trim_start_matches('/');
    let candidate = if trimmed.is_empty() {
        PathBuf::from("index.html")
    } else {
        PathBuf::from(trimmed)
    };
    if candidate
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return None;
    }
    Some(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Host / Origin validation (AD-9) -----------------------------------

    #[test]
    fn host_authority_accepts_bound_loopback_port() {
        assert!(host_authority_allowed("127.0.0.1:1420", Some(1420)));
        assert!(host_authority_allowed("localhost:1420", Some(1420)));
        assert!(host_authority_allowed("LOCALHOST:1420", Some(1420)));
        assert!(host_authority_allowed("[::1]:1420", Some(1420)));
    }

    #[test]
    fn host_authority_rejects_foreign_domains_and_wrong_ports() {
        // DNS rebinding: a hostile domain resolving to 127.0.0.1.
        assert!(!host_authority_allowed("evil.example.com", Some(1420)));
        assert!(!host_authority_allowed("evil.example.com:1420", Some(1420)));
        // Right host, wrong port — must not match another local service.
        assert!(!host_authority_allowed("127.0.0.1:9999", Some(1420)));
        // External interfaces are never a valid authority.
        assert!(!host_authority_allowed("0.0.0.0:1420", Some(1420)));
        assert!(!host_authority_allowed("192.168.1.10:1420", Some(1420)));
        // Port-less authorities cannot be tied to the bound listener.
        assert!(!host_authority_allowed("127.0.0.1", Some(1420)));
    }

    #[test]
    fn origin_value_accepts_only_loopback_origins() {
        assert!(origin_value_allowed("http://127.0.0.1:1420", Some(1420)));
        assert!(origin_value_allowed("http://localhost:1420", Some(1420)));
        assert!(!origin_value_allowed(
            "https://evil.example.com",
            Some(1420)
        ));
        assert!(!origin_value_allowed(
            "http://evil.example.com:1420",
            Some(1420)
        ));
        assert!(!origin_value_allowed("", Some(1420)));
        // A loopback origin for a *different* port is a different origin.
        assert!(!origin_value_allowed("http://127.0.0.1:3000", Some(1420)));
    }

    // --- Query / path handling ----------------------------------------------

    #[test]
    fn parse_source_id_query_accepts_only_source_id_shape() {
        assert_eq!(
            parse_source_id_query("source_id=src_7").map(|id| id.0),
            Some("src_7".to_string())
        );
        // Arbitrary paths are not a source_id (AD-4).
        assert_eq!(parse_source_id_query("source_id=/etc/passwd"), None);
        assert_eq!(parse_source_id_query("path=src_7"), None);
        assert_eq!(parse_source_id_query(""), None);
    }

    #[test]
    fn parse_search_query_rejects_oversized_encoded_values_before_decoding() {
        assert!(parse_search_query(&format!("q={}", "%41".repeat(MAX_QUERY_BYTES + 1))).is_err());
        assert!(parse_search_query(&format!(
            "q=ok&cursor={}",
            "%41".repeat(MAX_CURSOR_BYTES + 1)
        ))
        .is_err());
        assert!(parse_search_query("q=%E4%B8%AD%E6%96%87&limit=2").is_ok());
    }

    #[test]
    fn split_path_query_handles_missing_query() {
        assert_eq!(split_path_query("/api/ping"), ("/api/ping", ""));
        assert_eq!(
            split_path_query("/api/scan/status?source_id=src_1"),
            ("/api/scan/status", "source_id=src_1")
        );
    }

    #[test]
    fn sanitize_static_path_maps_root_and_rejects_traversal() {
        assert_eq!(sanitize_static_path("/"), Some(PathBuf::from("index.html")));
        assert_eq!(
            sanitize_static_path("/assets/app.js"),
            Some(PathBuf::from("assets/app.js"))
        );
        assert_eq!(sanitize_static_path("/../Cargo.toml"), None);
        assert_eq!(sanitize_static_path("/assets/../../secret"), None);
    }

    // --- Security headers (AD-9) --------------------------------------------

    #[test]
    fn json_responses_carry_full_security_header_set() {
        let response = json_response(StatusCode(200), &ping());
        let headers: Vec<String> = response
            .headers()
            .iter()
            .map(|h| format!("{}: {}", h.field, h.value.as_str()))
            .collect();
        let joined = headers.join("\n");
        assert!(joined.contains("Content-Security-Policy"), "got:\n{joined}");
        assert!(joined.contains("connect-src 'self'"), "got:\n{joined}");
        assert!(joined.contains("object-src 'none'"), "got:\n{joined}");
        assert!(joined.contains("frame-src 'none'"), "got:\n{joined}");
        assert!(
            joined.contains("X-Content-Type-Options: nosniff"),
            "got:\n{joined}"
        );
        assert!(
            joined.contains("Referrer-Policy: no-referrer"),
            "got:\n{joined}"
        );
        assert!(joined.contains("Cache-Control: no-store"), "got:\n{joined}");
        assert!(joined.contains("application/json"), "got:\n{joined}");
    }

    #[test]
    fn csp_has_no_remote_or_ipc_endpoints() {
        // The old Tauri CSP allowed `ipc:` / `http://ipc.localhost`; the web
        // delivery form must only allow this same origin (AD-12/NFR-2).
        assert!(!CONTENT_SECURITY_POLICY.contains("ipc:"));
        assert!(!CONTENT_SECURITY_POLICY.contains("ipc.localhost"));
        assert!(!CONTENT_SECURITY_POLICY.contains("http://"));
        assert!(!CONTENT_SECURITY_POLICY.contains("https://"));
        assert!(CONTENT_SECURITY_POLICY.contains("default-src 'self'"));
    }
}
