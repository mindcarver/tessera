use std::fs;
use std::time::SystemTime;

use rusqlite::{params, Connection};
use tempfile::tempdir;

use tessera_lib::adapters::opencode::OpenCodeAdapter;
use tessera_lib::domain::ports::provider_adapter::{
    CoverageLevel, DiscoveryBasis, ProviderAdapter,
};
use tessera_lib::domain::project::MAX_NATIVE_PROJECT_LEN;

fn write_agents(root: &std::path::Path, body: &str) {
    fs::create_dir_all(root).expect("create root");
    fs::write(root.join("AGENTS.md"), body).expect("write AGENTS.md");
}

fn create_project_db(data_home: &std::path::Path, rows: &[(&str, &std::path::Path)]) {
    let data_dir = data_home.join("opencode");
    fs::create_dir_all(&data_dir).expect("create data dir");
    let conn = Connection::open(data_dir.join("opencode.db")).expect("open fixture db");
    conn.execute_batch("CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT NOT NULL);")
        .expect("create project-only schema");
    for (id, worktree) in rows {
        conn.execute(
            "INSERT INTO project(id, worktree) VALUES (?1, ?2)",
            (id, worktree.to_string_lossy().as_ref()),
        )
        .expect("insert project");
    }
}

fn file_snapshot(path: &std::path::Path) -> (u64, SystemTime, Vec<u8>) {
    let metadata = fs::metadata(path).expect("metadata");
    (
        metadata.len(),
        metadata.modified().expect("mtime"),
        fs::read(path).expect("bytes"),
    )
}

#[test]
fn discovers_global_and_project_instruction_sources_from_first_party_locations() {
    let home = tempdir().expect("home");
    let config = home.path().join(".config").join("opencode");
    let project = home.path().join("worktree");
    write_agents(&config, "# global");
    write_agents(&project, "# project");
    create_project_db(
        &home.path().join(".local").join("share"),
        &[("project-1", &project)],
    );

    let candidates = OpenCodeAdapter.discover_with_env(None, None, None, home.path().to_str());

    assert_eq!(candidates.len(), 2);
    let global = candidates
        .iter()
        .find(|candidate| candidate.native_project.is_none())
        .expect("global candidate");
    assert_eq!(global.provider, "opencode");
    assert_eq!(global.root_path, config.to_string_lossy());
    assert_eq!(global.basis, DiscoveryBasis::OpencodeGlobalConfig);
    assert_eq!(global.coverage_level, CoverageLevel::Full);

    let project_candidate = candidates
        .iter()
        .find(|candidate| candidate.native_project.as_deref() == Some("project-1"))
        .expect("project candidate");
    assert_eq!(project_candidate.root_path, project.to_string_lossy());
    assert_eq!(
        project_candidate.basis,
        DiscoveryBasis::OpencodeProjectDatabase
    );
    assert_eq!(
        OpenCodeAdapter.native_project_for_root_with_env(
            &config,
            None,
            None,
            None,
            home.path().to_str(),
        ),
        Some(None)
    );
    assert_eq!(
        OpenCodeAdapter.native_project_for_root_with_env(
            &project,
            None,
            None,
            None,
            home.path().to_str(),
        ),
        Some(Some("project-1".to_string()))
    );
    assert_eq!(OpenCodeAdapter.provider_id(), "opencode");
    assert_eq!(OpenCodeAdapter.parser_version(), "opencode-agents-md/v1");
}

#[test]
fn xdg_config_home_takes_precedence_over_home_default() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let xdg_config_home = root.path().join("xdg-config");
    let home_config = home.join(".config").join("opencode");
    let xdg_config = xdg_config_home.join("opencode");
    write_agents(&home_config, "# home");
    write_agents(&xdg_config, "# xdg");

    let candidates =
        OpenCodeAdapter.discover_with_env(None, xdg_config_home.to_str(), None, home.to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].root_path, xdg_config.to_string_lossy());
    assert_eq!(candidates[0].basis, DiscoveryBasis::OpencodeGlobalConfig);
}

