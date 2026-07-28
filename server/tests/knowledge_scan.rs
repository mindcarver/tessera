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

/// Story 6.9 — Knowledge Browse returns paginated notes for a scanned Vault.
#[test]
fn browse_knowledge_returns_notes_after_scan() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("browse-vault");
    fs::create_dir_all(vault.join("Notes")).unwrap();
    fs::write(vault.join("Notes/alpha.md"), "# Alpha\nfirst note").unwrap();
    fs::write(vault.join("Notes/beta.md"), "# Beta\nsecond note").unwrap();
    fs::write(vault.join("Notes/gamma.md"), "no heading here").unwrap();

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();

    // Browse with limit 2 → first page of 2, has_more.
    let page = application::browse_knowledge(
        &registry,
        &conn,
        &source.source_id,
        2,
        None,
    )
    .expect("browse");
    assert_eq!(page.results.len(), 2, "first page has 2 notes");
    assert!(page.next_cursor.is_some(), "has_more → next_cursor");
    assert_eq!(page.empty_state, application::query::KnowledgeBrowseEmptyState::None);

    // The titles are derived: Alpha/Beta (sorted by path: alpha < beta < gamma).
    assert_eq!(page.results[0].excerpt.contains("Alpha"), true, "first is alpha");
    assert_eq!(page.results[1].excerpt.contains("Beta"), true, "second is beta");

    // Page 2: the remaining note (gamma), no next cursor.
    let cursor = page.next_cursor.unwrap();
    let page2 = application::browse_knowledge(
        &registry,
        &conn,
        &source.source_id,
        2,
        Some(&cursor),
    )
    .expect("browse page 2");
    assert_eq!(page2.results.len(), 1, "second page has 1 note (gamma)");
    assert!(page2.next_cursor.is_none(), "last page");
}

/// Story 6.9 — Browse of a confirmed-but-never-scanned Vault returns an
/// honest `not_yet_scanned` empty state.
#[test]
fn browse_knowledge_unscanned_vault_returns_not_yet_scanned() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("unscanned");
    fs::create_dir_all(&vault).unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    // Note: NOT scanned.
    let page =
        application::browse_knowledge(&registry, &conn, &source.source_id, 20, None).unwrap();
    assert!(page.results.is_empty());
    assert_eq!(
        page.empty_state,
        application::query::KnowledgeBrowseEmptyState::NotYetScanned
    );
}

// --- Story 6.9 — Knowledge Search tests -------------------------------------

#[test]
fn search_knowledge_finds_matches_across_vaults() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault_a = tmp.path().join("vault-a");
    let vault_b = tmp.path().join("vault-b");
    fs::create_dir_all(&vault_a).unwrap();
    fs::create_dir_all(&vault_b).unwrap();
    fs::write(vault_a.join("a.md"), "# Rust notes\nlearning rust").unwrap();
    fs::write(vault_b.join("b.md"), "# Python notes\nlearning python rust").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let sa = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault_a)).unwrap();
    let sb = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault_b)).unwrap();
    application::scan_source(&registry, &conn, &sa.source_id).unwrap();
    application::scan_source(&registry, &conn, &sb.source_id).unwrap();
    let page = application::search_knowledge(&registry, &conn, "rust", 20, None, None, None, None).unwrap();
    assert!(page.results.len() >= 2);
    let names: Vec<&str> = page.results.iter().map(|r| r.vault_name.as_str()).collect();
    assert!(names.contains(&"vault-a") && names.contains(&"vault-b"));
}

#[test]
fn search_knowledge_with_source_filter_narrows_to_one_vault() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault_a = tmp.path().join("vault-a");
    let vault_b = tmp.path().join("vault-b");
    fs::create_dir_all(&vault_a).unwrap();
    fs::create_dir_all(&vault_b).unwrap();
    fs::write(vault_a.join("a.md"), "# Rust\nrust content").unwrap();
    fs::write(vault_b.join("b.md"), "# Rust\nrust content").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let sa = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault_a)).unwrap();
    let sb = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault_b)).unwrap();
    application::scan_source(&registry, &conn, &sa.source_id).unwrap();
    application::scan_source(&registry, &conn, &sb.source_id).unwrap();
    let page = application::search_knowledge(&registry, &conn, "rust", 20, None, Some(&sa.source_id), None, None).unwrap();
    assert_eq!(page.results.len(), 1);
    assert_eq!(page.results[0].vault_name, "vault-a");
}

#[test]
fn search_knowledge_with_folder_prefix_narrows_results() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("Notes/sub")).unwrap();
    fs::write(vault.join("Notes/top.md"), "# Rust\nrust here").unwrap();
    fs::write(vault.join("Notes/sub/deep.md"), "# Rust\nrust deep").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let page = application::search_knowledge(&registry, &conn, "rust", 20, None, None, Some("Notes/sub"), None).unwrap();
    assert_eq!(page.results.len(), 1);
    assert!(page.results[0].vault_relative_path.starts_with("Notes/sub"));
}

#[test]
fn search_knowledge_no_match_returns_no_match_state() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("a.md"), "# Hello\nworld").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let page = application::search_knowledge(&registry, &conn, "zzznomatch", 20, None, None, None, None).unwrap();
    assert!(page.results.is_empty());
    assert_eq!(page.empty_state, application::query::KnowledgeSearchEmptyState::NoMatch);
}

