//! Read-only OpenCode persistent-instruction provider.
//!
//! OpenCode has no first-party memory directory or memory table. Tessera
//! therefore treats only the provider-owned `AGENTS.md` instruction files as
//! durable memory artifacts:
//! - `<config_dir>/AGENTS.md` for the global scope;
//! - `<project.worktree>/AGENTS.md` for project rows read from
//!   `<data_dir>/opencode.db`.
//!
//! Discovery opens the database with `SQLITE_OPEN_READ_ONLY` and issues one
//! narrow query selecting only `project.id` and `project.worktree`. Session,
//! message, part, prompt, auth, log, and body surfaces are never read.

use std::env;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags};

use crate::domain::ports::provider_adapter::{
    ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis, EnumerateError, FileUnit,
    ProviderAdapter, ProviderMemoryType, SupportedArtifact,
};
use crate::domain::project::MAX_NATIVE_PROJECT_LEN;

const INSTRUCTION_FILE: &str = "AGENTS.md";
const DATABASE_FILE: &str = "opencode.db";

#[derive(Debug, Clone, Copy, Default)]
pub struct OpenCodeAdapter;

impl OpenCodeAdapter {
    pub const PROVIDER_ID: &'static str = "opencode";
    pub const PARSER_VERSION: &'static str = "opencode-agents-md/v1";

    /// Testable discovery seam. Values mirror the corresponding environment
    /// variables; no process-global environment mutation is required.
    pub fn discover_with_env(
        &self,
        opencode_config_dir: Option<&str>,
        xdg_config_home: Option<&str>,
        xdg_data_home: Option<&str>,
        home: Option<&str>,
    ) -> Vec<CandidateSource> {
        let mut candidates = Vec::new();

        if let Some(config_dir) = resolve_config_dir(opencode_config_dir, xdg_config_home, home) {
            if has_contained_instruction_file(&config_dir) {
                candidates.push(CandidateSource {
                    provider: Self::PROVIDER_ID.to_string(),
                    root_path: config_dir.to_string_lossy().into_owned(),
                    basis: DiscoveryBasis::OpencodeGlobalConfig,
                    coverage_level: CoverageLevel::Full,
                    native_project: None,
                });
            }
        }

        if let Some(data_dir) = resolve_data_dir(xdg_data_home, home) {
            candidates.extend(project_candidates(&data_dir, self.coverage_level()));
        }

        let mut identities_by_root =
            std::collections::HashMap::<PathBuf, std::collections::BTreeSet<Option<String>>>::new();
        for candidate in &candidates {
            if let Ok(root) = std::fs::canonicalize(&candidate.root_path) {
                identities_by_root
                    .entry(root)
                    .or_default()
                    .insert(candidate.native_project.clone());
            }
        }
        candidates.retain(|candidate| {
            std::fs::canonicalize(&candidate.root_path)
                .ok()
                .and_then(|root| identities_by_root.get(&root))
                .is_some_and(|identities| identities.len() == 1)
        });

        candidates.sort_by(|a, b| {
            (
                a.root_path.as_str(),
                a.native_project.as_deref().unwrap_or_default(),
            )
                .cmp(&(
                    b.root_path.as_str(),
                    b.native_project.as_deref().unwrap_or_default(),
                ))
        });
        candidates
            .dedup_by(|a, b| a.root_path == b.root_path && a.native_project == b.native_project);
        candidates
    }

    /// Re-derive OpenCode identity from current provider metadata.
    ///
    /// The outer `Option` distinguishes "exactly one current identity" from
    /// missing/ambiguous metadata; the inner option is the provider-native
    /// project (`None` for the global config source).
    pub fn native_project_for_root_with_env(
        &self,
        root: &Path,
        opencode_config_dir: Option<&str>,
        xdg_config_home: Option<&str>,
        xdg_data_home: Option<&str>,
        home: Option<&str>,
    ) -> Option<Option<String>> {
        let canonical_root = std::fs::canonicalize(root).ok()?;
        let identities = self
            .discover_with_env(opencode_config_dir, xdg_config_home, xdg_data_home, home)
            .into_iter()
            .filter_map(|candidate| {
                let candidate_root = std::fs::canonicalize(candidate.root_path).ok()?;
                (candidate_root == canonical_root).then_some(candidate.native_project)
            })
            .collect::<Vec<_>>();

        match identities.as_slice() {
            [identity] => Some(identity.clone()),
            _ => None,
        }
    }

    pub fn native_project_for_current_root(&self, root: &Path) -> Option<Option<String>> {
        let opencode_config_dir = env::var("OPENCODE_CONFIG_DIR").ok();
        let xdg_config_home = env::var("XDG_CONFIG_HOME").ok();
        let xdg_data_home = env::var("XDG_DATA_HOME").ok();
        let home = env::var("HOME").ok();
        self.native_project_for_root_with_env(
            root,
            opencode_config_dir.as_deref(),
            xdg_config_home.as_deref(),
            xdg_data_home.as_deref(),
            home.as_deref(),
        )
    }
}

