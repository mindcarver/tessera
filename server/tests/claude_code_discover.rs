//! Claude Code discovery contract tests (Story 2.1 / spec-2-1-claude-discover.md).
//!
//! These tests cover the discovery slice of the Claude Code adapter against
//! the spec's I/O matrix and the architecture invariants:
//!
//! - **A-3 / AD-14 capability-honesty:** adapter declares
//!   `provider_id()=="claude_code"` and `coverage_level()==Full`.
//! - **I/O matrix rows:**
//!   - "default home, multiple projects" → N candidates (one per existing
//!     `projects/<P>/memory/`), `basis=claude_default_home`, sorted by
//!     `root_path`, each carrying `<P>` as `native_project`.
//!   - "CLAUDE_CONFIG_DIR override" → candidates only under
//!     `$CLAUDE_CONFIG_DIR/projects/...`; `~/.claude` is NOT also scanned.
//!   - "project without memory/" → silently skipped (no candidate).
//!   - "relative CLAUDE_CONFIG_DIR" → no candidates; no fallback to `~/.claude`
//!     (explicit override is final).
//!   - "dir contains only excluded artifacts" → still emits a candidate;
//!     discovery does NOT inspect contents (NFR-5).
//!
//! ## Why tempdir injection, not `std::env::set_var`
//!
//! Mirrors `codex_discover.rs`. `cargo test` runs tests in parallel inside one
//! process; `set_var` is process-global and races other tests, and on edition
//! 2024 it is `unsafe`. The adapter exposes
//! [`ClaudeCodeAdapter::discover_with_env`], which takes
//! `(claude_config_dir, home)` as parameters and runs the full discover path
//! (resolver → directory walk → candidate construction). Every matrix test
//! below drives the adapter's own code against tempdir roots, never a
//! test-local mirror of its logic.

use std::fs;

use tempfile::tempdir;

use tessera_lib::adapters::claude_code::{resolve_claude_projects_dir, ClaudeCodeAdapter};
use tessera_lib::domain::ports::provider_adapter::{
    CoverageLevel, DiscoveryBasis, ProviderAdapter,
};

/// Helper: create a `<parent>/.claude/projects/<project>/memory/` directory
/// tree under a tempdir home and return the path to that home's `.claude`
/// directory.
fn make_default_project_memory(home: &std::path::Path, project: &str) -> std::path::PathBuf {
    let memory = home.join(".claude").join("projects").join(project).join("memory");
    fs::create_dir_all(&memory).expect("create default project memory dir");
    memory
}

/// Helper: create a `<config_dir>/projects/<project>/memory/` directory tree
/// under an explicit CLAUDE_CONFIG_DIR-shaped tempdir and return the path to
/// that memory dir.
fn make_env_project_memory(config_dir: &std::path::Path, project: &str) -> std::path::PathBuf {
    let memory = config_dir.join("projects").join(project).join("memory");
    fs::create_dir_all(&memory).expect("create env project memory dir");
    memory
}

