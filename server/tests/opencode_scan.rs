use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use rusqlite::Connection;
use tempfile::tempdir;

use tessera_lib::adapters::opencode::OpenCodeAdapter;
use tessera_lib::application;
use tessera_lib::domain::ports::provider_adapter::{
    CandidateSource, CoverageLevel, DiscoveryBasis, ProviderAdapter, ProviderMemoryType,
};
use tessera_lib::domain::query::{SearchFilters, SearchRequest};
use tessera_lib::domain::source::{HealthCause, HealthState};
use tessera_lib::index::{migrations, SourceRegistry};

fn fresh_db() -> Connection {
    let mut conn = Connection::open_in_memory().expect("db");
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .expect("foreign keys");
    migrations::apply(&mut conn).expect("migrations");
    conn
}

fn candidate(root: &Path, native_project: Option<&str>) -> CandidateSource {
    CandidateSource {
        provider: "opencode".to_string(),
        root_path: root.to_string_lossy().into_owned(),
        basis: if native_project.is_some() {
            DiscoveryBasis::OpencodeProjectDatabase
        } else {
            DiscoveryBasis::OpencodeGlobalConfig
        },
        coverage_level: CoverageLevel::Full,
        native_project: native_project.map(str::to_string),
    }
}

fn snapshot(root: &Path) -> Vec<(PathBuf, SystemTime, u64, Vec<u8>)> {
    fn walk(path: &Path, output: &mut Vec<(PathBuf, SystemTime, u64, Vec<u8>)>) {
        for entry in fs::read_dir(path).expect("read dir") {
            let entry = entry.expect("entry");
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).expect("metadata");
            if metadata.is_dir() {
                walk(&path, output);
            } else {
                output.push((
                    path,
                    metadata.modified().expect("mtime"),
                    metadata.len(),
                    fs::read(entry.path()).expect("bytes"),
                ));
            }
        }
    }

    let mut output = Vec::new();
    walk(root, &mut output);
    output.sort_by(|left, right| left.0.cmp(&right.0));
    output
}

#[test]
fn enumerate_is_exactly_one_direct_agents_md_instruction_artifact() {
    let root = tempdir().expect("root");
    fs::write(root.path().join("AGENTS.md"), "# Durable\n\ninstruction").expect("AGENTS.md");
    fs::write(root.path().join("README.md"), "# unrelated").expect("readme");
    fs::create_dir(root.path().join("nested")).expect("nested");
    fs::write(root.path().join("nested").join("AGENTS.md"), "# nested").expect("nested agents");

    let observation = OpenCodeAdapter
        .enumerate_artifacts(root.path())
        .expect("enumerate");

    assert_eq!(observation.supported.len(), 1);
    assert_eq!(observation.supported[0].file.relative_path, "AGENTS.md");
    assert_eq!(
        observation.supported[0].memory_type,
        ProviderMemoryType::AgentInstruction
    );
    assert!(
        observation.diagnostics.is_empty(),
        "unrelated repository entries are ignored"
    );
}

#[test]
fn scan_indexes_searchable_opencode_records_without_mutating_source() {
    let root = tempdir().expect("root");
    fs::write(
        root.path().join("AGENTS.md"),
        "# Durable OpenCode\n\npersistent instruction federation",
    )
    .expect("agents");
    fs::write(root.path().join("source.rs"), "fn untouched() {}").expect("unrelated");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, &candidate(root.path(), Some("project-7")))
        .expect("confirm");
    let before = snapshot(root.path());

    application::scan_source(&registry, &conn, &source.source_id).expect("scan");

    assert_eq!(
        snapshot(root.path()),
        before,
        "source bytes/size/mtime unchanged"
    );
    let (provider, native_project, memory_type, parser_version): (
        String,
        Option<String>,
        String,
        String,
    ) = conn
        .query_row(
            "SELECT provider, native_project, provider_memory_type, parser_version
             FROM memory_records WHERE source_id = ?1 LIMIT 1",
            [source.source_id.to_rowid().expect("rowid")],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .expect("record");
    assert_eq!(provider, "opencode");
    assert_eq!(native_project.as_deref(), Some("project-7"));
    assert_eq!(memory_type, "agent_instruction");
    assert_eq!(parser_version, "opencode-agents-md/v1");

    let page = application::search(
        &registry,
        &conn,
        SearchRequest::new_with_filters(
            "persistent instruction".to_string(),
            None,
            Some(20),
            SearchFilters {
                provider: Some("opencode".to_string()),
                memory_type: Some(ProviderMemoryType::AgentInstruction),
                ..Default::default()
            },
        )
        .expect("request"),
    )
    .expect("search");
    assert!(!page.results().is_empty());
    assert!(page
        .results()
        .iter()
        .all(|result| result.provider() == "opencode"));
}

#[test]
fn missing_defining_file_degrades_and_preserves_last_successful_generation() {
    let root = tempdir().expect("root");
    let agents = root.path().join("AGENTS.md");
    fs::write(&agents, "# Durable\n\nlast successful body").expect("agents");

    let conn = fresh_db();
    let registry = SourceRegistry::new(&conn);
    let source =
        application::confirm_source(&registry, &candidate(root.path(), None)).expect("confirm");
    application::scan_source(&registry, &conn, &source.source_id).expect("first scan");
    let meta_key = format!(
        "active_generation:{}",
        source.source_id.to_rowid().expect("rowid")
    );
    let active_before: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = ?1",
            [&meta_key],
            |row| row.get(0),
        )
        .expect("active generation");

    fs::remove_file(agents).expect("remove defining file");
    assert!(
        application::scan_source(&registry, &conn, &source.source_id).is_err(),
        "missing AGENTS.md is terminal for this source"
    );

    let active_after: String = conn
        .query_row(
            "SELECT value FROM tessera_meta WHERE key = ?1",
            [&meta_key],
            |row| row.get(0),
        )
        .expect("previous generation remains active");
    assert_eq!(active_after, active_before);
    let source_after = registry
        .get(&source.source_id)
        .expect("registry")
        .expect("source");
    assert_eq!(source_after.health_state, HealthState::Degraded);
    assert_eq!(source_after.health_cause, HealthCause::ScanFailed);
}

#[cfg(unix)]
#[test]
fn enumeration_rejects_agents_md_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let outside = tempdir().expect("outside");
    let outside_file = outside.path().join("AGENTS.md");
    fs::write(&outside_file, "# outside").expect("outside file");
    symlink(outside_file, root.path().join("AGENTS.md")).expect("symlink");

    assert!(OpenCodeAdapter.enumerate_artifacts(root.path()).is_err());
}
