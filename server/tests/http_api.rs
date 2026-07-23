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
use std::path::PathBuf;
use std::sync::Arc;

use tessera_lib::http::server::{bind, serve_with};

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
