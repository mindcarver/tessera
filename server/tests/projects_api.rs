//! Wire-level integration tests for the Story 5.1 Tessera Project mapping
//! surface (`/api/projects/*`). Mirrors the existing `http_api.rs` shape: boot
//! a real loopback server on an ephemeral port and assert the versioned
//! envelope crosses HTTP end-to-end.
//!
//! These tests pin the AC's wire-level behavior:
//! - migration applies + `schema_version == "11"` post-boot (Story 6.4 bumped
//!   the schema version to seed `project_mapping_revision`);
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
fn schema_version_is_eleven_after_boot() {
    let (_port, state) = boot_projects_server();
    let conn = state.conn.lock().expect("conn lock");
    let v: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'schema_version'",
            [],
            |row| row.get(0),
        )
        .expect("schema_version readable");
    assert_eq!(v, "11");
    // Story 5.2 — the project_mapping_revision key is seeded to "0" by
    // migration id 8. Read it back so a missing / mis-seeded key fails loudly.
    let pmr: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = 'project_mapping_revision'",
            [],
            |row| row.get(0),
        )
        .expect("project_mapping_revision readable");
    assert_eq!(pmr, "0", "project_mapping_revision seeded to 0 by migration id 8");
    // The two project tables exist and have the expected STRICT shape (a
    // CREATE TABLE IF NOT EXISTS that finds them already present is a no-op).
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

// ---------------------------------------------------------------------------
// Story 5.2 — `project_mapping_revision` bump semantics (AD-26 / AD-31).
//
// The revision is the single monotonic scalar bumped inside the existing
// `ProjectStore::with_transaction` on every scope-set-changing op. It is the
// signal that folds into `current_index_revision` so any mapping change
// invalidates every outstanding search/browse cursor. These tests pin the
// bump-on-add/remove/delete and the no-bump-on-create/rename/idempotent-re-add
// branches directly against SQLite (the boot helper returns the IndexState so
// the test can read `tessera_meta` without going through HTTP).
// ---------------------------------------------------------------------------

/// Read the `project_mapping_revision` scalar from the live connection.
fn project_mapping_revision(state: &tessera_lib::IndexState) -> i64 {
    let conn = state.conn.lock().expect("conn lock");
    conn.query_row(
        "SELECT value FROM tessera_meta WHERE key = 'project_mapping_revision'",
        [],
        |row| row.get::<_, String>(0),
    )
    .expect("project_mapping_revision readable")
    .parse::<i64>()
    .unwrap_or(0)
}

