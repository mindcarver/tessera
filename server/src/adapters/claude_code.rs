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
//! ## Scanning is deferred to Story 2.2 (and hard-fails loudly until then)
//!
//! Enumeration of Claude Code memory files is Story 2.2. The application
//! layer guards any scan attempt on a `claude_code` source
//! (`ScanError::ProviderNotScannable`) so the Codex parser is never applied
//! to Claude files. [`ProviderAdapter::enumerate_file_units`] and
//! [`ProviderAdapter::enumerate_artifacts`] therefore return `Err` for
//! `claude_code`: returning empty `Ok` would let a misrouted scan (e.g. a
//! future change that bypasses the guard) commit an empty generation as a
//! false-positive success. The hard-fail turns a guard bypass into a loud
//! `EnumerationFailed` instead.

use std::env;
use std::path::{Path, PathBuf};

use crate::domain::ports::provider_adapter::{
    ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis, EnumerateError, FileUnit,
    ProviderAdapter,
};

/// Claude Code Provider adapter (Story 2.1 discovery slice).
///
/// Unit struct — the slice is stateless. The adapter reads provider files
/// but never writes them (NFR-1 zero-write); discovery checks directory
/// existence only, never body content (NFR-5).
#[derive(Debug, Clone, Copy, Default)]
pub struct ClaudeCodeAdapter;

const PROVIDER_ID: &str = "claude_code";

impl ProviderAdapter for ClaudeCodeAdapter {
    fn provider_id(&self) -> &'static str {
        PROVIDER_ID
    }

    fn coverage_level(&self) -> CoverageLevel {
        // Claude Code's `projects/*/memory/` is a local directory tree that
        // the adapter will be able to enumerate in full once Story 2.2 lands.
        // Declaring `Full` here is honest capability disclosure (AD-3 / AD-18)
        // of the provider surface; it does not bypass 2.2. The trait only
        // grows the discovery slice in 2.1.
        CoverageLevel::Full
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

    fn enumerate_file_units(&self, _root: &Path) -> Result<Vec<FileUnit>, EnumerateError> {
        // Story 2.2 lands Claude-specific enumeration. For 2.1, scanning is
        // guarded at the application layer (ScanError::ProviderNotScannable)
        // so this method is never reached in production. Returning `Err`
        // (NEVER empty `Ok`) ensures that if the guard is ever bypassed, a
        // misrouted scan fails loudly as `EnumerationFailed` instead of
        // committing an empty generation as a false-positive success — see
        // the module doc "Scanning is deferred to Story 2.2".
        Err(EnumerateError::Unreadable)
    }

    fn enumerate_artifacts(&self, _root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
        // Same 2.2 boundary + hard-fail rationale as `enumerate_file_units`.
        Err(EnumerateError::Unreadable)
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
}
