//! `domain::scan` — scan state machine, generation identity, and pure hashing
//! helpers (Story 1.4).
//!
//! This module fixes the scan domain model that `index::scan_store` persists
//! and that `application::scan` orchestrates:
//!
//! - [`ScanRunState`] — the persisted state machine `queued → running →
//!   staging → committing → succeeded | failed` (AD-5/AD-16). The `retry`
//!   variant exists in the enum but is **never written** in 1.4 (spec Never:
//!   bounded retry is Carver manually re-scanning; no automatic retry
//!   scheduling).
//! - [`Generation`] — opaque `gen_<n>` newtype where `n` is the `scan_runs`
//!   AUTOINCREMENT id (Design Notes — generation comes from the same
//!   AUTOINCREMENT as the run id, no clock/rand dependency).
//! - [`ScanOutcome`] / [`ScanStatus`] — DTOs returned to the UI. Neither
//!   carries body content or path detail (AD-13 safe surface; spec Design
//!   Notes "ScanOutcome/ScanStatus 无正文无路径细节").
//! - [`ScanError`] — application-layer error type mapped onto stable IPC codes
//!   by the IPC layer.
//! - [`fnv1a_hex`] / [`build_record_id`] — pure, dependency-free FNV-1a
//!   hashing and locator-based `record_id` generation (AD-15/AD-30). No `sha2`/
//!   `blake3`/`rand`/`chrono` — the Phase 0 locked stack forbids new
//!   dependencies (spec Never).

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::domain::source::SourceId;

/// The persisted scan state machine (AD-5/AD-16).
///
/// Serialization renames to stable snake_case wire strings; the TS mirror
/// (`src/api/scan.ts`) must match exactly. The `retry` variant exists so the
/// enum is complete against the architecture spine, but 1.4 **never writes
/// it** — the spec's Never list forbids automatic retry scheduling in this
/// Story.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScanRunState {
    /// Run row created, not yet executing.
    Queued,
    /// Manifest built; enumeration / hashing in progress.
    Running,
    /// File-level records being written to the staging generation.
    Staging,
    /// Final manifest re-validation passed; CAS commit in progress.
    Committing,
    /// CAS commit succeeded; this generation is now the active one.
    Succeeded,
    /// Run failed (any stage). `error_code` on the row records the category
    /// (e.g. `dirty_after_validation` — AD-36).
    Failed,
    /// Architecture-spine state for a scheduled retry. **Never written in
    /// 1.4** — bounded retry is Carver manually re-scanning (spec Never). The
    /// variant exists so the persisted enum is complete for Story 1.8.
    Retry,
}

impl ScanRunState {
    /// Stable wire string for storage (matches the serde rename).
    pub fn as_str(self) -> &'static str {
        match self {
            ScanRunState::Queued => "queued",
            ScanRunState::Running => "running",
            ScanRunState::Staging => "staging",
            ScanRunState::Committing => "committing",
            ScanRunState::Succeeded => "succeeded",
            ScanRunState::Failed => "failed",
            ScanRunState::Retry => "retry",
        }
    }

    /// Parse the stable wire string back. Named `parse_str` (not `from_str`)
    /// to avoid clashing with the `std::str::FromStr` trait method.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "queued" => Some(ScanRunState::Queued),
            "running" => Some(ScanRunState::Running),
            "staging" => Some(ScanRunState::Staging),
            "committing" => Some(ScanRunState::Committing),
            "succeeded" => Some(ScanRunState::Succeeded),
            "failed" => Some(ScanRunState::Failed),
            "retry" => Some(ScanRunState::Retry),
            _ => None,
        }
    }
}

/// Opaque generation handle — `gen_<n>` where `n` is the `scan_runs`
/// AUTOINCREMENT id (Design Notes — fencing token 方案). Both the run id and
/// the generation come from the same AUTOINCREMENT sequence, so there is no
/// clock or random dependency (same precedent as `src_<rowid>` in Story 1.3).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Generation(pub String);

impl Generation {
    /// Build a `Generation` from a `scan_runs` rowid. Kept crate-internal:
    /// only the scan store / application layer construct one from a rowid.
    pub(crate) fn from_rowid(rowid: i64) -> Self {
        Generation(format!("gen_{rowid}"))
    }
}

impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The DTO returned by a successful `scan_source` command (AD-13 safe
/// surface). Carries only counts and generation identity — no body content,
/// no path detail (spec Design Notes). `Ok` means success, so there is no
/// redundant `outcome` field on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanOutcome {
    /// The scanned Source's stable handle.
    pub source_id: SourceId,
    /// The `scan_runs` AUTOINCREMENT id of the run that committed.
    pub scan_id: i64,
    /// The generation that became active as a result of this scan.
    pub generation: Generation,
    /// Number of canonical records indexed into the active generation.
    pub records_indexed: u64,
}

/// The DTO returned by `get_scan_status` (AD-13 safe surface). Reports the
/// most recent run's state plus the currently-active generation and its
/// record count. No body, no path detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanStatus {
    /// The Source this status describes.
    pub source_id: SourceId,
    /// State of the most recent scan run, or `None` when the Source has never
    /// been scanned.
    pub state: Option<ScanRunState>,
    /// The currently-active generation, or `None` when no generation has
    /// committed successfully yet.
    pub active_generation: Option<Generation>,
    /// Number of records in the currently-active generation (`0` when none).
    pub active_records: u64,
}

/// A server-derived Source Inventory row. `complete_record_count` is absent
/// unless the provider declares full coverage; a missing value is never a
/// disguised zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceInventory {
    pub source_id: SourceId,
    pub provider: String,
    pub lifecycle_state: crate::domain::source::SourceLifecycle,
    pub root: String,
    pub native_project: Option<String>,
    pub coverage_level: String,
    pub health_state: crate::domain::source::HealthState,
    pub last_successful_scan: Option<i64>,
    pub complete_record_count: Option<u64>,
    pub latest_error: Option<String>,
}

/// Application-layer scan error. Each variant maps onto a stable IPC error
/// code in the IPC layer (AD-13). No body / credential / path detail is
/// carried — the safe message lives in the IPC envelope constructor.
#[derive(Debug)]
pub enum ScanError {
    /// `source_id` matched no registry row. Maps to `source_not_found`.
    SourceNotFound,
    /// The Source exists but is not in `confirmed` lifecycle (rejected /
    /// disabled). Maps to `scan_failed`.
    NotConfirmed,
    /// The Source's confirmed root failed validation at scan time (deleted,
    /// became a file, not a directory). Maps to `confirm_failed` (reused —
    /// root validation failed).
    RootInvalid,
    /// The confirmed path still resolves, but now identifies a different
    /// filesystem object than the one the user confirmed. Maps to
    /// `confirm_failed` and requires explicit re-confirmation.
    RootIdentityChanged,
    /// Enumeration returned no units while an earlier generation is active.
    /// An initial empty scan is valid, but activating an empty replacement
    /// would erase useful derived data after an unreadable-root failure.
    EmptyScanWithActiveGeneration,
    /// Enumeration of file units failed (root unreadable mid-scan). Maps to
    /// `scan_failed`.
    EnumerationFailed,
    /// A file read failed during hashing (permissions / vanished mid-scan).
    /// Maps to `scan_failed`.
    ReadFailed,
    /// An allowlisted Markdown source could not be decoded or canonicalized.
    /// Its body and path remain out of the error surface; the persisted run is
    /// marked `parse_failed` and any prior active generation stays visible.
    ParseFailed,
    /// The final manifest re-validation detected a source change (size /
    /// mtime / file-set drift). Maps to `scan_failed`; the run is marked
    /// `error_code='dirty_after_validation'` and its generation is never
    /// activated (AD-34/AD-36).
    DirtyAfterValidation,
    /// The commit CAS affected 0 rows — the run is no longer owned by this
    /// holder (recovered / superseded). Maps to `scan_failed`. The run is
    /// left in `committing` for the next boot to recover (spec Design Notes).
    CommitCasFailed,
    /// A persisted cancel request changed this run out of its active state.
    Cancelled,
    /// An unexpected internal error (SQLite failure). Maps to `internal`.
    Internal,
}

