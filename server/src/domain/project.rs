//! `domain::project` — Tessera Project identity, mappings, and DTOs (Story 5.1).
//!
//! This module fixes the Tessera Project domain model that
//! [`crate::index::project_store::ProjectStore`] persists and that the
//! application layer ([`crate::application::project`]) orchestrates.
//!
//! Story 5.1 introduces:
//! - [`ProjectId`] — opaque `proj_<n>` handle, stable across restarts.
//! - [`TesseraProject`] — the persisted row (`name`, `created_at`,
//!   `updated_at`).
//! - [`NativeProjectRef`] — `{ provider, native_project }` pair: the mapping
//!   target. The same native identity is already carried on Sources and
//!   canonical records, so projection (Story 5.2) can filter records with a
//!   direct predicate — no copy of canonical rows, no native-identity change
//!   (AD-2).
//! - [`ProjectMapping`] — one persisted mapping row.
//! - [`TesseraProjectView`] — the DTO returned to the UI: a project plus its
//!   ordered mappings. The wire shape the TS mirror (`src/api/projects.ts`)
//!   must match.
//! - Request DTOs for the six versioned endpoints.
//!
//! Architecture invariants honoured (AD-2/AD-27/AD-33):
//! - `project_id` is the stable handle, independent of source rebind (AD-33:
//!   rebind re-derives `native_project` from the new root, so the
//!   `(provider, native_project)` mapping key survives a rebind — it is keyed
//!   on the native identity, never on a `source_id`).
//! - Matching scope is exact `(provider, native_project)` equality, with
//!   `COALESCE(NULL, '')` collapsing Codex's global store so NULL scopes are
//!   unique under the uniqueness index (AD-27 storage backstop).
//! - Project rows carry `created_at` / `updated_at` as Unix seconds
//!   (`INTEGER`), reusing the existing `unix_seconds_now_i64()` style — no
//!   `chrono` / `time` (spec Never). `updated_at` reflects project-metadata
//!   changes (rename) only; mapping rows carry their own `created_at`.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum length, in UTF-8 bytes, of a Tessera Project `name`. Names longer
/// than this are rejected as `bad_request` (phase `project`). The bound keeps
/// the wire contract bounded (AD-17) without imposing an unreasonable limit
/// on real-world project names.
pub const MAX_PROJECT_NAME_LEN: usize = 128;

/// Maximum length, in UTF-8 bytes, of a `native_project` string. Claude Code
/// project keys are filesystem path segments; 1024 bytes is far above any
/// realistic encoded path. The bound keeps the wire contract bounded (AD-17).
pub const MAX_NATIVE_PROJECT_LEN: usize = 1024;

/// The provider ids Tessera knows how to map. Lowercase, matching
/// [`crate::domain::source`] and the adapter registry
/// ([`crate::application::source::adapter_for`]). `codex` is the global
/// memory store (`native_project = null`); `claude_code` is per-project
/// (`native_project` is a non-empty, non-whitespace string ≤
/// `MAX_NATIVE_PROJECT_LEN`).
pub const KNOWN_PROVIDERS: &[&str] = &["codex", "claude_code"];

/// Opaque, stable Tessera Project handle — `proj_<n>` where `n` is the
/// `tessera_projects` `INTEGER PRIMARY KEY AUTOINCREMENT` value.
///
/// `AUTOINCREMENT` guarantees an id is never reused, even after a row is
/// deleted (Story 5.1 ships a `delete_project` command, so a deleted
/// project's handle cannot be reattached to a different row). The handle is
/// path/provider-independent: the mapping key is `(provider,
/// native_project)`, the handle lives here.
///
/// Construction is intentionally limited: only the project store builds a
/// `ProjectId` from a rowid. Application and IPC code consume them.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectId(pub String);

impl ProjectId {
    /// Build a `ProjectId` from a `tessera_projects` rowid. Kept crate-internal:
    /// callers outside the project store go through the store's own
    /// row-mapping.
    pub(crate) fn from_rowid(rowid: i64) -> Self {
        ProjectId(format!("proj_{rowid}"))
    }

