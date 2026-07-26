//! Wire-level integration tests for the Story 5.1 Tessera Project mapping
//! surface (`/api/projects/*`). Mirrors the existing `http_api.rs` shape: boot
//! a real loopback server on an ephemeral port and assert the versioned
//! envelope crosses HTTP end-to-end.
//!
//! These tests pin the AC's wire-level behavior:
//! - migration applies + `schema_version == "7"` post-boot;
//! - create / list / rename / delete round-trip the versioned envelope;
//! - add-mapping cardinality conflict surfaces 409 `mapping_conflict` naming
//!   the owning project and creates no row;
//! - idempotent re-add returns the unchanged view with exactly one mapping;
//! - Codex `(codex, null)` scope is unique across projects;
//! - unknown provider / invalid name surface 400 `bad_request`;
//! - non-destruction: a flurry of project ops leaves `source_registry` and
//!   `memory_records` counts unchanged (the I/O matrix's non-destruction AC).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;

use tessera_lib::http::server::{bind, serve_with};

/// Boot a real server on an ephemeral loopback port with a scratch app-data
/// dir and return `(port, state)` so the test can inspect SQLite directly
/// for the non-destruction gate.
fn boot_projects_server() -> (u16, Arc<tessera_lib::IndexState>) {
    let dir = tempfile::tempdir().expect("scratch app-data dir");
    let state = tessera_lib::boot(dir.path()).expect("boot must succeed on scratch dir");
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound addr").port();
    let state = Arc::new(state);
    let server_state = Arc::clone(&state);
    std::thread::spawn(move || {
        // Keep the tempdir alive for the server's lifetime.
        let _dir = dir;
        serve_with(server, server_state, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    (port, state)
}

/// Send one raw HTTP/1.1 request and read the full response text (mirrors
/// `http_api.rs::raw_http`).
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

/// POST a JSON body and return the full response text.
fn post_json(port: u16, path: &str, body: &str) -> String {
    raw_http(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

/// GET a path and return the full response text.
fn get(port: u16, path: &str) -> String {
    raw_http(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    )
}

#[test]
fn schema_version_is_seven_after_boot() {
    let (_port, state) = boot_projects_server();
    let conn = state.conn.lock().expect("conn lock");
    let v: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    assert_eq!(v, "7");
    // The two new tables exist and have the expected STRICT shape (a CREATE
    // TABLE IF NOT EXISTS that finds them already present is a no-op).
    let tables: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('tessera_projects', 'project_mappings')",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(tables, 2, "both project tables exist");
    // The scope uniqueness index exists.
    let idx: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'project_mappings_scope_unique'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(idx, 1, "scope uniqueness index exists");
}

#[test]
fn create_returns_versioned_envelope_with_empty_mappings() {
    let (port, _state) = boot_projects_server();
    let body = r#"{"name":"A"}"#;
    let response = post_json(port, "/api/projects/create", body);
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(response.contains("\"api_version\":\"1\""));
    // project_id matches ^proj_\d+$.
    assert!(response.contains("\"project_id\":\"proj_"));
    assert!(response.contains("\"name\":\"A\""));
    assert!(response.contains("\"mappings\":[]"));
}

#[test]
fn list_returns_empty_envelope_when_no_projects() {
    let (port, _state) = boot_projects_server();
    let response = get(port, "/api/projects");
    assert!(response.starts_with("HTTP/1.1 200"), "got:\n{response}");
    assert!(response.contains("\"api_version\":\"1\""));
    assert!(response.contains("\"payload\":[]"));
}

#[test]
fn rename_advances_updated_at_and_404_for_unknown_id() {
    let (port, _state) = boot_projects_server();
    let created = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    // Extract project_id from the create response — it appears as
    // "project_id":"proj_<n>".
    let pid = created
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id in create response")
        .to_string();

    // Sleep so unix_seconds_now advances past created_at.
    std::thread::sleep(std::time::Duration::from_secs(1));
    let renamed = post_json(
        port,
        "/api/projects/rename",
        &format!(r#"{{"project_id":"{pid}","name":"B"}}"#),
    );
    assert!(renamed.starts_with("HTTP/1.1 200"), "got:\n{renamed}");
    assert!(renamed.contains("\"name\":\"B\""));
    // updated_at strictly greater than created_at.
    let updated_at: i64 = renamed
        .split("\"updated_at\":")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit() && c != '-').next())
        .expect("updated_at")
        .parse()
        .expect("i64");
    let created_at: i64 = renamed
        .split("\"created_at\":")
        .nth(1)
        .and_then(|s| s.split(|c: char| !c.is_ascii_digit() && c != '-').next())
        .expect("created_at")
        .parse()
        .expect("i64");
    assert!(updated_at > created_at, "updated_at must advance");

    // Unknown id → 404 project_not_found.
    let unknown = post_json(
        port,
        "/api/projects/rename",
        r#"{"project_id":"proj_99999","name":"X"}"#,
    );
    assert!(
        unknown.starts_with("HTTP/1.1 404"),
        "unknown project_id should 404; got:\n{unknown}"
    );
    assert!(unknown.contains("\"code\":\"project_not_found\""));
}

#[test]
fn add_mapping_cardinality_conflict_and_idempotent_re_add() {
    let (port, _state) = boot_projects_server();
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let b_resp = post_json(port, "/api/projects/create", r#"{"name":"B"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    let b_id = b_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();

    // Add (claude_code, "<key>") to A — happy path.
    let key = "<key>";
    let a_add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(
            r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"{key}"}}"#
        ),
    );
    assert!(a_add.starts_with("HTTP/1.1 200"), "got:\n{a_add}");
    assert!(a_add.contains("\"provider\":\"claude_code\""));
    assert!(a_add.contains("\"native_project\":\"<key>\""));

    // Add the same scope to B — 409 mapping_conflict naming A.
    let b_add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(
            r#"{{"project_id":"{b_id}","provider":"claude_code","native_project":"{key}"}}"#
        ),
    );
    assert!(
        b_add.starts_with("HTTP/1.1 409"),
        "cardinality conflict should 409; got:\n{b_add}"
    );
    assert!(b_add.contains("\"code\":\"mapping_conflict\""));
    // The owning project name "A" surfaces in the safe message (NFR-3: same
    // posture as the Source Inventory).
    assert!(b_add.contains("A"));

    // GET /api/projects shows <key> mapped only to A; B has no mappings.
    let listed = get(port, "/api/projects");
    assert!(listed.contains("\"name\":\"A\""));
    assert!(listed.contains("\"native_project\":\"<key>\""));
    // Prove NO row was created for B by observing B's mapping count directly:
    // deleting B reports removed_mappings == 0 (a stronger check than
    // substring-matching the list payload — it cannot pass if B erroneously
    // carried <key>).
    let b_delete = post_json(
        port,
        "/api/projects/delete",
        &format!(r#"{{"project_id":"{b_id}"}}"#),
    );
    assert!(b_delete.starts_with("HTTP/1.1 200"), "got:\n{b_delete}");
    assert!(b_delete.contains("\"removed_mappings\":0"));

    // Idempotent re-add to A — same scope already owned by A returns the
    // unchanged view with exactly one entry for <key>. The response should
    // 200 and contain exactly one mapping (no duplicate row).
    let a_again = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(
            r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"{key}"}}"#
        ),
    );
    assert!(a_again.starts_with("HTTP/1.1 200"), "got:\n{a_again}");
    // Count occurrences of "<key>" in the response payload — exactly 1.
    let key_count = a_again.matches("<key>").count();
    assert_eq!(key_count, 1, "idempotent re-add must not duplicate");
}

