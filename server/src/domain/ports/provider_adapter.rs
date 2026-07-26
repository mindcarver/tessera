//! `domain::ports::provider_adapter` — the ProviderAdapter contract.
//!
//! This module is the single place where the application core's interface to
//! providers is fixed (AD-3/AD-25). The hexagonal rule is strict: the
//! application core depends only on this trait; concrete adapters (in
//! `crate::adapters`) implement it.
//!
//! Story-by-story growth (per the Phase 0 doc and the architecture spine):
//! - **Story 1.2 (this commit):** the *discovery slice* — `provider_id`
//!   (Phase 0), `coverage_level`, and `discover`. Discovery produces Candidate
//!   Source metadata only: it does NOT read chat/transcript/body content
//!   (NFR-5), does NOT persist, does NOT canonicalize, and does NOT allocate
//!   `source_id` (AD-4 — those land in Story 1.3).
//! - Stories 1.4–1.6 append `enumerate` / `search` / `stable_native_ids` to
//!   this same trait as their slices land. The trait is grown incrementally; it
//!   is intentionally NOT given speculative method stubs for un-shipped slices
//!   (Phase 0 doc, "trait 增量生长").
//!
//! Locked names (from the architecture spine):
//! - `discover` — produces Candidate Source metadata; never reads chat body.
//! - `enumerate` — full canonical enumeration of an already-confirmed Source.
//! - `search` — search-only / search-assisted provider queries.
//! - `stable_native_ids` — declare whether the provider emits stable native
//!   unit ids; falls back to file-level unit when unstable (AD-30).
//! - `coverage_level` — declare `full | search_only | existence_only |
//!   unsupported`; partial results never become complete index truth (AD-18).
//!
//! ## Watching lives in `application::reconcile`, NOT on this trait
//!
//! The architecture spine once listed `watch` ("produce debounced dirty hints"
//! — AD-8) as a future trait method. Story 4.1 deliberately places watching in
//! [`application::reconcile`](crate::application::reconcile) instead, keyed off
//! the confirmed Source's canonical root path. Every provider Tessera supports
//! today (Codex, Claude Code) is filesystem-root-based, so a `ProviderAdapter::
//! watch` method would couple every future adapter to a filesystem model it
//! may not have. Root-path-based watching at the application layer is
//! adapter-agnostic and matches how `begin_run` / `run_pipeline` already key
//! off `source_id`. Do NOT re-add a `watch` method to this trait without
//! re-deciding that placement (see spec-4-1-watcher-reconcile Design Notes).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Provider-declared memory artifact roles. This is provider metadata, not
/// inferred from a path or a Markdown body by the application layer.
///
/// Story 2.2 adds [`ProviderMemoryType::TopicMemory`] for Claude Code's topic
/// `*.md` files (distinct from `Memory`, which is reserved for `MEMORY.md` —
/// the auto-managed index). Honest role tagging lets 2.3/2.4 filter across
/// providers without re-inferring the role from the path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderMemoryType {
    /// Codex `MEMORY.md` or Claude Code `MEMORY.md` — the auto-managed index.
    Memory,
    /// Codex `memory_summary.md`.
    MemorySummary,
    /// Codex `raw_memories.md`.
    RawMemories,
    /// Codex `rollout_summaries/*.md` direct children.
    RolloutSummary,
    /// Claude Code topic `*.md` under a project `memory/` dir (Story 2.2).
    /// Distinct from [`ProviderMemoryType::Memory`] so 2.3/2.4 filtering can
    /// separate the auto-managed index from user-shaped topic files.
    TopicMemory,
    /// OpenCode's durable first-party instruction artifact (`AGENTS.md`).
    AgentInstruction,
}

