//! Story 6.5 follow-up — Knowledge (Obsidian Vault) scan pipeline integration
//! tests. Confirms the Knowledge pipeline indexes notes into
//! `knowledge_records`, reports the correct count, and is zero-write (NFR-14).
//!
//! These tests build a temp Vault, confirm it through the Knowledge confirm
//! path, scan it through `scan_source` (which dispatches by source_kind), and
//! verify the active generation + note count. They exercise the same
//! begin_run → enumerate → stage → manifest-revalidate → commit_cas path as
//! the Agent-Memory pipeline tests, but for the Knowledge domain.

use std::fs;
use std::path::Path;

use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{CandidateSource, CoverageLevel, DiscoveryBasis};
use tessera_lib::domain::source::SourceLifecycle;
use tessera_lib::index::migrations;
use tessera_lib::index::{scan_store::ScanStore, SourceRegistry};
use rusqlite::Connection;

fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("in-memory db");
    migrations::apply(&mut conn).expect("migrations apply");
    conn
}

fn knowledge_candidate(root: &Path) -> CandidateSource {
    CandidateSource {
        provider: "obsidian".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: DiscoveryBasis::ObsidianVaultRegistry,
        coverage_level: CoverageLevel::Full,
        native_project: None,
    }
}

/// Build a small Vault with 3 in-matrix notes + excluded artifacts.
fn build_vault(root: &Path) {
    fs::create_dir_all(root.join("Notes/sub")).unwrap();
    fs::write(root.join("Notes/a.md"), "# A\nbody a").unwrap();
    fs::write(root.join("Notes/sub/b.md"), "# B\nbody b").unwrap();
    fs::write(root.join("readme.md"), "top level").unwrap();
    // Excluded artifacts (must NOT be indexed).
    fs::create_dir_all(root.join(".obsidian")).unwrap();
    fs::write(root.join(".obsidian/workspace.json"), "{}").unwrap();
    fs::create_dir_all(root.join(".git")).unwrap();
    fs::write(root.join(".git/HEAD"), "ref").unwrap();
    fs::write(root.join("Notes/canvas.canvas"), "{}").unwrap();
    fs::write(root.join("Notes/img.png"), b"\x89PNG").unwrap();
}

#[test]
fn scan_knowledge_source_indexes_notes_and_reports_count() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("my-vault");
    fs::create_dir_all(&vault).unwrap();
    build_vault(&vault);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault))
            .expect("confirm");

    // Scan through the dispatch entry point (routes by source_kind).
    let outcome = application::scan_source(&registry, &conn, &source.source_id)
        .expect("scan succeeds");
    assert_eq!(outcome.records_indexed, 3, "3 in-matrix notes indexed");

    // The active generation has 3 knowledge records.
    let store = ScanStore::new(&conn);
    let rowid = ScanStore::source_rowid(&source.source_id).unwrap();
    let count = store.count_active_knowledge_records(rowid).unwrap();
    assert_eq!(count, 3, "active generation has 3 knowledge records");

    // Knowledge Inventory now reports the real count (not null).
    let inventory = application::list_knowledge_inventory(&registry, &conn).unwrap();
    assert_eq!(inventory.len(), 1);
    assert_eq!(
        inventory[0].complete_note_count,
        Some(3),
        "Inventory shows real note count after scan"
    );
    assert_eq!(inventory[0].vault_name, "my-vault");
    assert_eq!(inventory[0].lifecycle_state, SourceLifecycle::Confirmed);
}

/// Story 6.5 AC: the records carry krec_ identity, the Knowledge parser
/// version, and Vault-relative locators.
#[test]
fn knowledge_records_carry_krec_identity_and_parser_version() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("v");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("note.md"), "# Hello").unwrap();

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();

    let rowid = ScanStore::source_rowid(&source.source_id).unwrap();
    let (rid, pv, loc, uk, nuid): (String, String, String, String, String) = conn
        .query_row(
            "SELECT record_id, parser_version, native_locator, unit_kind, native_unit_id \
             FROM knowledge_records WHERE source_id = ?1 LIMIT 1",
            rusqlite::params![rowid],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert!(rid.starts_with("krec_"), "krec_ identity; got {rid}");
    assert_eq!(pv, "obsidian-markdown/v1", "Knowledge parser version");
    assert_eq!(loc, "note.md", "Vault-relative locator");
    assert_eq!(uk, "note", "file-level unit kind");
    assert_eq!(nuid, "note", "native_unit_id = filename stem");
}

/// Story 6.5 / NFR-14: scanning does not mutate the Vault. Snapshot before
/// and after, assert byte-identical (path, mtime, size, bytes).
#[test]
fn scan_knowledge_source_does_not_mutate_vault_nfr14() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault-zero-write");
    fs::create_dir_all(&vault).unwrap();
    build_vault(&vault);
    let before = snapshot_vault(&vault);

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();

    let after = snapshot_vault(&vault);
    assert_eq!(before, after, "NFR-14: scan mutated Vault files");
}

/// Story 6.5 AC: an empty Vault (0 in-matrix notes) scans successfully with
/// records_indexed = 0 (a truthful zero, not a failure).
#[test]
fn scan_empty_vault_succeeds_with_zero_notes() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("empty-vault");
    fs::create_dir_all(&vault).unwrap();
    // Only excluded artifacts.
    fs::create_dir_all(vault.join(".obsidian")).unwrap();
    fs::write(vault.join(".obsidian/workspace.json"), "{}").unwrap();

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    let outcome = application::scan_source(&registry, &conn, &source.source_id)
        .expect("empty scan succeeds");
    assert_eq!(outcome.records_indexed, 0, "0 notes is a valid empty scan");
}

/// Story 6.5 AC: re-scanning an unchanged Vault is idempotent (same count,
/// stable krec_ identity).
#[test]
fn rescan_unchanged_vault_is_idempotent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("stable");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("a.md"), "# A").unwrap();
    fs::write(vault.join("b.md"), "# B").unwrap();

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    let first = application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let second = application::scan_source(&registry, &conn, &source.source_id).unwrap();
    assert_eq!(first.records_indexed, 2);
    assert_eq!(second.records_indexed, 2, "idempotent re-scan");
}

fn snapshot_vault(root: &Path) -> Vec<(String, std::time::SystemTime, u64, Vec<u8>)> {
    let mut out = Vec::new();
    walk_snap(root, root, &mut out);
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
fn walk_snap(
    root: &Path,
    dir: &Path,
    out: &mut Vec<(String, std::time::SystemTime, u64, Vec<u8>)>,
) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let meta = entry.metadata().unwrap();
        if meta.is_dir() {
            walk_snap(root, &path, out);
            continue;
        }
        if meta.is_file() {
            out.push((
                path.strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned(),
                meta.modified().unwrap(),
                meta.len(),
                fs::read(&path).unwrap(),
            ));
        }
    }
}