    /// Parse the rowid back out of a `proj_<n>` handle, or `None` if the string
    /// is malformed. Used by the project store to translate an
    /// externally-supplied id into the SQLite rowid for `UPDATE ... WHERE id
    /// = ?` and for the cardinality pre-check's `SELECT ... WHERE
    /// tessera_project_id = ?`.
    ///
    /// Rejects non-positive values (`proj_0`, `proj_-5`): SQLite
    /// `INTEGER PRIMARY KEY AUTOINCREMENT` rowids are always `>= 1`, so a
    /// non-positive handle is not a well-formed Tessera Project id regardless
    /// of whether it parses as an integer. Mirrors the `SourceId::to_rowid`
    /// guard so a hand-edited request cannot smuggle `proj_-5` past the
    /// lookup.
    pub fn to_rowid(&self) -> Option<i64> {
        let s = &self.0;
        let n = s.strip_prefix("proj_")?.parse::<i64>().ok()?;
        (n > 0).then_some(n)
    }
}

impl fmt::Display for ProjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A persisted Tessera Project. The DTO returned to the UI is
/// [`TesseraProjectView`] (project + its ordered mappings); this type is the
/// bare row the project store reads back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesseraProject {
    /// Opaque stable handle (`proj_<n>`).
    pub project_id: ProjectId,
    /// User-supplied project name (≤ [`MAX_PROJECT_NAME_LEN`] bytes).
    pub name: String,
    /// Unix-epoch seconds at project creation. Equal to `created_at` until the
    /// first rename advances `updated_at`.
    pub created_at: i64,
    /// Unix-epoch seconds at the most recent rename. Equal to `created_at`
    /// until the first rename (the I/O matrix row "Rename project" requires
    /// `updated_at` strictly greater than `created_at` after a rename).
    pub updated_at: i64,
}

/// A `(provider, native_project)` reference — the mapping target. This is the
/// same native identity already carried on Sources (`source_registry` row)
/// and canonical records (`memory_records` row), so projection (Story 5.2)
/// can filter records with a direct predicate without copying canonical rows
/// or changing native identity (AD-2).
///
/// `native_project` is `null` for Codex's global store; for Claude Code it is
/// the provider-native project key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeProjectRef {
    pub provider: String,
    pub native_project: Option<String>,
}

/// A persisted mapping row. The wire shape the UI consumes is
/// [`NativeProjectRef`]; this type is the full row the project store reads
/// back (mapping rows carry their own `created_at`, separate from the
/// project's `updated_at`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectMapping {
    pub id: i64,
    pub tessera_project_id: i64,
    pub provider: String,
    pub native_project: Option<String>,
    pub created_at: i64,
}

impl ProjectMapping {
    /// Project this row onto the wire-shape [`NativeProjectRef`] (drops the
    /// rowid / project id / `created_at` that the UI never needs).
    pub fn to_ref(&self) -> NativeProjectRef {
        NativeProjectRef {
            provider: self.provider.clone(),
            native_project: self.native_project.clone(),
        }
    }
}

/// The DTO returned to the UI for a Tessera Project. Carries the project row
/// plus its ordered mappings, so a single round-trip per project is enough
/// for the UI to render a card. `mappings` is ordered by `id` ascending
/// (stable UI ordering matching [`SourceRegistry::list`]); see
/// [`crate::index::project_store::ProjectStore::list_mappings`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesseraProjectView {
    pub project_id: ProjectId,
    pub name: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub mappings: Vec<NativeProjectRef>,
}

// ---------------------------------------------------------------------------
// Request DTOs (mirror the Rust endpoints; the HTTP layer deserializes them
// straight from the JSON body the TS client posts)
// ---------------------------------------------------------------------------

/// `POST /api/projects/create { name }`. Mirrors the Rust endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateProjectRequest {
    pub name: String,
}

/// `POST /api/projects/rename { project_id, name }`. Mirrors the Rust
/// endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameProjectRequest {
    pub project_id: ProjectId,
    pub name: String,
}

/// `POST /api/projects/delete { project_id }`. Mirrors the Rust endpoint. The
/// response carries `removed_mappings: u32`; the request is just the id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteProjectRequest {
    pub project_id: ProjectId,
}

/// `POST /api/projects/mappings/add` and `POST /api/projects/mappings/remove`
/// share the same body shape: `{ project_id, provider, native_project }`. The
/// application layer validates the provider id and the `native_project` shape
/// (non-empty, non-whitespace string for `claude_code`; `null` for `codex`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingRequest {
    pub project_id: ProjectId,
    pub provider: String,
    pub native_project: Option<String>,
}

