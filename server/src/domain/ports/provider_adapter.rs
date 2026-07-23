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
//! - Stories 1.4–1.6 append `enumerate` / `search` / `watch` /
//!   `stable_native_ids` to this same trait as their slices land. The trait is
//!   grown incrementally; it is intentionally NOT given speculative method
//!   stubs for un-shipped slices (Phase 0 doc, "trait 增量生长").
//!
//! Locked names (from the architecture spine):
//! - `discover` — produces Candidate Source metadata; never reads chat body.
//! - `enumerate` — full canonical enumeration of an already-confirmed Source.
//! - `search` — search-only / search-assisted provider queries.
//! - `watch` — produce debounced dirty hints (AD-8: watchers are hints only).
//! - `stable_native_ids` — declare whether the provider emits stable native
//!   unit ids; falls back to file-level unit when unstable (AD-30).
//! - `coverage_level` — declare `full | search_only | existence_only |
//!   unsupported`; partial results never become complete index truth (AD-18).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// The error returned by [`ProviderAdapter::enumerate_file_units`].
///
/// The spec forbids `Result<_, ()>` on the port (Code Map: "错误类型由实现
/// 定义，不要返回 `Result<_,()>`"). This concrete error type carries a
/// category the application layer can map onto its own structured
/// [`crate::domain::scan::ScanError`]; it deliberately carries NO path/body
/// detail (AD-13 safe surface).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnumerateError {
    /// The root itself could not be canonicalized / resolved (missing,
    /// unreadable, or not a directory). Maps to scan `EnumerationFailed`.
    RootUnresolvable,
    /// A directory inside the root (e.g. `rollout_summaries/`) could not be
    /// read during enumeration. Maps to scan `EnumerationFailed`.
    Unreadable,
}

impl std::fmt::Display for EnumerateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnumerateError::RootUnresolvable => f.write_str("root unresolvable"),
            EnumerateError::Unreadable => f.write_str("directory unreadable"),
        }
    }
}

impl std::error::Error for EnumerateError {}

/// The ProviderAdapter contract.
///
/// Story 1.2 ships the *discovery slice* only: `provider_id` (Phase 0),
/// `coverage_level`, and `discover`. The remaining architecture-spine methods
/// (`enumerate`, `search`, `watch`, `stable_native_ids`) are appended in
/// Stories 1.4–1.6 as their slices land — see the module doc.
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
}
