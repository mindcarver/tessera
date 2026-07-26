//! `http::envelope` — versioned API envelope and structured error envelope.
//!
//! Phase 0 fixes the *shape*; later Stories populate concrete payload fields
//! and error codes as the corresponding endpoints land.

use serde::{Deserialize, Serialize};

/// The API contract major version. Carried on every response envelope so the
/// UI can route or reject payloads whose major it does not understand
/// (AD-17/A-6). Phase 0 starts at `"1"`; bump only on a breaking DTO change.
pub const API_VERSION: &str = "1";

/// Versioned success envelope wrapping a typed payload (AD-9/AD-17/A-6).
///
/// Every HTTP endpoint response uses this shape. The TypeScript mirror lives
/// in `src/api/ping.ts` and must be updated in lock-step with any change to
/// this struct.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    /// IPC contract major version (string, e.g. `"1"`).
    pub api_version: &'static str,
    /// Command-specific typed payload.
    pub payload: T,
}

/// `ping` payload — Phase 0 contract sample.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pong {
    /// Crate name from `CARGO_PKG_NAME` at build time.
    pub name: String,
    /// Crate version from `CARGO_PKG_VERSION` at build time.
    pub version: String,
}

/// Structured error envelope (AD-13).
///
/// Stable `code` + safe `message` + operation context; never carries body,
/// query text, credentials, or filesystem paths. `source_id` is `null` for
/// failures that cannot be attributed to a Source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// Stable, machine-readable error code (e.g. `"internal"`). Never
    /// localized and never contains user data.
    pub code: String,
    /// Safe, user-facing message. Defaults to a generic string; never
    /// includes memory body, query text, or credentials.
    pub message: String,
    /// Source handle associated with the failed operation, when one exists.
    /// The stable id is safe to expose; paths remain server-side only.
    pub source_id: Option<String>,
    /// Coarse operation phase (`source`, `scan`, `transport`, `internal`) so
    /// clients can distinguish context without parsing a display message.
    pub phase: String,
}

impl ErrorEnvelope {
    fn new(code: &str, message: &str, source_id: Option<&str>, phase: &str) -> Self {
        Self {
            code: code.to_string(),
            message: message.to_string(),
            source_id: source_id.map(str::to_string),
            phase: phase.to_string(),
        }
    }

    /// Construct a generic internal error envelope. Phase 0 has no real
    /// failure path; this helper exists so later Stories use the same shape.
    pub fn internal() -> Self {
        Self::new(
            "internal",
            "Tessera hit an internal error.",
            None,
            "internal",
        )
    }

    /// Construct an internal error tied to a safe Source handle and phase.
    pub fn internal_for(source_id: Option<&str>, phase: &str) -> Self {
        Self::new(
            "internal",
            "Tessera hit an internal error.",
            source_id,
            phase,
        )
    }

    pub fn bad_request(phase: &str) -> Self {
        let message = match phase {
            "search" => "The request did not match Tessera's search contract.",
            "open" => "The request did not match Tessera's open contract.",
            "project" => "The request did not match Tessera's project contract.",
            _ => "The request did not match Tessera's API contract.",
        };
        Self::new("bad_request", message, None, phase)
    }

    /// `cursor_stale` carries the endpoint `phase` so the UI can distinguish a
    /// browse pagination staleness from a search one without parsing the
    /// message. The safe message never includes body / query / record ids.
    pub fn cursor_stale(phase: &str) -> Self {
        let message = match phase {
            "browse" => "The index changed. Run the browse again.",
            _ => "The index changed. Run the search again.",
        };
        Self::new("cursor_stale", message, None, phase)
    }

    pub fn record_not_found() -> Self {
        Self::new(
            "record_not_found",
            "Tessera could not find that memory record.",
            None,
            "open",
        )
    }

    pub fn open_failed(source_id: Option<&str>) -> Self {
        Self::new(
            "open_failed",
            "Tessera could not open the original location.",
            source_id,
            "open",
        )
    }

    /// Construct a `confirm_failed` error envelope (Story 1.3). Stable code
    /// per AD-13; the safe message never includes body / query text /
    /// credentials. Emitted when confirm/reject cannot canonicalize the root
    /// (root missing / not a directory / not absolute — NFR-5/6).
    pub fn confirm_failed(source_id: Option<&str>, phase: &str) -> Self {
        Self::new(
            "confirm_failed",
            "Tessera could not confirm this source. The root must be an existing directory.",
            source_id,
            phase,
        )
    }