/// I/O matrix row 1 — default home with multiple projects → one candidate per
/// existing `projects/<P>/memory/` dir, `basis=claude_default_home`, sorted by
/// `root_path`. A project whose `memory/` is empty still produces a candidate
/// (discovery is content-blind — NFR-5).
#[test]
fn discover_default_home_emits_one_candidate_per_project_memory_dir() {
    let home = tempdir().expect("home tempdir");
    let mem_a = make_default_project_memory(home.path(), "proj-a");
    let mem_b = make_default_project_memory(home.path(), "proj-b");
    // Empty memory dir is fine — discovery does not inspect contents.
    let _mem_empty = make_default_project_memory(home.path(), "proj-empty");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 3, "one candidate per project memory dir");

    // All candidates share the Claude Code provider id, the default-home
    // basis, and Full coverage (AD-3 capability-honesty).
    for c in &candidates {
        assert_eq!(c.provider, "claude_code");
        assert_eq!(c.basis, DiscoveryBasis::ClaudeDefaultHome);
        assert_eq!(c.coverage_level, CoverageLevel::Full);
    }

    // The native project key is preserved verbatim — no reverse-mapping.
    let native_projects: Vec<&str> =
        candidates.iter().map(|c| c.native_project.as_deref().unwrap()).collect();
    assert!(native_projects.contains(&"proj-a"));
    assert!(native_projects.contains(&"proj-b"));
    assert!(native_projects.contains(&"proj-empty"));

    // Sorted by root_path (deterministic output).
    let root_paths: Vec<&str> = candidates.iter().map(|c| c.root_path.as_str()).collect();
    let mut sorted = root_paths.clone();
    sorted.sort();
    assert_eq!(root_paths, sorted, "candidates are sorted by root_path");

    // Spot-check that the memory dirs match what we created.
    let a_candidate = candidates
        .iter()
        .find(|c| c.native_project.as_deref() == Some("proj-a"))
        .expect("proj-a candidate");
    assert_eq!(a_candidate.root_path, mem_a.to_string_lossy());
    let b_candidate = candidates
        .iter()
        .find(|c| c.native_project.as_deref() == Some("proj-b"))
        .expect("proj-b candidate");
    assert_eq!(b_candidate.root_path, mem_b.to_string_lossy());
}

/// I/O matrix row 2 — `CLAUDE_CONFIG_DIR` override → only
/// `$CLAUDE_CONFIG_DIR/projects/...` is scanned; `~/.claude` is NOT also
/// scanned even when it has memory dirs.
#[test]
fn discover_claude_config_dir_override_does_not_scan_default_home() {
    let config_dir = tempdir().expect("config_dir tempdir");
    let default_home = tempdir().expect("default_home tempdir");
    // Populate BOTH; only the env-side candidate must surface.
    let env_mem = make_env_project_memory(config_dir.path(), "env-proj");
    let default_mem = make_default_project_memory(default_home.path(), "default-proj");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(
        config_dir.path().to_str(),
        default_home.path().to_str(),
    );

    assert_eq!(candidates.len(), 1, "only the CLAUDE_CONFIG_DIR candidate");
    let c = &candidates[0];
    assert_eq!(c.basis, DiscoveryBasis::ClaudeConfigDirEnv);
    assert_eq!(c.root_path, env_mem.to_string_lossy());
    assert_ne!(c.root_path, default_mem.to_string_lossy());
    assert_eq!(c.native_project.as_deref(), Some("env-proj"));
}

/// I/O matrix row 3 — a project without `memory/` is silently skipped: no
/// candidate emitted for it, no error.
#[test]
fn discover_skips_project_without_memory_dir() {
    let home = tempdir().expect("home tempdir");
    // proj-with-memory gets a candidate; proj-without-memory does not.
    let _ = make_default_project_memory(home.path(), "proj-with-memory");
    let projects_dir = home.path().join(".claude").join("projects");
    fs::create_dir_all(projects_dir.join("proj-without-memory")).expect("mkdir empty project");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1, "only the project with memory/");
    assert_eq!(
        candidates[0].native_project.as_deref(),
        Some("proj-with-memory"),
    );
}

/// I/O matrix row 4 — relative `CLAUDE_CONFIG_DIR` yields no candidates; no
/// silent fallback to `~/.claude` (explicit override is final).
#[test]
fn discover_relative_claude_config_dir_returns_empty_without_fallback() {
    let default_home = tempdir().expect("default_home tempdir");
    // Default home has a memory dir; it must NOT be surfaced because
    // CLAUDE_CONFIG_DIR is explicit (even though invalid).
    let _ = make_default_project_memory(default_home.path(), "ignored-proj");

    let adapter = ClaudeCodeAdapter;
    let candidates =
        adapter.discover_with_env(Some("relative/path"), default_home.path().to_str());

    assert!(candidates.is_empty(), "explicit-but-invalid override must not fall back");
}