impl ProviderMemoryType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::MemorySummary => "memory_summary",
            Self::RawMemories => "raw_memories",
            Self::RolloutSummary => "rollout_summary",
            Self::TopicMemory => "topic_memory",
            Self::AgentInstruction => "agent_instruction",
        }
    }

    /// Reverse of [`ProviderMemoryType::as_str`] — parse the wire vocabulary
    /// back into the enum. Returns `None` for an unknown value so the caller
    /// (Story 2.4 `SearchRequest::new`) can map it to a 400 `bad_request`
    /// rather than inventing a new variant. The set of accepted strings is
    /// exactly the set `as_str` produces, so the filter vocabulary has a single
    /// source of truth (Design Notes — "validate the memory-type vocabulary
    /// from one source of truth"). Named `parse_str` (not `from_str`) to
    /// avoid clashing with the `std::str::FromStr` trait, matching the
    /// `HealthState::parse_str` / `SourceKind::parse_str` convention.
    pub fn parse_str(value: &str) -> Option<Self> {
        match value {
            "memory" => Some(Self::Memory),
            "memory_summary" => Some(Self::MemorySummary),
            "raw_memories" => Some(Self::RawMemories),
            "rollout_summary" => Some(Self::RolloutSummary),
            "topic_memory" => Some(Self::TopicMemory),
            "agent_instruction" => Some(Self::AgentInstruction),
            _ => None,
        }
    }
}

/// Coverage Level declared by a provider adapter (AD-3 / AD-7 / AD-18).
///
/// The level describes *what the provider surface allows*, not what a single
/// scan returned. `search_only` results never become complete index truth and
/// never emit missing-deletion tombstones; only `full` may perform complete
/// enumeration semantics (AD-18). The UI must surface this distinction
/// honestly (AD-3 capability-honesty).
///
/// Serialization renames to the stable lowercase wire strings so the IPC
/// contract never leaks Rust identifiers to the TypeScript mirror
/// (`src/api/discover.ts`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageLevel {
    /// Provider root is locally enumerable in full. The adapter can produce a
    /// complete canonical record set (AD-18 "full"). Codex's memories root
    /// is a local directory tree and qualifies.
    Full,
    /// Provider only exposes a search interface; complete enumeration is not
    /// possible. Results carry `observed_at` and never generate missing
    /// deletions (AD-18).
    SearchOnly,
    /// Provider can only confirm existence of a source, not enumerate or
    /// search it. Honesty level: "we know something is here".
    ExistenceOnly,
    /// Provider is present but unsupported at this coverage tier. The UI must
    /// NOT display it as "fully synced" (AD-3).
    Unsupported,
}

/// Why a Candidate Source was produced by discovery (AD-4 / Story 1.2 I/O
/// matrix).
///
/// Carried on [`CandidateSource`] purely as human-facing metadata so the UI
/// can explain how a candidate was found. Not part of source identity —
/// `source_id` and the canonical root fingerprint land in Story 1.3
/// (AD-33/AD-35). Serialization renames to stable wire strings.
///
/// ## Wire-contract discipline
///
/// Existing snake_case wire strings (`default_home`, `codex_home_env`) are
/// frozen by the `api_version=1` contract; new providers append their own
/// variants alongside rather than rename existing ones (Story 2.1 adds
/// `claude_default_home`, `claude_config_dir_env`, and
/// `claude_auto_memory_dir` for the Claude Code adapter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryBasis {
    /// Candidate discovered via the provider's default home location (e.g.
    /// `$HOME/.codex/memories` when `CODEX_HOME` is not set).
    DefaultHome,
    /// Candidate discovered via an explicit provider environment override
    /// (e.g. `CODEX_HOME=/x` → `/x/memories`). When this basis is used, the
    /// default home is intentionally NOT also probed — see Codex adapter.
    CodexHomeEnv,
    /// Candidate discovered via Claude Code's default home location
    /// (`$HOME/.claude/projects/<project>/memory/` when `CLAUDE_CONFIG_DIR`
    /// is not set). Story 2.1 — see Claude Code adapter.
    ClaudeDefaultHome,
    /// Candidate discovered via an explicit `CLAUDE_CONFIG_DIR` override
    /// (absolute paths only; an explicit-but-relative value yields no
    /// candidate with no silent fallback, mirroring Codex's `CODEX_HOME`
    /// rule). When this basis is used, the default home is intentionally NOT
    /// also probed. Story 2.1.
    ClaudeConfigDirEnv,
    /// Candidate discovered via the user-scope `autoMemoryDirectory` key in
    /// `<config_dir>/settings.json` (Story 2.1). The value is an absolute
    /// path or `~/`-prefixed; `~/` is expanded via `HOME`. Only emitted when
    /// the resolved path is an existing UTF-8 directory, deduplicated against
    /// the `projects/*` candidates by canonicalized path. Invalid / missing /
    /// unparseable values safe-degrade to no candidate.
    ClaudeAutoMemoryDir,
    /// Candidate discovered via the Obsidian vault registry
    /// (`~/Library/Application Support/obsidian/obsidian.json` on macOS). One
    /// candidate per registered vault whose root currently exists. Story 6.2 —
    /// see `adapters::obsidian`. The registry is an observed surface, not a
    /// stable API (AD-37); a missing/corrupt registry produces a visible
    /// diagnostic rather than a silently-empty set.
    ObsidianVaultRegistry,
    /// OpenCode global instruction file at `<config_dir>/AGENTS.md`.
    OpencodeGlobalConfig,
    /// OpenCode project instruction file discovered from the read-only
    /// `project(id, worktree)` metadata in `opencode.db`.
    OpencodeProjectDatabase,
}

