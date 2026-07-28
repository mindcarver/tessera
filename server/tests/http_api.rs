//! Wire-level integration tests for the loopback-only HTTP transport
//! (revised AD-9, 2026-07-22; sprint-change-proposal-2026-07-22).
//!
//! These tests pin the AD-9/AD-17 contract end-to-end at the socket level:
//! the versioned envelope actually crosses HTTP, the AD-9 security headers are
//! on the wire, and the loopback hardening (Host / Origin validation) rejects
//! hostile requests before any handler runs. Raw `TcpStream` HTTP keeps the
//! test surface dependency-free (no HTTP client crate in dev-dependencies).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rusqlite::params;
use tessera_lib::adapters::codex::file_uri;
use tessera_lib::domain::{CandidateSource, CoverageLevel, DiscoveryBasis};
use tessera_lib::http::server::{bind, serve_with};
use tessera_lib::index::SourceRegistry;

static HTTP_OPEN_TEST_LOCK: Mutex<()> = Mutex::new(());
static HTTP_OPENED_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

fn capture_http_open(path: &Path) -> std::io::Result<()> {
    *HTTP_OPENED_PATH.lock().expect("opened path lock") = Some(path.to_path_buf());
    Ok(())
}

/// Boot a real server on an ephemeral loopback port with a scratch app-data
/// dir, and return its bound port. The server thread lives for the rest of
/// the test process — tests use distinct ephemeral ports, so they coexist.
fn boot_test_server() -> u16 {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot must succeed on scratch dir");
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    std::thread::spawn(move || {
        // Keep the tempdir alive for the server's lifetime.
        let _dir = dir;
        serve_with(server, Arc::new(state), PathBuf::from("dist"), Some(port));
    });
    // Give the accept loop a moment to start; connect retries below absorb
    // any remaining race.
    std::thread::sleep(std::time::Duration::from_millis(50));
    port
}

/// A live loopback server with a populated active generation. This keeps the
/// wire contract test at the HTTP boundary rather than relying on a mocked
/// application response.
fn boot_populated_search_server() -> (u16, Arc<tessera_lib::IndexState>) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot must succeed on scratch dir");
    {
        let conn = state.conn.lock().expect("connection lock");
        conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/fixture', 'fixture', NULL)", []).expect("source");
        conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')", []).expect("scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("active generation");
        for (id, title, body) in [
            ("rec_a", "keyword first", "body one"),
            ("rec_b", "keyword second", "body two"),
        ] {
            conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES (?1, 1, 'gen_1', 'codex', 'section', ?1, 'file:///fixture#semantic', 'hash', 'v1', ?2, ?3, NULL, 'memory', 'full', 1, 'revision', 'file:///fixture#L1-L2')", params![id, title, body]).expect("record");
        }
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, state)
}

fn boot_populated_open_server() -> (u16, PathBuf) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let source_root = tempfile::tempdir().expect("source root");
    let memory = source_root.path().join("MEMORY.md");
    std::fs::write(&memory, "keyword original").expect("source memory");
    let expected = std::fs::canonicalize(&memory).expect("canonical memory");
    let locator = file_uri(&memory).expect("file uri");
    let state = tessera_lib::boot(dir.path()).expect("boot must succeed on scratch dir");
    {
        let conn = state.conn.lock().expect("connection lock");
        let root = std::fs::canonicalize(source_root.path()).expect("canonical source root");
        let root = root.to_string_lossy().into_owned();
        conn.execute("INSERT INTO source_registry (provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project) VALUES ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', ?1, 'fixture', NULL)", params![root]).expect("source");
        conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')", []).expect("scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("active generation");
        conn.execute("INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator) VALUES ('rec_a', 1, 'gen_1', 'codex', 'section', 'rec_a', ?1, 'hash', 'v1', 'keyword', 'body', NULL, 'memory', 'full', 1, 'revision', ?2)", params![locator, format!("{locator}#L1-L2")]).expect("record");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    std::thread::spawn(move || {
        let _dir = dir;
        let _source_root = source_root;
        serve_with(server, state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, expected)
}

fn boot_rescan_server() -> u16 {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let source_root = tempfile::tempdir().expect("source root");
    std::fs::write(source_root.path().join("MEMORY.md"), "# rescan\nbody").expect("memory");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection");
        let registry = SourceRegistry::new(&conn);
        let source = tessera_lib::application::confirm_source(&registry, &CandidateSource {
            provider: "codex".into(), root_path: source_root.path().to_string_lossy().into_owned(),
            basis: DiscoveryBasis::CodexHomeEnv, coverage_level: CoverageLevel::Full, native_project: None,
        }).expect("confirm");
        tessera_lib::application::scan_source(&registry, &conn, &source.source_id).expect("initial scan");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound").port();
    std::thread::spawn(move || {
        let _dir = dir;
        let _source_root = source_root;
        serve_with(server, Arc::new(state), PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    port
}

/// Send one raw HTTP/1.1 request and read the full response text. A read
/// timeout turns a stuck connection into a loud test failure instead of an
/// infinite hang.
fn raw_http(port: u16, request: &str) -> String {
    let mut last_err = None;
    for _ in 0..20 {
        match TcpStream::connect(("127.0.0.1", port)) {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .expect("set read timeout");
                stream.write_all(request.as_bytes()).expect("write request");
                let mut buf = String::new();
                stream.read_to_string(&mut buf).expect("read response");
                return buf;
            }
            Err(e) => {
                last_err = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        }
    }
    panic!("could not connect to test server: {last_err:?}");
}

/// POST a JSON body and return the full response text. Story 5.2 — shared by
/// the project-API tests under `http_api.rs` so the Content-Length is always
/// computed from the actual body (mirrors `projects_api.rs::post_json`).
fn post_json(port: u16, path: &str, body: &str) -> String {
    raw_http(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

/// `GET /api/ping` returns the versioned envelope with the full AD-9 security
/// header set — the UI→core→UI round-trip on the new transport (Story 1.1 AC).
#[test]
fn ping_round_trip_carries_versioned_envelope_and_security_headers() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/ping HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"api_version\":\"1\""),
        "got:\n{response}"
    );
    assert!(
        response.contains("\"name\":\"tessera\""),
        "got:\n{response}"
    );
    // AD-9 security headers on the wire.
    assert!(
        response.contains("Content-Security-Policy"),
        "got:\n{response}"
    );
    assert!(response.contains("connect-src 'self'"), "got:\n{response}");
    assert!(
        response.contains("X-Content-Type-Options: nosniff"),
        "got:\n{response}"
    );
    assert!(
        response.contains("Cache-Control: no-store"),
        "got:\n{response}"
    );
}

/// A request addressed to a foreign Host (DNS-rebinding shape) is rejected
/// before any handler runs (AD-9).
#[test]
fn foreign_host_header_is_rejected() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        "GET /api/ping HTTP/1.1\r\nHost: evil.example.com\r\nConnection: close\r\n\r\n",
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("forbidden_host"), "got:\n{response}");
}

/// A cross-origin request (browser-marked with a foreign Origin) is rejected
/// (AD-9); only the server's own loopback origin may call the API.
#[test]
fn foreign_origin_header_is_rejected() {
    let port = boot_test_server();
    let body = "{\"source_id\":\"src_1\"}";
    let response = raw_http(
        port,
        &format!(
            "POST /api/scan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: https://evil.example.com\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 403"), "got:\n{response}");
    assert!(response.contains("forbidden_origin"), "got:\n{response}");
}

/// The server's own loopback Origin passes validation (the served UI's
/// same-origin fetches are the legitimate caller).
#[test]
fn own_loopback_origin_is_accepted() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        &format!(
            "GET /api/sources/discover HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"api_version\":\"1\""),
        "got:\n{response}"
    );
}

/// Story 6.2 — the Knowledge (Obsidian Vault) discovery endpoint is on the wire
/// with the versioned envelope and the `{candidates, diagnostic}` payload
/// shape. The endpoint reads the host's real registry, so the candidate count
/// varies by host; we assert the structural contract (envelope + payload keys +
/// provider id for any candidate present) rather than a fixed count.
#[test]
fn knowledge_discover_endpoint_returns_versioned_envelope_with_diagnostic_field() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        &format!(
            "GET /api/knowledge/discover HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"api_version\":\"1\""),
        "got:\n{response}"
    );
    let body = response.split("\r\n\r\n").nth(1).expect("knowledge body");
    let page: serde_json::Value = serde_json::from_str(body).expect("versioned knowledge JSON");
    // The payload MUST carry both keys regardless of host state.
    assert!(
        page["payload"]["candidates"].is_array(),
        "candidates must be an array; got: {body}"
    );
    assert!(
        page["payload"].get("diagnostic").is_some(),
        "diagnostic key must be present (null when registry is clean); got: {body}"
    );
    // Every candidate, if any, is an Obsidian local_knowledge candidate.
    for c in page["payload"]["candidates"].as_array().unwrap() {
        assert_eq!(c["provider"], "obsidian", "got: {body}");
        assert_eq!(c["basis"], "obsidian_vault_registry", "got: {body}");
    }
}

