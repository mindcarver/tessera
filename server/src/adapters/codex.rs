//! `adapters::codex` — Codex Provider adapter.
//!
//! Story 1.2 ships the **discovery slice** of the Codex adapter: it resolves
//! the local Codex Agent Memory root and reports whether that root appears to
//! exist. It does NOT read memory content (NFR-5), does NOT enumerate files
//! (Story 1.5), and does NOT canonicalize the root or allocate `source_id`
//! (Story 1.3). The Codex data boundary is enforced by the Supported Artifact
//! Matrix (AD-11): transcripts, session JSONL and human-authored rule files
//! are out of scope here — at discovery time we only check the *memories
//! directory* exists, and even then we do not look at what is inside.
//!
//! ## `CODEX_HOME` priority
//!
//! Per spec I/O matrix:
//! - If `CODEX_HOME` is set AND non-empty (after trimming whitespace), probe
//!   `$CODEX_HOME/memories` only. Do NOT fall back to `~/.codex/memories` — an
//!   explicit `CODEX_HOME` means the user has relocated their Codex home and
//!   silently double-reporting the default location would violate capability
//!   honesty (AD-3) and produce a misleading candidate list. An explicit but
//!   invalid `CODEX_HOME` (relative, or pointing at a missing/non-dir path)
//!   yields no candidate rather than a silent fallback.
//! - Otherwise probe `$HOME/.codex/memories`.
//! - Empty / whitespace-only values are treated as "unset" (Design Notes).
//!
//! ## Root validity
//!
//! A candidate root must be an **absolute**, **existing directory** with a
//! **UTF-8** path:
//! - Absolute: a relative `CODEX_HOME`/`HOME` would resolve against the
//!   process CWD and produce a candidate whose root drifts between launches.
//!   `ponytail:` roots are absolute by nature; reject relative rather than
//!   emit CWD-dependent candidates.
//! - Directory (`is_dir`, not `exists`): a regular file named `memories` is
//!   not a usable root, and a broken symlink correctly reports false. A
//!   symlink-to-dir is followed and accepted.
//! - UTF-8: a non-UTF-8 root would stringify to `U+FFFD` replacement chars —
//!   a display path that does not exist on disk and cannot be confirmed in
//!   1.3. Drop rather than emit garbage.
//!
//! ## Testability
//!
//! Path resolution is factored into [`resolve_codex_memories_root`] as a pure
//! function of `(codex_home, home)` — no env reads. Discovery is factored into
//! [`CodexAdapter::discover_with_env`], which takes the same injected values
//! and runs the full path (resolver → directory check → candidate
//! construction). Both are deliberate: under `cargo test`'s parallel executor
//! `std::env::set_var` races other tests and is `unsafe` on edition 2024, so
//! tests inject tempdir paths directly and exercise the adapter's own code
//! (not a test-local mirror of it). `discover()` is three steps of glue: read
//! env → `discover_with_env`.

use std::env;
use std::path::{Path, PathBuf};

use crate::domain::ports::provider_adapter::{
    ArtifactDiagnostic, ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis,
    EnumerateError, FileUnit, ProviderAdapter, ProviderMemoryType, SupportedArtifact,
};

// Re-export the shared Markdown parser + helpers at the codex module path so
// existing call sites (`crate::adapters::codex::canonicalize_markdown`,
// `file_uri`, `percent_encode_fragment`, `safe_relative_path`,
// `CanonicalMarkdownUnit`, `MarkdownParseError`) keep working unchanged after
// the Story 2.2 extraction to `adapters::markdown`. New providers should
// import from `crate::adapters::markdown` directly. One parser, many version
// tags — no behavior change to Codex parsing.
pub use crate::adapters::markdown::{
    canonicalize_markdown, file_uri, percent_encode_fragment, safe_relative_path,
    CanonicalMarkdownUnit, MarkdownParseError,
};

/// Codex Provider adapter (Story 1.2 discovery slice; Story 1.4 adds the
/// enumeration slice).
///
/// Unit struct — both slices are stateless. The adapter reads provider files
/// but never writes them (NFR-1 zero-write); enumeration reads metadata only,
/// never body content (NFR-5).
#[derive(Debug, Clone, Copy, Default)]
pub struct CodexAdapter;