/// Candidate Source metadata produced by discovery (AD-4 / Story 1.2).
///
/// A Candidate is a *pre-confirmation* observation: it tells the UI "here is
/// a provider root that appears to exist on this machine". It is NOT a
/// confirmed Source, has no `source_id`, and is not persisted. Re-discovery
/// runs on every boot and may return a different set (path moved, env
/// changed, directory deleted). Confirmation, fingerprint identity, and
/// persistence land in Story 1.3 (AD-33/AD-35).
///
/// Invariants honored here (Story 1.2 boundaries):
/// - **No body / transcript content** (NFR-5): discovery only checks directory
///   *existence*. This type carries paths and metadata, never memory content.
/// - **No `observed_at`** (Design Notes): a Candidate is a transient
///   observation on every boot. Persistent timestamps belong to the confirmed
///   Source (Story 1.3) and canonical record (Story 1.5); introducing a date
///   field here would force a date-format / `chrono` dependency that is not
///   in the Phase 0 locked stack.
/// - **No canonicalized root** (AD-4): `root_path` is the path discovery
///   probed. Canonicalization happens at confirm time in 1.3.
/// - **`native_project: None` for Codex** (Design Notes): Codex memories are
///   a global store with no discoverable per-project split. The field is
///   `Option` so a future provider that *can* discover a native project may
///   populate it; "可判定" (discoverable) is the bar — Codex cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CandidateSource {
    /// Stable lowercase provider id (e.g. `codex`, `claude_code`). Mirrors
    /// [`ProviderAdapter::provider_id`].
    pub provider: String,
    /// The provider root path discovery probed (NOT canonicalized — AD-4).
    /// UTF-8 string; serialized as-is for the IPC round-trip.
    pub root_path: String,
    /// How discovery found this candidate. Pure UI metadata.
    pub basis: DiscoveryBasis,
    /// The provider's declared coverage level. This is the *provider's*
    /// capability, not a per-candidate observation (AD-3).
    pub coverage_level: CoverageLevel,
    /// Provider-native project identifier, when the provider can discover a
    /// per-project split from root metadata alone (no body read). Codex
    /// memories are global → `None`.
    pub native_project: Option<String>,
}

/// A single file-level unit enumerated within a confirmed Source root (Story
/// 1.4 — AD-11/AD-30).
///
/// Produced by [`ProviderAdapter::enumerate_file_units`]. This is the
/// enumeration half of the scan pipeline: the adapter walks the Supported
/// Artifact Matrix boundary (Codex: `MEMORY.md`, `memory_summary.md`,
/// `raw_memories.md`, `rollout_summaries/*.md`) and yields one `FileUnit` per
/// in-matrix file. **No body content is read** (NFR-5) — the application
/// layer reads bytes separately to compute the content hash.
///
/// Invariants honored here:
/// - **AD-11 boundary:** only in-matrix files appear; unknown files are
///   skipped, not indexed.
/// - **Symlink escape rejected:** `absolute_path` is realpath-validated to be
///   inside the canonical root by the adapter; a file that escapes is skipped.
/// - **No body:** `size` and `mtime` are metadata only.
/// - **Sub-second mtime precision (AD-34):** `mtime` is the modification time
///   in **nanoseconds** since the Unix epoch (i64), NOT whole seconds. A
///   whole-second truncation would let a same-second same-size rewrite pass
///   the commit-time manifest re-validation undetected (spec Design Notes —
///   "manifest 时间精度").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileUnit {
    /// Root-relative path (e.g. `rollout_summaries/2026-07-01.md`). This is
    /// the `native_unit_id` for file-level records (AD-30).
    pub relative_path: String,
    /// Absolute, realpath-validated path inside the canonical root. Used to
    /// read bytes for the content hash; re-validated by the application layer
    /// before each read (AD-4).
    pub absolute_path: PathBuf,
    /// File size in bytes (metadata snapshot at enumeration time).
    pub size: u64,
    /// Modification time as **nanoseconds** since the Unix epoch (metadata
    /// snapshot at enumeration time). Sub-second precision is required so a
    /// same-second same-size rewrite still changes the manifest (AD-34). Part
    /// of the manifest boundary.
    pub mtime: i64,
}

