//! `domain::source` — Source identity, lifecycle, and fingerprint types.
//!
//! This module fixes the Source domain model that the Source Registry
//! (`index::source_registry`) persists and that the application layer
//! (`application::source`) orchestrates. Story 1.3 introduces:
//!
//! - [`SourceId`] — opaque `src_<n>` handle, stable across restarts.
//! - [`SourceLifecycle`] — `confirmed | disabled | rejected`, all persisted.
//! - [`SourceKind`] — MVP only `agent_memory` (AD-10/A-19).
//! - [`HealthState`] — column exists in 1.3 but is always `unknown` (AD-7;
//!   health tracking is 1.8/4.x).
//! - [`FilesystemIdentity`] — `(device, file_id)` tuple used in fingerprints.
//! - [`Source`] — the DTO returned to the UI. Fingerprint is hidden on the
//!   wire (`#[serde(skip)]`) because it is an internal matching key with no
//!   business value to expose (Design Notes — "为何 Source DTO 隐藏
//!   fingerprint").
//! - [`build_fingerprint`] — pure, dependency-free, versioned
//!   (`root-fingerprint/v1`) netstring encoding of
//!   `provider + root kind + normalized root path + filesystem identity`.
//!
//! Architecture invariants honoured (AD-33/AD-35):
//! - `source_id` is the stable handle, independent of path/inode (Design
//!   Notes). The fingerprint is the *match key*; the id is the *handle*.
//! - Matching is **exact equality** (AD-35 "no fuzzy merge"). A path or inode
//!   change produces a different fingerprint and therefore a different Source
//!   row — the old row is preserved, never auto-merged.
//! - Source rows carry no timestamps: the architecture SOURCE ER entity has
//!   none, and last-scan / last-error times belong to Story 1.8 (Design
//!   Notes — "为何不加时间戳"). Avoiding timestamps also keeps `chrono`/`time`
//!   out of the Phase 0 locked stack.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Opaque, stable Source handle — `src_<n>` where `n` is the `source_registry`
/// `INTEGER PRIMARY KEY AUTOINCREMENT` value (Design Notes — "source_id 方案").
///
/// `AUTOINCREMENT` guarantees an id is never reused, even after a row is
/// deleted (there is no remove command in 1.3; that is a future feature —
/// A-7). The id is path/inode-independent: identity lives in the fingerprint,
/// the handle lives here.
///
/// Construction is intentionally limited: only the registry builds a
/// `SourceId` from a rowid. Application and IPC code consume them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl SourceId {
    /// Build a `SourceId` from a registry rowid. Kept crate-internal: callers
    /// outside the registry go through the registry's own row-mapping.
    pub(crate) fn from_rowid(rowid: i64) -> Self {
        SourceId(format!("src_{rowid}"))
    }

    /// Parse the rowid back out of a `src_<n>` handle, or `None` if the string
    /// is malformed. Used by the registry to translate an externally-supplied
    /// id into the SQLite rowid for `UPDATE ... WHERE id = ?`.
    pub fn to_rowid(&self) -> Option<i64> {
        let s = &self.0;
        s.strip_prefix("src_").and_then(|n| n.parse::<i64>().ok())
    }
}

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Lifecycle state of a Source (AD-7). Persisted on every confirm / reject /
/// disable so decisions survive restart (including rejection decisions — see
/// Design Notes "lifecycle 模型").
///
/// Serialization renames to stable snake_case wire strings; the TS mirror
/// (`src/api/sources.ts`) must match exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceLifecycle {
    /// User confirmed this Source is readable. Only `confirmed` Sources are
    /// eligible for scanning / indexing (Story 1.4). Confirming a previously
    /// rejected/disabled Source flips it back here (idempotent wake-up).
    Confirmed,
    /// User paused this Source. The row is preserved; the Source is not
    /// scanned until re-confirmed.
    Disabled,
    /// User rejected this candidate. The decision is persisted so the
    /// candidate does not re-surface as "new" on every boot.
    Rejected,
}

/// Domain kind of a Source (AD-10/A-19). MVP ships only Agent Memory; future
/// Knowledge Source kinds (`local_knowledge`, `remote_knowledge`) get their
/// own namespace, identity prefix, parser and migration history and must NOT
/// alias Agent Memory canonical schema (AD-19).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Codex / Claude Code auto-generated Agent Memory. The only MVP value.
    AgentMemory,
}