impl CodexAdapter {
    /// Canonical provider id for Codex (Story 2.1 review fix). The single
    /// source of truth referenced by both the multi-provider registry
    /// (`application::source::adapter_for`) and the Story 2.1 provider-
    /// scannable scan guard (`application::scan`), so a rename cannot desync
    /// the scan guard from the registry. Mirrors the const-on-adapter pattern
    /// the Codex slice already uses for `CODEX_MARKDOWN_PARSER_VERSION`.
    pub const PROVIDER_ID: &'static str = "codex";
}

/// The three known first-level filenames in the Supported Artifact Matrix
/// (AD-11). Only these exact names at the root's first level are indexed;
/// everything else at the first level is skipped.
const KNOWN_ROOT_FILES: &[&str] = &["MEMORY.md", "memory_summary.md", "raw_memories.md"];

/// The one directory whose direct `*.md` children are indexed (AD-11). Only
/// direct children (one level, not recursive) are considered.
const ROLLOUT_SUMMARIES_DIR: &str = "rollout_summaries";

impl ProviderAdapter for CodexAdapter {
    fn provider_id(&self) -> &'static str {
        Self::PROVIDER_ID
    }

    fn coverage_level(&self) -> CoverageLevel {
        // Codex's memories root is a local directory tree that the adapter
        // will be able to enumerate in full once `enumerate` lands in Story
        // 1.5. Declaring `Full` here is honest capability disclosure (AD-3 /
        // AD-18): it describes the *provider surface*, not the slice that
        // ships in this Story. The actual enumeration implementation belongs
        // to 1.5; declaring Full now does not bypass that — the trait only
        // grows the discovery slice in 1.2.
        CoverageLevel::Full
    }

    fn parser_version(&self) -> &'static str {
        CODEX_MARKDOWN_PARSER_VERSION
    }

    fn discover(&self) -> Vec<CandidateSource> {
        // Three-step glue (Design Notes): read env → injected-env discover.
        // All input validation (trim, absolute) lives in the resolver; all FS
        // checks + candidate building live in `candidate_if_existing_dir`.
        // `discover_with_env` is the testable seam tests drive directly.
        let codex_home = env::var("CODEX_HOME").ok();
        let home = env::var("HOME").ok();
        self.discover_with_env(codex_home.as_deref(), home.as_deref())
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
        enumerate_codex_artifacts(root)
    }
}

impl CodexAdapter {
    /// Discovery driven by injected env values rather than `std::env`, so the
    /// full discover path (resolver → directory check → candidate construction)
    /// is exercisable under `cargo test` parallelism without env mutation.
    /// [`ProviderAdapter::discover`] reads `CODEX_HOME`/`HOME` and delegates
    /// here.
    pub fn discover_with_env(
        &self,
        codex_home: Option<&str>,
        home: Option<&str>,
    ) -> Vec<CandidateSource> {
        match resolve_codex_memories_root(codex_home, home) {
            Some((basis, root)) => self.candidate_if_existing_dir(basis, &root),
            None => Vec::new(),
        }
    }

    /// Build the single candidate for a resolved root iff the root is an
    /// existing UTF-8 directory. A regular file, a missing path, a broken
    /// symlink, or a non-UTF-8 root yields no candidate. NFR-5: this is an
    /// existence/type check only — directory contents are never read.
    fn candidate_if_existing_dir(
        &self,
        basis: DiscoveryBasis,
        root: &Path,
    ) -> Vec<CandidateSource> {
        if !root.is_dir() {
            return Vec::new();
        }
        // Non-UTF-8 root: `to_string_lossy` would emit U+FFFD, producing a
        // display path that does not exist on disk and cannot be confirmed
        // (1.3). Drop instead of emitting garbage.
        let Some(path_str) = root.to_str() else {
            return Vec::new();
        };
        vec![CandidateSource {
            provider: Self::PROVIDER_ID.to_string(),
            root_path: path_str.to_string(),
            basis,
            coverage_level: self.coverage_level(),
            // Codex memories are a global store with no discoverable
            // per-project split (Design Notes). Story 1.5 may revisit if the
            // parsed content lets us infer a project boundary; until then,
            // honestly report "unknown".
            native_project: None,
        }]
    }
}