    /// Construct a `source_not_found` error envelope (Story 1.3). Emitted when
    /// a `source_id`-keyed operation (disable) targets an id that matches no
    /// registry row.
    pub fn source_not_found(source_id: Option<&str>, phase: &str) -> Self {
        Self::new(
            "source_not_found",
            "Tessera could not find that source.",
            source_id,
            phase,
        )
    }

    /// Construct a `scan_failed` error envelope (Story 1.4). Stable code per
    /// AD-13; the safe message never includes body / query text / credentials
    /// or path detail beyond what the user already confirmed. Emitted when a
    /// scan fails for any reason other than an unknown source / invalid root
    /// (mid-scan read failure, source changed during scan, commit CAS loss,
    /// non-confirmed source).
    pub fn scan_failed(source_id: &str) -> Self {
        Self::new(
            "scan_failed",
            "Tessera could not complete the scan. The previous index is unchanged.",
            Some(source_id),
            "scan",
        )
    }

    /// Construct a `scan_failed` envelope for the not-confirmed case (Story
    /// 1.4, spec amendment: the generic message wrongly implies a previous
    /// index exists and that a scan ran). Same stable code — the UI keys on
    /// `code` only — but an accurate message.
    pub fn scan_failed_not_confirmed(source_id: &str) -> Self {
        Self::new(
            "scan_failed",
            "This source is not confirmed; confirm it before scanning.",
            Some(source_id),
            "scan",
        )
    }

    /// A scan observed source data drift after staging. The previous active
    /// generation remains visible; the user can safely retry the scan.
    pub fn scan_failed_source_changed(source_id: &str) -> Self {
        Self::new(
            "scan_failed",
            "The source changed while Tessera was scanning it. The previous index is unchanged; retry the scan.",
            Some(source_id),
            "scan",
        )
    }

    /// Story 4.4 — construct a `rebuild_failed` envelope for the in-flight
    /// rejection (the primary race guard: a scan is already mid-flight across
    /// any source, so the wipe cannot proceed safely). Stable code per AD-13;
    /// the safe message never includes body / query text / credentials or any
    /// source file path. Maps to HTTP 409 via the existing `respond_result`
    /// convention at `server.rs` (alongside `scan_failed` / `confirm_failed` /
    /// `cursor_stale`). The UI tells the user to wait for or cancel the
    /// in-flight scan, then retry the rebuild.
    pub fn rebuild_failed() -> Self {
        Self::new(
            "rebuild_failed",
            "A scan is in progress. Wait for it to finish or cancel it, then rebuild again.",
            None,
            "rebuild",
        )
    }

    /// Story 5.1 — construct a `project_not_found` envelope. Emitted when a
    /// `project_id`-keyed operation (rename / delete / add_mapping /
    /// remove_mapping) targets an id that matches no `tessera_projects` row.
    /// Maps to HTTP 404 via the existing `respond_result` convention at
    /// `server.rs`.
    pub fn project_not_found(project_id: Option<&str>) -> Self {
        Self::new(
            "project_not_found",
            "Tessera could not find that project.",
            project_id,
            "project",
        )
    }

    /// Story 5.1 — construct a `mapping_conflict` envelope naming the owning
    /// project (AD-27 cardinality: within one mapping scope `(provider,
    /// native_project)`, a Native Project belongs to at most one active
    /// Tessera Project). Emitted when `add_mapping` rejects because the scope
    /// is already owned by another project. Maps to HTTP 409 via the existing
    /// `respond_result` convention. The owning project name is user-visible
    /// metadata (consistent with the Source Inventory's posture — NFR-3); no
    /// body / query text / credentials are surfaced.
    pub fn mapping_conflict(owning_project_name: &str) -> Self {
        // The owning project name is already user-visible (it appears in the
        // project list); naming it here is the same posture as the Source
        // Inventory surfacing `provider`/`native_project`. The message tells
        // the user how to recover — remove the mapping from the owning
        // project first, then re-add it here (AD-27: never silently moved).
        Self::new(
            "mapping_conflict",
            &format!(
                "That native project is already mapped to project “{owning_project_name}”. Remove it there first."
            ),
            None,
            "project",
        )
    }

