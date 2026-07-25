//! `adapters::claude_code` — Claude Code Provider adapter (Epic 2, Story 2.1).
//!
//! Story 2.1 ships the **discovery slice** of the Claude Code adapter. It
//! resolves Claude Code's official `<config_dir>/projects/<project>/memory/`
//! roots and reports one candidate per existing project memory dir, PLUS any
//! user-configured `autoMemoryDirectory` declared in the user-scope
//! `<config_dir>/settings.json`. It does NOT read memory content (NFR-5),
//! does NOT enumerate files (Story 2.2), and does NOT canonicalize the root
//! or allocate `source_id` (the confirm pipeline in `application::source`
//! does both, exactly as it does for Codex). The Supported Artifact Matrix
//! (AD-11) — `MEMORY.md` and topic Markdown under a project memory dir,
//! excluding `CLAUDE.md`, `AGENTS.md`, `.claude/rules`, session/transcript
//! content, and any manually-added directory — is enforced at parse time in
//! Story 2.2, not at discovery time; here we only check the *memory dir*
//! exists.
//!
//! ## `CLAUDE_CONFIG_DIR` priority
//!
//! Per spec I/O matrix (mirrors Codex's `CODEX_HOME` rule):
//! - If `CLAUDE_CONFIG_DIR` is set AND non-empty (after trimming whitespace),
//!   probe `$CLAUDE_CONFIG_DIR/projects/<project>/memory/` (and read
//!   `$CLAUDE_CONFIG_DIR/settings.json` for `autoMemoryDirectory`) only. Do
//!   NOT fall back to `~/.claude/...` — an explicit `CLAUDE_CONFIG_DIR` means
//!   the user has relocated their Claude home and silently double-reporting
//!   the default location would violate capability honesty (AD-3) and produce
//!   a misleading candidate list. An explicit but invalid `CLAUDE_CONFIG_DIR`
//!   (relative, or pointing at a missing/non-dir path) yields no candidate
//!   rather than a silent fallback.
//! - Otherwise probe `$HOME/.claude/projects/<project>/memory/` (and read
//!   `$HOME/.claude/settings.json`).
//! - Empty / whitespace-only values are treated as "unset" (Design Notes).
//!
//! ## `autoMemoryDirectory` (user scope)
//!
//! Claude Code's official memory docs (`code.claude.com/docs/en/memory`,
//! "Storage location") document `autoMemoryDirectory` as a real
//! `settings.json` key whose value is an absolute path or `~/`-prefixed;
//! it relocates where Claude writes auto-memory. Story 2.1 honors it at the
//! **user scope only** (`<config_dir>/settings.json`). Project-scope
//! `.claude/settings.json` is intentionally NOT read — Tessera has no
//! project context at discovery time. Settings parsing:
//! - The file is config, not memory content (NFR-5 permits the read).
//! - `//` line comments are stripped before `serde_json::from_str` so a
//!   JSONC-style file still parses. A genuine parse failure is a safe
//!   degrade (no candidate, no error).
//! - Only an `autoMemoryDirectory` value that is absolute OR `~/`-prefixed
//!   is honored; relative values are invalid and safe-degrade. `~/` is
//!   expanded via `HOME`.
//! - The resolved path must be an existing UTF-8 directory; the resulting
//!   candidate carries `basis=ClaudeAutoMemoryDir`, `native_project=None`,
//!   `coverage=Full`.
//! - Dedup against the `projects/*` candidates is by **canonicalized path**:
//!   if `autoMemoryDirectory` resolves to the same physical dir as a
//!   `projects/<P>/memory/` candidate, exactly one candidate is emitted for
//!   that dir (the project-keyed one wins — it carries the project key).
//!
//! ## Multi-candidate-per-provider
//!
//! Unlike Codex (0..1 candidate), Claude Code emits 0..N candidates — one per
//! existing project memory dir, plus optionally `autoMemoryDirectory`. Each
//! candidate carries the encoded `<project>` key as `native_project`
//! verbatim (no reverse-mapping to a real repo path; that protocol is not
//! stable and is owned by Epic 5). Candidates are sorted by `root_path` so
//! discovery is deterministic.
//!
//! ## Root validity
//!
//! A candidate's memory dir must be an **absolute**, **existing directory**
//! with a **UTF-8** path (same rules as Codex):
//! - Absolute: a relative `CLAUDE_CONFIG_DIR`/`HOME` would resolve against
//!   the process CWD and produce candidates whose roots drift between
//!   launches. Reject relative rather than emit CWD-dependent candidates.
//! - Directory (`is_dir`, not `exists`): a regular file named `memory` is not
//!   a usable root; a broken symlink correctly reports false. A symlink-to-dir
//!   is followed and accepted.
//! - UTF-8: a non-UTF-8 root would stringify to `U+FFFD` replacement chars —
//!   a display path that does not exist on disk and cannot be confirmed. Drop
//!   rather than emit garbage.
//!
//! ## Testability
//!
//! Path resolution is factored into [`resolve_claude_projects_dir`] as a pure
//! function of `(claude_config_dir, home)` — no env reads. Discovery is
//! factored into [`ClaudeCodeAdapter::discover_with_env`], which takes the
//! same injected values and runs the full path (resolver → directory walk →
//! `settings.json` read → candidate construction). Both are deliberate:
//! under `cargo test`'s parallel executor `std::env::set_var` races other
//! tests and is `unsafe` on edition 2024, so tests inject tempdir paths
//! directly and exercise the adapter's own code. `discover()` is three steps
//! of glue: read env → `discover_with_env`.
//!
//! ## Story 2.2 — enumeration + read-only indexing
//!
//! [`ProviderAdapter::enumerate_file_units`] and
//! [`ProviderAdapter::enumerate_artifacts`] parse a confirmed Claude `memory/`
//! dir into canonical records by walking the **Supported Artifact Matrix**
//! (A-18) for Claude Code:
//! - Index **every direct-child `*.md`** of the confirmed `memory/` dir
//!   (`MEMORY.md` + topic Markdown). **No recursion, no subdirectory walking**
//!   (verified real layout: flat, `.md`-only, no subdirs).
//! - `MEMORY.md` is tagged [`ProviderMemoryType::Memory`] (the auto-managed
//!   index); every other direct-child `*.md` is tagged
//!   [`ProviderMemoryType::TopicMemory`] (a distinct topic type for honest
//!   2.3/2.4 filtering).
//! - `CLAUDE.md` and `AGENTS.md` are **rejected by name** as
//!   `unsupported_artifact` diagnostics — they are human instruction files,
//!   not memory. Non-`*.md` files and subdirectories are likewise rejected as
//!   `unsupported_artifact` diagnostics; never indexed, never recursed.
//! - The same realpath containment (symlink-escape) check Codex uses is
//!   applied: a `*.md` child whose realpath escapes the canonical root is
//!   skipped.
//!
//! Parsing reuses the shared, generic Markdown parser
//! ([`crate::adapters::markdown::canonicalize_markdown`]) — the same parser
//! Codex uses. Only the persisted `parser_version` tag differs
//! (`claude-markdown/v1`). Claude records flow through Epic 1's atomic
//! generational pipeline unchanged.