/// Empty/whitespace `CLAUDE_CONFIG_DIR` is treated as unset → falls back to
/// default home (mirrors Codex).
#[test]
fn discover_treats_empty_claude_config_dir_as_unset() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(Some("   "), home.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
}

/// A regular FILE at `projects/<P>/memory` (not a directory) is skipped —
/// `is_dir` not `exists`. No candidate is emitted for that project.
#[test]
fn discover_skips_when_memory_path_is_a_file() {
    let home = tempdir().expect("home tempdir");
    let projects_dir = home.path().join(".claude").join("projects").join("file-proj");
    fs::create_dir_all(&projects_dir).expect("mkdir file-proj");
    // `memory` is a regular file, not a directory.
    fs::write(projects_dir.join("memory"), "not a directory").expect("write file");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert!(candidates.is_empty(), "a file at memory/ is not a source");
}

/// A memory dir containing only AD-11-excluded artifacts (e.g. `CLAUDE.md`)
/// STILL produces a candidate — discovery is content-blind (NFR-5). The
/// Supported Artifact Matrix is enforced at parse time in Story 2.2, not at
/// discovery time.
#[test]
fn discover_returns_candidate_for_dir_with_only_excluded_artifacts_nfr5() {
    let home = tempdir().expect("home tempdir");
    let memory = make_default_project_memory(home.path(), "excluded-only");
    // Write artifacts the matrix excludes. Discovery must not care.
    let sample = memory.join("CLAUDE.md");
    fs::write(&sample, "# rules\nthis is a manual rule file").expect("write sample");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1, "discovery is content-blind (NFR-5)");
    assert_eq!(candidates[0].root_path, memory.to_string_lossy());

    // NFR-5 invariant: the test never needs to open the sample to verify
    // discovery. (Structurally guaranteed by the adapter — it only checks
    // directory existence.)
    drop(sample);
}

/// No `~/.claude/projects/` dir at all → empty vec, not an error.
#[test]
fn discover_returns_empty_when_no_projects_dir_exists() {
    let home = tempdir().expect("home tempdir");
    // Do NOT create .claude/projects/.

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());
    assert!(candidates.is_empty(), "no projects dir → no candidates (not an error)");
}

/// A-3 / AD-14 capability-honesty: the adapter's declarations match the
/// contract the application/UI rely on. This is the precondition for the
/// Claude Code adapter being allowed in the default build (AD-14).
#[test]
fn claude_code_adapter_capability_declaration_is_honest() {
    let adapter = ClaudeCodeAdapter;
    assert_eq!(adapter.provider_id(), "claude_code");
    // Claude Code's projects/<P>/memory/ is a local directory tree → fully
    // enumerable once Story 2.2 lands; declaring Full now is honest capability
    // disclosure (AD-3), not a 2.2 bypass.
    assert_eq!(adapter.coverage_level(), CoverageLevel::Full);
}

/// The resolver returns an absolute `PathBuf` ending in `projects` under
/// `$CLAUDE_CONFIG_DIR/projects` or `$HOME/.claude/projects`. Pin the exact
/// path-join semantics so the wire shape is stable for the UI mirror.
#[test]
fn resolver_paths_are_exactly_documented() {
    let (_, dir) = resolve_claude_projects_dir(Some("/custom"), Some("/ignored"))
        .expect("claude_config_dir set");
    assert_eq!(dir, std::path::PathBuf::from("/custom/projects"));

    let (_, dir) = resolve_claude_projects_dir(None, Some("/home/u")).expect("home set");
    assert_eq!(dir, std::path::PathBuf::from("/home/u/.claude/projects"));
}