/// Pure path resolver for the Codex memories root.
///
/// Extracted from [`CodexAdapter::discover`] so tests can exercise the
/// `CODEX_HOME`-priority rule without touching the process environment
/// (env-mutation races under `cargo test` parallelism; edition 2024 marks
/// `set_var` unsafe). The function does no FS I/O — `discover_with_env` ->
/// `candidate_if_existing_dir` applies the directory check to the returned
/// path.
///
/// Validation (applied here, the single source of truth):
/// - Empty / whitespace-only values are treated as unset.
/// - Values must be absolute; a relative root would resolve against the
///   process CWD and drift between launches, so it is rejected (returns
///   `None`, never a fallback for an explicit-but-invalid `CODEX_HOME`).
///
/// Returns `None` when neither a usable `codex_home` nor a usable `home` is
/// supplied, or when a supplied value fails validation.
pub fn resolve_codex_memories_root(
    codex_home: Option<&str>,
    home: Option<&str>,
) -> Option<(DiscoveryBasis, PathBuf)> {
    // Priority 1: explicit CODEX_HOME (non-empty after trim, absolute). No
    // fallback to ~/.codex — an explicit override is final, even when invalid.
    if let Some(ch) = codex_home.filter(|s| !s.trim().is_empty()) {
        let p = Path::new(ch);
        if p.is_absolute() {
            return Some((DiscoveryBasis::CodexHomeEnv, p.join("memories")));
        }
        return None;
    }
    // Priority 2: default home (non-empty after trim, absolute).
    let home = home.filter(|s| !s.trim().is_empty())?;
    let p = Path::new(home);
    if !p.is_absolute() {
        return None;
    }
    Some((
        DiscoveryBasis::DefaultHome,
        p.join(".codex").join("memories"),
    ))
}

/// Parser-version contract. Output changes require a deliberate version
/// decision rather than silently changing record identities or bodies.
pub const CODEX_MARKDOWN_PARSER_VERSION: &str = "codex-markdown/v1";