impl ScanError {
    /// The stable `scan_runs.error_code` vocabulary value for this error
    /// (spec Design Notes — "error_code 稳定词汇"). This is the domain-layer
    /// mapping, kept SEPARATE from the IPC error-code mapping in the IPC
    /// layer: one is the persisted diagnostic category, the other is the
    /// wire error code.
    ///
    /// Vocabulary (non-empty `error_code` values):
    /// - `dirty_after_validation` — manifest drift (AD-36 persistent slot).
    /// - `read_failed` — a file read failed mid-scan.
    /// - `enumeration_failed` — enumeration of file units failed.
    /// - `internal` — an unexpected internal (SQLite) error.
    ///
    /// `stale_recovered` is the fifth vocabulary value but is written ONLY by
    /// boot recovery (it is not a `ScanError` variant). Variants that never
    /// reach a persisted run row ([`ScanError::SourceNotFound`],
    /// [`ScanError::NotConfirmed`], [`ScanError::RootInvalid`]) or that must
    /// NOT re-mark the row ([`ScanError::CommitCasFailed`]) map to the
    /// internal catch-all; the orchestrator does not call `fail_run` for them.
    pub fn error_code(&self) -> &'static str {
        match self {
            ScanError::DirtyAfterValidation => "dirty_after_validation",
            ScanError::ReadFailed => "read_failed",
            ScanError::ParseFailed => "parse_failed",
            ScanError::EnumerationFailed | ScanError::EmptyScanWithActiveGeneration => {
                "enumeration_failed"
            }
            // SourceNotFound / NotConfirmed / RootInvalid / CommitCasFailed
            // never write a row via fail_run (see fail-run policy in
            // application::scan). The internal catch-all keeps the mapping
            // total without inventing row-less vocabulary.
            ScanError::SourceNotFound
            | ScanError::NotConfirmed
            | ScanError::RootInvalid
            | ScanError::RootIdentityChanged
            | ScanError::CommitCasFailed
            | ScanError::Cancelled
            | ScanError::Internal => "internal",
        }
    }
}

// ---------------------------------------------------------------------------
// FNV-1a hashing — pure, dependency-free, deterministic.
// ---------------------------------------------------------------------------