#[test]
fn project_only_database_proves_no_session_table_dependency_and_is_unchanged() {
    let root = tempdir().expect("root");
    let data_home = root.path().join("data");
    let project = root.path().join("project");
    write_agents(&project, "# project");
    create_project_db(&data_home, &[("p-only", &project)]);
    let database = data_home.join("opencode").join("opencode.db");
    let before = file_snapshot(&database);

    let candidates =
        OpenCodeAdapter.discover_with_env(None, None, data_home.to_str(), root.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].native_project.as_deref(), Some("p-only"));
    assert_eq!(file_snapshot(&database), before, "database is read-only");
    assert!(
        !data_home
            .join("opencode")
            .join("opencode.db-journal")
            .exists(),
        "read-only discovery creates no rollback journal"
    );
}

#[test]
fn incompatible_row_decoding_discards_all_project_candidates() {
    let root = tempdir().expect("root");
    let config = root.path().join("config");
    let data_home = root.path().join("data");
    let project = root.path().join("project");
    write_agents(&config, "# global");
    write_agents(&project, "# project");
    let data_dir = data_home.join("opencode");
    fs::create_dir_all(&data_dir).expect("data dir");
    {
        let conn = Connection::open(data_dir.join("opencode.db")).expect("open fixture db");
        conn.execute_batch("CREATE TABLE project (id TEXT PRIMARY KEY, worktree TEXT NOT NULL);")
            .expect("create project table");
        conn.execute(
            "INSERT INTO project(id, worktree) VALUES (?1, ?2)",
            params!["valid-project", project.to_string_lossy().as_ref()],
        )
        .expect("insert valid row");
        conn.execute(
            "INSERT INTO project(id, worktree) VALUES (?1, ?2)",
            params![vec![0_u8, 1_u8], project.to_string_lossy().as_ref()],
        )
        .expect("insert incompatible id row");
    }

    let candidates = OpenCodeAdapter.discover_with_env(
        config.to_str(),
        None,
        data_home.to_str(),
        root.path().to_str(),
    );

    assert_eq!(candidates.len(), 1, "project rows fail as one unit");
    assert_eq!(candidates[0].basis, DiscoveryBasis::OpencodeGlobalConfig);
}

#[test]
fn invalid_or_oversized_project_ids_are_rejected_without_normalization() {
    let root = tempdir().expect("root");
    let data_home = root.path().join("data");
    let empty_root = root.path().join("empty");
    let whitespace_root = root.path().join("whitespace");
    let leading_root = root.path().join("leading");
    let trailing_root = root.path().join("trailing");
    let oversized_root = root.path().join("oversized");
    let valid_root = root.path().join("valid");
    for project in [
        &empty_root,
        &whitespace_root,
        &leading_root,
        &trailing_root,
        &oversized_root,
        &valid_root,
    ] {
        write_agents(project, "# project");
    }
    let oversized = "x".repeat(MAX_NATIVE_PROJECT_LEN + 1);
    create_project_db(
        &data_home,
        &[
            ("", &empty_root),
            ("   ", &whitespace_root),
            (" leading", &leading_root),
            ("trailing ", &trailing_root),
            (oversized.as_str(), &oversized_root),
            ("opaque id", &valid_root),
        ],
    );

    let candidates =
        OpenCodeAdapter.discover_with_env(None, None, data_home.to_str(), root.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].native_project.as_deref(), Some("opaque id"));
    assert_eq!(candidates[0].root_path, valid_root.to_string_lossy());
}

#[test]
fn unavailable_or_incompatible_database_does_not_block_global_discovery() {
    let root = tempdir().expect("root");
    let config = root.path().join("config");
    let data_home = root.path().join("data");
    write_agents(&config, "# global");
    fs::create_dir_all(data_home.join("opencode")).expect("data dir");
    fs::write(
        data_home.join("opencode").join("opencode.db"),
        b"not sqlite",
    )
    .expect("malformed db");

    let candidates = OpenCodeAdapter.discover_with_env(
        config.to_str(),
        None,
        data_home.to_str(),
        root.path().to_str(),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::OpencodeGlobalConfig);
}

