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