/// The resolver mirrors Codex's env-priority rule: explicit non-empty absolute
/// wins; relative explicit yields None (no fallback); whitespace explicit is
/// treated as unset.
#[test]
fn resolver_mirrors_codex_env_priority() {
    // Explicit absolute wins over default home.
    let (basis, _) =
        resolve_claude_projects_dir(Some("/x"), Some("/home/u")).expect("env absolute");
    assert_eq!(basis, DiscoveryBasis::ClaudeConfigDirEnv);

    // Relative explicit yields None — no fallback to default home.
    assert!(resolve_claude_projects_dir(Some("relative/path"), Some("/home/u")).is_none());

    // Whitespace-only explicit is treated as unset → falls back to default.
    let (basis, _) =
        resolve_claude_projects_dir(Some("   "), Some("/home/u")).expect("home fallback");
    assert_eq!(basis, DiscoveryBasis::ClaudeDefaultHome);

    // Relative HOME is rejected (would resolve against CWD).
    assert!(resolve_claude_projects_dir(None, Some("./home")).is_none());

    // Neither set → None.
    assert!(resolve_claude_projects_dir(None, None).is_none());
    assert!(resolve_claude_projects_dir(Some(""), Some("")).is_none());
}

/// Adapter `discover()` against the real process environment. This pins the
/// env-read glue (`discover()` → env → `discover_with_env`) is infallible: it
/// returns a Vec (never panics) and every candidate it happens to find is
/// `claude_code`/Full. Count is host-dependent and intentionally not
/// asserted — the business behavior is pinned by the `discover_with_env`
/// matrix tests above.
#[test]
fn discover_glue_returns_vec_without_panicking() {
    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover();
    for c in &candidates {
        assert_eq!(c.provider, "claude_code");
        assert_eq!(c.coverage_level, CoverageLevel::Full);
    }
}

// ---------------------------------------------------------------------------
// Story 2.1 — `autoMemoryDirectory` (user scope) discovery matrix
// ---------------------------------------------------------------------------

/// Write a `<config_dir>/settings.json` with the given `autoMemoryDirectory`
/// value. The config dir is the `.claude`-shaped root (i.e. the dir that
/// contains `projects/` and `settings.json`). The parent dir is created if
/// needed so a test that ONLY exercises `autoMemoryDirectory` (no project
/// memory dir) does not have to mkdir `.claude` separately.
fn write_settings(config_dir: &std::path::Path, auto_memory_directory: &str) {
    fs::create_dir_all(config_dir).expect("mkdir config_dir for settings.json");
    let json = format!(
        "{{\n  \"autoMemoryDirectory\": \"{auto_memory_directory}\"\n}}\n"
    );
    fs::write(config_dir.join("settings.json"), json).expect("write settings.json");
}

/// Locate the config dir used by `discover_with_env` for a given home
/// (`<home>/.claude`).
fn default_config_dir(home: &std::path::Path) -> std::path::PathBuf {
    home.join(".claude")
}

/// I/O matrix — `autoMemoryDirectory` absolute path → an extra Claude
/// candidate (`basis=claude_auto_memory_dir`, `native_project=None`,
/// `coverage=Full`) appears ALONGSIDE the project candidates.
#[test]
fn auto_memory_directory_absolute_emits_extra_candidate() {
    let home = tempdir().expect("home tempdir");
    let auto_dir = tempdir().expect("auto memory dir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    write_settings(
        &default_config_dir(home.path()),
        auto_dir.path().to_str().expect("abs utf-8"),
    );

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 2, "project candidate + auto candidate");
    let auto_cand = candidates
        .iter()
        .find(|c| c.basis == DiscoveryBasis::ClaudeAutoMemoryDir)
        .expect("auto-memory candidate present");
    assert_eq!(auto_cand.provider, "claude_code");
    assert_eq!(auto_cand.coverage_level, CoverageLevel::Full);
    assert!(auto_cand.native_project.is_none(), "auto candidate has no project key");
    // The root is the resolved absolute path of the tempdir (canonicalized
    // fs::canonicalize may differ on macOS via /private); assert by suffix.
    let auto_root = std::path::PathBuf::from(&auto_cand.root_path);
    assert!(
        auto_root.ends_with(auto_dir.path()),
        "root {} should end with {}",
        auto_root.display(),
        auto_dir.path().display()
    );
}