use std::env;
use std::path::{Path, PathBuf};

use crate::domain::ports::provider_adapter::{
    ArtifactDiagnostic, ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis,
    EnumerateError, FileUnit, ProviderAdapter, ProviderMemoryType, SupportedArtifact,
};

/// Claude Code Provider adapter (Story 2.1 discovery slice; Story 2.2 adds
/// the enumeration + parsing slice).
///
/// Unit struct — the slice is stateless. The adapter reads provider files
/// but never writes them (NFR-1 zero-write); discovery checks directory
/// existence only, never body content (NFR-5).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCodeAdapter;

impl ClaudeCodeAdapter {
    /// Canonical provider id for Claude Code.
    pub const PROVIDER_ID: &'static str = "claude_code";

    /// Parser-version contract for the Claude Code Markdown parser. Claude's
    /// `MEMORY.md` and topic `*.md` reuse the shared generic Markdown parser
    /// (the same `canonicalize_markdown` Codex uses); a distinct version tag
    /// lets a future Claude-specific grammar bump trigger a reparse without
    /// touching Codex identity.
    pub const PARSER_VERSION: &'static str = "claude-markdown/v1";
}

const PROVIDER_ID: &str = ClaudeCodeAdapter::PROVIDER_ID;

impl ProviderAdapter for ClaudeCodeAdapter {
    fn provider_id(&self) -> &'static str {
        Self::PROVIDER_ID
    }

    fn coverage_level(&self) -> CoverageLevel {
        // Claude Code's `projects/*/memory/` is a local directory tree that
        // the adapter fully enumerates as of Story 2.2 (flat `*.md` only, no
        // recursion). `Full` is honest capability disclosure (AD-3 / AD-18).
        CoverageLevel::Full
    }

    fn parser_version(&self) -> &'static str {
        Self::PARSER_VERSION
    }

    fn discover(&self) -> Vec<CandidateSource> {
        // Three-step glue (mirrors Codex): read env → injected-env discover.
        // All input validation (trim, absolute) lives in the resolver; all FS
        // checks + candidate building live in `candidates_for_projects` and
        // `auto_memory_candidate`.
        // `discover_with_env` is the testable seam tests drive directly.
        let claude_config_dir = env::var("CLAUDE_CONFIG_DIR").ok();
        let home = env::var("HOME").ok();
        self.discover_with_env(claude_config_dir.as_deref(), home.as_deref())
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
        enumerate_claude_artifacts(root)
    }
}

