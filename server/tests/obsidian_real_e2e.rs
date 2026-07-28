//! Story 6.6 real end-to-end smoke: discover → confirm a real vault → see it
//! in the Knowledge Inventory. Gated on REAL_VAULTS. Uses the live HTTP server.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::Arc;
use tessera_lib::http::server::{bind, serve_with};
use tessera_lib::{boot, IndexState};

fn boot_server() -> (u16, Arc<IndexState>) {
    let dir = tempfile::tempdir().expect("scratch app-data");
    let state = boot(dir.path()).expect("boot");
    let server = bind("127.0.0.1:0");
    let port = server.server_addr().to_ip().expect("bound").port();
    let state2 = Arc::new(state);
    let state3 = state2.clone();
    std::thread::spawn(move || {
        let _dir = dir;
        serve_with(server, state3, PathBuf::from("dist"), Some(port));
    });
    std::thread::sleep(std::time::Duration::from_millis(80));
    (port, state2)
}

fn http_get(port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).expect("write");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("read");
    resp.split("\r\n\r\n").nth(1).expect("body").to_string()
}

fn http_post(port: u16, path: &str, body: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes()).expect("write");
    let mut resp = String::new();
    stream.read_to_string(&mut resp).expect("read");
    resp.split("\r\n\r\n").nth(1).expect("body").to_string()
}

#[test]
fn confirm_real_vault_then_it_appears_in_knowledge_inventory() {
    if std::env::var("REAL_VAULTS").map(|v| v != "1").unwrap_or(true) {
        eprintln!("skipping real e2e test (set REAL_VAULTS=1)");
        return;
    }
    let (port, _state) = boot_server();

    // 1. Knowledge discovery returns the real vaults.
    let discover_body = http_get(port, "/api/knowledge/discover");
    let page: serde_json::Value = serde_json::from_str(&discover_body).expect("discover json");
    let candidates = page["payload"]["candidates"].as_array().expect("candidates");
    assert!(candidates.len() >= 2, "expected >= 2 vaults; got {discover_body}");

    // 2. Confirm the dev-repo vault (smallest, fast).
    let devrepo = candidates
        .iter()
        .find(|c| c["root_path"].as_str().unwrap_or("").ends_with("dev-repo"))
        .expect("dev-repo candidate present");
    let devrepo_json = serde_json::to_string(devrepo).unwrap();
    let confirm_body = http_post(
        port,
        "/api/knowledge/confirm",
        &format!(r#"{{"candidate":{devrepo_json}}}"#),
    );
    let confirm: serde_json::Value = serde_json::from_str(&confirm_body).expect("confirm json");
    assert_eq!(confirm["payload"]["source_kind"], "local_knowledge", "got: {confirm_body}");
    assert_eq!(confirm["payload"]["provider"], "obsidian");
    assert_eq!(confirm["payload"]["lifecycle_state"], "confirmed");

    // 3. Knowledge Inventory now lists dev-repo.
    let inv_body = http_get(port, "/api/knowledge/inventory");
    let inv: serde_json::Value = serde_json::from_str(&inv_body).expect("inventory json");
    let rows = inv["payload"].as_array().expect("inventory array");
    assert_eq!(rows.len(), 1, "one confirmed vault; got: {inv_body}");
    assert_eq!(rows[0]["vault_name"], "dev-repo");
    assert_eq!(rows[0]["provider"], "obsidian");
    assert_eq!(rows[0]["lifecycle_state"], "confirmed");
    // Note count is null until the staging write path lands (out-of-slice);
    // the Inventory must NOT fabricate a number.
    assert!(
        rows[0]["complete_note_count"].is_null(),
        "note count must be null (staging not wired), not fabricated; got: {inv_body}"
    );
    println!("e2e OK: confirmed dev-repo, appears in Knowledge Inventory: {inv_body}");
}