/// An allowlisted source artifact plus its provider-declared role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SupportedArtifact {
    pub file: FileUnit,
    pub memory_type: ProviderMemoryType,
}

/// A safe source-scoped observation for an in-root artifact which is not in
/// the supported matrix. `observed_path` is a reversible percent-encoded
/// lexical path; it never contains source body text.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ArtifactDiagnostic {
    pub kind: &'static str,
    pub observed_path: String,
}

/// Complete artifact observation used as both the initial scan boundary and
/// the final validation boundary. Diagnostics are part of the observation so
/// they cannot silently drift between staging and activation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactEnumeration {
    pub supported: Vec<SupportedArtifact>,
    pub diagnostics: Vec<ArtifactDiagnostic>,
}

impl ArtifactEnumeration {
    pub fn from_file_units(units: Vec<FileUnit>) -> Self {
        Self {
            supported: units
                .into_iter()
                .map(|file| SupportedArtifact {
                    file,
                    // This default only supports existing Story 1.4 test
                    // adapters. The real Codex adapter always supplies its
                    // exact allowlist role.
                    memory_type: ProviderMemoryType::Memory,
                })
                .collect(),
            diagnostics: Vec::new(),
        }
    }
}

/// The error returned by [`ProviderAdapter::enumerate_file_units`].
///
/// The spec forbids `Result<_, ()>` on the port (Code Map: "错误类型由实现
/// 定义，不要返回 `Result<_,()>`"). This concrete error type carries a
/// category the application layer can map onto its own structured
/// [`crate::domain::scan::ScanError`] AND a cause-shape for the Story 4.2
/// [`HealthCause`](crate::domain::source::HealthCause) taxonomy; it
/// deliberately carries NO path/body detail (AD-13 safe surface).
///
/// ## Story 4.2 — cause classification at the I/O boundary
///
/// The architecture mandates the cause distinguish at minimum: path missing,
/// permission denied, format unsupported, scan failed. Cause classification
/// happens at the I/O boundary by inspecting `io::Error::kind()` (NOT by
/// string-matching OS messages — AD-13 forbids surfacing them and they are
/// locale/OS-dependent). The variants below are the refined shape:
/// - [`EnumerateError::RootMissing`] — `ErrorKind::NotFound` at the root
///   canonicalize site. Maps to `HealthCause::PathMissing`.
/// - [`EnumerateError::RootPermissionDenied`] — `ErrorKind::PermissionDenied`
///   at the root canonicalize site. Maps to `HealthCause::PermissionDenied`.
/// - [`EnumerateError::RootUnresolvable`] — any other io kind at the root
///   canonicalize site (fallback for kinds that are neither NotFound nor
///   PermissionDenied). Maps to `HealthCause::ScanFailed`.
/// - [`EnumerateError::DirMissing`] / [`EnumerateError::DirPermissionDenied`]
///   / [`EnumerateError::Unreadable`] — the analogous split for a directory
///   inside the root (e.g. `rollout_summaries/`).
/// - [`EnumerateError::AllowlistedArtifactUnresolvable`] — an allowlisted
///   artifact could not be resolved/inspected/read; terminal rather than a
///   diagnostic so a supported memory cannot disappear from a successful
///   generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerateError {
    /// The root itself could not be canonicalized / resolved because it is
    /// missing, not a directory, or `ErrorKind::NotFound`. Maps to scan
    /// `EnumerationFailed` and `HealthCause::PathMissing`.
    RootMissing,
    /// The root itself could not be canonicalized / resolved because
    /// `ErrorKind::PermissionDenied`. Maps to scan `EnumerationFailed` and
    /// `HealthCause::PermissionDenied`.
    RootPermissionDenied,
    /// The root itself could not be canonicalized / resolved for any other io
    /// kind (fallback — e.g. `ErrorKind::Other`, `ErrorKind::InvalidInput`).
    /// Maps to scan `EnumerationFailed` and `HealthCause::ScanFailed`.
    RootUnresolvable,
    /// A directory inside the root (e.g. `rollout_summaries/`) could not be
    /// read because `ErrorKind::NotFound`. Maps to scan `EnumerationFailed`
    /// and `HealthCause::PathMissing`.
    DirMissing,
    /// A directory inside the root could not be read because
    /// `ErrorKind::PermissionDenied`. Maps to scan `EnumerationFailed` and
    /// `HealthCause::PermissionDenied`.
    DirPermissionDenied,
    /// A directory inside the root could not be read for any other io kind
    /// (fallback). Maps to scan `EnumerationFailed` and
    /// `HealthCause::ScanFailed`.
    Unreadable,
    /// An observed allowlisted artifact could not be resolved, inspected, or
    /// read. This is terminal rather than a diagnostic so a supported memory
    /// cannot disappear from a successful generation. Maps to scan
    /// `EnumerationFailed` (the cause is refined by the caller from the io
    /// kind at the artifact site; defaulting to `ScanFailed`).
    AllowlistedArtifactUnresolvable,
}

