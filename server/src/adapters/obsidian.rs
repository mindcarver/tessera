//! `adapters::obsidian` — read-only Obsidian Vault discovery (Story 6.2).
//!
//! Phase C.0 activates Obsidian Knowledge as a `local_knowledge` Source kind.
//! This module ships the **discovery slice**: parse the host's Obsidian vault
//! registry and emit one [`CandidateSource`] per registered vault whose root
//! currently exists. It does NOT read note bodies (NFR-5/NFR-14), does NOT
//! enumerate Markdown (Story 6.5), and does NOT confirm Sources or allocate
//! `source_id` (Story 6.3).
//!
//! ## The registry is an observed surface, not a stable API (AD-37)
//!
//! Obsidian does not document its vault-registry format as a public API. The
//! shape parsed here is the **observed** format on macOS/Windows/Linux:
//!
//! ```json
//! {
//!   "vaults": {
//!     "<16-hex-char-id>": { "path": "<abs path>", "ts": 1778076427361, "open"?: true }
//!   }
//! }
//! ```
//!
//! Because it is not a stable API, the parser is **fail-safe and
//! fixture-protected**: a missing, corrupt, or unknown-shape registry produces
//! a visible [`RegistryDiagnostic`] rather than a silently-empty vault set
//! (Story 6.2 AC). Agent Memory discovery/startup is never blocked by a
//! registry problem — Knowledge discovery is source-scoped (AD-13).
//!
//! ## Zero-write (NFR-14)
//!
//! Discovery reads the registry file and calls `metadata()`/`is_dir()` on each
//! vault root. It never opens a note, never writes anything, and never follows
//! symlink directories (AD-39 exclusion is enforced at enumeration in 6.5;
//! discovery itself only checks the root exists as a real directory).
//!
//! ## Testability
//!
//! Registry parsing is factored into [`parse_registry`] — a pure function of
//! bytes, no filesystem I/O — so fixtures exercise every observed and
//! unsupported shape under `cargo test` parallelism. [`discover_with_root`]
//! is the injected-root seam tests drive; [`discover`] is the production glue
//! that resolves the platform registry path via [`registry_path`].

use std::path::{Path, PathBuf};

use crate::domain::ports::provider_adapter::{CandidateSource, CoverageLevel, DiscoveryBasis};

/// Canonical provider id for Obsidian Knowledge Sources. Referenced by the
/// Knowledge confirm path (Story 6.3) and the Inventory (6.6); a rename here
/// propagates everywhere. Mirrors the const-on-adapter pattern Codex/Claude
/// use for `PROVIDER_ID`.
pub const PROVIDER_ID: &str = "obsidian";

/// Stable wire string persisted as `source_kind` for Obsidian vaults. Matches
/// [`crate::domain::source::SourceKind::LocalKnowledge`].
pub const SOURCE_KIND: &str = "local_knowledge";

/// Knowledge parser version persisted on every `knowledge_records` row
/// (Story 6.4 / AD-38). Independent from the Agent-Memory parser versions
/// (`codex-markdown/v1`, `claude-markdown/v1`) — a format change in Obsidian
/// note parsing bumps THIS tag and triggers a Knowledge rebuild, never an
/// Agent-Memory rebuild.
pub const KNOWLEDGE_PARSER_VERSION: &str = "obsidian-markdown/v1";

/// The file-level unit kind for Knowledge records (AD-38: one Markdown file =
/// one Knowledge Record; no heading/block identity in Phase C.0).
pub const UNIT_KIND_NOTE: &str = "note";

/// Build a stable, locator-based `krec_` record id for a Knowledge note
/// (Story 6.4 / AD-38). Independent from the Agent-Memory `rec_` id scheme:
/// `krec_<fnv1a(netstring(source_id|provider|vault_relative_path|unit_kind))>`.
///
/// The identity is **Vault-relative-path-based**, not content-based (mirrors
/// AD-15 for Agent Memory): re-indexing an unchanged note at the same
/// Vault-relative path produces the SAME `krec_` id; only `content_hash`
/// changes. A rename or move creates a new locator → new `krec_` id (no fuzzy
/// merge — AD-35/AD-38).
///
/// `vault_relative_path` is the note's path relative to the Vault root, using
/// forward slashes regardless of OS (stable across platforms). The
/// `native_locator` persisted in `knowledge_records` is the same string.
pub fn build_knowledge_record_id(
    source_id: &str,
    provider: &str,
    vault_relative_path: &str,
    unit_kind: &str,
) -> String {
    let mut buf = String::new();
    push_netstring(&mut buf, source_id.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, provider.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, vault_relative_path.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, unit_kind.as_bytes());
    format!("krec_{:016x}", fnv1a_hex(buf.as_bytes()))
}