impl ProviderAdapter for OpenCodeAdapter {
    fn provider_id(&self) -> &'static str {
        Self::PROVIDER_ID
    }

    fn coverage_level(&self) -> CoverageLevel {
        CoverageLevel::Full
    }

    fn parser_version(&self) -> &'static str {
        Self::PARSER_VERSION
    }

    fn discover(&self) -> Vec<CandidateSource> {
        let opencode_config_dir = env::var("OPENCODE_CONFIG_DIR").ok();
        let xdg_config_home = env::var("XDG_CONFIG_HOME").ok();
        let xdg_data_home = env::var("XDG_DATA_HOME").ok();
        let home = env::var("HOME").ok();
        self.discover_with_env(
            opencode_config_dir.as_deref(),
            xdg_config_home.as_deref(),
            xdg_data_home.as_deref(),
            home.as_deref(),
        )
    }

    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        Ok(self
            .enumerate_artifacts(root)?
            .supported
            .into_iter()
            .map(|artifact| artifact.file)
            .collect())
    }

    fn enumerate_artifacts(&self, root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
        let canonical_root =
            std::fs::canonicalize(root).map_err(EnumerateError::from_root_io_error)?;
        let lexical_file = canonical_root.join(INSTRUCTION_FILE);
        let canonical_file = std::fs::canonicalize(&lexical_file)
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
        let relative = canonical_file
            .strip_prefix(&canonical_root)
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
        if relative != Path::new(INSTRUCTION_FILE) {
            return Err(EnumerateError::AllowlistedArtifactUnresolvable);
        }
        let metadata = std::fs::metadata(&canonical_file)
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
        if !metadata.is_file() {
            return Err(EnumerateError::AllowlistedArtifactUnresolvable);
        }
        let mtime = metadata
            .modified()
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
            .as_nanos()
            .try_into()
            .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;

        Ok(ArtifactEnumeration {
            supported: vec![SupportedArtifact {
                file: FileUnit {
                    relative_path: INSTRUCTION_FILE.to_string(),
                    absolute_path: canonical_file,
                    size: metadata.len(),
                    mtime,
                },
                memory_type: ProviderMemoryType::AgentInstruction,
            }],
            diagnostics: Vec::new(),
        })
    }
}

fn resolve_config_dir(
    opencode_config_dir: Option<&str>,
    xdg_config_home: Option<&str>,
    home: Option<&str>,
) -> Option<PathBuf> {
    if let Some(value) = nonempty(opencode_config_dir) {
        return absolute_path(value);
    }
    if let Some(value) = nonempty(xdg_config_home) {
        return absolute_path(value).map(|path| path.join("opencode"));
    }
    absolute_path(nonempty(home)?).map(|path| path.join(".config").join("opencode"))
}

fn resolve_data_dir(xdg_data_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    if let Some(value) = nonempty(xdg_data_home) {
        return absolute_path(value).map(|path| path.join("opencode"));
    }
    absolute_path(nonempty(home)?).map(|path| path.join(".local").join("share").join("opencode"))
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.filter(|value| !value.trim().is_empty())
}

fn absolute_path(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(value);
    path.is_absolute().then_some(path)
}

fn has_contained_instruction_file(root: &Path) -> bool {
    if !root.is_absolute() || !root.is_dir() {
        return false;
    }
    let Ok(canonical_root) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(canonical_file) = std::fs::canonicalize(root.join(INSTRUCTION_FILE)) else {
        return false;
    };
    canonical_file
        .strip_prefix(&canonical_root)
        .is_ok_and(|relative| relative == Path::new(INSTRUCTION_FILE))
        && canonical_file.is_file()
}

fn project_candidates(data_dir: &Path, coverage_level: CoverageLevel) -> Vec<CandidateSource> {
    let db_path = data_dir.join(DATABASE_FILE);
    if !db_path.is_file() {
        return Vec::new();
    }

    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let Ok(connection) = Connection::open_with_flags(&db_path, flags) else {
        return Vec::new();
    };
    let Ok(mut statement) = connection.prepare("SELECT id, worktree FROM project") else {
        return Vec::new();
    };
    let Ok(rows) = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    }) else {
        return Vec::new();
    };

    let Ok(rows) = rows.collect::<rusqlite::Result<Vec<_>>>() else {
        return Vec::new();
    };

    rows.into_iter()
        .filter_map(|(project_id, worktree)| {
            if project_id.is_empty()
                || project_id.trim() != project_id
                || project_id.len() > MAX_NATIVE_PROJECT_LEN
            {
                return None;
            }
            let root = PathBuf::from(worktree);
            if !has_contained_instruction_file(&root) {
                return None;
            }
            Some(CandidateSource {
                provider: OpenCodeAdapter::PROVIDER_ID.to_string(),
                root_path: root.to_str()?.to_string(),
                basis: DiscoveryBasis::OpencodeProjectDatabase,
                coverage_level,
                native_project: Some(project_id),
            })
        })
        .collect()
}