impl std::fmt::Display for EnumerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnumerateError::RootMissing => f.write_str("root missing"),
            EnumerateError::RootPermissionDenied => f.write_str("root permission denied"),
            EnumerateError::RootUnresolvable => f.write_str("root unresolvable"),
            EnumerateError::DirMissing => f.write_str("directory missing"),
            EnumerateError::DirPermissionDenied => f.write_str("directory permission denied"),
            EnumerateError::Unreadable => f.write_str("directory unreadable"),
            EnumerateError::AllowlistedArtifactUnresolvable => {
                f.write_str("allowlisted artifact unresolvable")
            }
        }
    }
}

impl std::error::Error for EnumerateError {}

impl EnumerateError {
    /// Classify an io error at the **root** canonicalize/resolution site into
    /// the refined [`EnumerateError`] variants (Story 4.2). Adapters call this
    /// at every `std::fs::canonicalize(root)` / `read_dir(root)` /
    /// `metadata(root)` failure site so the four health-cause categories are
    /// genuinely distinguishable by `io::Error::kind()` (not string-matched).
    ///
    /// Per the spec's `PathMissing` definition ("root gone / not-a-dir /
    /// `ErrorKind::NotFound`"), three io kinds classify as `RootMissing`:
    /// - `NotFound` — the root is gone.
    /// - `InvalidInput` — the policy layer synthesizes this for a root that
    ///   exists but is a regular file (`policy::canonicalize_root`).
    /// - `NotADirectory` — a path component is not a directory (surfaced by
    ///   `read_dir` on Rust 1.85+).
    ///
    /// `PermissionDenied` → [`Self::RootPermissionDenied`];
    /// any other kind → [`Self::RootUnresolvable`] (the fallback).
    ///
    /// Takes the error by value so it composes with `Result::map_err`.
    pub fn from_root_io_error(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound
            | std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::NotADirectory => EnumerateError::RootMissing,
            std::io::ErrorKind::PermissionDenied => EnumerateError::RootPermissionDenied,
            _ => EnumerateError::RootUnresolvable,
        }
    }

    /// Classify an io error at a **directory-inside-root** site (e.g.
    /// `rollout_summaries/`) into the refined [`EnumerateError`] variants
    /// (Story 4.2). Mirrors [`Self::from_root_io_error`] but yields the
    /// `Dir*` / `Unreadable` variants. The same three path-missing-shape kinds
    /// (`NotFound` / `InvalidInput` / `NotADirectory`) classify as `DirMissing`
    /// — `NotADirectory` in particular surfaces from `read_dir` when a path
    /// component is a regular file. Takes the error by value so it composes
    /// with `Result::map_err`.
    pub fn from_dir_io_error(err: std::io::Error) -> Self {
        match err.kind() {
            std::io::ErrorKind::NotFound
            | std::io::ErrorKind::InvalidInput
            | std::io::ErrorKind::NotADirectory => EnumerateError::DirMissing,
            std::io::ErrorKind::PermissionDenied => EnumerateError::DirPermissionDenied,
            _ => EnumerateError::Unreadable,
        }
    }
}