#[test]
fn discovery_skips_missing_relative_and_nested_instruction_candidates() {
    let root = tempdir().expect("root");
    let home = root.path().join("home");
    let config = home.join(".config").join("opencode");
    let project = root.path().join("project");
    fs::create_dir_all(config.join("nested")).expect("nested config");
    fs::write(config.join("nested").join("AGENTS.md"), "# nested").expect("nested file");
    fs::create_dir_all(&project).expect("project");
    create_project_db(
        &home.join(".local").join("share"),
        &[
            ("missing-agents", &project),
            ("relative-worktree", std::path::Path::new("relative")),
        ],
    );

    assert!(
        OpenCodeAdapter
            .discover_with_env(None, None, None, home.to_str())
            .is_empty(),
        "only direct-child AGENTS.md and absolute worktrees are candidates"
    );

    write_agents(&config, "# default should be ignored");
    assert!(
        OpenCodeAdapter
            .discover_with_env(Some("relative/config"), None, None, home.to_str())
            .iter()
            .all(|candidate| candidate.basis != DiscoveryBasis::OpencodeGlobalConfig),
        "explicit relative OPENCODE_CONFIG_DIR never falls back"
    );
}

#[test]
fn rebind_identity_is_exact_and_fails_closed_when_metadata_is_ambiguous() {
    let root = tempdir().expect("root");
    let config = root.path().join("config");
    let data_home = root.path().join("data");
    write_agents(&config, "# same root");
    create_project_db(
        &data_home,
        &[
            ("project-overlap-a", &config),
            ("project-overlap-b", &config),
        ],
    );

    assert!(
        OpenCodeAdapter
            .native_project_for_root_with_env(
                &config,
                config.to_str(),
                None,
                data_home.to_str(),
                root.path().to_str(),
            )
            .is_none(),
        "global and project identities on one root are ambiguous"
    );
    assert!(
        OpenCodeAdapter
            .discover_with_env(
                config.to_str(),
                None,
                data_home.to_str(),
                root.path().to_str(),
            )
            .is_empty(),
        "discovery omits every candidate for an ambiguous canonical root"
    );
}

#[cfg(unix)]
#[test]
fn non_writable_database_is_still_discovered_read_only() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempdir().expect("root");
    let data_home = root.path().join("data");
    let project = root.path().join("project");
    write_agents(&project, "# project");
    create_project_db(&data_home, &[("read-only", &project)]);
    let database = data_home.join("opencode").join("opencode.db");
    fs::set_permissions(&database, fs::Permissions::from_mode(0o444)).expect("chmod database");

    let candidates =
        OpenCodeAdapter.discover_with_env(None, None, data_home.to_str(), root.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].native_project.as_deref(), Some("read-only"));
}

#[test]
fn exclusive_database_lock_safe_degrades_to_global_candidate() {
    let root = tempdir().expect("root");
    let config = root.path().join("config");
    let data_home = root.path().join("data");
    let project = root.path().join("project");
    write_agents(&config, "# global");
    write_agents(&project, "# project");
    create_project_db(&data_home, &[("locked-project", &project)]);
    let database = data_home.join("opencode").join("opencode.db");
    let lock = Connection::open(&database).expect("open lock connection");
    lock.execute_batch("BEGIN EXCLUSIVE;")
        .expect("hold exclusive lock");

    let candidates = OpenCodeAdapter.discover_with_env(
        config.to_str(),
        None,
        data_home.to_str(),
        root.path().to_str(),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::OpencodeGlobalConfig);
    lock.execute_batch("ROLLBACK;").expect("release lock");
}

#[cfg(unix)]
#[test]
fn discovery_rejects_agents_md_symlink_escape() {
    use std::os::unix::fs::symlink;

    let root = tempdir().expect("root");
    let config = root.path().join("config");
    fs::create_dir_all(&config).expect("config");
    let outside = root.path().join("outside.md");
    fs::write(&outside, "# outside").expect("outside");
    symlink(&outside, config.join("AGENTS.md")).expect("symlink");

    let candidates =
        OpenCodeAdapter.discover_with_env(config.to_str(), None, None, root.path().to_str());
    assert!(candidates.is_empty());
}