/// I/O matrix — `autoMemoryDirectory` with a `~/` prefix → expanded via HOME
/// to `<HOME>/<rest>`, then emitted as an extra candidate.
#[test]
fn auto_memory_directory_tilde_prefixed_expands_via_home() {
    let home = tempdir().expect("home tempdir");
    let memory_dir = home.path().join("my-auto-memory");
    fs::create_dir_all(&memory_dir).expect("mkdir auto memory");
    write_settings(&default_config_dir(home.path()), "~/my-auto-memory");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    let auto_cand = candidates
        .iter()
        .find(|c| c.basis == DiscoveryBasis::ClaudeAutoMemoryDir)
        .expect("auto-memory candidate present after ~ expansion");
    let auto_root = std::path::PathBuf::from(&auto_cand.root_path);
    assert!(
        auto_root.ends_with("my-auto-memory"),
        "expanded root should end with the relative tail: {auto_root:?}"
    );
    // The expanded dir is the one we created under HOME.
    let canon_setting = std::fs::canonicalize(&memory_dir).expect("canon setting");
    let canon_cand = std::fs::canonicalize(&auto_root).expect("canon cand");
    assert_eq!(canon_setting, canon_cand, "expansion resolved to HOME/my-auto-memory");
}

/// I/O matrix — `autoMemoryDirectory` duplicates a `projects/<P>/memory/`
/// dir → that physical dir emits EXACTLY ONE candidate (deduped by
/// canonicalized path; the project-keyed one wins).
#[test]
fn auto_memory_directory_duplicates_project_dir_is_deduped() {
    let home = tempdir().expect("home tempdir");
    // Create the project memory dir under the default config, then point
    // `autoMemoryDirectory` at the SAME physical dir.
    let memory_dir =
        default_config_dir(home.path()).join("projects").join("dup-proj").join("memory");
    fs::create_dir_all(&memory_dir).expect("mkdir project memory");
    write_settings(
        &default_config_dir(home.path()),
        memory_dir.to_str().expect("utf-8"),
    );

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    // Exactly one candidate for that physical dir, keyed by the project.
    assert_eq!(
        candidates.len(),
        1,
        "duplicate autoMemoryDirectory must be deduped to a single candidate"
    );
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
    assert_eq!(candidates[0].native_project.as_deref(), Some("dup-proj"));
}

/// I/O matrix — `autoMemoryDirectory` is a RELATIVE string → invalid value,
/// safe-degrades to no extra candidate. Project candidates are unaffected.
#[test]
fn auto_memory_directory_relative_value_safe_degrades_no_extra_candidate() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    write_settings(&default_config_dir(home.path()), "relative/path");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    // Only the project candidate; no `ClaudeAutoMemoryDir` candidate.
    assert_eq!(candidates.len(), 1, "relative value safe-degrades");
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
    assert!(
        candidates
            .iter()
            .all(|c| c.basis != DiscoveryBasis::ClaudeAutoMemoryDir),
        "no auto candidate for a relative value"
    );
}

/// I/O matrix — `autoMemoryDirectory` points at a non-existent / non-dir
/// path → no extra candidate. Project candidates unaffected.
#[test]
fn auto_memory_directory_missing_dir_safe_degrades_no_extra_candidate() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    write_settings(
        &default_config_dir(home.path()),
        "/this/does/not/exist/tessera-2-1",
    );

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1, "missing target safe-degrades");
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
}

/// I/O matrix — `settings.json` is absent → no `ClaudeAutoMemoryDir`
/// candidate; project candidates unaffected (safe degrade).
#[test]
fn settings_json_absent_safe_degrades_project_candidates_unaffected() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    // No settings.json created.

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
}

/// I/O matrix — `settings.json` is unparseable (malformed JSON) → no
/// `ClaudeAutoMemoryDir` candidate; project candidates unaffected.
#[test]
fn settings_json_unparseable_safe_degrades() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    fs::create_dir_all(default_config_dir(home.path())).expect("mkdir config_dir");
    fs::write(
        default_config_dir(home.path()).join("settings.json"),
        "{not valid json",
    )
    .expect("write malformed settings");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
}