/// A malformed JSON body on a `source_id` endpoint surfaces `bad_request`,
/// never an internal error or a panic (AD-13/AD-17 bounded contracts).
#[test]
fn malformed_scan_body_is_bad_request() {
    let port = boot_test_server();
    let body = "not json at all";
    let response = raw_http(
        port,
        &format!(
            "POST /api/scan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("bad_request"), "got:\n{response}");
}

#[test]
fn search_wire_contract_serializes_provenance_and_rejects_invalid_input_safely() {
    let (port, state) = boot_populated_search_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=keyword&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("search body");
    let page: serde_json::Value = serde_json::from_str(body).expect("versioned search JSON");
    assert_eq!(page["api_version"], "1");
    let result = &page["payload"]["results"][0];
    for field in [
        "record_id",
        "excerpt",
        "provider",
        "source_id",
        "native_locator",
        "display_locator",
        "observed_at",
        "coverage_level",
        "health_state",
    ] {
        assert!(!result[field].is_null(), "missing {field}: {result}");
    }
    assert!(
        result.get("native_project").is_some(),
        "missing native_project: {result}"
    );
    // Story 2.3: the FR-14 per-query sidecar is present on every page.
    let sources = page["payload"]["sources"].as_array().expect("sources sidecar array");
    assert!(!sources.is_empty(), "sidecar must list confirmed sources");
    assert!(
        sources.iter().all(|entry| {
            entry["source_id"].is_string()
                && entry["provider"].is_string()
                && matches!(entry["status"].as_str(), Some("available") | Some("degraded") | Some("unavailable"))
        }),
        "sidecar entries must carry source_id/provider/status: {sources:?}"
    );
    // The single confirmed source in this fixture is healthy + indexed.
    assert!(
        sources.iter().any(|entry| entry["status"].as_str() == Some("available")),
        "healthy source must be available in sidecar: {sources:?}"
    );
    let cursor = page["payload"]["next_cursor"]
        .as_str()
        .expect("continuation cursor");
    let continuation = raw_http(
        port,
        &format!("GET /api/search?q=keyword&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(
        continuation.starts_with("HTTP/1.1 200"),
        "got:\n{continuation}"
    );
    assert!(continuation.contains("rec_b"), "got:\n{continuation}");

    {
        let conn = state.conn.lock().expect("connection lock");
        conn.execute("INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision) VALUES (1, 'gen_2', 'succeeded', 2, 'gen_2', 'fixture')", []).expect("new scan run");
        conn.execute(
            "UPDATE tessera_meta SET value = 'gen_2' WHERE key = 'active_generation:1'",
            [],
        )
        .expect("activate new generation");
    }
    let stale = raw_http(
        port,
        &format!("GET /api/search?q=keyword&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(stale.starts_with("HTTP/1.1 409"), "got:\n{stale}");
    assert!(stale.contains("\"code\":\"cursor_stale\""), "got:\n{stale}");
    assert!(
        !stale.contains("keyword"),
        "safe error must not reflect the query: {stale}"
    );
    assert!(
        !continuation.contains("rec_a\""),
        "continuation duplicated first result: {continuation}"
    );

    let invalid = raw_http(
        port,
        &format!("GET /api/search?q=%20%20%20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(invalid.starts_with("HTTP/1.1 400"), "got:\n{invalid}");
    assert!(
        invalid.contains("\"code\":\"bad_request\""),
        "got:\n{invalid}"
    );
    assert!(
        !invalid.contains("%20%20%20"),
        "query must not cross the wire: {invalid}"
    );
}

#[test]
fn inventory_and_rescan_routes_are_versioned_and_reject_unknown_sources() {
    let (port, _state) = boot_populated_search_server();
    let inventory = raw_http(
        port,
        &format!(
            "GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(inventory.starts_with("HTTP/1.1 200"), "got:\n{inventory}");
    assert!(inventory.contains("\"api_version\":\"1\""), "got:\n{inventory}");
    assert!(inventory.contains("\"complete_record_count\":2"), "got:\n{inventory}");
    let body = "{\"source_id\":\"src_99\"}";
    let rejected = raw_http(
        port,
        &format!(
            "POST /api/sources/rescan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(), body
        ),
    );
    assert!(rejected.starts_with("HTTP/1.1 404"), "got:\n{rejected}");
    assert!(rejected.contains("source_not_found"), "got:\n{rejected}");
    assert!(!rejected.contains("/fixture"), "got:\n{rejected}");
}

/// Story 2.5 AC — `GET /api/sources/inventory` returns both providers' rows
/// over HTTP for a mixed Codex + Claude Code fixture. Each row carries its own
/// `provider`, `health_state`, and honest per-source `complete_record_count`.
/// Pins the multi-provider panorama at the wire boundary: the inventory
/// endpoint has been multi-provider at the row level since 2.1, and the 2.5
/// panorama UI (grouping, summary header) depends on this wire-level
/// guarantee. The `provider_not_scannable`-absent negative assertion lives in
/// `rescan_claude_code_source_succeeds_and_activates_on_wire` (2.2 removed the
/// vocabulary; this test does not re-introduce it).
#[test]
fn inventory_returns_both_providers_rows_over_http() {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection lock");
        // Source 1: Codex, healthy, indexed with two records.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-codex', 'fp-codex', NULL)",
            [],
        )
        .expect("codex source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture', 100)",
            [],
        )
        .expect("codex scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("codex active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_codex_a', 1, 'gen_1', 'codex', 'section', 'rec_codex_a', 'file:///codex#a', 'hash', 'v1', 'title a', 'body a', NULL, 'memory', 'full', 100, 'revision', 'file:///codex#L1-L2'),
                    ('rec_codex_b', 1, 'gen_1', 'codex', 'section', 'rec_codex_b', 'file:///codex#b', 'hash', 'v1', 'title b', 'body b', NULL, 'memory', 'full', 101, 'revision', 'file:///codex#L3-L4')",
            [],
        )
        .expect("codex records");
        // Source 2: Claude Code, healthy, indexed with one record and a native project.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-claude', 'fp-claude', 'proj-claude')",
            [],
        )
        .expect("claude source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at)
             VALUES (2, 'gen_2', 'succeeded', 1, 'gen_2', 'fixture', 200)",
            [],
        )
        .expect("claude scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:2', 'gen_2')",
            [],
        )
        .expect("claude active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_claude', 2, 'gen_2', 'claude_code', 'section', 'rec_claude', 'file:///claude#x', 'hash', 'v1', 'title c', 'body c', 'proj-claude', 'memory', 'full', 200, 'revision', 'file:///claude#L1-L2')",
            [],
        )
        .expect("claude record");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let response = raw_http(
        port,
        &format!(
            "GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"api_version\":\"1\""),
        "versioned envelope on the wire: {response}",
    );
    let body = response.split("\r\n\r\n").nth(1).expect("inventory body");
    let json: serde_json::Value = serde_json::from_str(body).expect("inventory json");
    let rows = json["payload"].as_array().expect("inventory array");
    assert_eq!(
        rows.len(),
        2,
        "both providers' rows return over HTTP: {rows:?}",
    );

    let codex = rows
        .iter()
        .find(|r| r["provider"].as_str() == Some("codex"))
        .expect("codex row on the wire");
    assert_eq!(codex["health_state"].as_str(), Some("healthy"));
    assert_eq!(
        codex["complete_record_count"].as_u64(),
        Some(2),
        "codex count is its own: {:?}",
        codex["complete_record_count"],
    );
    assert!(
        codex["native_project"].is_null(),
        "codex native_project is null (global store): {:?}",
        codex["native_project"],
    );
    assert_eq!(
        codex["last_successful_scan"].as_u64(),
        Some(100),
        "codex last_successful_scan is the succeeded run's finished_at: {:?}",
        codex["last_successful_scan"],
    );

    let claude = rows
        .iter()
        .find(|r| r["provider"].as_str() == Some("claude_code"))
        .expect("claude row on the wire");
    assert_eq!(claude["health_state"].as_str(), Some("healthy"));
    assert_eq!(
        claude["complete_record_count"].as_u64(),
        Some(1),
        "claude count is its own, independent of codex: {:?}",
        claude["complete_record_count"],
    );
    assert_eq!(
        claude["native_project"].as_str(),
        Some("proj-claude"),
        "claude native_project preserved verbatim",
    );
    assert_eq!(
        claude["last_successful_scan"].as_u64(),
        Some(200),
        "claude last_successful_scan is the succeeded run's finished_at: {:?}",
        claude["last_successful_scan"],
    );
}

/// Story 4.2 AC — `GET /api/sources/inventory` surfaces the structured
/// `cause` and derived `stale` fields on the wire for a degraded source, and
/// reports `cause=null, stale=false` for a healthy source. The DTO change is
/// additive (cause + stale); `latest_error` keeps its existing derivation.
#[test]
fn inventory_surfaces_cause_and_stale_on_the_wire() {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection lock");
        // Source 1: Codex, healthy, indexed — the additive fields read
        // `cause=null, stale=false`.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project, health_cause)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-codex', 'fp-codex', NULL, 'none')",
            [],
        )
        .expect("codex source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture', 100)",
            [],
        )
        .expect("codex scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("codex active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_codex', 1, 'gen_1', 'codex', 'section', 'rec_codex', 'file:///codex', 'hash', 'v1', 'title', 'body', NULL, 'memory', 'full', 100, 'revision', 'file:///codex#L1')",
            [],
        )
        .expect("codex record");
        // Source 2: Claude Code, degraded, stale (active generation exists),
        // persisted cause = path_missing.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project, health_cause)
             VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'degraded', 'full', '/fixture-claude', 'fp-claude', 'proj-claude', 'path_missing')",
            [],
        )
        .expect("claude source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, finished_at)
             VALUES (2, 'gen_2', 'succeeded', 1, 'gen_2', 'fixture', 200)",
            [],
        )
        .expect("claude scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:2', 'gen_2')",
            [],
        )
        .expect("claude active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_claude', 2, 'gen_2', 'claude_code', 'section', 'rec_claude', 'file:///claude', 'hash', 'v1', 'title', 'body', 'proj-claude', 'memory', 'full', 200, 'revision', 'file:///claude#L1')",
            [],
        )
        .expect("claude record");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let response = raw_http(
        port,
        &format!(
            "GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("inventory body");
    let json: serde_json::Value = serde_json::from_str(body).expect("inventory json");
    let rows = json["payload"].as_array().expect("inventory array");
    assert_eq!(rows.len(), 2, "both rows return: {rows:?}");

    let codex = rows
        .iter()
        .find(|r| r["provider"].as_str() == Some("codex"))
        .expect("codex row on the wire");
    assert_eq!(codex["health_state"].as_str(), Some("healthy"));
    // Story 4.2 — a healthy source reports cause=null, stale=false.
    assert!(
        codex["cause"].is_null(),
        "healthy source cause is null: {:?}",
        codex["cause"],
    );
    assert_eq!(
        codex["stale"].as_bool(),
        Some(false),
        "healthy source is not stale",
    );

    let claude = rows
        .iter()
        .find(|r| r["provider"].as_str() == Some("claude_code"))
        .expect("claude row on the wire");
    assert_eq!(claude["health_state"].as_str(), Some("degraded"));
    // Story 4.2 — a degraded source with an active generation reports
    // cause=path_missing (persisted on the source row) and stale=true.
    assert_eq!(
        claude["cause"].as_str(),
        Some("path_missing"),
        "degraded source surfaces its persisted cause on the wire: {:?}",
        claude["cause"],
    );
    assert_eq!(
        claude["stale"].as_bool(),
        Some(true),
        "degraded source with an active generation is stale",
    );
}

#[test]
fn rescan_is_singleton_and_events_are_versioned_ordered_and_job_scoped() {
    let port = boot_rescan_server();
    let body = "{\"source_id\":\"src_1\"}";
    let start = raw_http(port, &format!("POST /api/sources/rescan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body));
    assert!(start.starts_with("HTTP/1.1 200"), "got:\n{start}");
    let payload: serde_json::Value = serde_json::from_str(start.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    let job_id = payload["payload"]["job_id"].as_str().expect("job id");
    let duplicate = raw_http(port, &format!("POST /api/sources/rescan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body));
    let duplicate_json: serde_json::Value = serde_json::from_str(duplicate.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(duplicate_json["payload"]["job_id"], job_id, "duplicate must join the existing job");
    let mut events = Vec::new();
    for _ in 0..40 {
        let response = raw_http(port, &format!("GET /api/sources/rescan/events?source_id=src_1&job_id={job_id}&after=0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"));
        assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
        events = response.split("\r\n\r\n").nth(1).expect("sse body").split("\n\n").filter_map(|block| block.lines().find_map(|line| line.strip_prefix("data: "))).map(|json| serde_json::from_str::<serde_json::Value>(json).expect("event json")).collect::<Vec<_>>();
        if events.last().and_then(|event| event["state"].as_str()).is_some_and(|state| matches!(state, "succeeded" | "failed" | "cancelled")) { break; }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert!(events.len() >= 2, "queued plus terminal event");
    for pair in events.windows(2) { assert_eq!(pair[1]["sequence"].as_u64(), pair[0]["sequence"].as_u64().map(|value| value + 1)); }
    assert!(events.iter().all(|event| event["api_version"] == "1" && event["job_id"] == job_id));
    assert_eq!(events.last().and_then(|event| event["state"].as_str()), Some("succeeded"));
}

#[test]
fn open_wire_contract_invokes_server_opener_and_rejects_missing_record_safely() {
    let _guard = HTTP_OPEN_TEST_LOCK.lock().expect("test lock");
    *HTTP_OPENED_PATH.lock().expect("opened path lock") = None;
    tessera_lib::application::set_open_path_for_tests(capture_http_open);
    let (port, expected) = boot_populated_open_server();
    let body = r#"{"record_id":"rec_a"}"#;
    let response = raw_http(
        port,
        &format!(
            "POST /api/open HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"api_version\":\"1\""),
        "got:\n{response}"
    );
    assert!(
        response.contains("\"record_id\":\"rec_a\""),
        "got:\n{response}"
    );
    assert_eq!(HTTP_OPENED_PATH.lock().unwrap().as_ref(), Some(&expected));

    let missing_body = r#"{"record_id":"rec_missing"}"#;
    let missing = raw_http(
        port,
        &format!(
            "POST /api/open HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            missing_body.len(),
            missing_body
        ),
    );
    assert!(missing.starts_with("HTTP/1.1 404"), "got:\n{missing}");
    assert!(
        missing.contains("\"code\":\"record_not_found\""),
        "got:\n{missing}"
    );
    assert!(
        !missing.contains(expected.to_string_lossy().as_ref()),
        "open error must not leak paths: {missing}"
    );

    let invalid_body = r#"{"record_id":"not-a-record"}"#;
    let invalid = raw_http(
        port,
        &format!(
            "POST /api/open HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            invalid_body.len(),
            invalid_body
        ),
    );
    assert!(invalid.starts_with("HTTP/1.1 400"), "got:\n{invalid}");
    assert!(invalid.contains("open contract"), "got:\n{invalid}");
    tessera_lib::application::reset_open_path_for_tests();
}

#[test]
fn malformed_confirmed_source_returns_safe_scan_failure_on_wire() {
    let source_root = tempfile::tempdir().expect("source root");
    let memory = source_root.path().join("MEMORY.md");
    std::fs::write(&memory, "# Valid\nbody\n").expect("valid source");
    let port = boot_test_server();
    let confirm_body = serde_json::json!({
        "candidate": {
            "provider": "codex",
            "root_path": source_root.path(),
            "basis": "codex_home_env",
            "coverage_level": "full",
            "native_project": null
        }
    })
    .to_string();
    let confirmed = raw_http(
        port,
        &format!(
            "POST /api/sources/confirm HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            confirm_body.len(),
            confirm_body
        ),
    );
    assert!(confirmed.starts_with("HTTP/1.1 200"), "got:\n{confirmed}");
    let confirmed_body = confirmed
        .split("\r\n\r\n")
        .nth(1)
        .expect("confirmation body");
    let source_id = serde_json::from_str::<serde_json::Value>(confirmed_body)
        .expect("confirmation JSON")["payload"]["source_id"]
        .as_str()
        .expect("source id")
        .to_string();

    let secret = "BODY_MUST_NOT_LEAK";
    let mut malformed = vec![0xff];
    malformed.extend_from_slice(secret.as_bytes());
    std::fs::write(&memory, malformed).expect("malformed source");
    let scan_body = serde_json::json!({ "source_id": source_id }).to_string();
    let response = raw_http(
        port,
        &format!(
            "POST /api/scan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            scan_body.len(),
            scan_body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 409"), "got:\n{response}");
    assert!(
        response.contains("\"code\":\"scan_failed\""),
        "got:\n{response}"
    );
    assert!(response.contains("\"phase\":\"scan\""), "got:\n{response}");
    assert!(
        response.contains("\"source_id\":\"src_"),
        "got:\n{response}"
    );
    let source_path = source_root.path().to_string_lossy();
    assert!(
        !response.contains(source_path.as_ref()),
        "source path must not cross the wire: {response}"
    );
    assert!(
        !response.contains(secret),
        "source body must not cross the wire: {response}"
    );
}

/// Static-file path traversal can never escape the UI root (AD-4's allowlist
/// mindset applied to the one directory the server may expose).
#[test]
fn static_path_traversal_is_rejected() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        &format!(
            "GET /../server/Cargo.toml HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    // Either the client-side normalization is impossible (we send the raw
    // path) and the server rejects with 400, or it cannot find the file (404)
    // — but never a 200 with file content.
    assert!(!response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("bad_request") || response.contains("not_found"),
        "got:\n{response}"
    );
}

/// Story 2.1 AC — confirming a Claude Code candidate through the wire contract
/// reuses the Codex confirm pipeline (provider-neutral). The wire response
/// carries `provider="claude_code"`, the encoded project key as
/// `native_project`, Full coverage, and `lifecycle_state="confirmed"` — all
/// under the same `api_version="1"` envelope, with no Codex-specific framing.
#[test]
fn claude_code_candidate_confirms_over_wire_with_native_project() {
    let source_root = tempfile::tempdir().expect("source root");
    let memory = source_root.path().join("memory");
    std::fs::create_dir_all(&memory).expect("claude memory dir");
    std::fs::write(memory.join("MEMORY.md"), "# memory\nbody").expect("write memory");
    let port = boot_test_server();
    let confirm_body = serde_json::json!({
        "candidate": {
            "provider": "claude_code",
            "root_path": memory,
            "basis": "claude_default_home",
            "coverage_level": "full",
            "native_project": "encoded-project-key",
        }
    })
    .to_string();
    let confirmed = raw_http(
        port,
        &format!(
            "POST /api/sources/confirm HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            confirm_body.len(),
            confirm_body
        ),
    );
    assert!(confirmed.starts_with("HTTP/1.1 200"), "got:\n{confirmed}");
    assert!(confirmed.contains("\"api_version\":\"1\""), "got:\n{confirmed}");
    assert!(confirmed.contains("\"provider\":\"claude_code\""), "got:\n{confirmed}");
    assert!(
        confirmed.contains("\"native_project\":\"encoded-project-key\""),
        "got:\n{confirmed}"
    );
    assert!(confirmed.contains("\"lifecycle_state\":\"confirmed\""), "got:\n{confirmed}");
    assert!(confirmed.contains("\"coverage_level\":\"full\""), "got:\n{confirmed}");
    assert!(confirmed.contains("\"source_id\":\"src_"), "got:\n{confirmed}");
    // The encoded key is preserved verbatim — no reverse-mapping to a path.
    assert!(!confirmed.contains("encoded_project_key"), "got:\n{confirmed}");

    // Re-confirming is idempotent on the wire (same `source_id`).
    let reconfirmed = raw_http(
        port,
        &format!(
            "POST /api/sources/confirm HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            confirm_body.len(),
            confirm_body
        ),
    );
    let first_id = confirmed
        .split("source_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("");
    let second_id = reconfirmed
        .split("source_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap_or("");
    assert_eq!(first_id, second_id, "re-confirm returns the same source_id");
}

/// Story 2.2 AC — a rescan of a confirmed Claude Code source SUCCEEDS over
/// HTTP: the SSE terminal event is `succeeded` (NOT `failed`), the generation
/// activates, and the inventory reflects a healthy source with no
/// `latest_error`. Replaces the 2.1 test that pinned the now-removed
/// `provider_not_scannable` outcome. The query service returning the indexed
/// records is covered by `source_registry.rs` and the contract suite.
#[test]
fn rescan_claude_code_source_succeeds_and_activates_on_wire() {
    let source_root = tempfile::tempdir().expect("source root");
    let memory = source_root.path().join("memory");
    std::fs::create_dir_all(&memory).expect("claude memory dir");
    std::fs::write(memory.join("MEMORY.md"), "# memory\nbody").expect("write memory");
    std::fs::write(memory.join("topic.md"), "# topic\ntopic body").expect("write topic");
    let port = boot_test_server();
    let confirm_body = serde_json::json!({
        "candidate": {
            "provider": "claude_code",
            "root_path": memory,
            "basis": "claude_default_home",
            "coverage_level": "full",
            "native_project": "proj",
        }
    })
    .to_string();
    let confirmed = raw_http(
        port,
        &format!(
            "POST /api/sources/confirm HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            confirm_body.len(),
            confirm_body
        ),
    );
    let source_id = confirmed
        .split("source_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("source_id");

    let rescan_body = serde_json::json!({ "source_id": source_id }).to_string();
    let rescan = raw_http(
        port,
        &format!(
            "POST /api/sources/rescan HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            rescan_body.len(),
            rescan_body
        ),
    );
    assert!(rescan.starts_with("HTTP/1.1 200"), "got:\n{rescan}");
    let payload: serde_json::Value = serde_json::from_str(
        rescan.split("\r\n\r\n").nth(1).expect("rescan body"),
    )
    .expect("rescan json");
    let job_id = payload["payload"]["job_id"].as_str().expect("job id");

    // Drain SSE until the terminal event surfaces. Story 2.2: the rescan
    // SUCCEEDS (Claude is scannable). Poll budget is 200 × 25 ms (~5 s) to
    // tolerate a loaded CI runner scheduling the rescan worker thread late.
    let mut final_state = String::new();
    let mut final_message = String::new();
    for _ in 0..200 {
        let response = raw_http(
            port,
            &format!("GET /api/sources/rescan/events?source_id={source_id}&job_id={job_id}&after=0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
        );
        let body = response.split("\r\n\r\n").nth(1).unwrap_or("");
        let events: Vec<serde_json::Value> = body
            .split("\n\n")
            .filter_map(|block| {
                block.lines().find_map(|line| line.strip_prefix("data: "))
            })
            .filter_map(|json| serde_json::from_str::<serde_json::Value>(json).ok())
            .collect();
        if let Some(terminal) = events.last() {
            let state = terminal["state"].as_str().unwrap_or("");
            if matches!(state, "succeeded" | "failed" | "cancelled") {
                final_state = state.to_string();
                final_message =
                    terminal["message"].as_str().unwrap_or("").to_string();
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    assert_eq!(
        final_state, "succeeded",
        "claude rescan must succeed (Claude is scannable in 2.2); message: {final_message:?}"
    );
    assert!(
        !final_message.contains("not available yet"),
        "the removed provider_not_scannable message must NOT appear: {final_message:?}"
    );

    // Inventory: the Claude row is healthy with a non-zero record count and
    // no `latest_error` (a succeeded scan clears the error surface).
    let inventory = raw_http(
        port,
        &format!("GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(inventory.starts_with("HTTP/1.1 200"), "got:\n{inventory}");
    let inventory_json: serde_json::Value = serde_json::from_str(
        inventory.split("\r\n\r\n").nth(1).expect("inventory body"),
    )
    .expect("inventory json");
    let claude_row = inventory_json["payload"]
        .as_array()
        .expect("inventory array")
        .iter()
        .find(|row| row["source_id"].as_str() == Some(source_id))
        .expect("claude source row in inventory");
    assert_eq!(claude_row["health_state"].as_str(), Some("healthy"));
    let record_count = claude_row["complete_record_count"]
        .as_u64()
        .expect("record count");
    assert!(record_count > 0, "claude row has indexed records");
    assert!(
        claude_row.get("latest_error").map(|v| v.is_null()).unwrap_or(true),
        "healthy claude row has no latest_error: {:?}",
        claude_row["latest_error"]
    );
}

/// An unknown API route is a structured 404, not an empty socket or an HTML
/// error page (AD-13 envelope discipline everywhere).
#[test]
fn unknown_api_route_is_structured_404() {
    let port = boot_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/nope HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 404"), "got:\n{response}");
    assert!(response.contains("not_found"), "got:\n{response}");
}

/// Story 2.3 AC / FR-14 prototype over HTTP — with a healthy Codex source and
/// a confirmed Claude Code source whose latest scan Failed (no active
/// generation), a search for a shared keyword returns the healthy source's
/// results, the sidecar flags the failed source `unavailable`, and the query
/// does NOT fail (HTTP 200, no error envelope).
#[test]
fn search_sidecar_flags_mixed_availability_over_http() {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection lock");
        // Source 1: Codex, healthy, indexed with a matching record.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-codex', 'fp-codex', NULL)",
            [],
        )
        .expect("codex source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
            [],
        )
        .expect("codex scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("codex active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_codex', 1, 'gen_1', 'codex', 'section', 'rec_codex', 'file:///fixture#x', 'hash', 'v1', 'keyword match', 'body', NULL, 'memory', 'full', 100, 'revision', 'file:///fixture#L1-L2')",
            [],
        )
        .expect("codex record");
        // Source 2: Claude Code, confirmed but Failed scan, no active generation.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'error', 'full', '/fixture-claude', 'fp-claude', 'proj-claude')",
            [],
        )
        .expect("claude source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, error_code)
             VALUES (2, 'gen_2', 'failed', 1, 'gen_2', 'fixture', 'enumeration_failed')",
            [],
        )
        .expect("claude failed run");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));

    let response = raw_http(
        port,
        &format!("GET /api/search?q=keyword&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "query must not fail when one source is down: {response}");
    let body = response.split("\r\n\r\n").nth(1).expect("search body");
    let page: serde_json::Value = serde_json::from_str(body).expect("search JSON");
    // The healthy source's results return.
    let results = page["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "healthy source results must return: {results:?}");
    assert_eq!(results[0]["provider"].as_str(), Some("codex"));
    // The sidecar lists both sources with the correct statuses.
    let sources = page["payload"]["sources"].as_array().expect("sources sidecar");
    assert_eq!(sources.len(), 2, "sidecar must list both confirmed sources: {sources:?}");
    let codex = sources.iter().find(|s| s["provider"].as_str() == Some("codex")).expect("codex in sidecar");
    assert_eq!(codex["status"].as_str(), Some("available"));
    let claude = sources.iter().find(|s| s["provider"].as_str() == Some("claude_code")).expect("claude in sidecar");
    assert_eq!(claude["status"].as_str(), Some("unavailable"), "failed source must be flagged: {claude:?}");
    assert_eq!(claude["native_project"].as_str(), Some("proj-claude"));
    // No empty_state — the query succeeded partially.
    assert!(page["payload"]["empty_state"].is_null(), "partial unavailability must not produce an empty_state");
}

// ---------------------------------------------------------------------------
// Story 2.4 — cross-provider filter params on the /api/search wire contract
// ---------------------------------------------------------------------------

/// Boot a live loopback server with a Codex + Claude Code fixture for Story
/// 2.4 filter-param tests. Both sources confirmed + indexed with records
/// carrying the shared keyword "federation"; the two providers carry different
/// `native_project` (Codex NULL, Claude "proj-claude") and different
/// `provider_memory_type` (Codex "memory", Claude "topic_memory") so each
/// filter dimension has a discriminating wire-level fixture.
fn boot_filter_test_server() -> u16 {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection lock");
        // Source 1: Codex, NULL native_project.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-codex', 'fp-codex', NULL)",
            [],
        ).expect("codex source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
            [],
        ).expect("codex scan run");
        conn.execute("INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')", []).expect("codex active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_codex', 1, 'gen_1', 'codex', 'section', 'rec_codex', 'file:///fixture#x', 'hash', 'v1', 'federation patterns', 'body', NULL, 'memory', 'full', 100, 'revision', 'file:///fixture#L1-L2')",
            [],
        ).expect("codex record");
        // Source 2: Claude Code, proj-claude, topic_memory type.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-claude', 'fp-claude', 'proj-claude')",
            [],
        ).expect("claude source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (2, 'gen_2', 'succeeded', 1, 'gen_2', 'fixture')",
            [],
        ).expect("claude scan run");
        conn.execute("INSERT INTO tessera_meta(key, value) VALUES ('active_generation:2', 'gen_2')", []).expect("claude active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_claude', 2, 'gen_2', 'claude_code', 'section', 'rec_claude', 'file:///fixture#y', 'hash', 'v1', 'federation topic', 'body', 'proj-claude', 'topic_memory', 'full', 200, 'revision', 'file:///fixture#L3-L4')",
            [],
        ).expect("claude record");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    port
}

/// Story 2.4 AC — `/api/search?provider=codex` narrows the result set to Codex
/// records on the wire. The unfiltered query returns both providers; the
/// provider-filtered query returns only Codex.
#[test]
fn search_provider_filter_narrows_results_over_http() {
    let port = boot_filter_test_server();
    // Baseline: no filter → both providers.
    let baseline = raw_http(
        port,
        &format!("GET /api/search?q=federation&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(baseline.starts_with("HTTP/1.1 200"), "baseline: {baseline}");
    let baseline_body = baseline.split("\r\n\r\n").nth(1).expect("body");
    let baseline_json: serde_json::Value = serde_json::from_str(baseline_body).expect("json");
    let baseline_providers: std::collections::HashSet<String> = baseline_json["payload"]["results"]
        .as_array().expect("results array")
        .iter()
        .map(|value| value["provider"].as_str().expect("provider string").to_string())
        .collect();
    assert!(baseline_providers.contains("codex") && baseline_providers.contains("claude_code"),
        "baseline must include both providers: {baseline_providers:?}");

    // Filtered: provider=codex → only Codex.
    let filtered = raw_http(
        port,
        &format!("GET /api/search?q=federation&provider=codex&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(filtered.starts_with("HTTP/1.1 200"), "filtered: {filtered}");
    let filtered_body = filtered.split("\r\n\r\n").nth(1).expect("body");
    let filtered_json: serde_json::Value = serde_json::from_str(filtered_body).expect("json");
    let filtered_providers: std::collections::HashSet<String> = filtered_json["payload"]["results"]
        .as_array().expect("results array")
        .iter()
        .map(|value| value["provider"].as_str().expect("provider string").to_string())
        .collect();
    assert!(filtered_providers.contains("codex"), "codex must remain: {filtered_providers:?}");
    assert!(!filtered_providers.contains("claude_code"), "claude must be excluded: {filtered_providers:?}");
}

/// Story 2.4 AC (Spec Change Log 2026-07-25) — `/api/search?source=src_2`
/// narrows to that one specific source's records on the wire, distinct from the
/// coarser provider filter. The fixture has src_1 (codex) + src_2 (claude).
#[test]
fn search_source_filter_narrows_results_over_http() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&source=src_2&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let results = json["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "only the src_2 (claude) record must match: {results:?}");
    assert_eq!(results[0]["provider"].as_str(), Some("claude_code"));
    assert_eq!(results[0]["source_id"].as_str(), Some("src_2"));
    // A non-confirmed source id is accepted at the contract layer but yields no
    // rows (the SQL JOIN on lifecycle_state='confirmed' excludes it).
    let empty = raw_http(
        port,
        &format!("GET /api/search?q=federation&source=src_99&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(empty.starts_with("HTTP/1.1 200"), "non-confirmed source is not an error: {empty}");
    let empty_body = empty.split("\r\n\r\n").nth(1).expect("body");
    let empty_json: serde_json::Value = serde_json::from_str(empty_body).expect("json");
    assert_eq!(empty_json["payload"]["results"].as_array().expect("results").len(), 0, "src_99 yields no rows");
}

/// Story 2.4 I/O matrix — a malformed `source` handle → 400 `bad_request`.
#[test]
fn search_rejects_malformed_source_handle_with_bad_request() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&source=not-a-source&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
}

/// Story 2.4 AC — `/api/search?memory_type=topic_memory` narrows to the Claude
/// topic_memory record on the wire.
#[test]
fn search_memory_type_filter_narrows_results_over_http() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&memory_type=topic_memory&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let results = json["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "only the topic_memory record must match: {results:?}");
    assert_eq!(results[0]["record_id"].as_str(), Some("rec_claude"));
}

/// Story 2.4 AC — `/api/search?native_project=proj-claude` matches Claude's
/// record and excludes Codex's NULL native_project (SQL `NULL = 'x'` is NULL).
#[test]
fn search_native_project_filter_excludes_null_over_http() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&native_project=proj-claude&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let results = json["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "only the proj-claude record must match: {results:?}");
    assert_eq!(results[0]["provider"].as_str(), Some("claude_code"));
}

/// Story 2.4 AC — `/api/search?since=N` narrows by `observed_at >= N`. The
/// fixture has Codex at 100 and Claude at 200; `since=150` must exclude Codex.
#[test]
fn search_since_filter_narrows_results_over_http() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&since=150&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let results = json["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "only the observed_at>=150 record must match: {results:?}");
    assert_eq!(results[0]["provider"].as_str(), Some("claude_code"));
}

/// Story 2.4 I/O matrix — unknown `provider` value → 400 `bad_request`.
#[test]
fn search_rejects_unknown_provider_value_with_bad_request() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&provider=bogus_provider&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
}

/// Story 2.4 I/O matrix — unknown `memory_type` value → 400 `bad_request`.
#[test]
fn search_rejects_unknown_memory_type_value_with_bad_request() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&memory_type=bogus_type&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
}

/// Story 5.2 — `tessera_project` param (was reserved in 2.4) now narrows the
/// result set to records whose `(provider, native_project)` is in the
/// project's mapping scope set. Replaces the 2.4 "accepted and ignored" wire
/// test: the slot is live. The fixture's boot_filter_test_server has Codex
/// (NULL native_project) + Claude (proj-claude) confirmed sources; we create
/// a project + map the Codex global scope to it via the project HTTP API
/// (exercising the same code path users hit), then assert a project-scoped
/// search returns only Codex records and the sidecar narrows.
#[test]
fn search_tessera_project_param_narrows_results_over_http() {
    let port = boot_filter_test_server();

    // Create a project + add the (codex, null) mapping via the HTTP API.
    let create = post_json(port, "/api/projects/create", r#"{"name":"Codex-only"}"#);
    assert!(create.starts_with("HTTP/1.1 200"), "create project: {create}");
    let create_body = create.split("\r\n\r\n").nth(1).expect("body");
    let create_json: serde_json::Value = serde_json::from_str(create_body).expect("json");
    let project_id = create_json["payload"]["project_id"].as_str().expect("project_id");

    let add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{project_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(add.starts_with("HTTP/1.1 200"), "add mapping: {add}");

    // Project-scoped search → only Codex records match.
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&tessera_project={project_id}&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let results = json["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "tessera_project must narrow to Codex only: {results:?}");
    assert_eq!(results[0]["provider"].as_str(), Some("codex"));
    // Sidecar narrows to the project's mapped source (Codex only).
    let sources = json["payload"]["sources"].as_array().expect("sources array");
    assert_eq!(sources.len(), 1, "sidecar must narrow to the 1 mapped source: {sources:?}");
    assert_eq!(sources[0]["provider"].as_str(), Some("codex"));

    // Unknown project id over the wire → empty results, NOT a 400.
    let unknown = raw_http(
        port,
        &format!("GET /api/search?q=federation&tessera_project=proj_999&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(unknown.starts_with("HTTP/1.1 200"), "unknown project is not an error: {unknown}");
    let unknown_body = unknown.split("\r\n\r\n").nth(1).expect("body");
    let unknown_json: serde_json::Value = serde_json::from_str(unknown_body).expect("json");
    assert_eq!(
        unknown_json["payload"]["results"].as_array().expect("results").len(),
        0,
        "unknown project yields no rows",
    );
}

/// Story 2.4 AC — unknown query keys are still rejected (the filter params are
/// allowlisted, not arbitrary). This keeps the contract bounded.
#[test]
fn search_rejects_unknown_query_key_with_bad_request() {
    let port = boot_filter_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/search?q=federation&unknown_key=value&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
}

// ---------------------------------------------------------------------------
// Story 3.1 — /api/browse wire contract.
//
// Mirrors the search wire tests at the HTTP boundary: the versioned envelope
// crosses HTTP, the browse list reuses SearchResult rows, the cursor paginates
// stably, a generation change surfaces `cursor_stale` (409), non-confirmed /
// unknown sources surface `bad_request` (400, phase `browse`), and the three
// distinct empty states are present on page 1 only.
// ---------------------------------------------------------------------------

/// A live loopback server with a confirmed Codex source carrying three records
/// under one active generation, plus a confirmed-but-never-scanned source and
/// a disabled source (for the lifecycle-exclusion + bad-request wire tests).
fn boot_browse_test_server() -> (u16, Arc<tessera_lib::IndexState>) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    {
        let conn = state.conn.lock().expect("connection lock");
        // Source 1: confirmed Codex with three records (varied observed_at +
        // coverage so the browse ORDER BY is exercised on the wire).
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-codex', 'fp-codex', NULL)",
            [],
        )
        .expect("codex source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'fixture')",
            [],
        )
        .expect("codex scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .expect("codex active gen");
        for (id, observed_at, coverage) in
            [("rec_old", 100, "full"), ("rec_new", 300, "full"), ("rec_mid", 200, "search_only")]
        {
            conn.execute(
                "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
                 VALUES (?1, 1, 'gen_1', 'codex', 'section', ?1, 'file:///fixture#x', 'hash', 'v1', ?1, 'body', NULL, 'memory', ?2, ?3, 'revision', 'file:///fixture#L1-L2')",
                params![id, coverage, observed_at],
            )
            .expect("codex record");
        }
        // Story 3.2 — a `topic_memory` record under src_1 so the wire-level
        // memory-type filter has discriminating data (the three records above
        // are all `memory`). Newer than rec_new so it sorts first under the
        // filter (asserts the recency ORDER BY still holds within the filtered
        // set).
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_topic', 1, 'gen_1', 'codex', 'section', 'rec_topic', 'file:///fixture#topic', 'hash', 'v1', 'rec_topic', 'body', NULL, 'topic_memory', 'full', 400, 'revision', 'file:///fixture#L1-L2')",
            [],
        )
        .expect("codex topic record");
        // Source 2: confirmed Claude Code, never scanned — exercises the
        // `not_yet_scanned` empty state on the wire.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (2, 'claude_code', 'agent_memory', 'confirmed', 'unknown', 'full', '/fixture-claude', 'fp-claude', 'proj-claude')",
            [],
        )
        .expect("claude source");
        // Source 3: a DISABLED source with an active generation — exercises the
        // lifecycle-exclusion boundary: its records must NEVER appear in browse.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (3, 'codex', 'agent_memory', 'disabled', 'unknown', 'full', '/fixture-disabled', 'fp-disabled', NULL)",
            [],
        )
        .expect("disabled source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (3, 'gen_3', 'succeeded', 1, 'gen_3', 'fixture')",
            [],
        )
        .expect("disabled scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:3', 'gen_3')",
            [],
        )
        .expect("disabled active gen");
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
             VALUES ('rec_hidden_disabled', 3, 'gen_3', 'codex', 'section', 'rec_hidden_disabled', 'file:///fixture#hidden', 'hash', 'v1', 'hidden', 'body', NULL, 'memory', 'full', 999, 'revision', 'file:///fixture#L1-L2')",
            [],
        )
        .expect("disabled record");
        // Source 4: confirmed, active generation but ZERO records → exercises
        // the `no_indexable_memory` empty state on the wire.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (4, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/fixture-empty', 'fp-empty', NULL)",
            [],
        )
        .expect("empty source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (4, 'gen_4', 'succeeded', 1, 'gen_4', 'fixture')",
            [],
        )
        .expect("empty scan run");
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:4', 'gen_4')",
            [],
        )
        .expect("empty active gen");
        // Source 5: confirmed, latest run FAILED, no active generation →
        // exercises the `source_unavailable` empty state on the wire.
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (5, 'codex', 'agent_memory', 'confirmed', 'error', 'full', '/fixture-failed', 'fp-failed', NULL)",
            [],
        )
        .expect("failed source");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision, error_code)
             VALUES (5, 'gen_5', 'failed', 1, 'gen_5', 'fixture', 'enumeration_failed')",
            [],
        )
        .expect("failed scan run");
    }
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, state)
}

/// Story 3.1 AC — `/api/browse?source=src_<n>` returns a paginated list from
/// the confirmed source's active generation on the wire. The result rows
/// reuse SearchResult's shape verbatim; the cursor paginates stably; the
/// sidecar lists every confirmed source.
#[test]
fn browse_wire_contract_paginates_and_reuses_search_result_shape() {
    let (port, state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("browse body");
    let page: serde_json::Value = serde_json::from_str(body).expect("versioned browse JSON");
    assert_eq!(page["api_version"], "1");
    let results = page["payload"]["results"].as_array().expect("results array");
    assert_eq!(results.len(), 1, "limit=1 returned one row: {results:?}");
    // ORDER BY observed_at DESC → rec_topic (observed_at=400, the Story 3.2
    // fixture record) sorts first. (rec_new=300, rec_mid=200, rec_old=100.)
    assert_eq!(results[0]["record_id"], "rec_topic");
    // Provenance fields from SearchResult render on the wire.
    for field in [
        "record_id",
        "excerpt",
        "provider",
        "source_id",
        "native_locator",
        "display_locator",
        "observed_at",
        "coverage_level",
        "health_state",
    ] {
        assert!(!results[0][field].is_null(), "missing {field}: {results:?}");
    }
    // The per-confirmed-source sidecar is present on every page (mirrors
    // search). Sources 1 and 2 are confirmed; source 3 (disabled) is not.
    let sources = page["payload"]["sources"].as_array().expect("sources sidecar array");
    let source_ids: std::collections::HashSet<&str> = sources
        .iter()
        .map(|entry| entry["source_id"].as_str().expect("source_id"))
        .collect();
    assert!(source_ids.contains("src_1"), "src_1 in sidecar: {source_ids:?}");
    assert!(source_ids.contains("src_2"), "src_2 in sidecar: {source_ids:?}");
    assert!(!source_ids.contains("src_3"), "src_3 (disabled) must NOT be in sidecar: {source_ids:?}");

    let cursor = page["payload"]["next_cursor"].as_str().expect("continuation cursor");
    let continuation = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(continuation.starts_with("HTTP/1.1 200"), "got:\n{continuation}");
    // ORDER BY observed_at DESC → rec_new (300) sorts next. rec_mid (200) and
    // rec_old (100) are on later pages (pagination stability is covered by the
    // application-layer tests).
    assert!(continuation.contains("rec_new"), "got:\n{continuation}");

    // Stale cursor: activate a new generation under src_1 → revision changes.
    {
        let conn = state.conn.lock().expect("connection lock");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_99', 'succeeded', 2, 'gen_99', 'fixture')",
            [],
        )
        .expect("new scan run");
        conn.execute(
            "UPDATE tessera_meta SET value = 'gen_99' WHERE key = 'active_generation:1'",
            [],
        )
        .expect("activate new generation");
    }
    let stale = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(stale.starts_with("HTTP/1.1 409"), "got:\n{stale}");
    assert!(stale.contains("\"code\":\"cursor_stale\""), "got:\n{stale}");
    // cursor_stale now specializes the phase per endpoint (`browse` here) so
    // the UI can distinguish a browse pagination staleness. The safe message
    // must NOT carry any source id / record id / query detail.
    assert!(stale.contains("\"phase\":\"browse\""), "browse cursor_stale must carry phase=browse: {stale}");
    assert!(!stale.contains("rec_"), "cursor_stale must not carry record ids: {stale}");
}

/// Story 3.1 AC — `/api/browse?source=src_2` (confirmed but never scanned)
/// returns `empty_state = "not_yet_scanned"` on the wire.
#[test]
fn browse_wire_returns_not_yet_scanned_empty_state() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_2 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"empty_state\":\"not_yet_scanned\""),
        "got:\n{response}"
    );
}

/// Story 3.1 AC — `/api/browse?source=src_4` (confirmed, active generation,
/// zero records) returns `empty_state = "no_indexable_memory"` on the wire.
#[test]
fn browse_wire_returns_no_indexable_memory_empty_state() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_4 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"empty_state\":\"no_indexable_memory\""),
        "got:\n{response}"
    );
}

/// Story 3.1 AC — `/api/browse?source=src_5` (confirmed, latest run failed,
/// no active generation) returns `empty_state = "source_unavailable"` on the
/// wire.
#[test]
fn browse_wire_returns_source_unavailable_empty_state() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_5 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(
        response.contains("\"empty_state\":\"source_unavailable\""),
        "got:\n{response}"
    );
}

/// Story 3.1 AC — `/api/browse?source=src_3` (disabled) returns 400
/// `bad_request` (phase `browse`) and NEVER returns the disabled source's
/// records.
#[test]
fn browse_wire_rejects_disabled_source_with_bad_request() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_3 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
    // Phase is `browse` so the UI can distinguish the failure context without
    // parsing the display message.
    assert!(response.contains("\"phase\":\"browse\""), "got:\n{response}");
    // The disabled source's records must NEVER appear.
    assert!(!response.contains("rec_hidden_disabled"), "got:\n{response}");
}

/// Story 3.1 I/O matrix — `/api/browse?source=src_99` (unknown) returns 400
/// `bad_request` (phase `browse`).
#[test]
fn browse_wire_rejects_unknown_source_with_bad_request() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_99 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
    assert!(response.contains("\"phase\":\"browse\""), "got:\n{response}");
}

/// Story 3.1 I/O matrix — a missing `source` param, malformed source handle,
/// or invalid `limit` is rejected with 400 `bad_request` (phase `browse`).
#[test]
fn browse_wire_rejects_invalid_input() {
    let (port, _state) = boot_browse_test_server();
    // Missing source.
    let missing = raw_http(
        port,
        &format!("GET /api/browse?limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(missing.starts_with("HTTP/1.1 400"), "got:\n{missing}");
    assert!(missing.contains("\"code\":\"bad_request\""), "got:\n{missing}");
    // Malformed source handle.
    let malformed = raw_http(
        port,
        &format!("GET /api/browse?source=not-a-source HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(malformed.starts_with("HTTP/1.1 400"), "got:\n{malformed}");
    // Invalid limit (zero).
    let bad_limit = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&limit=0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(bad_limit.starts_with("HTTP/1.1 400"), "got:\n{bad_limit}");
    // Unknown query key.
    let unknown_key = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&unknown=value HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(unknown_key.starts_with("HTTP/1.1 400"), "got:\n{unknown_key}");
}

// ---------------------------------------------------------------------------
// Story 3.2 — `/api/browse?memory_type=...` filter on the wire contract.
//
// The filter narrows the browse WHERE with `AND m.provider_memory_type = ?`,
// mirroring Search's predicate shape. An invalid value → 400 (phase `browse`);
// the cursor binds the filter so a filter change mid-pagination → 409
// `cursor_stale`. The per-confirmed-source sidecar is unaffected by the filter
// (mirrors search's "sidecar stays unfiltered" guarantee).
// ---------------------------------------------------------------------------

/// Story 3.2 AC — `/api/browse?source=src_1&memory_type=topic_memory` narrows
/// the result set to the topic_memory record on the wire, the cursor binds the
/// filter (b4. envelope), and "Load more" stays within the filtered snapshot.
/// The sidecar still lists every confirmed source (the filter does not narrow
/// availability info).
#[test]
fn browse_wire_memory_type_filter_narrows_results() {
    let (port, _state) = boot_browse_test_server();
    // Baseline: no filter → all four records (rec_topic, rec_new, rec_mid,
    // rec_old).
    let baseline = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(baseline.starts_with("HTTP/1.1 200"), "baseline: {baseline}");
    let baseline_json: serde_json::Value =
        serde_json::from_str(baseline.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(
        baseline_json["payload"]["results"].as_array().expect("results").len(),
        4,
        "baseline has all four records",
    );

    // Filtered: memory_type=topic_memory → only rec_topic.
    let filtered = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=topic_memory&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(filtered.starts_with("HTTP/1.1 200"), "filtered: {filtered}");
    let filtered_json: serde_json::Value =
        serde_json::from_str(filtered.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    let results = filtered_json["payload"]["results"].as_array().expect("results");
    assert_eq!(results.len(), 1, "only the topic_memory record matches: {results:?}");
    assert_eq!(results[0]["record_id"].as_str(), Some("rec_topic"));

    // Filtered to memory → the three memory records, in observed_at DESC order.
    let memory_filter = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=memory&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(memory_filter.starts_with("HTTP/1.1 200"), "memory filter: {memory_filter}");
    let memory_json: serde_json::Value =
        serde_json::from_str(memory_filter.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    let memory_results = memory_json["payload"]["results"].as_array().expect("results");
    let memory_ids: Vec<&str> = memory_results
        .iter()
        .map(|value| value["record_id"].as_str().expect("record_id"))
        .collect();
    assert_eq!(memory_ids, vec!["rec_new", "rec_mid", "rec_old"], "memory records only, observed_at DESC: {memory_ids:?}");

    // The sidecar lists every confirmed source regardless of the filter
    // (Design Notes: sidecar stays unfiltered, mirrors Search 2.4).
    let sources = filtered_json["payload"]["sources"].as_array().expect("sidecar");
    let source_ids: std::collections::HashSet<&str> = sources
        .iter()
        .map(|entry| entry["source_id"].as_str().expect("source_id"))
        .collect();
    assert!(source_ids.contains("src_1"), "src_1 in filtered sidecar: {source_ids:?}");
    assert!(source_ids.contains("src_2"), "src_2 in filtered sidecar: {source_ids:?}");
}

/// Story 3.2 AC — `/api/browse?memory_type=memory` pagination: the cursor
/// binds the filter so "Load more" continues within the filtered snapshot,
/// never leaking records of other types in on page 2.
#[test]
fn browse_wire_memory_type_filter_paginates_within_filtered_snapshot() {
    let (port, _state) = boot_browse_test_server();
    // Page 1 (limit 1): one memory record + a continuation cursor.
    let page1 = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=memory&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(page1.starts_with("HTTP/1.1 200"), "page1: {page1}");
    let page1_json: serde_json::Value =
        serde_json::from_str(page1.split("\r\n\r\n").nth(1).expect("body")).expect("json");
    assert_eq!(page1_json["payload"]["results"].as_array().expect("results").len(), 1);
    // The b4. envelope prefix is on the wire.
    let cursor = page1_json["payload"]["next_cursor"]
        .as_str()
        .expect("continuation cursor");
    assert!(cursor.starts_with("b4."), "cursor must use the b4. envelope (post-3.2): {cursor}");

    // Page 2 (continuation): the next memory record; no topic_memory leak.
    let page2 = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=memory&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(page2.starts_with("HTTP/1.1 200"), "page2: {page2}");
    assert!(
        !page2.contains("rec_topic"),
        "page 2 under memory_type=memory must NOT contain the topic_memory record: {page2}",
    );
}

/// Story 3.2 AC — a cursor bound to one `memory_type` is rejected as
/// `cursor_stale` (409) when the next-page request carries a different
/// `memory_type` (mirrors Search's "resolve filter once on page 1" invariant).
#[test]
fn browse_wire_memory_type_filter_change_returns_cursor_stale() {
    let (port, _state) = boot_browse_test_server();
    // Issue a cursor under memory_type=memory (page 1, limit 1).
    let page1 = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=memory&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(page1.starts_with("HTTP/1.1 200"), "page1: {page1}");
    let cursor = page1
        .split("\"next_cursor\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("continuation cursor");

    // Replay that cursor under memory_type=topic_memory → 409 cursor_stale.
    let stale = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=topic_memory&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(stale.starts_with("HTTP/1.1 409"), "got:\n{stale}");
    assert!(stale.contains("\"code\":\"cursor_stale\""), "got:\n{stale}");
    assert!(stale.contains("\"phase\":\"browse\""), "phase=browse: {stale}");
}

/// Story 3.2 I/O matrix — an unknown `memory_type` value → 400 `bad_request`,
/// mirroring Search's invalid-memory-type behavior so the two surfaces share
/// one vocabulary. (The HTTP layer's strict-parse error carries
/// `phase:"transport"` because the rejection happens before the browse
/// handler runs — that matches Search's wire behavior for the same case.)
#[test]
fn browse_wire_rejects_unknown_memory_type_with_bad_request() {
    let (port, _state) = boot_browse_test_server();
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&memory_type=bogus_type&limit=20 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 400"), "got:\n{response}");
    assert!(response.contains("\"code\":\"bad_request\""), "got:\n{response}");
}

/// Story 3.2 — a 3.1-era `b3.` cursor is rejected as `cursor_stale` (409) at
/// the prefix check (the envelope moved to `b4.` so the filter could bind
/// in). The UI's existing `cursor_stale` recovery path re-runs page 1.
#[test]
fn browse_wire_rejects_legacy_b3_cursor_as_stale() {
    let (port, _state) = boot_browse_test_server();
    // A well-formed b3. envelope from a 3.1 client. The prefix check rejects
    // it as cursor_stale before any decode is attempted.
    let payload = r#"{"version":3,"source":"src_1","revision":"deadbeef","last_record_id":"rec_a","last_observed_at":0,"last_coverage_full":false}"#;
    let hex: String = payload.bytes().map(|byte| format!("{byte:02x}")).collect();
    let legacy_cursor = format!("b3.{hex}");
    let response = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&cursor={legacy_cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(response.starts_with("HTTP/1.1 409"), "got:\n{response}");
    assert!(response.contains("\"code\":\"cursor_stale\""), "got:\n{response}");
    assert!(response.contains("\"phase\":\"browse\""), "phase=browse: {response}");
}

// ===========================================================================
// Story 4.3 — POST /api/sources/rebind wire contract
// ===========================================================================
//
// Rebind is the explicit recovery path for a Confirmed Source whose root
// moved / lost permissions / changed filesystem identity. The wire contract:
// - Body: `{ "source_id": "src_<n>", "root_path": "<new-root>" }`.
// - 200 on success: envelope wraps the NEW Confirmed Source row (fresh
//   source_id at the new fingerprint, Unknown health, fresh cause=none).
// - 404 `source_not_found`: unknown old source_id (no state change).
// - 409 `confirm_failed`: old source not Confirmed, OR new root missing / not
//   a dir / not absolute. The 409 (NOT 400) follows the existing
//   `respond_result` envelope convention at server.rs — `confirm_failed` maps
//   to 409 alongside `scan_failed` and `cursor_stale`.

/// Boot a test server with one pre-confirmed Codex source and return
/// `(port, source_id)`. The source root's `MEMORY.md` is created inside the
/// helper; the root tempdir itself moves into the server thread (kept alive
/// for the server's lifetime there). Callers that need a NEW root for rebind
/// create their own tempdir — there is no need to hand back a caller-owned
/// root handle (P6: the previous signature returned a misleading third
/// element that was an UNRELATED empty tempdir created after the move).
fn boot_rebind_server() -> (u16, String) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let source_root = tempfile::tempdir().expect("source root");
    std::fs::write(source_root.path().join("MEMORY.md"), "# memory\nbody").expect("memory");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    let source_id = {
        let conn = state.conn.lock().expect("conn lock");
        let registry = SourceRegistry::new(&conn);
        let source = tessera_lib::application::confirm_source(&registry, &CandidateSource {
            provider: "codex".into(),
            root_path: source_root.path().to_string_lossy().into_owned(),
            basis: DiscoveryBasis::CodexHomeEnv,
            coverage_level: CoverageLevel::Full,
            native_project: None,
        })
        .expect("confirm");
        source.source_id.0
    };
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    std::thread::spawn(move || {
        let _dir = dir;
        let _source_root = source_root;
        serve_with(server, Arc::new(state), PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, source_id)
}

/// Story 4.3 AC — a successful rebind returns the new Confirmed Source on the
/// wire, and the OLD row appears Disabled via the inventory endpoint. The new
/// row carries a fresh `source_id` distinct from the old.
#[test]
fn rebind_returns_new_source_and_disables_old_on_the_wire() {
    let (port, old_source_id) = boot_rebind_server();
    let new_root = tempfile::tempdir().expect("new source root");
    std::fs::write(new_root.path().join("MEMORY.md"), "# moved\nbody").expect("new memory");

    let body = serde_json::json!({
        "source_id": old_source_id,
        "root_path": new_root.path().to_string_lossy(),
    })
    .to_string();
    let response = raw_http(
        port,
        &format!(
            "POST /api/sources/rebind HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("response body");
    let payload = serde_json::from_str::<serde_json::Value>(body).expect("envelope JSON");
    let new_source_id = payload["payload"]["source_id"].as_str().expect("new source_id");
    assert!(
        new_source_id.starts_with("src_"),
        "new source_id has src_ shape: {new_source_id}"
    );
    assert_ne!(
        new_source_id, old_source_id,
        "rebind produces a fresh source_id at the new fingerprint"
    );
    assert_eq!(payload["payload"]["lifecycle_state"], "confirmed");
    assert_eq!(payload["payload"]["health_state"], "unknown");

    // The OLD row is now Disabled on the wire (visible via inventory).
    let inv = raw_http(
        port,
        &format!("GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(inv.starts_with("HTTP/1.1 200"), "got:\n{inv}");
    let inv_body = inv.split("\r\n\r\n").nth(1).expect("inventory body");
    let inv_payload =
        serde_json::from_str::<serde_json::Value>(inv_body).expect("inventory JSON");
    let rows = inv_payload["payload"].as_array().expect("inventory rows");
    let old_row = rows
        .iter()
        .find(|row| row["source_id"].as_str() == Some(old_source_id.as_str()))
        .expect("old row present in inventory");
    assert_eq!(
        old_row["lifecycle_state"],
        "disabled",
        "old row is Disabled after rebind: {old_row}"
    );
    let new_row = rows
        .iter()
        .find(|row| row["source_id"].as_str() == Some(new_source_id))
        .expect("new row present in inventory");
    assert_eq!(new_row["lifecycle_state"], "confirmed");
    assert_eq!(new_row["health_state"], "unknown");
}

/// Story 4.3 AC — rebind with an unknown `source_id` returns a 404
/// `source_not_found` envelope (NOT 400 / NOT 409). Pins the envelope
/// convention at the wire surface.
#[test]
fn rebind_unknown_source_id_returns_404_source_not_found() {
    let (port, _old_source_id) = boot_rebind_server();
    let new_root = tempfile::tempdir().expect("new source root");
    let body = serde_json::json!({
        "source_id": "src_99999",
        "root_path": new_root.path().to_string_lossy(),
    })
    .to_string();
    let response = raw_http(
        port,
        &format!(
            "POST /api/sources/rebind HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(response.starts_with("HTTP/1.1 404"), "got:\n{response}");
    assert!(
        response.contains("\"code\":\"source_not_found\""),
        "got:\n{response}"
    );
}

/// Story 4.3 AC (amendment F2) — rebind to a missing new root returns 409
/// `confirm_failed` (NOT 400). `confirm_failed` maps to 409 per the existing
/// `respond_result` convention at server.rs alongside `scan_failed` and
/// `cursor_stale`.
#[test]
fn rebind_missing_new_root_returns_409_confirm_failed() {
    let (port, old_source_id) = boot_rebind_server();
    let body = serde_json::json!({
        "source_id": old_source_id,
        "root_path": "/this/does/not/exist/tessera-4-3-rebind",
    })
    .to_string();
    let response = raw_http(
        port,
        &format!(
            "POST /api/sources/rebind HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        ),
    );
    assert!(
        response.starts_with("HTTP/1.1 409"),
        "amendment F2: confirm_failed maps to 409, not 400 — got:\n{response}"
    );
    assert!(
        response.contains("\"code\":\"confirm_failed\""),
        "got:\n{response}"
    );
    // The source path the user supplied must NOT cross the wire (NFR-3).
    assert!(
        !response.contains("/this/does/not/exist"),
        "user-supplied path must not leak: {response}"
    );
}

// ===========================================================================
// Story 4.4 — POST /api/index/rebuild wire contract
// ===========================================================================
//
// Rebuild is the explicit recovery path for a corrupt Derived Index: it
// atomically wipes EXACTLY `memory_records` + `scan_runs` + `scan_diagnostics`
// + `tessera_meta` rows matching `active_generation:%` (preserving
// `source_registry`, `schema_version`, `tessera_migrations_applied`, and any
// other `tessera_meta` key), then re-scans every Confirmed Source by reusing
// the existing scan pipeline. The wire contract:
// - 200 on success: envelope wraps `RebuildOutcome { sources_rescanning }`.
// - 409 `rebuild_failed`: any source has an in-flight scan (race guard).
// - 500 `internal`: wipe / DB failure.

/// Boot a test server with one pre-confirmed + scanned Codex source and
/// return `(port, source_id, db_path)`. The source root is kept alive for the
/// server's lifetime. The `db_path` is returned so individual tests can open
/// their OWN connection (SQLite handles multi-connection fine) to seed state
/// the live server does not expose via HTTP — e.g. injecting a non-terminal
/// `scan_runs` row to deterministically trigger the rebuild race guard
/// (Patch E). Used by the rebuild wire tests.
fn boot_rebuild_server() -> (u16, String, PathBuf) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let source_root = tempfile::tempdir().expect("source root");
    std::fs::write(source_root.path().join("MEMORY.md"), "# memory\nbody").expect("memory");
    let state = tessera_lib::boot(dir.path()).expect("boot");
    let db_path = state.db_path.clone();
    let source_id = {
        let conn = state.conn.lock().expect("conn lock");
        let registry = SourceRegistry::new(&conn);
        let source = tessera_lib::application::confirm_source(&registry, &CandidateSource {
            provider: "codex".into(),
            root_path: source_root.path().to_string_lossy().into_owned(),
            basis: DiscoveryBasis::CodexHomeEnv,
            coverage_level: CoverageLevel::Full,
            native_project: None,
        })
        .expect("confirm");
        tessera_lib::application::scan_source(&registry, &conn, &source.source_id)
            .expect("initial scan");
        source.source_id.0
    };
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    std::thread::spawn(move || {
        let _dir = dir;
        let _source_root = source_root;
        serve_with(server, Arc::new(state), PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, source_id, db_path)
}

/// Story 4.4 AC — a successful rebuild returns `200` with a
/// `RebuildOutcome` envelope naming the number of Confirmed Sources being
/// re-scanned. The wipe is observable via `/api/inventory` once the
/// (synchronous, in this single-source case) per-source re-scan settles.
#[test]
fn rebuild_returns_200_with_sources_rescanning_count_on_the_wire() {
    let (port, _source_id, _db_path) = boot_rebuild_server();

    let response = raw_http(
        port,
        &format!(
            "POST /api/index/rebuild HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    let body = response.split("\r\n\r\n").nth(1).expect("response body");
    let payload = serde_json::from_str::<serde_json::Value>(body).expect("envelope JSON");
    assert_eq!(payload["api_version"], "1");
    // Exactly one Confirmed source → one re-scan dispatched.
    assert_eq!(payload["payload"]["sources_rescanning"], 1);
    // AD-13/NFR-3: no credential leak in the safe envelope. (The payload is a
    // pure count; no source path / query text / memory body crosses the wire.)
    assert!(!body.to_lowercase().contains("credential"));
}

/// Story 4.4 AC (Patch E — deterministic 409). Rebuild while ANY source has
/// an in-flight scan run MUST return 409 `rebuild_failed` with a redacted
/// safe message. The race guard is pinned DETERMINISTICALLY by seeding a
/// non-terminal `scan_runs` row directly via a separate SQLite connection
/// (SQLite handles multi-connection fine); the row is `queued` so the
/// rebuild's `any_in_flight_run` race guard MUST fire. This drops the
/// 200-acceptance branch the previous version had (the race-loss branch is
/// not part of the contract — the race guard fires whenever the row exists).
#[test]
fn rebuild_returns_409_rebuild_failed_when_a_scan_is_in_flight() {
    let (port, source_id, db_path) = boot_rebuild_server();
    let source_rowid: i64 = source_id
        .strip_prefix("src_")
        .and_then(|s| s.parse::<i64>().ok())
        .expect("source_id rowid");

    // Seed a non-terminal `queued` run via a SEPARATE connection. The rebuild
    // handler will observe it (the state's own connection reads the same
    // SQLite file) and reject with 409. The row stays `queued` — the rebuild
    // never runs, so nothing transitions it.
    {
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        conn.execute_batch("PRAGMA foreign_keys = ON;").expect("pragma");
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (?1, 'gen_inflight', 'queued', 999, 'gen_inflight', 'inflight')",
            params![source_rowid],
        )
        .expect("seed queued scan_runs row");
    }

    let response = raw_http(
        port,
        &format!(
            "POST /api/index/rebuild HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
    );
    // Patch E — deterministic: the race guard MUST fire (no 200-acceptance
    // branch). Exactly 409 + `rebuild_failed` code + safe message.
    assert!(
        response.starts_with("HTTP/1.1 409"),
        "Patch E: race guard MUST fire deterministically; got:\n{response}"
    );
    assert!(
        response.contains("\"code\":\"rebuild_failed\""),
        "got:\n{response}"
    );
    // The message is the safe, redacted one — no source path / body / query
    // / credential leak (AD-13/NFR-3).
    assert!(!response.contains("/fixture"), "no path leak: {response}");
    assert!(!response.to_lowercase().contains("credential"));
}

/// Story 4.4 AC (Patch G) — end-to-end rebuild via the LIVE server: boot
/// with one pre-confirmed + pre-scanned source, POST /api/index/rebuild, then
/// poll `GET /api/sources/inventory` until the source's record count is
/// restored to its pre-rebuild value. This pins AC#1 (each Confirmed source
/// re-scanned to a fresh generation) at the CONSUMPTION surface — a dispatch
/// bug (wrong scan_id / fencing_token / connection, dropped/panicked worker)
/// would leave the inventory with a zero count forever, and this test would
/// time out instead of silently passing.
#[test]
fn rebuild_restores_records_via_live_inventory_poll() {
    let (port, source_id, _db_path) = boot_rebuild_server();

    // Snapshot the pre-rebuild record count via the inventory endpoint so we
    // know what value to wait for after the rebuild's re-scan settles.
    let pre_count = {
        let inv = raw_http(
            port,
            &format!("GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
        );
        let body = inv.split("\r\n\r\n").nth(1).expect("inv body");
        let payload = serde_json::from_str::<serde_json::Value>(body).expect("inv JSON");
        let row = payload["payload"]
            .as_array()
            .expect("inv rows")
            .iter()
            .find(|row| row["source_id"].as_str() == Some(source_id.as_str()))
            .expect("source row in inv");
        row["complete_record_count"]
            .as_i64()
            .expect("count is a number")
    };
    assert!(
        pre_count > 0,
        "fixture: pre-rebuild source has indexed records ({pre_count})"
    );

    // POST the rebuild — returns immediately after wipe + dispatch.
    let response = raw_http(
        port,
        &format!(
            "POST /api/index/rebuild HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");

    // Poll inventory until the source's record count is restored. Bounded
    // retry so a runaway dispatch surfaces as a test failure rather than a
    // hang. The Codex scan is fast (one-file fixture); 5 seconds is plenty.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut restored = false;
    while std::time::Instant::now() < deadline {
        let inv = raw_http(
            port,
            &format!("GET /api/sources/inventory HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
        );
        let body = inv.split("\r\n\r\n").nth(1).expect("inv body");
        let payload = serde_json::from_str::<serde_json::Value>(body).expect("inv JSON");
        if let Some(row) = payload["payload"]
            .as_array()
            .expect("inv rows")
            .iter()
            .find(|row| row["source_id"].as_str() == Some(source_id.as_str()))
        {
            if row["complete_record_count"].as_i64() == Some(pre_count) {
                restored = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    assert!(
        restored,
        "Patch G: rebuild did not restore records via the live inventory within 5s (dispatch bug?)"
    );
}

/// Story 4.4 AC (Patch F) — per-source progress flows through the existing
/// `rescan_jobs` map + `GET /api/sources/rescan/events` SSE channel during a
/// rebuild (Boundaries Always #5). Pins the queued→running→(succeeded|failed)
/// progression at the wire level so a regression that bypasses
/// `rescan_jobs` (e.g. spawning workers that emit progress somewhere else)
/// would surface here.
#[test]
fn rebuild_per_source_progress_surfaces_via_rescan_events_sse() {
    let (port, source_id, db_path) = boot_rebuild_server();

    // POST the rebuild — returns immediately after wipe + dispatch.
    let response = raw_http(
        port,
        &format!(
            "POST /api/index/rebuild HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        ),
    );
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");

    // The job_id scheme is `job_<scan_id>` (mirrors start_rescan). We don't
    // know the scan_id up front (AUTOINCREMENT after the wipe). Read it from
    // scan_runs via a separate connection (the rebuild's reserved run is the
    // highest id for this source because the wipe cleared every prior row).
    let source_rowid: i64 = source_id
        .strip_prefix("src_")
        .and_then(|s| s.parse::<i64>().ok())
        .expect("source_id rowid");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let job_id = loop {
        if std::time::Instant::now() > deadline {
            panic!("Patch F: rebuild did not reserve a scan_run within 2s");
        }
        let conn = rusqlite::Connection::open(&db_path).expect("open db");
        if let Ok(latest_scan_id) = conn.query_row::<i64, _, _>(
            "SELECT id FROM scan_runs WHERE source_id = ?1 ORDER BY id DESC LIMIT 1",
            params![source_rowid],
            |row| row.get(0),
        ) {
            let _ = conn;
            break format!("job_{latest_scan_id}");
        }
        let _ = conn;
        std::thread::sleep(std::time::Duration::from_millis(50));
    };

    // Poll the SSE channel until we see at least queued + (running OR
    // terminal). Bounded retry so a runaway dispatch surfaces as a test
    // failure rather than a hang. The Codex scan is fast (one-file fixture).
    let inner_deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    let mut seen_queued = false;
    let mut seen_running = false;
    let mut seen_terminal = false;
    while std::time::Instant::now() < inner_deadline {
        let sse = raw_http(
            port,
            &format!(
                "GET /api/sources/rescan/events?source_id={source_id}&job_id={job_id}&after=0 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
            ),
        );
        if sse.starts_with("HTTP/1.1 200") {
            if sse.contains("\"state\":\"queued\"") {
                seen_queued = true;
            }
            if sse.contains("\"state\":\"running\"") {
                seen_running = true;
            }
            if sse.contains("\"state\":\"succeeded\"")
                || sse.contains("\"state\":\"failed\"")
                || sse.contains("\"state\":\"cancelled\"")
            {
                seen_terminal = true;
            }
            if seen_queued && (seen_running || seen_terminal) {
                // Progression pinned: queued + at least one later event.
                return;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!(
        "Patch F: SSE did not reach queued->running/terminal within 3s (queued={}, running={}, terminal={})",
        seen_queued, seen_running, seen_terminal
    );
}