#[test]
fn codex_null_scope_is_unique_across_projects() {
    let (port, _state) = boot_projects_server();
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let b_resp = post_json(port, "/api/projects/create", r#"{"name":"B"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    let b_id = b_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();

    // (codex, null) on A — happy.
    let a_add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(a_add.starts_with("HTTP/1.1 200"), "got:\n{a_add}");

    // (codex, null) on B — 409 mapping_conflict (AD-27 with NULL collapsed).
    let b_add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{b_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(
        b_add.starts_with("HTTP/1.1 409"),
        "codex-null uniqueness should 409; got:\n{b_add}"
    );
    assert!(b_add.contains("\"code\":\"mapping_conflict\""));
}

#[test]
fn remove_mapping_distinguishes_missing_project_and_missing_mapping() {
    let (port, _state) = boot_projects_server();
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );

    // Unknown project → 404 project_not_found.
    let unknown = post_json(
        port,
        "/api/projects/mappings/remove",
        r#"{"project_id":"proj_99999","provider":"codex","native_project":null}"#,
    );
    assert!(unknown.starts_with("HTTP/1.1 404"));
    assert!(unknown.contains("\"code\":\"project_not_found\""));

    // Existing project, missing mapping → 404 mapping_not_found.
    let missing = post_json(
        port,
        "/api/projects/mappings/remove",
        &format!(
            r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"not-mapped"}}"#
        ),
    );
    assert!(missing.starts_with("HTTP/1.1 404"));
    assert!(missing.contains("\"code\":\"mapping_not_found\""));

    // Existing mapping → removed; the response view no longer carries it.
    let removed = post_json(
        port,
        "/api/projects/mappings/remove",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(removed.starts_with("HTTP/1.1 200"));
    assert!(removed.contains("\"mappings\":[]"));
}