/// I/O matrix — `settings.json` carries `//` line comments (JSONC-style) →
/// the comment is stripped and the value is still honored.
#[test]
fn settings_json_with_line_comments_still_parsed() {
    let home = tempdir().expect("home tempdir");
    let auto_dir = tempdir().expect("auto memory dir");
    let config_dir = default_config_dir(home.path());
    fs::create_dir_all(&config_dir).expect("mkdir config_dir");
    fs::write(
        config_dir.join("settings.json"),
        format!(
            "{{\n  // user-scope setting\n  \"autoMemoryDirectory\": \"{}\"\n}}\n",
            auto_dir.path().to_str().expect("utf-8"),
        ),
    )
    .expect("write jsonc settings");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert!(
        candidates
            .iter()
            .any(|c| c.basis == DiscoveryBasis::ClaudeAutoMemoryDir),
        "JSONC line comments must not block parsing: {candidates:?}"
    );
}

/// I/O matrix — `autoMemoryDirectory` is present but not a string → safe
/// degrade (no extra candidate, project candidates unaffected).
#[test]
fn auto_memory_directory_non_string_value_safe_degrades() {
    let home = tempdir().expect("home tempdir");
    let _ = make_default_project_memory(home.path(), "proj-a");
    fs::create_dir_all(default_config_dir(home.path())).expect("mkdir config_dir");
    fs::write(
        default_config_dir(home.path()).join("settings.json"),
        "{\n  \"autoMemoryDirectory\": 42\n}\n",
    )
    .expect("write non-string setting");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeDefaultHome);
}

/// `CLAUDE_CONFIG_DIR` override — `settings.json` is read from the
/// override dir (`$CLAUDE_CONFIG_DIR/settings.json`), NOT `~/.claude/...`.
#[test]
fn auto_memory_directory_read_from_claude_config_dir_override() {
    let config_dir = tempdir().expect("config_dir tempdir");
    let home = tempdir().expect("home tempdir");
    let auto_dir = tempdir().expect("auto memory dir");
    // Project candidate only on the home side; only the env override has a
    // settings.json. The override has no projects/ — proving the auto
    // candidate is read from $CLAUDE_CONFIG_DIR/settings.json.
    let _ = make_default_project_memory(home.path(), "home-proj");
    write_settings(
        config_dir.path(),
        auto_dir.path().to_str().expect("utf-8"),
    );

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(
        config_dir.path().to_str(),
        home.path().to_str(),
    );

    // No projects/ under the override → only the auto candidate surfaces.
    // The home-side project must NOT appear (explicit override is final).
    assert_eq!(candidates.len(), 1, "only the env-side auto candidate");
    assert_eq!(candidates[0].basis, DiscoveryBasis::ClaudeAutoMemoryDir);
    assert!(
        candidates
            .iter()
            .all(|c| c.native_project.as_deref() != Some("home-proj")),
        "default-home project must not surface when CLAUDE_CONFIG_DIR is set"
    );
}

// ---------------------------------------------------------------------------
// Story 2.1 review fix — enumerate_* must HARD-FAIL for claude_code
// ---------------------------------------------------------------------------

/// `enumerate_file_units` returns `Err` for `claude_code` — never empty `Ok`.
/// Returning empty `Ok` would let a misrouted scan (a future change that
/// bypasses the `ProviderNotScannable` guard) commit an empty generation as
/// a false-positive success. The hard-fail turns a guard bypass into a loud
/// `EnumerationFailed`.
#[test]
fn enumerate_file_units_hard_fails_for_claude_code() {
    let adapter = ClaudeCodeAdapter;
    let err = adapter
        .enumerate_file_units(std::path::Path::new("/tmp/any"))
        .expect_err("must Err, not empty Ok");
    assert!(
        matches!(err, tessera_lib::domain::ports::provider_adapter::EnumerateError::Unreadable),
        "expected Unreadable, got {err:?}"
    );
}

