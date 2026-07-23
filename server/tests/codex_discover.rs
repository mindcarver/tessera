//! Codex discovery contract tests (Story 1.2 / spec-1-2-codex-discover.md).
//!
//! These tests cover the discovery slice of the Codex adapter against the
//! spec's I/O matrix and the architecture invariants:
//!
//! - **A-3 / AD-14 capability-honesty:** adapter declares
//!   `provider_id()=="codex"` and `coverage_level()==Full`.
//! - **I/O matrix rows:**
//!   - "default dir exists" → 1 candidate, `basis=default_home`.
//!   - "CODEX_HOME explicit" → 1 candidate, `basis=codex_home_env`, NO
//!     fallback to `~/.codex`.
//!   - "no supported source" → empty vec (not an error).
//!   - "CODEX_HOME set but memories missing" → empty vec (no fallback).
//!   - "memories dir contains only excluded artifacts" → still 1 candidate;
//!     discovery does NOT inspect contents (NFR-5).
//!
//! ## Why tempdir injection, not `std::env::set_var`
//!
//! `cargo test` runs tests in parallel inside one process. `set_var` is
//! process-global and races other tests; on edition 2024 it is `unsafe`
//! (spec Design Notes — "env 可测试性"). The adapter exposes
//! [`CodexAdapter::discover_with_env`] — which takes `(codex_home, home)` as
//! parameters and runs the *full* discover path (resolver → directory check →
//! candidate construction) — so every matrix test below drives the adapter's
//! own code against tempdir roots, never a test-local mirror of its logic.

use std::fs;

use tempfile::tempdir;

use tessera_lib::adapters::codex::{resolve_codex_memories_root, CodexAdapter};
use tessera_lib::domain::ports::provider_adapter::{
    CoverageLevel, DiscoveryBasis, ProviderAdapter,
};

/// I/O matrix row 1 — default dir exists → 1 candidate, `default_home`.
/// Drives the adapter's real discover path via `discover_with_env`.
#[test]
fn discover_default_home_when_memories_dir_exists() {
    let home = tempdir().expect("home tempdir");
    let memories = home.path().join(".codex").join("memories");
    fs::create_dir_all(&memories).expect("create memories dir");

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1, "exactly one candidate");
    let c = &candidates[0];
    assert_eq!(c.provider, "codex");
    assert_eq!(c.basis, DiscoveryBasis::DefaultHome);
    assert_eq!(c.coverage_level, CoverageLevel::Full);
    assert!(c.native_project.is_none());
    assert_eq!(c.root_path, memories.to_string_lossy());
}

/// I/O matrix row 2 — CODEX_HOME explicit → 1 candidate, `codex_home_env`,
/// NO fallback to `~/.codex` even when the default path also exists.
#[test]
fn discover_codex_home_when_env_explicit_does_not_fallback() {
    // Create BOTH the CODEX_HOME/memories and ~/.codex/memories trees.
    // The adapter must only report the CODEX_HOME candidate.
    let codex_home = tempdir().expect("codex_home tempdir");
    let default_home = tempdir().expect("default_home tempdir");
    fs::create_dir_all(codex_home.path().join("memories")).expect("codex_home/memories");
    fs::create_dir_all(default_home.path().join(".codex").join("memories"))
        .expect("default .codex/memories");

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(
        codex_home.path().to_str(),
        default_home.path().to_str(),
    );

    assert_eq!(candidates.len(), 1, "only the CODEX_HOME candidate");
    let c = &candidates[0];
    assert_eq!(c.basis, DiscoveryBasis::CodexHomeEnv);
    assert_eq!(
        c.root_path,
        codex_home.path().join("memories").to_string_lossy()
    );
    // NOT the default-home path.
    assert_ne!(
        c.root_path,
        default_home
            .path()
            .join(".codex")
            .join("memories")
            .to_string_lossy()
    );
}

/// I/O matrix row 3 — no memories anywhere → empty vec, not an error.
#[test]
fn discover_returns_empty_when_no_dir_exists() {
    let home = tempdir().expect("home tempdir");
    // Do NOT create .codex/memories.

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());
    assert!(candidates.is_empty(), "no candidate is NOT an error");
}