impl SourceKind {
    /// Stable wire string for storage (matches the serde rename).
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::AgentMemory => "agent_memory",
        }
    }

    /// Parse the stable wire string back. Returns `None` on an unknown value
    /// so the registry layer can surface corruption rather than silently
    /// coerce. Named `parse_str` (not `from_str`) to avoid clashing with the
    /// `std::str::FromStr` trait method.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "agent_memory" => Some(SourceKind::AgentMemory),
            _ => None,
        }
    }
}

/// Health state of a Source (AD-7). Column exists in 1.3 for schema stability
/// but is **always** written as `unknown`; health tracking (reachable /
/// permission-denied / format-drift / etc.) is Story 1.8 / 4.x. Kept as an
/// enum so the TS mirror and future migrations do not have to rewrite it from
/// a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    /// 1.3 sentinel — no health probe has run yet.
    Unknown,
}

impl HealthState {
    pub fn as_str(self) -> &'static str {
        match self {
            HealthState::Unknown => "unknown",
        }
    }

    /// Parse the stable wire string back. Named `parse_str` (not `from_str`)
    /// to avoid clashing with the `std::str::FromStr` trait method.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "unknown" => Some(HealthState::Unknown),
            _ => None,
        }
    }
}

/// Filesystem identity tuple `(device, file_id)` used in fingerprints when
/// available (AD-35). On Unix this is `(st_dev, st_ino)`; the field is
/// `Option` because non-Unix platforms or missing metadata fall back to
/// normalized-path-only fingerprints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FilesystemIdentity {
    pub device: u64,
    pub file_id: u64,
}

/// Root kind label embedded in the fingerprint. MVP Source roots are
/// directories; the label is part of the fingerprint so a future non-dir
/// provider cannot collide with a dir provider over the same path.
///
/// Kept as a function-local helper rather than a registry enum because the
/// only call site is the application's confirm path, which always passes
/// `"dir"`. A future non-dir provider (Epic 2+) will extend the enum.
pub const ROOT_KIND_DIR: &str = "dir";

/// Versioned, deterministic Source fingerprint (AD-33/AD-35).
///
/// Built from `provider + root kind + normalized root path + filesystem
/// identity` via [`build_fingerprint`]. Matched by exact string equality —
/// no fuzzy merge (AD-35). The version tag `root-fingerprint/v1` is embedded
/// in the string so a future v2 scheme can coexist.
///
/// `Default` is derived only because `#[serde(skip)]` on the `Source` DTO's
/// fingerprint field requires it for `Deserialize`; a default-constructed
/// fingerprint is never a valid match key (it has the empty string).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct SourceFingerprint(pub String);

impl SourceFingerprint {
    /// The version tag embedded at the head of every fingerprint string.
    pub const VERSION_TAG: &'static str = "root-fingerprint/v1";
}

/// The Source DTO returned to the UI. Fingerprint is hidden on the wire
/// (`#[serde(skip)]`): it is an internal matching key, not user-facing data
/// (Design Notes). Normalized root path IS surfaced so the user can see the
/// real root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Source {
    /// Opaque stable handle (`src_<n>`).
    pub source_id: SourceId,
    /// Stable lowercase provider id (`codex`, `claude_code`, ...).
    pub provider: String,
    /// Domain kind (MVP: `agent_memory`).
    pub source_kind: SourceKind,
    /// Lifecycle state (`confirmed` / `disabled` / `rejected`).
    pub lifecycle_state: SourceLifecycle,
    /// Health state (always `unknown` in 1.3).
    pub health_state: HealthState,
    /// Provider's declared coverage level. Single source of truth on confirm:
    /// taken from the adapter, NOT from the candidate payload (Design Notes —
    /// "coverage 单一事实源").
    pub coverage_level: crate::domain::ports::provider_adapter::CoverageLevel,
    /// Canonicalized root path (AD-4). Shown to the user so they see the real
    /// root, not the discovery-time probe path.
    pub normalized_root_path: String,
    /// Provider-native project id when discoverable; `None` for Codex (global
    /// store).
    pub native_project: Option<String>,
    /// Internal matching key. `#[serde(skip)]` so it stays in memory but
    /// never crosses the IPC boundary.
    #[serde(skip)]
    pub fingerprint: SourceFingerprint,
}

// ---------------------------------------------------------------------------
// build_fingerprint — pure, dependency-free, versioned netstring encoding.
// ---------------------------------------------------------------------------