#[test]
fn delete_cascades_mappings_and_reports_count() {
    let (port, _state) = boot_projects_server();
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    // Add 3 mappings.
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"p1"}}"#),
    );
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"p2"}}"#),
    );

    let outcome = post_json(
        port,
        "/api/projects/delete",
        &format!(r#"{{"project_id":"{a_id}"}}"#),
    );
    assert!(outcome.starts_with("HTTP/1.1 200"), "got:\n{outcome}");
    assert!(outcome.contains("\"removed_mappings\":3"));

    // GET /api/projects no longer lists A.
    let listed = get(port, "/api/projects");
    assert!(listed.contains("\"payload\":[]"));
}

#[test]
fn unknown_provider_and_invalid_name_are_bad_request() {
    let (port, _state) = boot_projects_server();
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();

    // Unknown provider on add_mapping.
    let unknown = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(
            r#"{{"project_id":"{a_id}","provider":"not_a_provider","native_project":null}}"#
        ),
    );
    assert!(unknown.starts_with("HTTP/1.1 400"));
    assert!(unknown.contains("\"code\":\"bad_request\""));
    assert!(unknown.contains("\"phase\":\"project\""));

    // Empty name on create — 400 bad_request phase project (status + stable
    // code + phase, mirroring the unknown-provider branch above).
    let empty = post_json(port, "/api/projects/create", r#"{"name":"   "}"#);
    assert!(empty.starts_with("HTTP/1.1 400"), "got:\n{empty}");
    assert!(empty.contains("\"code\":\"bad_request\""));
    assert!(empty.contains("\"phase\":\"project\""));
}

#[test]
fn project_ops_never_modify_source_registry_or_memory_records() {
    let (port, state) = boot_projects_server();
    // Seed a source_registry + memory_records row directly so the
    // non-destruction gate has something to compare against.
    {
        let conn = state.conn.lock().expect("conn lock");
        conn.execute(
            "INSERT INTO source_registry (provider, source_kind, lifecycle_state, \
             health_state, coverage_level, normalized_root_path, fingerprint, \
             native_project, health_cause) VALUES \
             ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/x', \
             'fp-1', NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, \
             unit_kind, native_unit_id, native_locator, content_hash, parser_version, \
             title, body, native_project, provider_memory_type, coverage_level, \
             observed_at, source_revision, display_locator) VALUES \
             ('rec_1', 1, 'gen_1', 'codex', 'memory', 'u1', 'loc', 'hash', \
             'file-level/v1', 't', 'b', NULL, 'memory', 'full', 0, 'r1', 'd')",
            [],
        )
        .unwrap();
    }
    let sources_before: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
            .unwrap()
    };
    let records_before: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
            .unwrap()
    };
    let scan_runs_before: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
            .unwrap()
    };
    let scan_diagnostics_before: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM scan_diagnostics", [], |row| row.get(0))
            .unwrap()
    };
    let inventory_before = get(port, "/api/sources/inventory");

    // Flurry: create / rename / add / remove / delete via the HTTP surface.
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let b_resp = post_json(port, "/api/projects/create", r#"{"name":"B"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    let b_id = b_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    post_json(
        port,
        "/api/projects/rename",
        &format!(r#"{{"project_id":"{a_id}","name":"A2"}}"#),
    );
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(
            r#"{{"project_id":"{b_id}","provider":"claude_code","native_project":"proj"}}"#
        ),
    );
    post_json(
        port,
        "/api/projects/mappings/remove",
        &format!(
            r#"{{"project_id":"{b_id}","provider":"claude_code","native_project":"proj"}}"#
        ),
    );
    post_json(
        port,
        "/api/projects/delete",
        &format!(r#"{{"project_id":"{a_id}"}}"#),
    );

    // Counts UNCHANGED — project ops never delete or modify canonical records
    // or sources (non-destruction AC).
    let sources_after: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM source_registry", [], |row| row.get(0))
            .unwrap()
    };
    let records_after: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM memory_records", [], |row| row.get(0))
            .unwrap()
    };
    let scan_runs_after: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM scan_runs", [], |row| row.get(0))
            .unwrap()
    };
    let scan_diagnostics_after: i64 = {
        let conn = state.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM scan_diagnostics", [], |row| row.get(0))
            .unwrap()
    };
    let inventory_after = get(port, "/api/sources/inventory");
    // Counts UNCHANGED across every Derived-Index table — project ops never
    // delete or modify canonical records or sources (non-destruction AC).
    assert_eq!(sources_before, sources_after);
    assert_eq!(records_before, records_after);
    assert_eq!(scan_runs_before, scan_runs_after);
    assert_eq!(scan_diagnostics_before, scan_diagnostics_after);
    // The Source Inventory HTTP surface is byte-identical before/after the
    // flurry — health/coverage/native_project untouched.
    assert_eq!(inventory_before, inventory_after);
}