#[test]
fn search_knowledge_unscanned_returns_not_indexed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let _ = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    let page = application::search_knowledge(&registry, &conn, "anything", 20, None, None, None, None).unwrap();
    assert!(page.results.is_empty());
    assert_eq!(page.empty_state, application::query::KnowledgeSearchEmptyState::NotIndexed);
}

#[test]
fn search_knowledge_paginates_across_pages() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    for i in 0..5 { fs::write(vault.join(format!("note{i}.md")), format!("# Note{i}\nrust keyword")).unwrap(); }
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let page1 = application::search_knowledge(&registry, &conn, "rust", 2, None, None, None, None).unwrap();
    assert_eq!(page1.results.len(), 2);
    let cursor = page1.next_cursor.expect("has_more");
    let page2 = application::search_knowledge(&registry, &conn, "rust", 2, Some(&cursor), None, None, None).unwrap();
    assert_eq!(page2.results.len(), 2);
    let ids1: Vec<&str> = page1.results.iter().map(|r| r.record_id.as_str()).collect();
    for id in page2.results.iter().map(|r| r.record_id.as_str()) {
        assert!(!ids1.contains(&id), "no duplicate: {id}");
    }
}

#[test]
fn search_knowledge_empty_query_is_bad_request() {
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let result = application::search_knowledge(&registry, &conn, "", 20, None, None, None, None);
    assert!(matches!(result.unwrap_err(), application::query::QueryError::BadRequest));
}

/// title_match: a note with the keyword in its title ranks before a body-only match.
#[test]
fn search_knowledge_title_match_ranks_first() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("title.md"), "# rust\nirrelevant body").unwrap();
    fs::write(vault.join("body.md"), "# Other\nrust in body only").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let page = application::search_knowledge(&registry, &conn, "rust", 20, None, None, None, None).unwrap();
    assert!(page.results.len() >= 2);
    assert!(page.results[0].title_match, "first result should be a title match");
    assert!(!page.results.iter().all(|r| r.title_match), "at least one body-only match");
}

/// Terminal pagination page: 5 notes, limit=2 → page 3 has 1 result, no cursor.
#[test]
fn search_knowledge_terminal_page_has_no_cursor() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    for i in 0..5 { fs::write(vault.join(format!("note{i}.md")), format!("# Note{i}\nrust keyword")).unwrap(); }
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let p1 = application::search_knowledge(&registry, &conn, "rust", 2, None, None, None, None).unwrap();
    let p2 = application::search_knowledge(&registry, &conn, "rust", 2, Some(&p1.next_cursor.clone().unwrap()), None, None, None).unwrap();
    let p3 = application::search_knowledge(&registry, &conn, "rust", 2, Some(&p2.next_cursor.clone().unwrap()), None, None, None).unwrap();
    assert_eq!(p3.results.len(), 1, "terminal page has 1 remaining result");
    assert!(p3.next_cursor.is_none(), "terminal page has no cursor");
    // All 5 distinct across 3 pages.
    let mut all_ids: Vec<String> = vec![];
    for page in [&p1, &p2, &p3] { all_ids.extend(page.results.iter().map(|r| r.record_id.clone())); }
    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(unique.len(), 5, "5 distinct notes across all pages");
}

/// Whitespace-only query → bad_request.
#[test]
fn search_knowledge_whitespace_only_query_is_bad_request() {
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let result = application::search_knowledge(&registry, &conn, "   ", 20, None, None, None, None);
    assert!(matches!(result.unwrap_err(), application::query::QueryError::BadRequest));
}

/// Unknown source id → bad_request (not silent empty).
#[test]
fn search_knowledge_unknown_source_is_bad_request() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(&vault).unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    let bad_id = tessera_lib::domain::source::SourceId("src_abc".to_string());
    let result = application::search_knowledge(&registry, &conn, "rust", 20, None, Some(&bad_id), None, None);
    assert!(matches!(result.unwrap_err(), application::query::QueryError::BadRequest));
}

/// LIKE wildcard in folder prefix is treated literally (not as a wildcard).
#[test]
fn search_knowledge_folder_prefix_escapes_like_wildcards() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let vault = tmp.path().join("vault");
    fs::create_dir_all(vault.join("Notes/sub")).unwrap();
    fs::write(vault.join("Notes/top.md"), "# Rust\nrust here").unwrap();
    fs::write(vault.join("Notes/sub/deep.md"), "# Rust\nrust deep").unwrap();
    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, &knowledge_candidate(&vault)).unwrap();
    application::scan_source(&registry, &conn, &source.source_id).unwrap();
    // folder="Notes/%" should match NOTHING (literal % in path, not wildcard).
    let page = application::search_knowledge(&registry, &conn, "rust", 20, None, None, Some("Notes/%"), None).unwrap();
    assert!(page.results.is_empty(), "literal % folder prefix matches no paths");
    // folder="Notes/sub" still matches normally.
    let page2 = application::search_knowledge(&registry, &conn, "rust", 20, None, None, Some("Notes/sub"), None).unwrap();
    assert_eq!(page2.results.len(), 1, "normal folder prefix works");
}