    /// Story 5.1 — construct a `mapping_not_found` envelope. Emitted when
    /// `remove_mapping` targets a `(provider, native_project)` that does not
    /// match any mapping row for the project (the project exists, the mapping
    /// does not). Maps to HTTP 404.
    pub fn mapping_not_found(project_id: Option<&str>) -> Self {
        Self::new(
            "mapping_not_found",
            "Tessera could not find that mapping.",
            project_id,
            "project",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_version_is_a_nonempty_static_string() {
        assert!(!API_VERSION.is_empty());
        // api_version is a contract major version; it must be a string at
        // build time, not computed.
        let _v: &'static str = API_VERSION;
    }

    #[test]
    fn envelope_serializes_with_api_version_field() {
        let env = Envelope {
            api_version: API_VERSION,
            payload: Pong {
                name: "tessera".to_string(),
                version: "0.0.1".to_string(),
            },
        };
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(json.contains("\"api_version\":\"1\""), "json was: {json}");
        assert!(json.contains("\"payload\""));
    }

    #[test]
    fn error_envelope_omits_payload_body() {
        let err = ErrorEnvelope::internal();
        let json = serde_json::to_string(&err).expect("serialize");
        assert!(json.contains("\"code\":\"internal\""));
        assert!(json.contains("\"source_id\":null"));
        assert!(json.contains("\"phase\":\"internal\""));
        // No body / credential / query text fields exist on the shape.
        assert!(!json.contains("body"));
        assert!(!json.contains("query"));
        assert!(!json.contains("credential"));
    }

    #[test]
    fn confirm_failed_envelope_carries_stable_code_and_safe_message() {
        let err = ErrorEnvelope::confirm_failed(None, "source");
        assert_eq!(err.code, "confirm_failed");
        let json = serde_json::to_string(&err).expect("serialize");
        // No body / query text / credential leak (AD-13/NFR-3).
        assert!(!json.contains("body"));
        assert!(!json.contains("query"));
        assert!(!json.contains("credential"));
    }

    #[test]
    fn source_not_found_envelope_carries_stable_code() {
        let err = ErrorEnvelope::source_not_found(Some("src_7"), "source");
        assert_eq!(err.code, "source_not_found");
        assert_eq!(err.source_id.as_deref(), Some("src_7"));
    }

    #[test]
    fn scan_failed_envelope_carries_stable_code_and_safe_message() {
        let err = ErrorEnvelope::scan_failed("src_7");
        assert_eq!(err.code, "scan_failed");
        let json = serde_json::to_string(&err).expect("serialize");
        // No body / query text / credential leak (AD-13/NFR-3).
        assert!(!json.contains("body"));
        assert!(!json.contains("query"));
        assert!(!json.contains("credential"));
    }

    #[test]
    fn scan_failed_not_confirmed_keeps_code_with_distinct_message() {
        let err = ErrorEnvelope::scan_failed_not_confirmed("src_7");
        assert_eq!(err.code, "scan_failed");
        assert!(err.message.contains("not confirmed"));
        assert_ne!(err.message, ErrorEnvelope::scan_failed("src_7").message);
        let json = serde_json::to_string(&err).expect("serialize");
        // No body / query text / credential leak (AD-13/NFR-3).
        assert!(!json.contains("body"));
        assert!(!json.contains("query"));
        assert!(!json.contains("credential"));
    }

    #[test]
    fn scan_changed_envelope_has_source_and_scan_phase() {
        let err = ErrorEnvelope::scan_failed_source_changed("src_7");
        assert_eq!(err.code, "scan_failed");
        assert_eq!(err.source_id.as_deref(), Some("src_7"));
        assert_eq!(err.phase, "scan");
        assert!(err.message.contains("changed"));
    }

    /// Story 5.1 — the three new project error constructors carry stable
    /// codes, the `"project"` phase, and safe messages (no body / query text
    /// / credential leak — NFR-3/AD-13).
    #[test]
    fn project_envelopes_carry_stable_codes_and_safe_messages() {
        let not_found = ErrorEnvelope::project_not_found(Some("proj_7"));
        assert_eq!(not_found.code, "project_not_found");
        assert_eq!(not_found.source_id.as_deref(), Some("proj_7"));
        assert_eq!(not_found.phase, "project");

        let conflict = ErrorEnvelope::mapping_conflict("A");
        assert_eq!(conflict.code, "mapping_conflict");
        assert_eq!(conflict.phase, "project");
        // The owning project name surfaces in the message (NFR-3: same
        // posture as the Source Inventory).
        assert!(conflict.message.contains("A"));
        assert!(conflict.message.contains("already mapped"));

        let missing = ErrorEnvelope::mapping_not_found(Some("proj_7"));
        assert_eq!(missing.code, "mapping_not_found");
        assert_eq!(missing.phase, "project");

        // AD-13/NFR-3: none of the new envelopes leak body / query text /
        // credentials in either the code, message, or phase.
        for env in [not_found, conflict, missing] {
            let lower = env.message.to_lowercase();
            assert!(!lower.contains("body"));
            assert!(!lower.contains("credential"));
            assert!(!env.code.contains("credential"));
        }
    }
}