/// Enumerate allowlisted Codex artifacts and lexical unknowns. A known
/// artifact that cannot be resolved or inspected is terminal; only a proven
/// root escape or resolved-role mismatch is silently excluded.
///
/// Story 4.2: every canonicalize/read_dir/metadata failure site classifies
/// the io error by `kind()` via [`EnumerateError::from_root_io_error`] /
/// [`EnumerateError::from_dir_io_error`] so the four health-cause categories
/// (path missing / permission denied / format unsupported / scan failed) are
/// genuinely distinguishable at the I/O boundary, not guessed from strings.
fn enumerate_codex_artifacts(root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
    let canonical_root = std::fs::canonicalize(root).map_err(EnumerateError::from_root_io_error)?;
    let entries =
        std::fs::read_dir(&canonical_root).map_err(EnumerateError::from_dir_io_error)?;
    let mut supported = Vec::new();
    let mut diagnostics = Vec::new();

    for entry in entries {
        let entry = entry.map_err(EnumerateError::from_dir_io_error)?;
        let lexical_path = entry.path();
        let name = entry.file_name();
        let name_utf8 = name.to_str();
        match name_utf8 {
            Some(name) if KNOWN_ROOT_FILES.contains(&name) => {
                let expected = root_memory_type(name).expect("known root artifact");
                if let Some(artifact) =
                    resolve_supported_artifact(&canonical_root, &lexical_path, expected, false)?
                {
                    supported.push(artifact);
                }
            }
            Some(ROLLOUT_SUMMARIES_DIR) => {
                let real_dir = match std::fs::canonicalize(&lexical_path) {
                    Ok(path) => path,
                    // `rollout_summaries/` is a directory inside the root, so
                    // its io kind feeds the dir classifier (Story 4.2).
                    Err(err) => return Err(EnumerateError::from_dir_io_error(err)),
                };
                if !real_dir.starts_with(&canonical_root)
                    || real_dir.strip_prefix(&canonical_root).ok()
                        != Some(Path::new(ROLLOUT_SUMMARIES_DIR))
                    || !std::fs::metadata(&real_dir)
                        .map_err(EnumerateError::from_dir_io_error)?
                        .is_dir()
                {
                    continue;
                }
                let rollout_entries = std::fs::read_dir(&real_dir)
                    .map_err(EnumerateError::from_dir_io_error)?;
                for rollout_entry in rollout_entries {
                    let rollout_entry =
                        rollout_entry.map_err(EnumerateError::from_dir_io_error)?;
                    let rollout_path = rollout_entry.path();
                    let observed = safe_relative_path(&canonical_root, &rollout_path);
                    let is_markdown = rollout_entry
                        .file_name()
                        .to_str()
                        .is_some_and(|entry_name| entry_name.ends_with(".md"));
                    if !is_markdown {
                        diagnostics.push(unsupported_diagnostic(observed));
                        continue;
                    }
                    if let Some(artifact) = resolve_supported_artifact(
                        &canonical_root,
                        &rollout_path,
                        ProviderMemoryType::RolloutSummary,
                        true,
                    )? {
                        supported.push(artifact);
                    }
                }
            }
            _ => diagnostics.push(unsupported_diagnostic(safe_relative_path(
                &canonical_root,
                &lexical_path,
            ))),
        }
    }

    supported.sort_by(|a, b| a.file.relative_path.cmp(&b.file.relative_path));
    supported.dedup_by(|a, b| a.file.relative_path == b.file.relative_path);
    diagnostics.sort();
    diagnostics.dedup();
    Ok(ArtifactEnumeration {
        supported,
        diagnostics,
    })
}

// Retained as a narrow Story 1.4 compatibility seam for the adapter's
// existing unit tests. Production scanning uses `enumerate_codex_artifacts`
// so it can validate diagnostics and exact artifact roles as one boundary.
#[cfg(test)]
fn enumerate_codex_file_units(root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
    Ok(enumerate_codex_artifacts(root)?
        .supported
        .into_iter()
        .map(|artifact| artifact.file)
        .collect())
}

fn root_memory_type(name: &str) -> Option<ProviderMemoryType> {
    match name {
        "MEMORY.md" => Some(ProviderMemoryType::Memory),
        "memory_summary.md" => Some(ProviderMemoryType::MemorySummary),
        "raw_memories.md" => Some(ProviderMemoryType::RawMemories),
        _ => None,
    }
}

fn resolve_supported_artifact(
    canonical_root: &Path,
    lexical_path: &Path,
    expected_type: ProviderMemoryType,
    is_rollout_child: bool,
) -> Result<Option<SupportedArtifact>, EnumerateError> {
    let real = match std::fs::canonicalize(lexical_path) {
        Ok(path) => path,
        Err(_) => return Err(EnumerateError::AllowlistedArtifactUnresolvable),
    };
    let Ok(relative) = real.strip_prefix(canonical_root) else {
        return Ok(None);
    };
    let role_matches = if is_rollout_child {
        relative.parent() == Some(Path::new(ROLLOUT_SUMMARIES_DIR))
            && relative.extension().and_then(|value| value.to_str()) == Some("md")
    } else {
        relative
            .to_str()
            .and_then(root_memory_type)
            .is_some_and(|actual| actual == expected_type)
    };
    if !role_matches {
        return Ok(None);
    }
    let metadata =
        std::fs::metadata(&real).map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?;
    if !metadata.is_file() {
        return Err(EnumerateError::AllowlistedArtifactUnresolvable);
    }
    let relative_path = relative
        .to_str()
        .ok_or(EnumerateError::AllowlistedArtifactUnresolvable)?
        .to_string();
    let mtime = metadata
        .modified()
        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| EnumerateError::AllowlistedArtifactUnresolvable)?
        .as_nanos() as i64;
    Ok(Some(SupportedArtifact {
        file: FileUnit {
            relative_path,
            absolute_path: real,
            size: metadata.len(),
            mtime,
        },
        memory_type: expected_type,
    }))
}