impl ClaudeCodeAdapter {
    /// Discovery driven by injected env values rather than `std::env`, so the
    /// full discover path (resolver → directory walk → settings read →
    /// candidate construction) is exercisable under `cargo test` parallelism
    /// without env mutation. [`ProviderAdapter::discover`] reads
    /// `CLAUDE_CONFIG_DIR`/`HOME` and delegates here.
    pub fn discover_with_env(
        &self,
        claude_config_dir: Option<&str>,
        home: Option<&str>,
    ) -> Vec<CandidateSource> {
        let Some((basis, config_dir)) = resolve_claude_config_dir(claude_config_dir, home) else {
            return Vec::new();
        };
        // Step 1: walk projects/<project>/memory/ and emit one candidate per
        // existing memory dir. `projects_dir` is `<config_dir>/projects`.
        let projects_dir = config_dir.join("projects");
        let project_bases_and_dirs = self.project_memory_dirs(basis, &projects_dir);
        let mut candidates = project_bases_and_dirs
            .iter()
            .map(|(proj_basis, dir, project_key)| {
                CandidateSource {
                    provider: PROVIDER_ID.to_string(),
                    root_path: dir.to_string_lossy().into_owned(),
                    basis: *proj_basis,
                    coverage_level: self.coverage_level(),
                    native_project: Some(project_key.clone()),
                }
            })
            .collect::<Vec<_>>();

        // Step 2: read user-scope `<config_dir>/settings.json` for
        // `autoMemoryDirectory`. Safe-degrades to no candidate on any parse
        // or validity failure. Dedup by canonicalized path against the
        // project candidates so a physical dir emits at most one row.
        if let Some(auto_candidate) =
            auto_memory_candidate(&config_dir, home, self.coverage_level())
        {
            let auto_canonical = canonical_dir(&auto_candidate.root_path);
            let already_present = candidates
                .iter()
                .any(|c| canonical_dir(&c.root_path) == auto_canonical);
            if !already_present {
                candidates.push(auto_candidate);
            }
        }

        // Deterministic ordering: sort by root_path (the cross-source stable
        // ordering in `application::source::discover_sources` re-sorts by
        // `(provider, root_path)`; sorting here keeps a single adapter's
        // output deterministic for direct unit testing of this seam).
        candidates.sort_by(|a, b| a.root_path.cmp(&b.root_path));
        candidates
    }