/// FNV-1a 64-bit hash → lowercase hex (same algorithm as Agent-Memory's
/// `domain::scan::fnv1a_hex`; duplicated here to keep the Knowledge pipeline
/// free of any Agent-Memory dependency, per AD-19).
fn fnv1a_hex(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn push_netstring(s: &mut String, bytes: &[u8]) {
    s.push_str(&bytes.len().to_string());
    s.push(':');
    s.push_str(std::str::from_utf8(bytes).expect("netstring content is UTF-8"));
}

/// A fail-safe diagnostic describing why vault discovery could not produce a
/// complete candidate set (AD-37 / Story 6.2 AC). Carried alongside the
/// (possibly empty) candidate list so the UI can distinguish "no vaults
/// registered" from "registry unreadable".
///
/// The diagnostic is safe by construction (NFR-3): it never carries the raw
/// registry payload, a vault path, or a note body — only a stable categorical
/// code and an opaque detail string the operator controls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryDiagnostic {
    /// The platform registry path does not exist (Obsidian not installed, or
    /// never opened). Distinct from an empty-but-valid registry.
    RegistryMissing,
    /// The registry file exists but could not be read (permission denied, I/O
    /// error). The OS error kind is mapped to this stable code; the raw OS
    /// message is NOT surfaced (NFR-3).
    RegistryUnreadable,
    /// The registry was readable but its JSON shape is not the observed
    /// `{"vaults": {…}}` structure, or individual entries are malformed. A
    /// best-effort parse still emits candidates for well-formed entries; this
    /// diagnostic flags that some or all entries were skipped.
    RegistryCorrupt,
}

/// The result of Obsidian vault discovery: zero or more vault candidates plus
/// an optional diagnostic when the registry was not fully usable. An
/// `Ok` discovery with candidates and `None` diagnostic is the happy path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryResult {
    /// One candidate per registered vault whose root currently exists.
    pub candidates: Vec<CandidateSource>,
    /// `None` when the registry parsed cleanly (including an empty vault set).
    pub diagnostic: Option<RegistryDiagnostic>,
}

impl DiscoveryResult {
    fn ok(candidates: Vec<CandidateSource>) -> Self {
        DiscoveryResult {
            candidates,
            diagnostic: None,
        }
    }

    fn warn(candidates: Vec<CandidateSource>, diag: RegistryDiagnostic) -> Self {
        DiscoveryResult {
            candidates,
            diagnostic: Some(diag),
        }
    }
}

/// Resolve the platform-specific Obsidian vault-registry path.
///
/// Returns `None` when the platform config/home directory cannot be resolved
/// (no `HOME` on Unix, no `APPDATA`/`LOCALAPPDATA` on Windows) — this maps to
/// [`RegistryDiagnostic::RegistryMissing`] at the discover layer rather than a
/// hard error, so Agent Memory keeps working on a host without Obsidian.
///
/// Paths (AD-37 observed surface):
/// - macOS: `$HOME/Library/Application Support/obsidian/obsidian.json`
/// - Linux: `$XDG_CONFIG_HOME/obsidian/obsidian.json` or
///   `$HOME/.config/obsidian/obsidian.json`
/// - Windows: `%APPDATA%/obsidian/obsidian.json`
pub fn registry_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/obsidian/obsidian.json"))
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            return Some(PathBuf::from(xdg).join("obsidian/obsidian.json"));
        }
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join(".config/obsidian/obsidian.json"))
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("obsidian/obsidian.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// Production discovery glue: resolve the registry path, read it, parse it,
/// validate each vault root. Never reads note bodies (NFR-5).
pub fn discover() -> DiscoveryResult {
    let Some(path) = registry_path() else {
        return DiscoveryResult::warn(Vec::new(), RegistryDiagnostic::RegistryMissing);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return DiscoveryResult::warn(Vec::new(), RegistryDiagnostic::RegistryMissing);
        }
        Err(_) => {
            return DiscoveryResult::warn(Vec::new(), RegistryDiagnostic::RegistryUnreadable);
        }
    };
    discover_from_bytes(&bytes)
}