/// FNV-1a 64-bit hash of `bytes`, rendered as a zero-padded 16-char lowercase
/// hex string. Pure and dependency-free (spec Never: no `sha2`/`blake3`/`rand`).
///
/// FNV-1a is a non-cryptographic hash; it is used here ONLY for content change
/// detection and stable `record_id` derivation (AD-15), never for security.
/// The 64-bit FNV-1a parameters are the standard ones: offset basis
/// `0xcbf29ce484222325`, prime `0x100000001b3`.
pub fn fnv1a_hex(bytes: &[u8]) -> String {
    const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash: u64 = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

/// Build a stable, locator-based `record_id` (AD-15/AD-30).
///
/// The identity is `rec_<fnv1a(netstring(source_id|provider|native_locator|unit_kind))>`.
/// Netstring length-prefixing makes the concatenation unambiguous and
/// injection-safe (same scheme as `build_fingerprint` in `domain::source`).
///
/// - **Locator-based, not content-based** (AD-15): re-scanning an unchanged
///   file produces the SAME `record_id`; only the `content_hash` changes.
/// - **`native_locator`** is the canonical file URI (`file://<absolute>`).
/// - **`unit_kind`** is `'file'` in 1.4 (file-level unit — no section
///   identity, AD-30).
///
/// Same inputs → same bytes → same id → idempotent re-scan.
pub fn build_record_id(
    source_id: &SourceId,
    provider: &str,
    native_locator: &str,
    unit_kind: &str,
) -> String {
    let mut buf = String::new();
    push_netstring(&mut buf, source_id.0.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, provider.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, native_locator.as_bytes());
    buf.push('|');
    push_netstring(&mut buf, unit_kind.as_bytes());
    format!("rec_{}", fnv1a_hex(buf.as_bytes()))
}

/// Append `<len>:<bytes>` to `s`, where `len` is the UTF-8 byte length of
/// `bytes`. Identical scheme to `domain::source::push_netstring` — the length
/// prefix is what makes the encoding unambiguous and injection-safe.
fn push_netstring(s: &mut String, bytes: &[u8]) {
    s.push_str(&bytes.len().to_string());
    s.push(':');
    // All inputs (SourceId, provider id, file URI, unit kind) are valid UTF-8
    // by construction; pushing via from_utf8 re-validates and panics on a
    // programmer error rather than silently corrupting the id.
    s.push_str(std::str::from_utf8(bytes).expect("netstring content is UTF-8"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_run_state_round_trips() {
        for state in [
            ScanRunState::Queued,
            ScanRunState::Running,
            ScanRunState::Staging,
            ScanRunState::Committing,
            ScanRunState::Succeeded,
            ScanRunState::Failed,
            ScanRunState::Retry,
        ] {
            assert_eq!(ScanRunState::parse_str(state.as_str()), Some(state));
        }
        assert_eq!(ScanRunState::parse_str("nope"), None);
    }

    #[test]
    fn scan_run_state_wire_strings_are_stable() {
        // Pin the exact snake_case wire strings the TS mirror depends on.
        assert_eq!(ScanRunState::Queued.as_str(), "queued");
        assert_eq!(ScanRunState::Running.as_str(), "running");
        assert_eq!(ScanRunState::Staging.as_str(), "staging");
        assert_eq!(ScanRunState::Committing.as_str(), "committing");
        assert_eq!(ScanRunState::Succeeded.as_str(), "succeeded");
        assert_eq!(ScanRunState::Failed.as_str(), "failed");
        assert_eq!(ScanRunState::Retry.as_str(), "retry");
    }

    #[test]
    fn generation_from_rowid_formats_gen_prefix() {
        let g = Generation::from_rowid(7);
        assert_eq!(g.0, "gen_7");
        assert_eq!(format!("{g}"), "gen_7");
    }

    #[test]
    fn fnv1a_hex_is_deterministic_and_16_chars() {
        let a = fnv1a_hex(b"hello");
        let b = fnv1a_hex(b"hello");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16, "zero-padded 16-char hex");
        // Different input → different hash (overwhelmingly likely).
        assert_ne!(fnv1a_hex(b"hello"), fnv1a_hex(b"world"));
    }

    #[test]
    fn fnv1a_hex_matches_known_vector() {
        // Standard FNV-1a 64-bit test vector: hash of empty string is the
        // offset basis.
        assert_eq!(fnv1a_hex(b""), "cbf29ce484222325");
        // Hash of "a" = (0xcbf29ce484222325 ^ 0x61) * 0x100000001b3 mod 2^64.
        assert_eq!(fnv1a_hex(b"a"), "af63dc4c8601ec8c");
    }

    #[test]
    fn build_record_id_is_stable_for_same_inputs() {
        let sid = SourceId("src_1".to_string());
        let a = build_record_id(&sid, "codex", "file:///x/MEMORY.md", "file");
        let b = build_record_id(&sid, "codex", "file:///x/MEMORY.md", "file");
        assert_eq!(a, b, "same inputs → same record_id (idempotent)");
        assert!(a.starts_with("rec_"), "rec_ prefix");
    }

    #[test]
    fn build_record_id_differs_when_any_input_differs() {
        let sid = SourceId("src_1".to_string());
        let base = build_record_id(&sid, "codex", "file:///x/MEMORY.md", "file");
        let diff_source = build_record_id(
            &SourceId("src_2".to_string()),
            "codex",
            "file:///x/MEMORY.md",
            "file",
        );
        let diff_provider = build_record_id(&sid, "claude_code", "file:///x/MEMORY.md", "file");
        let diff_locator = build_record_id(&sid, "codex", "file:///x/other.md", "file");
        let diff_kind = build_record_id(&sid, "codex", "file:///x/MEMORY.md", "section");
        assert_ne!(base, diff_source);
        assert_ne!(base, diff_provider);
        assert_ne!(base, diff_locator);
        assert_ne!(base, diff_kind);
    }

    #[test]
    fn build_record_id_is_injection_safe_against_length_collisions() {
        // Netstring length-prefixing disambiguates variable-length segments.
        let a = build_record_id(&SourceId("src_1".to_string()), "ab", "file:///c", "file");
        let b = build_record_id(&SourceId("src_1".to_string()), "a", "bfile:///c", "file");
        assert_ne!(a, b);
    }

    #[test]
    fn scan_outcome_wire_shape_is_stable() {
        let outcome = ScanOutcome {
            source_id: SourceId("src_3".to_string()),
            scan_id: 9,
            generation: Generation("gen_9".to_string()),
            records_indexed: 2,
        };
        let json = serde_json::to_string(&outcome).expect("serialize");
        assert!(json.contains("\"source_id\":\"src_3\""));
        assert!(json.contains("\"scan_id\":9"));
        assert!(json.contains("\"generation\":\"gen_9\""));
        assert!(json.contains("\"records_indexed\":2"));
        // No body / path detail leaks into the DTO.
        assert!(!json.contains("body"));
        assert!(!json.contains("path"));
        // No redundant outcome field (Ok means success).
        assert!(!json.contains("\"outcome\""));
    }

    #[test]
    fn scan_status_wire_shape_is_stable() {
        let status = ScanStatus {
            source_id: SourceId("src_1".to_string()),
            state: Some(ScanRunState::Succeeded),
            active_generation: Some(Generation("gen_1".to_string())),
            active_records: 4,
        };
        let json = serde_json::to_string(&status).expect("serialize");
        assert!(json.contains("\"state\":\"succeeded\""), "json: {json}");
        assert!(json.contains("\"active_generation\":\"gen_1\""));
        assert!(json.contains("\"active_records\":4"));

        // Never-scanned shape: state and active_generation are null.
        let empty = ScanStatus {
            source_id: SourceId("src_2".to_string()),
            state: None,
            active_generation: None,
            active_records: 0,
        };
        let json = serde_json::to_string(&empty).expect("serialize");
        assert!(json.contains("\"state\":null"));
        assert!(json.contains("\"active_generation\":null"));
    }

    #[test]
    fn scan_error_variants_are_debug() {
        // Compile-check that every variant exists and is Debug so the IPC
        // mapping can name them without surprises.
        for e in [
            ScanError::SourceNotFound,
            ScanError::NotConfirmed,
            ScanError::RootInvalid,
            ScanError::RootIdentityChanged,
            ScanError::EnumerationFailed,
            ScanError::EmptyScanWithActiveGeneration,
            ScanError::ReadFailed,
            ScanError::ParseFailed,
            ScanError::DirtyAfterValidation,
            ScanError::CommitCasFailed,
            ScanError::Internal,
        ] {
            let _ = format!("{e:?}");
        }
    }

    #[test]
    fn scan_error_error_code_vocabulary_is_stable() {
        // Pin the exact persisted error_code vocabulary (spec Design Notes —
        // "error_code 稳定词汇"). The 1.8 UX depends on these strings to
        // distinguish failure categories.
        assert_eq!(
            ScanError::DirtyAfterValidation.error_code(),
            "dirty_after_validation"
        );
        assert_eq!(ScanError::ReadFailed.error_code(), "read_failed");
        assert_eq!(ScanError::ParseFailed.error_code(), "parse_failed");
        assert_eq!(
            ScanError::EnumerationFailed.error_code(),
            "enumeration_failed"
        );
        assert_eq!(
            ScanError::EmptyScanWithActiveGeneration.error_code(),
            "enumeration_failed"
        );
        assert_eq!(ScanError::Internal.error_code(), "internal");
        // Variants that never reach fail_run map to the internal catch-all so
        // the mapping stays total (they are not written to a run row).
        assert_eq!(ScanError::SourceNotFound.error_code(), "internal");
        assert_eq!(ScanError::NotConfirmed.error_code(), "internal");
        assert_eq!(ScanError::RootInvalid.error_code(), "internal");
        assert_eq!(ScanError::RootIdentityChanged.error_code(), "internal");
        assert_eq!(ScanError::CommitCasFailed.error_code(), "internal");
    }
}