/// Response shape for `POST /api/projects/delete`: the deleted project's id
/// and the count of mappings that cascaded with it (per the I/O matrix row
/// "Delete project").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeleteProjectResponse {
    pub project_id: ProjectId,
    pub removed_mappings: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_id_from_rowid_roundtrips() {
        let id = ProjectId::from_rowid(42);
        assert_eq!(id.0, "proj_42");
        assert_eq!(id.to_rowid(), Some(42));
        assert_eq!(format!("{id}"), "proj_42");
    }

    #[test]
    fn project_id_to_rowid_rejects_malformed_and_non_positive() {
        assert!(ProjectId("proj_x".to_string()).to_rowid().is_none());
        assert!(ProjectId("not_prefixed".to_string()).to_rowid().is_none());
        assert!(ProjectId("proj_".to_string()).to_rowid().is_none());
        // AUTOINCREMENT rowids are always >= 1, so a non-positive handle is not
        // a well-formed Tessera Project id regardless of integer parsing.
        assert!(ProjectId("proj_0".to_string()).to_rowid().is_none());
        assert!(ProjectId("proj_-5".to_string()).to_rowid().is_none());
        // Leading zeros parse fine — mirrors `SourceId::to_rowid` so the two
        // opaque ids behave identically.
        assert_eq!(ProjectId("proj_02".to_string()).to_rowid(), Some(2));
    }

    #[test]
    fn known_providers_are_lowercase_and_codex_claude_only() {
        // Pin the provider vocabulary at the domain layer so the application
        // validation and the cardinality pre-check share one source of truth.
        assert!(KNOWN_PROVIDERS.contains(&"codex"));
        assert!(KNOWN_PROVIDERS.contains(&"claude_code"));
        for p in KNOWN_PROVIDERS {
            assert_eq!(*p, p.to_lowercase(), "provider ids are lowercase");
            assert!(!p.is_empty());
        }
    }

    #[test]
    fn native_project_ref_round_trips() {
        let claude = NativeProjectRef {
            provider: "claude_code".to_string(),
            native_project: Some("encoded-project-key".to_string()),
        };
        let json = serde_json::to_string(&claude).expect("serialize");
        assert!(json.contains("\"provider\":\"claude_code\""));
        assert!(json.contains("\"native_project\":\"encoded-project-key\""));
        let back: NativeProjectRef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, claude);

        let codex = NativeProjectRef {
            provider: "codex".to_string(),
            native_project: None,
        };
        let codex_json = serde_json::to_string(&codex).expect("serialize");
        assert!(codex_json.contains("\"native_project\":null"));
        let codex_back: NativeProjectRef = serde_json::from_str(&codex_json).expect("deserialize");
        assert_eq!(codex_back, codex);
    }

    #[test]
    fn tessera_project_view_serializes_with_versioned_envelope() {
        // Mirrors the `wrap_source` wire-shape test: the DTO must serialize
        // cleanly under serde so the HTTP layer's `Envelope<T>` round-trips
        // to the TS mirror.
        let view = TesseraProjectView {
            project_id: ProjectId::from_rowid(1),
            name: "A".to_string(),
            created_at: 100,
            updated_at: 100,
            mappings: vec![NativeProjectRef {
                provider: "codex".to_string(),
                native_project: None,
            }],
        };
        let env = crate::http::Envelope {
            api_version: crate::http::API_VERSION,
            payload: view,
        };
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(json.contains("\"api_version\":\"1\""));
        assert!(json.contains("\"project_id\":\"proj_1\""));
        assert!(json.contains("\"name\":\"A\""));
        assert!(json.contains("\"created_at\":100"));
        assert!(json.contains("\"updated_at\":100"));
        assert!(json.contains("\"provider\":\"codex\""));
        assert!(json.contains("\"native_project\":null"));
    }

    #[test]
    fn create_project_request_deserializes_name_only() {
        let json = r#"{"name":"A"}"#;
        let req: CreateProjectRequest = serde_json::from_str(json).expect("deserialize");
        assert_eq!(req.name, "A");
    }

    #[test]
    fn mapping_request_deserializes_codex_null_and_claude_some() {
        let codex_json = r#"{"project_id":"proj_1","provider":"codex","native_project":null}"#;
        let codex: MappingRequest = serde_json::from_str(codex_json).expect("deserialize");
        assert_eq!(codex.project_id.0, "proj_1");
        assert_eq!(codex.provider, "codex");
        assert!(codex.native_project.is_none());

        let claude_json =
            r#"{"project_id":"proj_1","provider":"claude_code","native_project":"key"}"#;
        let claude: MappingRequest = serde_json::from_str(claude_json).expect("deserialize");
        assert_eq!(claude.provider, "claude_code");
        assert_eq!(claude.native_project.as_deref(), Some("key"));
    }
}