/// The ProviderAdapter contract.
///
/// Story 1.2 ships the *discovery slice* only: `provider_id` (Phase 0),
/// `coverage_level`, and `discover`. The remaining architecture-spine methods
/// (`enumerate`, `search`, `stable_native_ids`) are appended in Stories
/// 1.4–1.6 as their slices land — see the module doc. The spine's `watch`
/// slice is intentionally NOT on this trait: watching is root-path-based and
/// lives in [`application::reconcile`](crate::application::reconcile) (Story
/// 4.1 — see the module doc).
///
/// `discover` is infallible by design (Design Notes): it returns `Vec`, not
/// `Result`. Discovery only calls `Path::exists()` (itself infallible — a stat
/// error is honestly reported as "no candidate"). In the pre-confirmation
/// phase there is no `source_id` yet, so AD-13's source-scoped error envelope
/// has nothing to attach to. Real domain error types arrive in Stories
/// 1.4/1.5 when scan/parse are actually fallible.
pub trait ProviderAdapter: std::fmt::Debug {
    /// Stable lowercase provider id (e.g. `codex`, `claude_code`).
    fn provider_id(&self) -> &'static str;

    /// The provider's declared coverage level (AD-3). Describes the provider
    /// surface, not a single observation.
    fn coverage_level(&self) -> CoverageLevel;

    /// Parser-version tag persisted onto every canonical record this adapter
    /// produces (Story 2.2 — single source of truth, replacing the hard-coded
    /// `CODEX_MARKDOWN_PARSER_VERSION` constant at the record-build site).
    ///
    /// A separate tag per provider lets a future grammar bump trigger a reparse
    /// of that provider's records without touching any other provider's
    /// identity. Output changes require a deliberate version decision rather
    /// than silently changing record identities or bodies.
    fn parser_version(&self) -> &'static str;

    /// Discover Candidate Sources for this provider on the local machine.
    ///
    /// Infallible (returns `Vec<CandidateSource>`, never `Result`). Returns
    /// an empty vec when no supported root exists — "no candidate" is NOT an
    /// error. Never reads chat / transcript / body content (NFR-5); only
    /// checks directory existence and provider-level env metadata.
    fn discover(&self) -> Vec<CandidateSource>;

    /// Enumerate the file-level units inside a confirmed Source root (Story
    /// 1.4 — AD-11).
    ///
    /// `root` is the canonicalized Source root. The adapter walks ONLY the
    /// Supported Artifact Matrix boundary (Codex: three known first-level
    /// filenames + `rollout_summaries/*.md` direct children) and returns one
    /// [`FileUnit`] per in-matrix file, with each `absolute_path`
    /// realpath-validated to be inside `root` (symlink escape → skipped).
    /// Files are read for metadata only — **never body content** (NFR-5).
    ///
    /// The result is sorted by `relative_path` and **deduplicated** by
    /// `relative_path`: an in-root symlink alias that canonicalizes to the
    /// same relative/real path as another unit collapses to a single entry,
    /// so the announced record count always equals the actual row count
    /// (spec Design Notes — "计数诚实").
    ///
    /// `Err(EnumerateError)` signals the root (or a required subdirectory)
    /// could not be enumerated, which the application layer maps to a scan
    /// failure. An empty `Ok(vec![])` is a legitimate "no memory artifacts
    /// present" result, NOT an error (spec I/O matrix — empty directory scan
    /// succeeds).
    fn enumerate_file_units(&self, root: &Path) -> Result<Vec<FileUnit>, EnumerateError>;