    /// Walk a resolved `projects/` directory and return `(basis, memory_dir,
    /// project_key)` for each existing `<project>/memory/` dir. A project
    /// without a `memory/` child is silently skipped (spec I/O matrix). The
    /// walk is existence/type only (NFR-5) — directory entries' contents are
    /// never read.
    fn project_memory_dirs(
        &self,
        basis: DiscoveryBasis,
        projects_dir: &Path,
    ) -> Vec<(DiscoveryBasis, PathBuf, String)> {
        if !projects_dir.is_dir() {
            return Vec::new();
        }
        let entries = match std::fs::read_dir(projects_dir) {
            Ok(entries) => entries,
            Err(_) => return Vec::new(),
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            // The encoded `<project>` key is preserved verbatim — no
            // reverse-mapping to a real repo path (Epic 5). Non-UTF-8 keys
            // are skipped rather than emitted as garbage. Bind `file_name`
            // to a local so its borrowed `&str` lives long enough.
            let file_name = entry.file_name();
            let Some(project_key) = file_name.to_str() else { continue };
            let memory_dir = entry.path().join("memory");
            if !memory_dir.is_dir() {
                continue;
            }
            let Some(memory_path) = memory_dir.to_str() else {
                continue;
            };
            out.push((basis, PathBuf::from(memory_path), project_key.to_string()));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Story 2.2 — enumeration (Supported Artifact Matrix for Claude Code)
// ---------------------------------------------------------------------------

/// The auto-managed index filename. Always tagged [`ProviderMemoryType::Memory`].
const MEMORY_INDEX_FILE: &str = "MEMORY.md";

/// Human-authored instruction files rejected by name (never indexed). These
/// are not Claude auto-memory — they are guidance Claude reads, not memory it
/// writes. Indexed rejection (a diagnostic, not a silent skip) keeps the
/// count honest and surfaces the boundary in 2.3/2.4.
const REJECTED_INSTRUCTION_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// Enumerate the direct-child `*.md` of a confirmed Claude `memory/` dir.
///
/// Boundary (A-18, mirrors Codex's discipline):
/// - **Direct children only.** No recursion, no subdirectory walking. The
///   real Claude layout is flat `*.md` (verified on 18 real dirs); a manually
///   added subdirectory is out of the matrix and surfaces as a diagnostic.
/// - **`*.md` only.** `MEMORY.md` → [`ProviderMemoryType::Memory`]; every
///   other direct-child `*.md` → [`ProviderMemoryType::TopicMemory`].
/// - **`CLAUDE.md` / `AGENTS.md`** are rejected by name as
///   `unsupported_artifact` diagnostics — human instruction files, not memory.
/// - **Non-`*.md` files and subdirectories** become `unsupported_artifact`
///   diagnostics; never indexed, never recursed.
/// - **Symlink escape** (a `*.md` child whose realpath is outside the
///   canonical root) is skipped via the same realpath containment check Codex
///   uses.
///
/// The result is sorted by `relative_path` and **deduplicated** by
/// `relative_path` so the announced record count always equals the actual row
/// count (same "计数诚实" rule as Codex). An empty dir is a legitimate `Ok`
/// with zero supported artifacts (spec I/O matrix — empty directory scan
/// succeeds).
fn enumerate_claude_artifacts(root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
    let canonical_root =
        std::fs::canonicalize(root).map_err(EnumerateError::from_root_io_error)?;
    let entries =
        std::fs::read_dir(&canonical_root).map_err(EnumerateError::from_dir_io_error)?;
    let mut supported = Vec::new();
    let mut diagnostics = Vec::new();

    for entry in entries {
        let entry = entry.map_err(EnumerateError::from_dir_io_error)?;
        let lexical_path = entry.path();
        let observed = crate::adapters::markdown::safe_relative_path(&canonical_root, &lexical_path);
        let name_utf8 = entry.file_name();
        let Some(name) = name_utf8.to_str() else {
            // Non-UTF-8 entry: percent-encode the observed lexical path into a
            // diagnostic (mirrors Codex). Never indexed.
            diagnostics.push(ArtifactDiagnostic {
                kind: "unsupported_artifact",
                observed_path: observed,
            });
            continue;
        };
        // Reject human instruction files by name regardless of extension —
        // they are not Claude auto-memory. The comparison is case-insensitive
        // so a `claude.md`/`agents.md` on a case-insensitive filesystem
        // (macOS APFS) is still rejected rather than leaking instructions into
        // the index.
        if REJECTED_INSTRUCTION_FILES
            .iter()
            .any(|rejected| rejected.eq_ignore_ascii_case(name))
        {
            diagnostics.push(ArtifactDiagnostic {
                kind: "unsupported_artifact",
                observed_path: observed,
            });
            continue;
        }
        // In-matrix names (`*.md`) must resolve to a readable file. Mirrors
        // Codex: an in-matrix name that resolves to the wrong type (e.g.
        // `MEMORY.md` as a directory) is a TERMINAL failure — a supported
        // memory cannot disappear from a successful generation. Non-`*.md`
        // names (including subdirectories) are out of the matrix and surface
        // as `unsupported_artifact` diagnostics.
        if is_markdown(name) {
            if let Some(artifact) = resolve_supported_claude_artifact(&canonical_root, &lexical_path)?
            {
                supported.push(artifact);
            }
        } else {
            diagnostics.push(ArtifactDiagnostic {
                kind: "unsupported_artifact",
                observed_path: observed,
            });
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

/// Resolve an in-matrix `*.md` child into a supported artifact after
/// realpath containment + metadata validation. Returns `Ok(None)` for a
/// proven root escape (silently excluded — mirrors Codex); returns `Err` for
/// an unresolvable in-matrix file (terminal — a supported memory cannot
/// disappear from a successful generation).
fn resolve_supported_claude_artifact(
    canonical_root: &Path,
    lexical_path: &Path,
) -> Result<Option<SupportedArtifact>, EnumerateError> {
    let real = match std::fs::canonicalize(lexical_path) {
        Ok(path) => path,
        Err(_) => return Err(EnumerateError::AllowlistedArtifactUnresolvable),
    };
    let Ok(relative) = real.strip_prefix(canonical_root) else {
        // Symlink escape: realpath is outside the confirmed root. Skip.
        return Ok(None);
    };
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
    // Case-insensitive so a `memory.md` (e.g. on a case-insensitive FS) still
    // tags as the auto-managed `Memory` index type rather than `TopicMemory`.
    let memory_type = if relative_path.eq_ignore_ascii_case(MEMORY_INDEX_FILE) {
        ProviderMemoryType::Memory
    } else {
        ProviderMemoryType::TopicMemory
    };
    Ok(Some(SupportedArtifact {
        file: FileUnit {
            relative_path,
            absolute_path: real,
            size: metadata.len(),
            mtime,
        },
        memory_type,
    }))
}

/// A name is markdown iff it has a `.md` extension (case-sensitive on the
/// documented Claude layout). Direct-child `*.md` are the only in-matrix
/// files for Claude Code (A-18).
fn is_markdown(name: &str) -> bool {
    name.ends_with(".md")
}

/// Pure path resolver for Claude Code's `<config_dir>/` directory (the
/// directory that contains `projects/` and `settings.json`).
///
/// Extracted from [`ClaudeCodeAdapter::discover`] so tests can exercise the
/// `CLAUDE_CONFIG_DIR`-priority rule without touching the process
/// environment (env-mutation races under `cargo test` parallelism; edition
/// 2024 marks `set_var` unsafe). The function does no FS I/O —
/// `discover_with_env` applies the directory checks to the returned path.
///
/// Validation (applied here, the single source of truth — mirrors Codex):
/// - Empty / whitespace-only values are treated as unset.
/// - Values must be absolute; a relative root would resolve against the
///   process CWD and drift between launches, so it is rejected (returns
///   `None`, never a fallback for an explicit-but-invalid
///   `CLAUDE_CONFIG_DIR`).
///
/// Returns the basis (default-home vs env-override) plus the config-dir
/// path. Returns `None` when neither a usable `claude_config_dir` nor a
/// usable `home` is supplied, or when a supplied value fails validation.
fn resolve_claude_config_dir(
    claude_config_dir: Option<&str>,
    home: Option<&str>,
) -> Option<(DiscoveryBasis, PathBuf)> {
    // Priority 1: explicit CLAUDE_CONFIG_DIR (non-empty after trim, absolute).
    // No fallback to ~/.claude — an explicit override is final, even when
    // invalid.
    if let Some(ch) = claude_config_dir.filter(|s| !s.trim().is_empty()) {
        let p = Path::new(ch);
        if p.is_absolute() {
            return Some((DiscoveryBasis::ClaudeConfigDirEnv, p.to_path_buf()));
        }
        return None;
    }
    // Priority 2: default home (non-empty after trim, absolute).
    let home = home.filter(|s| !s.trim().is_empty())?;
    let p = Path::new(home);
    if !p.is_absolute() {
        return None;
    }
    Some((DiscoveryBasis::ClaudeDefaultHome, p.join(".claude")))
}

/// Backwards-compat: the original Story 2.1 resolver name returned the
/// `projects/` directory directly. Some tests in `claude_code_discover.rs`
/// still drive it; keep it as a thin delegate so the resolver's documented
/// path-join semantics (`<root>/projects`) remain pinned.
///
/// Pure function of `(claude_config_dir, home)` — no env reads, no FS I/O.
/// Returns `(basis, projects_dir)` or `None` on invalid input.
pub fn resolve_claude_projects_dir(
    claude_config_dir: Option<&str>,
    home: Option<&str>,
) -> Option<(DiscoveryBasis, PathBuf)> {
    let (basis, config_dir) = resolve_claude_config_dir(claude_config_dir, home)?;
    Some((basis, config_dir.join("projects")))
}

/// Read `<config_dir>/settings.json`, extract `autoMemoryDirectory`, and
/// return a Claude candidate when it resolves to an existing UTF-8 dir.
///
/// Honors only absolute or `~/`-prefixed values; expands `~/` via `HOME`.
/// Safe-degrades (returns `None`) when:
/// - `settings.json` is absent or unreadable.
/// - The file is unparseable (after stripping `//` line comments).
/// - `autoMemoryDirectory` is absent, not a string, empty, or relative.
/// - The resolved path does not exist, is not a directory, or is non-UTF-8.
///
/// `native_project = None` and `coverage = Full` per spec Always list.
fn auto_memory_candidate(
    config_dir: &Path,
    home: Option<&str>,
    coverage: CoverageLevel,
) -> Option<CandidateSource> {
    let raw = std::fs::read_to_string(config_dir.join("settings.json")).ok()?;
    let stripped = strip_jsonc_comments(&raw);
    let parsed = serde_json::from_str::<serde_json::Value>(&stripped).ok()?;
    let value = parsed.get("autoMemoryDirectory")?.as_str()?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Only absolute or `~/`-prefixed values are valid. A relative value
    // (e.g. "mem") is invalid — safe-degrade to no candidate rather than
    // anchor it to a CWD-dependent path that drifts between launches.
    let resolved = if let Some(rest) = trimmed.strip_prefix("~/") {
        // `~/`-expansion uses HOME as the anchor. HOME must be non-empty AND
        // absolute: the config-dir resolver validates HOME only when HOME is
        // the config basis (default-home path); when `CLAUDE_CONFIG_DIR` is
        // set, HOME is not the config basis and is therefore NOT validated
        // by the resolver, so a relative HOME would otherwise flow unvalidated
        // into this join and emit a candidate with a RELATIVE root_path —
        // violating the module's absolute-root invariant. Reject rather than
        // emit a CWD-dependent candidate.
        let home = home
            .filter(|h| !h.trim().is_empty())
            .filter(|h| Path::new(h).is_absolute())?;
        // Strip any additional leading `/` from the remainder before joining.
        // `Path::join("/foo")` replaces the base entirely, so `"~//foo"` would
        // otherwise resolve to `/foo` instead of `$HOME/foo`. Trim leading
        // slashes so the remainder is treated as a strict relative tail.
        let rest = rest.trim_start_matches('/');
        // Re-join via Path so platform separators are correct. Treat the
        // remainder as a relative path under HOME.
        Path::new(home).join(rest)
    } else if Path::new(trimmed).is_absolute() {
        PathBuf::from(trimmed)
    } else {
        // Relative and not `~/`-prefixed — invalid. Do not silently invent
        // a path anchor the user did not specify.
        return None;
    };
    if !resolved.is_dir() {
        return None;
    }
    let path_str = resolved.to_str()?;
    Some(CandidateSource {
        provider: PROVIDER_ID.to_string(),
        root_path: path_str.to_string(),
        basis: DiscoveryBasis::ClaudeAutoMemoryDir,
        coverage_level: coverage,
        native_project: None,
    })
}

/// Strip `//` line comments from a JSONC-ish string so Claude Code's
/// `settings.json` (which the official editor permits to carry `//`
/// comments) parses via `serde_json`. Only line comments are handled — block
/// comments are not part of the documented format. A genuine parse failure
/// after stripping is the caller's signal to safe-degrade.
///
/// This is deliberately minimal: it handles `//` inside double-quoted
/// strings and tracks `\"` escaping so an escaped quote does NOT toggle the
/// in-string state (otherwise `"autoMemoryDirectory": "/a/\"//b"` would
/// mis-truncate at the `//` after the escaped quote and the JSON parse would
/// fail → safe-degrade). Block comments are not handled. The contract is
/// "best-effort parse or safe-degrade", per spec Design Notes.
fn strip_jsonc_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        // Naively cut at the first `//` outside a quoted string. The
        // settings schema is simple enough that a substring scan inside
        // quotes is not warranted; if a value contained `//` we would
        // over-truncate and the JSON parse would fail → safe-degrade, which
        // is the spec-acceptable outcome.
        if let Some(idx) = find_line_comment_index(line) {
            out.push_str(&line[..idx]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Find the byte index of a `//` line-comment marker that is not inside a
/// double-quoted string. Returns `None` if the line has no comment marker
/// outside a string.
///
/// Backslash-escaping inside a string is tracked so an escaped quote (`\"`)
/// does NOT toggle the in-string state. Without this, a value like
/// `"autoMemoryDirectory": "/a/\"//b"` would mis-truncate at the `//` after
/// the escaped quote and the JSON parse would fail → safe-degrade (still
/// the caller's fallback for any other shape we fail to handle).
fn find_line_comment_index(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escape = false;
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        let c = bytes[i];
        if escape {
            escape = false;
        } else if in_string && c == b'\\' {
            escape = true;
        } else if c == b'"' {
            in_string = !in_string;
        } else if !in_string && c == b'/' && bytes[i + 1] == b'/' {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Best-effort canonicalization for dedup. Returns the verbatim path on any
/// canonicalize error (e.g. the path is valid but the filesystem does not
/// resolve symlinks) so dedup degrades to exact-string equality rather than
/// failing discovery.
fn canonical_dir(path: &str) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `strip_jsonc_comments` removes `//` line comments and leaves other
    /// lines intact. Used by `auto_memory_candidate`; pinned here so the
    /// safe-degrade contract does not silently regress.
    #[test]
    fn strip_jsonc_comments_removes_line_comments_only() {
        let input = "{\n  // a comment\n  \"k\": 1,\n  \"v\": \"//not a comment\"\n}\n";
        let stripped = strip_jsonc_comments(input);
        assert!(!stripped.contains("// a comment"), "stripped: {stripped}");
        // The // inside a quoted string MUST be preserved — a naive
        // find-and-cut would corrupt the value.
        assert!(
            stripped.contains("\"//not a comment\""),
            "string literal must survive comment strip: {stripped}"
        );
        // Parses cleanly into a JSON object after stripping.
        let parsed: serde_json::Value =
            serde_json::from_str(&stripped).expect("parses after strip");
        assert_eq!(parsed["k"], 1);
        assert_eq!(parsed["v"], "//not a comment");
    }

    /// `find_line_comment_index` does not flag `//` inside a quoted string.
    #[test]
    fn find_line_comment_index_ignores_quoted_slashes() {
        // `//` inside a string → None.
        assert_eq!(find_line_comment_index("\"a//b\""), None);
        // `//` after a string closes → index of the marker.
        let line = "\"a\" // trailing";
        let idx = find_line_comment_index(line).expect("trailing comment");
        assert_eq!(&line[idx..], "// trailing");
        // No comment at all.
        assert_eq!(find_line_comment_index("\"a/b/c\""), None);
    }

    // --- Story 2.2: enumerate_artifacts boundary (A-18) --------------------

    /// Direct-child `*.md` index; `MEMORY.md` → Memory, other `*.md` →
    /// TopicMemory. `CLAUDE.md`/`AGENTS.md`/non-`.md`/subdir become
    /// `unsupported_artifact` diagnostics.
    #[test]
    fn enumerate_indexes_direct_child_md_and_classifies_roles() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::write(root.join("MEMORY.md"), "# memory\nbody").expect("write memory");
        std::fs::write(root.join("topic-a.md"), "# topic A\nbody").expect("write topic a");
        std::fs::write(root.join("topic-b.md"), "# topic B\nbody").expect("write topic b");
        // Rejected by name.
        std::fs::write(root.join("CLAUDE.md"), "rules").expect("write claude");
        std::fs::write(root.join("AGENTS.md"), "rules").expect("write agents");
        // Non-markdown.
        std::fs::write(root.join("notes.txt"), "notes").expect("write txt");
        std::fs::write(root.join("data.json"), "{}").expect("write json");
        // Subdirectory (must NOT be recursed).
        std::fs::create_dir_all(root.join("subdir")).expect("mkdir subdir");
        std::fs::write(root.join("subdir").join("nested.md"), "nested").expect("write nested");

        let adapter = ClaudeCodeAdapter;
        let observation = adapter.enumerate_artifacts(root).expect("enumerate ok");

        // Supported: MEMORY.md + 2 topic files.
        assert_eq!(observation.supported.len(), 3, "exactly the 3 direct-child .md");
        let mut sorted: Vec<(&str, ProviderMemoryType)> = observation
            .supported
            .iter()
            .map(|a| (a.file.relative_path.as_str(), a.memory_type))
            .collect();
        sorted.sort_by_key(|(p, _)| *p);
        assert_eq!(
            sorted,
            vec![
                ("MEMORY.md", ProviderMemoryType::Memory),
                ("topic-a.md", ProviderMemoryType::TopicMemory),
                ("topic-b.md", ProviderMemoryType::TopicMemory),
            ]
        );

        // Diagnostics: CLAUDE.md, AGENTS.md, notes.txt, data.json, subdir.
        // (subdir/nested.md is NOT a diagnostic — the subdir itself is the
        // boundary; recursion is never attempted.)
        let mut diag_paths: Vec<&str> =
            observation.diagnostics.iter().map(|d| d.observed_path.as_str()).collect();
        diag_paths.sort();
        assert_eq!(
            diag_paths,
            vec!["AGENTS.md", "CLAUDE.md", "data.json", "notes.txt", "subdir"],
            "all non-matrix direct children surface as diagnostics"
        );
        for d in &observation.diagnostics {
            assert_eq!(d.kind, "unsupported_artifact");
        }
    }

    /// Case-insensitive filesystem discipline (macOS APFS is case-insensitive):
    /// lowercase `claude.md`/`agents.md` MUST still be rejected as instruction
    /// files (never indexed — no instruction leak), and lowercase `memory.md`
    /// MUST still tag as the `Memory` index type (not `TopicMemory`). The
    /// rejecter and the index role tag both compare case-insensitively.
    #[test]
    fn enumerate_reject_lowercase_instruction_files_and_tags_lowercase_memory() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        // Lowercase instruction files — must be rejected by name, not indexed.
        std::fs::write(root.join("claude.md"), "rules").expect("write claude.md");
        std::fs::write(root.join("agents.md"), "rules").expect("write agents.md");
        // Lowercase index file — must tag as Memory (the auto-managed index).
        std::fs::write(root.join("memory.md"), "# memory\nbody").expect("write memory.md");
        // A regular topic file for contrast (tags as TopicMemory).
        std::fs::write(root.join("topic.md"), "# topic\nbody").expect("write topic");

        let adapter = ClaudeCodeAdapter;
        let observation = adapter.enumerate_artifacts(root).expect("enumerate ok");

        // Supported: lowercase `memory.md` (Memory) + `topic.md` (TopicMemory).
        let mut supported: Vec<(&str, ProviderMemoryType)> = observation
            .supported
            .iter()
            .map(|a| (a.file.relative_path.as_str(), a.memory_type))
            .collect();
        supported.sort_by_key(|(p, _)| *p);
        assert_eq!(
            supported,
            vec![
                ("memory.md", ProviderMemoryType::Memory),
                ("topic.md", ProviderMemoryType::TopicMemory),
            ],
            "lowercase memory.md tags as Memory; instruction files rejected"
        );

        // Lowercase instruction files surface as diagnostics, never indexed.
        let mut diag_paths: Vec<&str> =
            observation.diagnostics.iter().map(|d| d.observed_path.as_str()).collect();
        diag_paths.sort();
        assert_eq!(
            diag_paths,
            vec!["agents.md", "claude.md"],
            "lowercase instruction files rejected as unsupported_artifact"
        );
        for d in &observation.diagnostics {
            assert_eq!(d.kind, "unsupported_artifact");
        }
    }

    /// Empty `memory/` dir → zero supported + zero diagnostics (spec I/O
    /// matrix — empty directory scan is a complete success).
    #[test]
    fn enumerate_empty_directory_succeeds_with_zero_artifacts() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let adapter = ClaudeCodeAdapter;
        let observation = adapter.enumerate_artifacts(tmp.path()).expect("enumerate ok");
        assert!(observation.supported.is_empty());
        assert!(observation.diagnostics.is_empty());
    }

    /// Enumeration of a missing root → `RootMissing` (Story 4.2: the root
    /// variants are split by io kind, and `NotFound` at canonicalize yields
    /// `RootMissing`).
    #[test]
    fn enumerate_fails_for_missing_root() {
        let bogus = Path::new("/this/does/not/exist/tessera-2-2-claude-enum");
        let adapter = ClaudeCodeAdapter;
        assert!(matches!(
            adapter.enumerate_artifacts(bogus),
            Err(EnumerateError::RootMissing)
        ));
    }

    /// `MEMORY.md` as a directory (not a file) is a terminal failure for that
    /// in-matrix artifact — a supported memory cannot disappear from a
    /// successful generation.
    #[test]
    fn enumerate_recognized_index_as_dir_fails_loudly() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path();
        std::fs::create_dir_all(root.join("MEMORY.md")).expect("mkdir MEMORY.md");
        let adapter = ClaudeCodeAdapter;
        assert!(matches!(
            adapter.enumerate_artifacts(root),
            Err(EnumerateError::AllowlistedArtifactUnresolvable)
        ));
    }

    /// A symlinked `*.md` whose realpath escapes the canonical root is
    /// skipped (AD-4 / spec Block If — symlink escape). Mirrors the Codex
    /// boundary discipline.
    #[cfg(unix)]
    #[test]
    fn enumerate_skips_symlink_escape() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = tmp.path().join("memory");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&root).expect("mkdir root");
        std::fs::create_dir_all(&outside).expect("mkdir outside");
        std::fs::write(root.join("MEMORY.md"), "# memory\n").expect("write memory");
        std::fs::write(outside.join("secret.md"), "secret").expect("write secret");
        std::os::unix::fs::symlink(outside.join("secret.md"), root.join("topic-leak.md"))
            .expect("symlink");

        let adapter = ClaudeCodeAdapter;
        let observation = adapter.enumerate_artifacts(&root).expect("enumerate ok");
        let rels: Vec<&str> = observation
            .supported
            .iter()
            .map(|a| a.file.relative_path.as_str())
            .collect();
        assert_eq!(rels, vec!["MEMORY.md"], "escaping symlink skipped");
    }

    /// Parser version is `claude-markdown/v1` — the single source of truth
    /// for the persisted parser version tag.
    #[test]
    fn adapter_declares_claude_markdown_v1_parser_version() {
        let adapter = ClaudeCodeAdapter;
        assert_eq!(adapter.parser_version(), "claude-markdown/v1");
        assert_eq!(ClaudeCodeAdapter::PARSER_VERSION, "claude-markdown/v1");
    }
}