/// Build a versioned Source fingerprint from its inputs (AD-33/AD-35).
///
/// The encoding is a netstring-style length-prefixed concatenation so that
/// variable-length strings cannot collide (injection-safe) and no parsing is
/// ever required — fingerprints are only compared for exact equality.
///
/// Shape:
/// ```text
/// root-fingerprint/v1|<len>:<bytes>|<len>:<bytes>|<len>:<bytes>|i<dev>:<file_id>
/// ```
/// where `<len>` is the UTF-8 byte length of the following `<bytes>`, and `|`
/// is a fixed separator. When identity is `None` (non-Unix or metadata
/// missing), the tail segment is the literal `n` (normalized-path explicit
/// fallback — Design Notes).
///
/// Same inputs → same bytes → same string → same row (idempotent confirm).
/// Different path OR different inode → different fingerprint → different row
/// (no auto-merge; degraded handling is Story 4.3).
pub fn build_fingerprint(
    provider: &str,
    root_kind: &str,
    normalized_path: &Path,
    identity: Option<FilesystemIdentity>,
) -> SourceFingerprint {
    let provider_bytes = provider.as_bytes();
    let root_kind_bytes = root_kind.as_bytes();
    // Use the raw bytes of the normalized path so non-UTF-8 cannot sneak in
    // via lossy conversion. (Confirm rejects non-UTF-8 roots upstream, so in
    // practice this is always valid UTF-8; we still operate on bytes for
    // fingerprint determinism.)
    let path_bytes = normalized_path.as_os_str().as_encoded_bytes();

    let mut s = String::new();
    s.push_str(SourceFingerprint::VERSION_TAG);
    s.push('|');
    push_netstring(&mut s, provider_bytes);
    s.push('|');
    push_netstring(&mut s, root_kind_bytes);
    s.push('|');
    push_netstring(&mut s, path_bytes);
    s.push('|');
    match identity {
        Some(id) => {
            // `i` prefix marks the identity segment and disambiguates it from
            // a path whose netstring happened to be `i...`.
            s.push('i');
            s.push_str(&id.device.to_string());
            s.push(':');
            s.push_str(&id.file_id.to_string());
        }
        None => {
            // Explicit normalized-path fallback (AD-35 / Design Notes).
            s.push('n');
        }
    }
    SourceFingerprint(s)
}