    /// Enumerate the complete provider observation. The default preserves the
    /// Story 1.4 test seam; real adapters override it to attach exact artifact
    /// roles and safe unsupported-artifact diagnostics.
    fn enumerate_artifacts(&self, root: &Path) -> Result<ArtifactEnumeration, EnumerateError> {
        self.enumerate_file_units(root)
            .map(ArtifactEnumeration::from_file_units)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Story 4.2 — `from_root_io_error` classifies an io error at the root
    /// canonicalize/resolution site by `ErrorKind::kind()` (NOT by
    /// string-matching OS messages — AD-13 forbids surfacing them and they
    /// are locale/OS-dependent). The classification is the middle hop of the
    /// 3-hop kind→variant→cause chain (the cause hop is the application
    /// layer's `cause_from_enumerate_error`); pinning it here means a future
    /// refactor cannot silently change a category at the middle hop without
    /// also failing the application-layer cause tests.
    ///
    /// Per the spec's `PathMissing` definition (Patch 3 of the Story 4.2
    /// review): `NotFound`, `InvalidInput` (the synthesized kind for a
    /// not-a-directory root), and `NotADirectory` all classify as
    /// `RootMissing`; `PermissionDenied` classifies as `RootPermissionDenied`;
    /// every other kind falls through to `RootUnresolvable`.
    #[test]
    fn from_root_io_error_classifies_by_io_kind() {
        // PathMissing-shape kinds.
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::NotADirectory,
        ] {
            let err = std::io::Error::from(kind);
            assert_eq!(
                EnumerateError::from_root_io_error(err),
                EnumerateError::RootMissing,
                "ErrorKind::{kind:?} at the root site must classify as RootMissing",
            );
        }
        // PermissionDenied.
        assert_eq!(
            EnumerateError::from_root_io_error(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            )),
            EnumerateError::RootPermissionDenied,
        );
        // Catch-all fallback for any other io kind.
        for kind in [std::io::ErrorKind::AlreadyExists, std::io::ErrorKind::UnexpectedEof] {
            let err = std::io::Error::from(kind);
            assert_eq!(
                EnumerateError::from_root_io_error(err),
                EnumerateError::RootUnresolvable,
                "ErrorKind::{kind:?} at the root site must fall through to RootUnresolvable",
            );
        }
    }

    /// Story 4.2 — `from_dir_io_error` mirrors `from_root_io_error` for a
    /// directory inside the root (e.g. `rollout_summaries/`), yielding the
    /// `Dir*` / `Unreadable` variants.
    #[test]
    fn from_dir_io_error_classifies_by_io_kind() {
        // PathMissing-shape kinds.
        for kind in [
            std::io::ErrorKind::NotFound,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::NotADirectory,
        ] {
            let err = std::io::Error::from(kind);
            assert_eq!(
                EnumerateError::from_dir_io_error(err),
                EnumerateError::DirMissing,
                "ErrorKind::{kind:?} at a dir site must classify as DirMissing",
            );
        }
        // PermissionDenied.
        assert_eq!(
            EnumerateError::from_dir_io_error(std::io::Error::from(
                std::io::ErrorKind::PermissionDenied
            )),
            EnumerateError::DirPermissionDenied,
        );
        // Catch-all fallback for any other io kind.
        for kind in [std::io::ErrorKind::AlreadyExists, std::io::ErrorKind::UnexpectedEof] {
            let err = std::io::Error::from(kind);
            assert_eq!(
                EnumerateError::from_dir_io_error(err),
                EnumerateError::Unreadable,
                "ErrorKind::{kind:?} at a dir site must fall through to Unreadable",
            );
        }
    }

    /// Pin the Display strings for every variant — they are the human-facing
    /// diagnostic surface and the variant set is fixed by the architecture.
    #[test]
    fn enumerate_error_display_strings_are_stable() {
        assert_eq!(EnumerateError::RootMissing.to_string(), "root missing");
        assert_eq!(
            EnumerateError::RootPermissionDenied.to_string(),
            "root permission denied",
        );
        assert_eq!(
            EnumerateError::RootUnresolvable.to_string(),
            "root unresolvable",
        );
        assert_eq!(EnumerateError::DirMissing.to_string(), "directory missing");
        assert_eq!(
            EnumerateError::DirPermissionDenied.to_string(),
            "directory permission denied",
        );
        assert_eq!(EnumerateError::Unreadable.to_string(), "directory unreadable");
        assert_eq!(
            EnumerateError::AllowlistedArtifactUnresolvable.to_string(),
            "allowlisted artifact unresolvable",
        );
    }

    #[test]
    fn opencode_discovery_basis_json_strings_are_stable() {
        assert_eq!(
            serde_json::to_string(&DiscoveryBasis::OpencodeGlobalConfig).expect("serialize"),
            "\"opencode_global_config\""
        );
        assert_eq!(
            serde_json::to_string(&DiscoveryBasis::OpencodeProjectDatabase).expect("serialize"),
            "\"opencode_project_database\""
        );
    }
}