/// Discovery driven by raw registry bytes — the testable seam for fixture
/// shapes. Parses the registry, then validates each vault root exists as a
/// directory. Never reads note bodies (NFR-5).
fn discover_from_bytes(bytes: &[u8]) -> DiscoveryResult {
    let parsed = parse_registry(bytes);
    let mut candidates = Vec::with_capacity(parsed.entries.len());
    for entry in parsed.entries {
        if root_exists_as_dir(&entry.path) {
            if let Some(c) = build_candidate(&entry.path) {
                candidates.push(c);
            }
        }
        // A root that does not exist or is not a directory is silently skipped
        // at discovery — it is not a corruption (the registry is valid, the
        // vault was just moved/removed). The user sees it disappear from the
        // candidate list; confirming a moved vault is Story 6.3's rebind path.
    }
    candidates.sort_by(|a, b| a.root_path.cmp(&b.root_path));
    match parsed.diagnostic {
        Some(d) => DiscoveryResult::warn(candidates, d),
        None => DiscoveryResult::ok(candidates),
    }
}

/// Check a vault root exists and is a directory. Symlinks-to-dir are followed
/// (the registry stores real vault paths); a regular file or missing path
/// yields false. No note bodies are read (NFR-5).
fn root_exists_as_dir(path: &str) -> bool {
    Path::new(path).is_dir()
}

/// Build a CandidateSource for a vault root whose path is a UTF-8 string. A
/// non-UTF-8 path is dropped (would stringify to U+FFFD and cannot be
/// confirmed). NFR-5: no contents read.
fn build_candidate(path: &str) -> Option<CandidateSource> {
    let path_str = Path::new(path).to_str()?;
    Some(CandidateSource {
        provider: PROVIDER_ID.to_string(),
        root_path: path_str.to_string(),
        basis: DiscoveryBasis::ObsidianVaultRegistry,
        coverage_level: CoverageLevel::Full,
        // Obsidian vaults have no Agent-Memory "native project" concept; the
        // vault name itself is the Knowledge domain label (surfaced in 6.6
        // Inventory). The field stays None so Knowledge candidates never
        // collide with Agent-Memory project filters.
        native_project: None,
    })
}

/// A single parsed vault entry from the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct VaultEntry {
    /// The absolute filesystem path Obsidian recorded for this vault.
    pub path: String,
}

/// The outcome of parsing registry bytes (pure, no I/O).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedRegistry {
    entries: Vec<VaultEntry>,
    /// `Some` if the JSON was malformed or had an unknown shape. Best-effort:
    /// well-formed entries are still returned.
    diagnostic: Option<RegistryDiagnostic>,
}