/// Append `<len>:<bytes>` to `s`, where `len` is the UTF-8 / byte length of
/// `bytes`. The length prefix is what makes the encoding unambiguous and
/// injection-safe: two different `(provider, kind, path)` triples can never
/// produce the same concatenated string.
fn push_netstring(s: &mut String, bytes: &[u8]) {
    s.push_str(&bytes.len().to_string());
    s.push(':');
    // SAFETY: we only ever push netstrings for UTF-8 content (provider ids,
    // root kind labels, and paths that confirm has already verified as UTF-8).
    // Operating on raw bytes above was for length correctness; pushing via
    // from_utf8 here re-validates and would panic on a programmer error
    // rather than silently corrupt the fingerprint.
    s.push_str(std::str::from_utf8(bytes).expect("netstring content is UTF-8"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_id_from_rowid_roundtrips() {
        let id = SourceId::from_rowid(42);
        assert_eq!(id.0, "src_42");
        assert_eq!(id.to_rowid(), Some(42));
        assert_eq!(format!("{id}"), "src_42");
    }

    #[test]
    fn source_id_to_rowid_rejects_malformed() {
        assert!(SourceId("src_x".to_string()).to_rowid().is_none());
        assert!(SourceId("not_prefixed".to_string()).to_rowid().is_none());
        assert!(SourceId("src_".to_string()).to_rowid().is_none());
    }

    #[test]
    fn build_fingerprint_is_deterministic() {
        let path = Path::new("/Users/c/.codex/memories");
        let id = Some(FilesystemIdentity { device: 16777231, file_id: 9876543 });
        let a = build_fingerprint("codex", ROOT_KIND_DIR, path, id);
        let b = build_fingerprint("codex", ROOT_KIND_DIR, path, id);
        assert_eq!(a, b);
    }

    #[test]
    fn build_fingerprint_matches_design_notes_example_shape() {
        // Design Notes example (representative, not byte-for-byte since the
        // real dev/ino come from the live fs). The netstring length prefix
        // is the UTF-8 byte length of each segment:
        //   "codex" = 5 bytes, "dir" = 3 bytes,
        //   "/Users/c/.codex/memories" = 24 bytes (not 25 — the spec's
        //   illustrative number was off by one; the real invariant is
        //   byte-length-prefixed segments).
        let path = Path::new("/Users/c/.codex/memories");
        let id = Some(FilesystemIdentity { device: 16777231, file_id: 9876543 });
        let fp = build_fingerprint("codex", ROOT_KIND_DIR, path, id).0;
        assert!(fp.starts_with("root-fingerprint/v1|"), "fp was: {fp}");
        assert!(fp.contains("|5:codex|"), "fp was: {fp}");
        assert!(fp.contains("|3:dir|"), "fp was: {fp}");
        // Path byte length is 24 ("/Users/c/.codex/memories" is 24 bytes).
        assert!(fp.contains("|24:/Users/c/.codex/memories|"), "fp was: {fp}");
        assert!(fp.ends_with("|i16777231:9876543"), "fp was: {fp}");
    }

    #[test]
    fn build_fingerprint_uses_normalized_path_fallback_when_identity_missing() {
        let path = Path::new("/x/memories");
        let fp = build_fingerprint("codex", ROOT_KIND_DIR, path, None).0;
        assert!(fp.ends_with("|n"), "explicit fallback marker; fp was: {fp}");
    }

    #[test]
    fn build_fingerprint_differs_when_path_differs() {
        let id = Some(FilesystemIdentity { device: 1, file_id: 2 });
        let a = build_fingerprint("codex", ROOT_KIND_DIR, Path::new("/a/memories"), id);
        let b = build_fingerprint("codex", ROOT_KIND_DIR, Path::new("/b/memories"), id);
        assert_ne!(a, b, "different path → different fingerprint");
    }

    #[test]
    fn build_fingerprint_differs_when_inode_differs() {
        // AD-35: same path but inode changed (directory rebuilt) → different
        // fingerprint → different Source. No auto-merge.
        let path = Path::new("/x/memories");
        let a = build_fingerprint(
            "codex",
            ROOT_KIND_DIR,
            path,
            Some(FilesystemIdentity { device: 1, file_id: 100 }),
        );
        let b = build_fingerprint(
            "codex",
            ROOT_KIND_DIR,
            path,
            Some(FilesystemIdentity { device: 1, file_id: 200 }),
        );
        assert_ne!(a, b, "different inode → different fingerprint");
    }

    #[test]
    fn build_fingerprint_differs_when_provider_or_kind_differs() {
        let id = Some(FilesystemIdentity { device: 1, file_id: 2 });
        let path = Path::new("/x/memories");
        let a = build_fingerprint("codex", ROOT_KIND_DIR, path, id);
        let b = build_fingerprint("claude_code", ROOT_KIND_DIR, path, id);
        assert_ne!(a, b);
        let c = build_fingerprint("codex", "file", path, id);
        assert_ne!(a, c, "different root kind → different fingerprint");
    }

    #[test]
    fn build_fingerprint_is_injection_safe_against_length_collisions() {
        // Two pairs that would collide under plain concatenation but not under
        // length-prefixed netstring encoding.
        let a = build_fingerprint(
            "ab",
            ROOT_KIND_DIR,
            Path::new("/c"),
            None,
        );
        let b = build_fingerprint(
            "a",
            ROOT_KIND_DIR,
            Path::new("b/c"),
            None,
        );
        assert_ne!(a, b, "netstring prefix must disambiguate");
    }

    #[test]
    fn source_dto_hides_fingerprint_on_wire() {
        let src = Source {
            source_id: SourceId::from_rowid(1),
            provider: "codex".to_string(),
            source_kind: SourceKind::AgentMemory,
            lifecycle_state: SourceLifecycle::Confirmed,
            health_state: HealthState::Unknown,
            coverage_level: crate::domain::ports::provider_adapter::CoverageLevel::Full,
            normalized_root_path: "/x/memories".to_string(),
            native_project: None,
            fingerprint: SourceFingerprint("root-fingerprint/v1|secret".to_string()),
        };
        let json = serde_json::to_string(&src).expect("serialize");
        assert!(!json.contains("fingerprint"), "wire shape hides fp; json: {json}");
        assert!(!json.contains("secret"), "internal key not leaked; json: {json}");
        assert!(json.contains("\"source_id\":\"src_1\""));
        assert!(json.contains("\"lifecycle_state\":\"confirmed\""));
        assert!(json.contains("\"source_kind\":\"agent_memory\""));
        assert!(json.contains("\"health_state\":\"unknown\""));
        assert!(json.contains("\"coverage_level\":\"full\""));
    }

    #[test]
    fn source_kind_round_trips() {
        assert_eq!(SourceKind::AgentMemory.as_str(), "agent_memory");
        assert_eq!(SourceKind::parse_str("agent_memory"), Some(SourceKind::AgentMemory));
        assert_eq!(SourceKind::parse_str("nope"), None);
    }

    #[test]
    fn health_state_round_trips() {
        assert_eq!(HealthState::Unknown.as_str(), "unknown");
        assert_eq!(HealthState::parse_str("unknown"), Some(HealthState::Unknown));
        assert_eq!(HealthState::parse_str("nope"), None);
    }
}