/// Story 5.2 — bump on first add-mapping (a new mapping row inserts +
/// `bump_project_mapping_revision`), and on remove-mapping that deletes a
/// row. Idempotent re-add does NOT bump.
#[test]
fn project_mapping_revision_bumps_on_add_remove_and_idempotent_no_bump() {
    let (port, state) = boot_projects_server();
    let baseline = project_mapping_revision(&state);
    assert_eq!(baseline, 0, "fresh DB seeds project_mapping_revision to 0");

    // Create a project (no mappings) — does NOT bump.
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A"}"#);
    assert!(a_resp.starts_with("HTTP/1.1 200"));
    assert_eq!(
        project_mapping_revision(&state),
        0,
        "create must NOT bump (no scope-set change)"
    );

    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id")
        .to_string();

    // Add a (codex, null) mapping — bumps 0 → 1.
    let add_resp = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(add_resp.starts_with("HTTP/1.1 200"), "add mapping: {add_resp}");
    assert_eq!(project_mapping_revision(&state), 1, "first add must bump to 1");

    // Idempotent re-add of the SAME scope to the SAME project — does NOT bump.
    let re_add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(re_add.starts_with("HTTP/1.1 200"), "idempotent re-add: {re_add}");
    assert_eq!(
        project_mapping_revision(&state),
        1,
        "idempotent re-add must NOT bump (scope set unchanged)"
    );

    // Rename the project — does NOT bump (rename leaves the scope set unchanged).
    std::thread::sleep(std::time::Duration::from_secs(1));
    let rename = post_json(
        port,
        "/api/projects/rename",
        &format!(r#"{{"project_id":"{a_id}","name":"A2"}}"#),
    );
    assert!(rename.starts_with("HTTP/1.1 200"), "rename: {rename}");
    assert_eq!(
        project_mapping_revision(&state),
        1,
        "rename must NOT bump (scope set unchanged)"
    );

    // Remove the mapping — bumps 1 → 2 (a row was actually deleted).
    let remove = post_json(
        port,
        "/api/projects/mappings/remove",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(remove.starts_with("HTTP/1.1 200"), "remove mapping: {remove}");
    assert_eq!(project_mapping_revision(&state), 2, "remove must bump to 2");

    // Removing a non-existent mapping (no row deleted) — does NOT bump.
    let no_op_remove = post_json(
        port,
        "/api/projects/mappings/remove",
        &format!(r#"{{"project_id":"{a_id}","provider":"codex","native_project":null}}"#),
    );
    assert!(no_op_remove.starts_with("HTTP/1.1 404"), "no-op remove 404: {no_op_remove}");
    assert_eq!(
        project_mapping_revision(&state),
        2,
        "no-op remove (no row deleted) must NOT bump"
    );
}

/// Story 5.2 — delete bumps when the project had mappings (their removal is a
/// scope-set change) and does NOT bump for an empty project.
#[test]
fn project_mapping_revision_bumps_on_delete_with_mappings_only() {
    let (port, state) = boot_projects_server();
    let baseline = project_mapping_revision(&state);
    assert_eq!(baseline, 0);

    // Create an EMPTY project A.
    let a_resp = post_json(port, "/api/projects/create", r#"{"name":"A-empty"}"#);
    let a_id = a_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id")
        .to_string();
    // Delete the empty project — no mappings removed → NO bump.
    let del_empty = post_json(
        port,
        "/api/projects/delete",
        &format!(r#"{{"project_id":"{a_id}"}}"#),
    );
    assert!(del_empty.starts_with("HTTP/1.1 200"), "delete empty: {del_empty}");
    assert_eq!(
        project_mapping_revision(&state),
        0,
        "delete of an empty project must NOT bump"
    );

    // Create a project B and add a mapping.
    let b_resp = post_json(port, "/api/projects/create", r#"{"name":"B-mapped"}"#);
    let b_id = b_resp
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id")
        .to_string();
    post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{b_id}","provider":"codex","native_project":null}}"#),
    );
    assert_eq!(project_mapping_revision(&state), 1);

    // Delete B (with 1 mapping) — bumps 1 → 2 (1 mapping removed).
    let del_mapped = post_json(
        port,
        "/api/projects/delete",
        &format!(r#"{{"project_id":"{b_id}"}}"#),
    );
    assert!(del_mapped.starts_with("HTTP/1.1 200"), "delete mapped: {del_mapped}");
    // The response carries removed_mappings: 1.
    assert!(del_mapped.contains("\"removed_mappings\":1"));
    assert_eq!(
        project_mapping_revision(&state),
        2,
        "delete of a project with mappings must bump"
    );
}

/// Story 5.2 — a mapping change mid-pagination invalidates every outstanding
/// SEARCH cursor via the shared `current_index_revision`. Page-1 search,
/// bump the revision (add a mapping to any project), then continue with the
/// page-1 cursor → 409 `cursor_stale`.
#[test]
fn search_cursor_goes_stale_after_project_mapping_change() {
    let (port, state) = boot_projects_server();
    // Seed two confirmed sources with records so search has data to paginate.
    {
        let conn = state.conn.lock().expect("conn lock");
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/codex', 'fp-c', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'mr')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .unwrap();
        for (id, observed) in [("rec_a", 100), ("rec_b", 200)] {
            conn.execute(
                "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
                 VALUES (?1, 1, 'gen_1', 'codex', 'section', ?1, 'file:///f#x', 'h', 'v1', 'federation record', 'body', NULL, 'memory', 'full', ?2, 'r', 'file:///f#L1')",
                rusqlite::params![id, observed],
            )
            .unwrap();
        }
    }

    // Page 1 (limit 1) — has more, returns a cursor.
    let page1 = raw_http(
        port,
        &format!("GET /api/search?q=federation&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(page1.starts_with("HTTP/1.1 200"));
    let body = page1.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let cursor = json["payload"]["next_cursor"]
        .as_str()
        .expect("page-1 cursor")
        .to_string();
    assert!(cursor.starts_with("v4."), "cursor envelope is v4: {cursor:?}");

    // Bump the revision via the HTTP API (create + add a mapping). Either
    // insert_mapping new OR remove_mapping row-deleted bumps; we use add.
    let proj = post_json(port, "/api/projects/create", r#"{"name":"P"}"#);
    let pid = proj
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id")
        .to_string();
    let add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{pid}","provider":"codex","native_project":null}}"#),
    );
    assert!(add.starts_with("HTTP/1.1 200"));

    // Page-2 with the now-stale cursor → 409 cursor_stale.
    let page2 = raw_http(
        port,
        &format!(
            "GET /api/search?q=federation&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(
        page2.starts_with("HTTP/1.1 409"),
        "mapping change must invalidate the cursor: got:\n{page2}"
    );
    assert!(page2.contains("\"code\":\"cursor_stale\""));
}

/// Story 5.2 — a mapping change mid-pagination invalidates every outstanding
/// BROWSE cursor too. `BrowseCursor` is structurally unchanged (b4 retained);
/// its `revision` simply now carries mapping state via the shared
/// `current_index_revision`, so a mapping change → revision mismatch → 409.
#[test]
fn browse_cursor_goes_stale_after_project_mapping_change() {
    let (port, state) = boot_projects_server();
    {
        let conn = state.conn.lock().expect("conn lock");
        conn.execute(
            "INSERT INTO source_registry (id, provider, source_kind, lifecycle_state, health_state, coverage_level, normalized_root_path, fingerprint, native_project)
             VALUES (1, 'codex', 'agent_memory', 'confirmed', 'healthy', 'full', '/codex', 'fp-c', NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO scan_runs (source_id, generation, state, fencing_token, intent, manifest_revision)
             VALUES (1, 'gen_1', 'succeeded', 1, 'gen_1', 'mr')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tessera_meta(key, value) VALUES ('active_generation:1', 'gen_1')",
            [],
        )
        .unwrap();
        for (id, observed) in [("rec_a", 100), ("rec_b", 200)] {
            conn.execute(
                "INSERT INTO memory_records (record_id, source_id, generation, provider, unit_kind, native_unit_id, native_locator, content_hash, parser_version, title, body, native_project, provider_memory_type, coverage_level, observed_at, source_revision, display_locator)
                 VALUES (?1, 1, 'gen_1', 'codex', 'section', ?1, 'file:///f#x', 'h', 'v1', 'browse record', 'body', NULL, 'memory', 'full', ?2, 'r', 'file:///f#L1')",
                rusqlite::params![id, observed],
            )
            .unwrap();
        }
    }

    // Page 1 browse.
    let page1 = raw_http(
        port,
        &format!("GET /api/browse?source=src_1&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert!(page1.starts_with("HTTP/1.1 200"));
    let body = page1.split("\r\n\r\n").nth(1).expect("body");
    let json: serde_json::Value = serde_json::from_str(body).expect("json");
    let cursor = json["payload"]["next_cursor"]
        .as_str()
        .expect("page-1 browse cursor")
        .to_string();

    // Bump the revision.
    let proj = post_json(port, "/api/projects/create", r#"{"name":"P"}"#);
    let pid = proj
        .split("\"project_id\":\"")
        .nth(1)
        .and_then(|s| s.split('"').next())
        .expect("project_id")
        .to_string();
    let add = post_json(
        port,
        "/api/projects/mappings/add",
        &format!(r#"{{"project_id":"{pid}","provider":"codex","native_project":null}}"#),
    );
    assert!(add.starts_with("HTTP/1.1 200"));

    // Page 2 with the stale cursor → 409 cursor_stale.
    let page2 = raw_http(
        port,
        &format!(
            "GET /api/browse?source=src_1&cursor={cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(
        page2.starts_with("HTTP/1.1 409"),
        "mapping change must invalidate the browse cursor: got:\n{page2}"
    );
    assert!(page2.contains("\"code\":\"cursor_stale\""));
}

/// Story 5.2 — an old `v3.<hex>` search cursor (pre-5.2 envelope) is rejected
/// as `cursor_stale` on the wire via the prefix-gate (mirrors v1./v2.
/// recovery). Forward-compatible: the UI's existing cursor_stale recovery
/// re-runs page 1.
#[test]
fn search_v3_cursor_is_rejected_as_stale_over_http() {
    let (port, _state) = boot_projects_server();
    // Hand-craft a v3 cursor envelope. The prefix gate fires before any
    // decode, but a realistic payload keeps the test honest about the v3
    // shape.
    let payload = r#"{"version":3,"query":"x","revision":"deadbeef","last_record_id":"rec_1","last_title_match":false,"last_observed_at":0,"last_coverage_full":false,"provider":null,"source":null,"memory_type":null,"native_project":null,"since":null}"#;
    let hex: String = payload.bytes().map(|b| format!("{b:02x}")).collect();
    let v3_cursor = format!("v3.{hex}");
    let response = raw_http(
        port,
        &format!(
            "GET /api/search?q=x&cursor={v3_cursor}&limit=1 HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
        ),
    );
    assert!(response.starts_with("HTTP/1.1 409"), "v3 cursor must 409: got:\n{response}");
    assert!(response.contains("\"code\":\"cursor_stale\""));
}