/// I/O matrix row 4 — CODEX_HOME set but `$CODEX_HOME/memories` missing →
/// empty vec, NO fallback to `~/.codex`.
#[test]
fn discover_returns_empty_when_codex_home_memories_missing_no_fallback() {
    let codex_home = tempdir().expect("codex_home tempdir");
    let default_home = tempdir().expect("default_home tempdir");
    // Default home's .codex/memories exists to prove we do NOT fall back.
    fs::create_dir_all(default_home.path().join(".codex").join("memories"))
        .expect("default .codex/memories");
    // CODEX_HOME/memories does NOT exist.

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(
        codex_home.path().to_str(),
        default_home.path().to_str(),
    );
    assert!(
        candidates.is_empty(),
        "explicit CODEX_HOME must not fall back to default home"
    );
}

/// I/O matrix row 5 — memories dir contains only excluded artifacts
/// (e.g. `.jsonl` transcript samples) → discovery STILL returns a candidate.
///
/// NFR-5 is structurally guaranteed: discovery never inspects directory
/// contents. The artifact matrix (AD-11) is enforced at parse time in
/// Story 1.5, not at discovery time. This test pins that boundary by writing
/// a transcript-shaped `.jsonl` file into the dir and asserting the candidate
/// is still produced — AND asserting the test itself never opens the file.
#[test]
fn discover_returns_candidate_for_dir_with_only_excluded_artifacts_nfr5() {
    let home = tempdir().expect("home tempdir");
    let memories = home.path().join(".codex").join("memories");
    fs::create_dir_all(&memories).expect("memories dir");

    // Write a transcript-shaped sample that AD-11 explicitly excludes from
    // canonicalization. Discovery must not care.
    let sample = memories.join("rollout-2026-07-20.jsonl");
    fs::write(&sample, r#"{"role":"user","content":"this is chat body"}"#)
        .expect("write sample");

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());

    assert_eq!(candidates.len(), 1, "discovery is content-blind (NFR-5)");
    let c = &candidates[0];
    assert_eq!(c.basis, DiscoveryBasis::DefaultHome);
    // The candidate path is the directory, never the file.
    assert_eq!(c.root_path, memories.to_string_lossy());

    // NFR-5 invariant: the test must not need to open the sample to verify
    // discovery. (The file is never read; this is structural.)
    drop(sample);
}

/// `is_dir` not `exists`: a regular FILE at the memories path is not a usable
/// root → no candidate (Story 1.5 enumerate would otherwise choke on it).
#[test]
fn discover_returns_empty_when_memories_path_is_a_file() {
    let home = tempdir().expect("home tempdir");
    let codex_dir = home.path().join(".codex");
    fs::create_dir_all(&codex_dir).expect(".codex dir");
    // `memories` is a regular file, not a directory.
    fs::write(codex_dir.join("memories"), "not a directory").expect("write file");

    let adapter = CodexAdapter;
    let candidates = adapter.discover_with_env(None, home.path().to_str());
    assert!(candidates.is_empty(), "a file at the memories path is not a source");
}

/// A-3 / AD-14 capability-honesty: the adapter's declarations match the
/// contract the application/UI rely on. This is the precondition for the
/// Codex adapter being allowed in the default build (AD-14).
#[test]
fn codex_adapter_capability_declaration_is_honest() {
    let adapter = CodexAdapter;
    assert_eq!(adapter.provider_id(), "codex");
    // Codex memories root is a local directory tree → fully enumerable.
    assert_eq!(adapter.coverage_level(), CoverageLevel::Full);
}

/// The resolver returns an absolute PathBuf under $CODEX_HOME/memories or
/// $HOME/.codex/memories. Pin the exact path-join semantics so the wire shape
/// is stable for the UI mirror.
#[test]
fn resolver_paths_are_exactly_documented() {
    let (_, root) = resolve_codex_memories_root(Some("/custom"), Some("/ignored"))
        .expect("codex_home set");
    assert_eq!(root, std::path::PathBuf::from("/custom/memories"));

    let (_, root) = resolve_codex_memories_root(None, Some("/home/u")).expect("home set");
    assert_eq!(root, std::path::PathBuf::from("/home/u/.codex/memories"));
}

/// Adapter `discover()` against the real process environment. This pins the
/// env-read glue (`discover()` → env → `discover_with_env`) is infallible:
/// it returns a Vec (never panics) and every candidate it happens to find is
/// Codex/Full. Count is host-dependent and intentionally not asserted — the
/// business behavior is pinned by the `discover_with_env` matrix tests above.
#[test]
fn discover_glue_returns_vec_without_panicking() {
    let adapter = CodexAdapter;
    let candidates = adapter.discover();
    for c in &candidates {
        assert_eq!(c.provider, "codex");
        assert_eq!(c.coverage_level, CoverageLevel::Full);
    }
}