#[test]
fn add_mapping_returns_404_for_unknown_project() {
    let (port, _state) = boot_projects_server();
    let unknown = post_json(
        port,
        "/api/projects/mappings/add",
        r#"{"project_id":"proj_99999","provider":"codex","native_project":null}"#,
    );
    assert!(unknown.starts_with("HTTP/1.1 404"), "got:\n{unknown}");
    assert!(unknown.contains("\"code\":\"project_not_found\""));
}

#[test]
fn list_returns_projects_ordered_by_id_ascending() {
    let (port, _state) = boot_projects_server();
    let a = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let b = post_json(port, "/api/projects/create", r#"{"name":"B"}"#);
    let c = post_json(port, "/api/projects/create", r#"{"name":"C"}"#);
    let pid_of = |resp: &str| -> u64 {
        resp.split("\"project_id\":\"proj_")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap()
            .parse()
            .unwrap()
    };
    assert!(pid_of(&a) < pid_of(&b));
    assert!(pid_of(&b) < pid_of(&c));
    // GET /api/projects serializes the list in id-ascending order.
    let listed = get(port, "/api/projects");
    let pos_of = |id: u64| listed.find(&format!("\"project_id\":\"proj_{id}\"")).unwrap();
    assert!(pos_of(pid_of(&a)) < pos_of(pid_of(&b)));
    assert!(pos_of(pid_of(&b)) < pos_of(pid_of(&c)));
}

#[test]
fn one_project_holds_mappings_from_both_providers() {
    let (port, _state) = boot_projects_server();
    let a = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let a_id = a
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();
    // Associate Codex (global, null) and Claude Code ("<key>") to the SAME
    // project — the cross-Agent federation the story intent names.
    let add_codex = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(add_codex.starts_with("HTTP/1.1 200"), "got:\n{add_codex}");
    let add_claude = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"<key>"}}"#),
    );
    assert!(add_claude.starts_with("HTTP/1.1 200"), "got:\n{add_claude}");
    // The project view carries BOTH providers' native projects.
    let listed = get(port, "/api/projects");
    assert!(listed.contains("\"provider\":\"codex\""));
    assert!(listed.contains("\"provider\":\"claude_code\""));
    assert!(listed.contains("\"native_project\":null"));
    assert!(listed.contains("\"native_project\":\"<key>\""));
}

#[test]
fn shape_validation_branches_rejected_at_wire_level() {
    let (port, _state) = boot_projects_server();
    let a = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    let a_id = a
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .unwrap()
        .to_string();

    let assert_project_400 = |label: &str, body: String| {
        let resp = post_json(port, "/api/projects/mappings/add", &body);
        assert!(resp.starts_with("HTTP/1.1 400"), "{label}: got {resp}");
        assert!(
            resp.contains("\"code\":\"bad_request\""),
            "{label}: missing stable code"
        );
        assert!(
            resp.contains("\"phase\":\"project\""),
            "{label}: missing phase"
        );
    };

    // claude_code with empty / whitespace native_project (would collide with
    // the codex null scope under COALESCE).
    assert_project_400(
        "claude empty np",
        format!(r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":""}}"#),
    );
    assert_project_400(
        "claude whitespace np",
        format!(r#"{{"project_id":"{a_id}","provider":"claude_code","native_project":"  "}}"#),
    );
    // codex with a non-null native_project (Codex has no project key).
    assert_project_400(
        "codex non-null np",
        format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":"something"}}"#),
    );

    // create with an over-length name (>128 bytes).
    let long_name = "x".repeat(129);
    let over = post_json(
        port,
        "/api/projects/create",
        &format!(r#"{{"name":"{long_name}"}}"#),
    );
    assert!(over.starts_with("HTTP/1.1 400"), "over-length name: got {over}");
    assert!(over.contains("\"code\":\"bad_request\""));
    assert!(over.contains("\"phase\":\"project\""));
}