/// Pure parser for the Obsidian vault registry. Takes raw bytes, returns
/// vault entries plus an optional corruption diagnostic. Fixture-protected
/// (AD-37): the observed shape is `{"vaults": {"<id>": {"path": "...", ...}}}`.
///
/// Malformed JSON, a missing `vaults` object, or entries missing/non-string
/// `path` produce [`RegistryDiagnostic::RegistryCorrupt`]. Well-formed entries
/// before/after a malformed one are still returned (best-effort), so a single
/// bad entry does not hide the others.
fn parse_registry(bytes: &[u8]) -> ParsedRegistry {
    // serde_json is in the locked stack (used by the HTTP layer). Parse as a
    // generic value so we never commit to more structure than observed.
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(_) => {
            return ParsedRegistry {
                entries: Vec::new(),
                diagnostic: Some(RegistryDiagnostic::RegistryCorrupt),
            };
        }
    };
    let Some(vaults) = value.get("vaults").and_then(|v| v.as_object()) else {
        return ParsedRegistry {
            entries: Vec::new(),
            diagnostic: Some(RegistryDiagnostic::RegistryCorrupt),
        };
    };
    let mut entries = Vec::new();
    let mut had_bad_entry = false;
    for (_id, entry) in vaults {
        // Each vault is an object whose "path" is a string. We intentionally
        // ignore "ts", "open", and any future keys — they are not part of the
        // discovery contract and relying on them would over-fit an unstable
        // surface.
        let path = match entry.get("path").and_then(|p| p.as_str()) {
            Some(p) => p,
            None => {
                had_bad_entry = true;
                continue;
            }
        };
        entries.push(VaultEntry {
            path: path.to_string(),
        });
    }
    ParsedRegistry {
        entries,
        diagnostic: had_bad_entry.then_some(RegistryDiagnostic::RegistryCorrupt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build registry bytes from a list of (id, path) pairs in the observed
    /// shape.
    fn registry_bytes(vaults: &[(&str, &str)]) -> Vec<u8> {
        let mut s = String::from(r#"{"vaults":{"#);
        for (i, (id, path)) in vaults.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                r#""{id}":{{"path":{},"ts":1778076427361}}"#,
                serde_json::to_string(path).unwrap()
            ));
        }
        s.push_str("}}");
        s.into_bytes()
    }

    #[test]
    fn parses_observed_registry_shape() {
        let bytes = registry_bytes(&[
            ("c761e43e9b3a4c5d", "/tmp/vault-a"),
            ("fd833c8a12345678", "/tmp/vault-b"),
        ]);
        let parsed = parse_registry(&bytes);
        assert_eq!(parsed.diagnostic, None);
        assert_eq!(parsed.entries.len(), 2);
        assert_eq!(parsed.entries[0].path, "/tmp/vault-a");
        assert_eq!(parsed.entries[1].path, "/tmp/vault-b");
    }

    #[test]
    fn empty_vaults_object_is_not_corruption() {
        // An empty-but-valid registry is NOT a diagnostic — it is the honest
        // "no registered vaults" state.
        let bytes = br#"{"vaults":{}}"#;
        let parsed = parse_registry(bytes);
        assert_eq!(parsed.diagnostic, None);
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn missing_vaults_key_is_corruption() {
        let bytes = br#"{"version":"1.0"}"#;
        let parsed = parse_registry(bytes);
        assert_eq!(parsed.diagnostic, Some(RegistryDiagnostic::RegistryCorrupt));
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn invalid_json_is_corruption() {
        let bytes = b"not json at all {{{";
        let parsed = parse_registry(bytes);
        assert_eq!(parsed.diagnostic, Some(RegistryDiagnostic::RegistryCorrupt));
        assert!(parsed.entries.is_empty());
    }

    #[test]
    fn entry_missing_path_is_flagged_but_good_entries_kept() {
        // Best-effort: a malformed entry sets the corrupt diagnostic but does
        // not hide well-formed siblings.
        let bytes = br#"{"vaults":{"good":{"path":"/tmp/good"},"bad":{"ts":123}}}"#;
        let parsed = parse_registry(bytes);
        assert_eq!(parsed.diagnostic, Some(RegistryDiagnostic::RegistryCorrupt));
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "/tmp/good");
    }

    #[test]
    fn extra_entry_keys_are_ignored() {
        // "ts" and "open" are observed but not relied upon; future keys must
        // not break the parser (AD-37 unstable surface).
        let bytes = br#"{"vaults":{"abc":{"path":"/tmp/v","ts":1,"open":true,"future":"x"}}}"#;
        let parsed = parse_registry(bytes);
        assert_eq!(parsed.diagnostic, None);
        assert_eq!(parsed.entries.len(), 1);
        assert_eq!(parsed.entries[0].path, "/tmp/v");
    }

    #[test]
    fn discover_emits_candidate_only_for_existing_dirs() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real_vault = tmp.path().join("real-vault");
        std::fs::create_dir_all(&real_vault).unwrap();
        let bytes = registry_bytes(&[
            ("a", real_vault.to_str().unwrap()),
            ("b", "/tmp/this-vault-was-moved-xyz"),
        ]);
        let result = discover_from_bytes(&bytes);
        assert_eq!(result.diagnostic, None, "missing root is not corruption");
        assert_eq!(result.candidates.len(), 1, "only the existing vault is a candidate");
        assert_eq!(result.candidates[0].provider, PROVIDER_ID);
        assert_eq!(
            result.candidates[0].basis,
            DiscoveryBasis::ObsidianVaultRegistry
        );
        assert_eq!(result.candidates[0].coverage_level, CoverageLevel::Full);
        assert_eq!(result.candidates[0].native_project, None);
    }

    #[test]
    fn discover_carries_corruption_diagnostic_alongside_good_candidates() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real_vault = tmp.path().join("ok");
        std::fs::create_dir_all(&real_vault).unwrap();
        let bytes = {
            let mut s = String::from(r#"{"vaults":{"#);
            s.push_str(&format!(
                r#""ok":{{"path":{}}},"#,
                serde_json::to_string(real_vault.to_str().unwrap()).unwrap()
            ));
            s.push_str(r#""bad":{"no_path":1}}}"#);
            s.into_bytes()
        };
        let result = discover_from_bytes(&bytes);
        assert_eq!(
            result.diagnostic,
            Some(RegistryDiagnostic::RegistryCorrupt)
        );
        assert_eq!(result.candidates.len(), 1, "good entry still surfaces");
    }

    #[test]
    fn discover_candidates_sorted_by_path() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let z = tmp.path().join("z-vault");
        let a = tmp.path().join("a-vault");
        std::fs::create_dir_all(&z).unwrap();
        std::fs::create_dir_all(&a).unwrap();
        let bytes = registry_bytes(&[
            ("1", z.to_str().unwrap()),
            ("2", a.to_str().unwrap()),
        ]);
        let result = discover_from_bytes(&bytes);
        assert_eq!(result.candidates.len(), 2);
        // Deterministic ordering regardless of registry iteration order.
        assert!(result.candidates[0].root_path < result.candidates[1].root_path);
    }

    /// Story 6.2 AC: same-name Vaults at different roots remain distinct.
    /// The registry keys vaults by opaque id, not name, so two entries with
    /// the same display name but different roots produce two candidates.
    #[test]
    fn same_name_different_root_remain_distinct() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let notes1 = tmp.path().join("kb-1/Notes");
        let notes2 = tmp.path().join("kb-2/Notes");
        std::fs::create_dir_all(&notes1).unwrap();
        std::fs::create_dir_all(&notes2).unwrap();
        let bytes = registry_bytes(&[
            ("id1", notes1.to_str().unwrap()),
            ("id2", notes2.to_str().unwrap()),
        ]);
        let result = discover_from_bytes(&bytes);
        assert_eq!(result.candidates.len(), 2);
        assert_ne!(
            result.candidates[0].root_path,
            result.candidates[1].root_path
        );
    }

    // --- Story 6.4 — independent krec_ identity + Knowledge parser version ---

    #[test]
    fn knowledge_record_id_is_locator_based_and_deterministic() {
        // Same inputs → same id (AD-15 locator-based identity).
        let a = build_knowledge_record_id("src_1", "obsidian", "Notes/foo.md", "note");
        let b = build_knowledge_record_id("src_1", "obsidian", "Notes/foo.md", "note");
        assert_eq!(a, b);
        assert!(a.starts_with("krec_"), "got: {a}");
    }

    #[test]
    fn knowledge_record_id_differs_when_vault_relative_path_differs() {
        // AD-38: rename/move → new locator → new id (no fuzzy merge).
        let a = build_knowledge_record_id("src_1", "obsidian", "Notes/foo.md", "note");
        let b = build_knowledge_record_id("src_1", "obsidian", "Notes/bar.md", "note");
        assert_ne!(a, b, "different path → different krec_ id");
    }

    #[test]
    fn knowledge_record_id_differs_when_source_differs() {
        let a = build_knowledge_record_id("src_1", "obsidian", "Notes/foo.md", "note");
        let b = build_knowledge_record_id("src_2", "obsidian", "Notes/foo.md", "note");
        assert_ne!(a, b, "different source → different krec_ id");
    }

    #[test]
    fn knowledge_record_id_does_not_collide_with_agent_memory_rec_scheme() {
        // AD-19: the Knowledge id namespace must be distinct from Agent Memory.
        // The Agent-Memory builder produces rec_<fnv1a(source_id|provider|
        // native_locator|unit_kind)>; even with the same logical inputs the
        // krec_ prefix partitions the namespaces so a record_id from one
        // domain can never be confused for the other.
        let krec = build_knowledge_record_id("src_1", "obsidian", "Notes/foo.md", "note");
        assert!(krec.starts_with("krec_"));
        assert!(!krec.starts_with("rec_"));
    }

    #[test]
    fn knowledge_parser_version_is_independent_from_agent_memory() {
        // AD-38: the parser version is a distinct tag, not a reuse of Codex/
        // Claude versions. A Knowledge format change never triggers an Agent
        // rebuild and vice versa.
        assert_eq!(KNOWLEDGE_PARSER_VERSION, "obsidian-markdown/v1");
        assert_ne!(KNOWLEDGE_PARSER_VERSION, "codex-markdown/v1");
        assert_ne!(KNOWLEDGE_PARSER_VERSION, "claude-markdown/v1");
        assert_eq!(UNIT_KIND_NOTE, "note");
    }
}