/// `enumerate_artifacts` returns `Err` for `claude_code` (same rationale as
/// `enumerate_file_units`).
#[test]
fn enumerate_artifacts_hard_fails_for_claude_code() {
    let adapter = ClaudeCodeAdapter;
    let err = adapter
        .enumerate_artifacts(std::path::Path::new("/tmp/any"))
        .expect_err("must Err, not empty Ok");
    assert!(
        matches!(err, tessera_lib::domain::ports::provider_adapter::EnumerateError::Unreadable),
        "expected Unreadable, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Story 2.1 pass-2 review fixes — `~/`-expansion edge cases
// ---------------------------------------------------------------------------

/// Pass-2 review fix — when `CLAUDE_CONFIG_DIR` is set, the config-dir
/// resolver does NOT validate `HOME` (HOME is only validated when it is the
/// config basis). A relative `HOME` would previously flow unvalidated into
/// the `~/`-expansion branch and emit a candidate with a RELATIVE `root_path`
/// — violating the module's absolute-root invariant. The `~/` branch now
/// requires HOME to be absolute and non-empty; otherwise no candidate is
/// emitted (no fallback).
#[test]
fn tilde_expansion_rejects_relative_home_when_claude_config_dir_is_set() {
    let config_dir = tempdir().expect("config_dir tempdir");
    // The auto-memory target physically exists, but the candidate must NOT
    // surface because HOME is relative (and therefore not a safe anchor).
    let auto_dir = config_dir.path().join("memory");
    fs::create_dir_all(&auto_dir).expect("mkdir auto memory target");
    write_settings(config_dir.path(), "~/memory");

    let adapter = ClaudeCodeAdapter;
    // CLAUDE_CONFIG_DIR is absolute (so the resolver returns Ok); HOME is
    // relative (would anchor `~/memory` to a CWD-dependent path).
    let candidates = adapter.discover_with_env(
        config_dir.path().to_str(),
        Some("relative/home"),
    );

    assert!(
        candidates
            .iter()
            .all(|c| c.basis != DiscoveryBasis::ClaudeAutoMemoryDir),
        "no claude_auto_memory_dir candidate when HOME is relative: {candidates:?}"
    );
}

/// Pass-2 review fix — `"~//m"` must resolve to `$HOME/m`, NOT `/m`.
/// `Path::join("/m")` would replace the base entirely (`/m`); the join-
/// replaces-base hazard is closed by stripping leading `/` from the
/// remainder before `Path::new(home).join(rest)`. The existing dir under
/// `$HOME/m` makes the candidate surface; the root_path assertion pins that
/// the join did NOT substitute.
#[test]
fn tilde_expansion_with_extra_leading_slash_does_not_replace_home() {
    let home = tempdir().expect("home tempdir");
    let memory_dir = home.path().join("m");
    fs::create_dir_all(&memory_dir).expect("mkdir $HOME/m");
    write_settings(&default_config_dir(home.path()), "~//m");

    let adapter = ClaudeCodeAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    let auto_cand = candidates
        .iter()
        .find(|c| c.basis == DiscoveryBasis::ClaudeAutoMemoryDir)
        .expect("`~//m` should expand to $HOME/m and emit a candidate");
    let auto_root = std::path::PathBuf::from(&auto_cand.root_path);
    assert!(
        auto_root.ends_with("m"),
        "expanded root should end with the relative tail `m`: {auto_root:?}"
    );
    assert!(
        auto_root.starts_with(home.path()),
        "expanded root should be anchored at HOME ({}), got: {auto_root:?}",
        home.path().display()
    );
    // Specifically NOT `/m` (the join-replaces-base hazard). Compare the
    // canonicalized candidate to the canonicalized `$HOME/m`.
    let canon_expected = std::fs::canonicalize(&memory_dir).expect("canon expected");
    let canon_actual = std::fs::canonicalize(&auto_root).expect("canon actual");
    assert_eq!(canon_expected, canon_actual, "`~//m` must resolve to $HOME/m");
}