fn unsupported_diagnostic(observed_path: String) -> ArtifactDiagnostic {
    ArtifactDiagnostic {
        kind: "unsupported_artifact",
        observed_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pure resolver: CODEX_HOME non-empty wins, returns memories under it.
    /// No fallback to ~/.codex (AD-3 honesty — explicit override is final).
    #[test]
    fn resolver_prefers_codex_home_when_set() {
        let (basis, root) = resolve_codex_memories_root(Some("/x"), Some("/home/u"))
            .expect("codex_home set → Some");
        assert_eq!(basis, DiscoveryBasis::CodexHomeEnv);
        assert_eq!(root, PathBuf::from("/x/memories"));
    }

    /// Empty CODEX_HOME is treated as unset → falls back to default home.
    #[test]
    fn resolver_treats_empty_codex_home_as_unset() {
        let (basis, root) =
            resolve_codex_memories_root(Some(""), Some("/home/u")).expect("home set → Some");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
        assert_eq!(root, PathBuf::from("/home/u/.codex/memories"));
    }

    /// Whitespace-only CODEX_HOME is treated as unset (same as empty).
    #[test]
    fn resolver_treats_whitespace_codex_home_as_unset() {
        let (basis, _) = resolve_codex_memories_root(Some("   "), Some("/home/u"))
            .expect("whitespace codex_home → fallback to home");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
    }

    /// Relative CODEX_HOME is rejected (would resolve against CWD); it does
    /// NOT fall back to ~/.codex — an explicit override is final.
    #[test]
    fn resolver_rejects_relative_codex_home_without_fallback() {
        // Even when a valid default home is supplied, an explicit (relative)
        // CODEX_HOME yields None rather than silently using the default.
        assert!(resolve_codex_memories_root(Some("relative/path"), Some("/home/u")).is_none());
    }

    /// Relative HOME is rejected (would resolve against CWD).
    #[test]
    fn resolver_rejects_relative_home() {
        assert!(resolve_codex_memories_root(None, Some("./home")).is_none());
    }

    /// No CODEX_HOME but HOME set → default home basis.
    #[test]
    fn resolver_falls_back_to_default_home() {
        let (basis, root) =
            resolve_codex_memories_root(None, Some("/home/u")).expect("home set → Some");
        assert_eq!(basis, DiscoveryBasis::DefaultHome);
        assert_eq!(root, PathBuf::from("/home/u/.codex/memories"));
    }

    /// Neither CODEX_HOME nor HOME → None. No candidate, no error.
    #[test]
    fn resolver_returns_none_when_neither_env_set() {
        assert!(resolve_codex_memories_root(None, None).is_none());
        // Empty/whitespace strings count as unset too.
        assert!(resolve_codex_memories_root(Some(""), Some("")).is_none());
        assert!(resolve_codex_memories_root(Some("  "), None).is_none());
        // CODEX_HOME set but HOME empty/whitespace: CODEX_HOME path is used.
        let (basis, root) = resolve_codex_memories_root(Some("/x"), Some(""))
            .expect("codex_home set with empty home still resolves");
        assert_eq!(basis, DiscoveryBasis::CodexHomeEnv);
        assert_eq!(root, PathBuf::from("/x/memories"));
    }

    /// Adapter declares Codex capability honestly (AD-3).
    #[test]
    fn adapter_declares_codex_id_and_full_coverage() {
        let adapter = CodexAdapter;
        assert_eq!(adapter.provider_id(), "codex");
        assert_eq!(adapter.coverage_level(), CoverageLevel::Full);
    }

    // --- Story 1.4: enumerate_file_units (AD-11 boundary) ------------------

    /// Enumeration indexes only in-matrix files: the three known root files +
    /// `rollout_summaries/*.md` direct children. Excluded files (sessions,
    /// JSONL, CLAUDE.md, unknown names) are skipped (AD-11).
    #[test]
    fn enumerate_indexes_only_supported_artifact_matrix() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w");
        std::fs::write(root.join("memory_summary.md"), "sum").expect("w");
        std::fs::write(root.join("raw_memories.md"), "raw").expect("w");
        // Excluded: human instruction file, unknown name, sessions dir + JSONL.
        std::fs::write(root.join("CLAUDE.md"), "rules").expect("w");
        std::fs::write(root.join("unknown.md"), "unknown").expect("w");
        std::fs::create_dir_all(root.join("sessions")).expect("mkdir sessions");
        std::fs::write(root.join("sessions").join("foo.jsonl"), "{}").expect("w");
        // rollout_summaries with an .md and a non-.md child.
        std::fs::create_dir_all(root.join("rollout_summaries")).expect("mkdir rollout");
        std::fs::write(root.join("rollout_summaries").join("2026-07-01.md"), "r").expect("w");
        std::fs::write(root.join("rollout_summaries").join("notes.txt"), "r").expect("w");

        let units = enumerate_codex_file_units(root).expect("enumerate ok");
        let mut rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        rels.sort_unstable();
        assert_eq!(
            rels,
            vec![
                "MEMORY.md",
                "memory_summary.md",
                "raw_memories.md",
                "rollout_summaries/2026-07-01.md",
            ],
            "only in-matrix files are enumerated"
        );
    }

    /// `rollout_summaries/` is NOT recursive: a nested subdirectory's `.md` is
    /// skipped (one-level rule, spec Design Notes "枚举边界").
    #[test]
    fn enumerate_rollout_summaries_is_not_recursive() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        let nested = root.join("rollout_summaries").join("nested");
        std::fs::create_dir_all(&nested).expect("mkdir nested");
        std::fs::write(root.join("rollout_summaries").join("top.md"), "r").expect("w");
        std::fs::write(nested.join("deep.md"), "r").expect("w");

        let units = enumerate_codex_file_units(root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(rels, vec!["rollout_summaries/top.md"], "nested .md skipped");
    }

    /// A symlinked file whose realpath escapes the canonical root is skipped
    /// (AD-4 / spec Block If — symlink escape). The in-root file is kept.
    #[cfg(unix)]
    #[test]
    fn enumerate_skips_symlink_escape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        // Real in-root file.
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w");
        // A file outside the root, symlinked in as memory_summary.md.
        std::fs::write(outside.join("secret.md"), "secret").expect("w");
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("memory_summary.md"))
            .expect("symlink");

        let units = enumerate_codex_file_units(&root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(rels, vec!["MEMORY.md"], "escaping symlink skipped");
    }

    /// A symlinked `rollout_summaries` directory that resolves outside the
    /// confirmed root is skipped before it is opened; only in-root supported
    /// artifacts remain enumerable.
    #[cfg(unix)]
    #[test]
    fn enumerate_skips_rollout_directory_symlink_escape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("root");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::fs::write(root.join("MEMORY.md"), "mem").expect("w root");
        std::fs::write(outside.join("secret.md"), "secret").expect("w outside");
        std::os::unix::fs::symlink(&outside, root.join(ROLLOUT_SUMMARIES_DIR))
            .expect("symlink rollout dir");

        let units = enumerate_codex_file_units(&root).expect("enumerate ok");
        let rels: Vec<&str> = units.iter().map(|u| u.relative_path.as_str()).collect();
        assert_eq!(
            rels,
            vec!["MEMORY.md"],
            "external rollout directory skipped"
        );
    }

    /// Enumeration of an empty directory succeeds with zero units (spec I/O
    /// matrix — empty directory scan is a complete success).
    #[test]
    fn enumerate_empty_directory_succeeds_with_zero_units() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let units = enumerate_codex_file_units(tmp.path()).expect("enumerate ok");
        assert!(units.is_empty());
    }

    /// Enumeration of a non-existent root returns Err (root unresolvable).
    #[test]
    fn enumerate_fails_for_missing_root() {
        let bogus = Path::new("/this/does/not/exist/tessera-1-4-enum");
        assert!(enumerate_codex_file_units(bogus).is_err());
    }
}
