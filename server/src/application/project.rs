//! `application::project` — orchestration of Tessera Project create / list /
//! rename / delete + add-mapping / remove-mapping (Story 5.1).
//!
//! This is the only layer allowed to coordinate input validation +
//! cardinality pre-check + transactional writes + DTO assembly (AD-1). The
//! HTTP layer calls these functions; it never touches the project store
//! directly.
//!
//! Architecture invariants honoured (AD-2/AD-24/AD-27/AD-29/AD-33):
//! - **Explicit-only mapping (AD-24).** `create_project` creates zero
//!   mappings; unmapped Native Projects are never auto-projected. Only
//!   `add_mapping` forms an association.
//! - **AD-27 cardinality.** Within one mapping scope `(provider,
//!   native_project)`, a Native Project belongs to at most one active Tessera
//!   Project. The pre-check inside [`add_mapping`] returns
//!   [`ProjectError::MappingConflict`] naming the owning project when the
//!   scope is owned by another project (never silently moved); re-adding the
//!   exact same `(project, provider, native_project)` is idempotent.
//! - **Zero-source-mutation.** No project or mapping operation reads or
//!   writes the Source filesystem or any canonical `memory_records` /
//!   `source_registry` row. The store touches only `tessera_projects` and
//!   `project_mappings`.
//! - **NFR-3 redaction.** Errors and logs carry only user-visible metadata
//!   (project name + provider + native_project) — the same shape the Source
//!   Inventory already exposes.
//! - **Mapping key is native identity, not `source_id`.** Re-derivation
//!   survives a Source rebind (AD-33): the `(provider, native_project)`
//!   pair is the same after a rebind that re-derives `native_project` from
//!   the new root, so the mapping key outlives the Source row.

use crate::domain::project::{
    CreateProjectRequest, DeleteProjectRequest, DeleteProjectResponse, MappingRequest, ProjectId,
    RenameProjectRequest, TesseraProjectView, KNOWN_PROVIDERS, MAX_NATIVE_PROJECT_LEN,
    MAX_PROJECT_NAME_LEN,
};
use crate::index::project_store::ProjectStore;

/// The error raised by the application project orchestration. Mirrors
/// [`crate::application::source::SourceError`]'s shape: each variant maps to
/// a stable [`crate::http::envelope::ErrorEnvelope`] code via
/// `crate::http::map_project_error`.
#[derive(Debug)]
pub enum ProjectError {
    /// A request carried an invalid name (empty / whitespace / over
    /// `MAX_PROJECT_NAME_LEN`), an unknown provider, or an invalid
    /// `native_project` (empty / whitespace / over
    /// `MAX_NATIVE_PROJECT_LEN` for `claude_code`). Maps to stable code
    /// `bad_request`.
    BadRequest,
    /// A `project_id`-keyed operation targeted an id that does not match any
    /// row. Maps to stable code `project_not_found`.
    ProjectNotFound,
    /// `add_mapping` rejected because the mapping scope `(provider,
    /// native_project)` is already owned by another active Tessera Project
    /// (AD-27 cardinality). Carries the owning project's name so the safe
    /// message can name it for the user.
    MappingConflict { owning_project_name: String },
    /// `remove_mapping` rejected because the project exists but no mapping
    /// matched `(provider, native_project)` for this project. Maps to stable
    /// code `mapping_not_found`.
    MappingNotFound,
    /// An unexpected internal error from the store (SQLite failure). Maps to
    /// stable code `internal`.
    Internal,
}

impl From<rusqlite::Error> for ProjectError {
    fn from(_: rusqlite::Error) -> Self {
        ProjectError::Internal
    }
}

// ---------------------------------------------------------------------------
// Validation helpers (single source of truth for the project layer)
// ---------------------------------------------------------------------------

/// Normalize + validate a Tessera Project name. Returns the trimmed name on
/// success, or [`ProjectError::BadRequest`] for empty / whitespace / over-
/// length names. Trimming matches the Source Inventory's posture: the user
/// types a name, leading/trailing whitespace is accidental.
fn validate_project_name(name: &str) -> Result<String, ProjectError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(ProjectError::BadRequest);
    }
    if trimmed.len() > MAX_PROJECT_NAME_LEN {
        return Err(ProjectError::BadRequest);
    }
    Ok(trimmed.to_string())
}

/// Validate the provider id and the `native_project` shape. Returns
/// `(provider, native_project)` on success, or [`ProjectError::BadRequest`]
/// for an unknown provider or an invalid `native_project`.
///
/// `native_project` shape rules (mirroring the spec I/O matrix + the
/// uniqueness index's NULL handling):
/// - `codex`: must be `None` (Codex is the global store). A `Some("")` /
///   `Some("  ")` from the wire would collide with the `null` scope under
///   `COALESCE` — reject it so a hand-crafted request cannot bypass the NULL
///   uniqueness. A non-empty `Some(_)` for `codex` is also rejected: Codex
///   has no native-project key.
/// - `claude_code`: must be `Some(non-empty, non-whitespace, ≤
///   MAX_NATIVE_PROJECT_LEN)`. An empty / whitespace string is rejected so
///   it cannot collide with the Codex `null` scope.
///
/// `provider` is matched against [`KNOWN_PROVIDERS`] (lowercase, matching
/// the adapter registry's provider ids exactly).
fn validate_mapping_scope(
    provider: &str,
    native_project: &Option<String>,
) -> Result<(String, Option<String>), ProjectError> {
    if !KNOWN_PROVIDERS.contains(&provider) {
        return Err(ProjectError::BadRequest);
    }
    match (provider, native_project) {
        ("codex", None) => Ok(("codex".to_string(), None)),
        ("codex", Some(_)) => {
            // Codex has no native-project key — any non-null value is a
            // contract violation.
            Err(ProjectError::BadRequest)
        }
        ("claude_code", Some(np)) => {
            let trimmed = np.trim();
            if trimmed.is_empty() {
                return Err(ProjectError::BadRequest);
            }
            if trimmed.len() > MAX_NATIVE_PROJECT_LEN {
                return Err(ProjectError::BadRequest);
            }
            // Use the trimmed form so a trailing-space value cannot evade
            // the uniqueness index by colliding with the trimmed version of
            // another row.
            Ok((
                "claude_code".to_string(),
                Some(trimmed.to_string()),
            ))
        }
        ("claude_code", None) => {
            // Claude Code is per-project; `null` is reserved for Codex's
            // global store. Reject so the wire shape cannot smuggle a Claude
            // source into the Codex-global scope.
            Err(ProjectError::BadRequest)
        }
        _ => Err(ProjectError::BadRequest),
    }
}

// ---------------------------------------------------------------------------
// Orchestration (one function per versioned endpoint)
// ---------------------------------------------------------------------------

/// `POST /api/projects/create` — create a Tessera Project. The I/O matrix
/// requires the response to carry an empty `mappings` array (AD-24: creating
/// a project creates zero mappings).
pub fn create_project(
    store: &ProjectStore<'_>,
    request: &CreateProjectRequest,
) -> Result<TesseraProjectView, ProjectError> {
    let name = validate_project_name(&request.name)?;
    let project = store.create(&name)?;
    // New project → no mappings yet; assemble the view (empty mappings).
    Ok(store.view_for(&project)?)
}

/// `GET /api/projects` — list every Tessera Project (any lifecycle is N/A:
/// projects have no lifecycle). Infallible at the application layer; any DB
/// failure surfaces as `internal`. Returns the views ordered by `id`
/// ascending (the store's `list` ordering).
pub fn list_projects(
    store: &ProjectStore<'_>,
) -> Result<Vec<TesseraProjectView>, ProjectError> {
    let projects = store.list()?;
    let mut views = Vec::with_capacity(projects.len());
    for project in projects {
        views.push(store.view_for(&project)?);
    }
    Ok(views)
}

/// `POST /api/projects/rename` — rename a Tessera Project. Advances
/// `updated_at`; the I/O matrix requires `updated_at` strictly greater than
/// `created_at` after a rename (the store stamps the new `updated_at` from
/// `unix_seconds_now_i64()`, which advances past `created_at` on any clock
/// that is not before the Unix epoch).
pub fn rename_project(
    store: &ProjectStore<'_>,
    request: &RenameProjectRequest,
) -> Result<TesseraProjectView, ProjectError> {
    let new_name = validate_project_name(&request.name)?;
    let updated = store
        .rename(&request.project_id, &new_name)?
        .ok_or(ProjectError::ProjectNotFound)?;
    Ok(store.view_for(&updated)?)
}

/// `POST /api/projects/delete` — delete a Tessera Project. Its mappings are
/// removed explicitly by the store (cascade-independent; `ON DELETE CASCADE`
/// is a belt-and-suspenders backstop), and the response carries the actual
/// removed count via [`DeleteProjectResponse`]. The mappings delete + the
/// project delete run inside ONE transaction (the store's
/// [`ProjectStore::with_transaction`]) so a crash between them cannot leave a
/// project row present with its mappings already gone.
pub fn delete_project(
    store: &ProjectStore<'_>,
    request: &DeleteProjectRequest,
) -> Result<DeleteProjectResponse, ProjectError> {
    store.with_transaction(|tx| -> Result<DeleteProjectResponse, ProjectError> {
        let removed = tx
            .delete(&request.project_id)?
            .ok_or(ProjectError::ProjectNotFound)?;
        Ok(DeleteProjectResponse {
            project_id: request.project_id.clone(),
            removed_mappings: removed,
        })
    })
}

/// `POST /api/projects/mappings/add` — add an explicit `(provider,
/// native_project)` mapping to a project. AD-27 cardinality: the pre-check
/// inside the transaction returns [`ProjectError::MappingConflict`] naming
/// the owning project when the scope is already owned by another project;
/// re-adding the exact same `(project, provider, native_project)` already
/// mapped to THIS project is idempotent (no duplicate row, return the
/// unchanged view).
///
/// The pre-check + insert run inside ONE transaction
/// ([`ProjectStore::with_transaction`]) so the pre-check's read and the
/// insert's write cannot observe a different state between them. The unique
/// index is the storage backstop for the rare race where two writers pass
/// the pre-check simultaneously (the second's INSERT then surfaces as a
/// constraint error → [`ProjectError::Internal`]; the operator can retry,
/// and the next attempt correctly hits the pre-check).
pub fn add_mapping(
    store: &ProjectStore<'_>,
    request: &MappingRequest,
) -> Result<TesseraProjectView, ProjectError> {
    let (provider, native_project) =
        validate_mapping_scope(&request.provider, &request.native_project)?;
    let native_ref = native_project.as_deref();

    store.with_transaction(|tx| -> Result<TesseraProjectView, ProjectError> {
        // Look up the project INSIDE the transaction so the project existence
        // check and the subsequent writes see the same snapshot.
        let project = tx
            .get(&request.project_id)?
            .ok_or(ProjectError::ProjectNotFound)?;
        let project_rowid = request
            .project_id
            .to_rowid()
            .ok_or(ProjectError::ProjectNotFound)?;

        // Cardinality pre-check (AD-27).
        match tx.find_mapping_owner(&provider, native_ref)? {
            None => {
                // Scope is free — insert the new mapping.
                tx.insert_mapping(project_rowid, &provider, native_ref)?;
            }
            Some(owner_rowid) if owner_rowid == project_rowid => {
                // Idempotent re-add: the scope is already owned by THIS
                // project. No INSERT — the unique index would reject a
                // duplicate anyway, and the I/O matrix requires "no
                // duplicate row, return the unchanged view".
            }
            Some(owner_rowid) => {
                // Scope is owned by ANOTHER project — return
                // `mapping_conflict` naming the owning project so the user
                // can see who owns the scope (AD-27: never silently move).
                let owner = tx
                    .get(&ProjectId::from_rowid(owner_rowid))?
                    .ok_or(ProjectError::Internal)?;
                return Err(ProjectError::MappingConflict {
                    owning_project_name: owner.name,
                });
            }
        }

        // Re-assemble the view from the (possibly updated) project row + its
        // mappings. The view re-read happens after the transaction body's
        // writes are staged but before commit; the commit makes them durable
        // atomically with the response.
        Ok(tx.view_for(&project)?)
    })
}

/// `POST /api/projects/mappings/remove` — remove a mapping. The I/O matrix
/// distinguishes "no such project" (404 `project_not_found`) from "project
/// exists, no such mapping" (404 `mapping_not_found`). Both are 404 but
/// carry different stable codes so the UI can surface the right copy.
pub fn remove_mapping(
    store: &ProjectStore<'_>,
    request: &MappingRequest,
) -> Result<TesseraProjectView, ProjectError> {
    let (provider, native_project) =
        validate_mapping_scope(&request.provider, &request.native_project)?;
    let native_ref = native_project.as_deref();

    // Validation rejects Codex-non-null and Claude-null upstream, so the
    // `COALESCE`-based DELETE in the store never accidentally matches the
    // wrong scope. The remove runs WITHOUT a transaction: a single DELETE is
    // already atomic, and the response view re-reads the post-delete state.
    match store.remove_mapping(&request.project_id, &provider, native_ref)? {
        None => Err(ProjectError::ProjectNotFound),
        Some(false) => Err(ProjectError::MappingNotFound),
        Some(true) => {
            let project = store
                .get(&request.project_id)?
                .ok_or(ProjectError::Internal)?;
            Ok(store.view_for(&project)?)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::migrations;

    /// Build a fresh project store over a leaked in-memory connection. Returns
    /// the connection alongside the store so tests that need to inspect
    /// non-project tables (e.g. the non-destruction gate's COUNT queries over
    /// `source_registry` / `memory_records`) can do so without touching the
    /// store's private `conn` field.
    fn fresh_store() -> (&'static rusqlite::Connection, ProjectStore<'static>) {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        migrations::apply(&mut conn).expect("migrations apply");
        let leaked: &'static rusqlite::Connection = Box::leak(Box::new(conn));
        (leaked, ProjectStore::new(leaked))
    }

    /// Convenience: COUNT a table on the leaked connection.
    fn count(conn: &rusqlite::Connection, table: &str) -> i64 {
        conn.query_row(
            &format!("SELECT COUNT(*) FROM {table}"),
            [],
            |row| row.get(0),
        )
        .expect("count rows")
    }

    #[test]
    fn validate_project_name_trims_and_rejects_empties_and_oversized() {
        assert_eq!(
            validate_project_name("  A  ").unwrap(),
            "A".to_string()
        );
        assert!(matches!(validate_project_name(""), Err(ProjectError::BadRequest)));
        assert!(matches!(
            validate_project_name("   "),
            Err(ProjectError::BadRequest)
        ));
        let long = "x".repeat(MAX_PROJECT_NAME_LEN + 1);
        assert!(matches!(
            validate_project_name(&long),
            Err(ProjectError::BadRequest)
        ));
        let max = "x".repeat(MAX_PROJECT_NAME_LEN);
        assert_eq!(validate_project_name(&max).unwrap().len(), MAX_PROJECT_NAME_LEN);
    }

    #[test]
    fn validate_mapping_scope_codex_null_and_claude_some_only() {
        // Codex global scope.
        let (p, np) = validate_mapping_scope("codex", &None).unwrap();
        assert_eq!(p, "codex");
        assert!(np.is_none());

        // Claude per-project scope.
        let (p, np) = validate_mapping_scope("claude_code", &Some("key".to_string())).unwrap();
        assert_eq!(p, "claude_code");
        assert_eq!(np.as_deref(), Some("key"));

        // Trim Claude whitespace.
        let (_, np) = validate_mapping_scope("claude_code", &Some("  key  ".to_string())).unwrap();
        assert_eq!(np.as_deref(), Some("key"));
    }

    #[test]
    fn validate_mapping_scope_rejects_invalid_combinations() {
        // Unknown provider.
        assert!(matches!(
            validate_mapping_scope("not_a_provider", &None),
            Err(ProjectError::BadRequest)
        ));
        // Codex MUST be null.
        assert!(matches!(
            validate_mapping_scope("codex", &Some("key".to_string())),
            Err(ProjectError::BadRequest)
        ));
        // Claude MUST be Some(non-empty).
        assert!(matches!(
            validate_mapping_scope("claude_code", &None),
            Err(ProjectError::BadRequest)
        ));
        // Claude non-empty after trim.
        assert!(matches!(
            validate_mapping_scope("claude_code", &Some("   ".to_string())),
            Err(ProjectError::BadRequest)
        ));
        // Claude oversized.
        let long = "x".repeat(MAX_NATIVE_PROJECT_LEN + 1);
        assert!(matches!(
            validate_mapping_scope("claude_code", &Some(long)),
            Err(ProjectError::BadRequest)
        ));
    }

    #[test]
    fn create_list_rename_delete_round_trips() {
        let (_conn, store) = fresh_store();

        // Empty list to start.
        assert!(store.list().unwrap().is_empty());

        let created = create_project(
            &store,
            &CreateProjectRequest { name: "  A  ".to_string() },
        )
        .unwrap();
        // Created view has no mappings and equal created_at/updated_at.
        assert!(created.mappings.is_empty());
        assert_eq!(created.name, "A");
        assert_eq!(created.created_at, created.updated_at);
        // project_id matches ^proj_\d+$.
        assert!(created.project_id.0.starts_with("proj_"));

        // List contains the created project (AD-24: no auto-mappings).
        let listed = list_projects(&store).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "A");
        assert!(listed[0].mappings.is_empty());

        // Rename advances updated_at past created_at — sleep so the
        // unix_seconds_now clock actually advances.
        std::thread::sleep(std::time::Duration::from_secs(1));
        let renamed = rename_project(
            &store,
            &RenameProjectRequest {
                project_id: created.project_id.clone(),
                name: "B".to_string(),
            },
        )
        .unwrap();
        assert_eq!(renamed.name, "B");
        assert!(renamed.updated_at > renamed.created_at);

        // Delete cascades zero mappings.
        let outcome = delete_project(
            &store,
            &DeleteProjectRequest {
                project_id: created.project_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(outcome.removed_mappings, 0);
        // List is empty again.
        assert!(list_projects(&store).unwrap().is_empty());

        // Renaming / deleting an unknown id returns ProjectNotFound.
        let unknown = ProjectId("proj_99999".to_string());
        assert!(matches!(
            rename_project(
                &store,
                &RenameProjectRequest {
                    project_id: unknown.clone(),
                    name: "X".to_string(),
                }
            ),
            Err(ProjectError::ProjectNotFound)
        ));
        assert!(matches!(
            delete_project(&store, &DeleteProjectRequest { project_id: unknown }),
            Err(ProjectError::ProjectNotFound)
        ));
    }

    #[test]
    fn add_mapping_cardinality_conflict_and_idempotent_re_add() {
        let (_conn, store) = fresh_store();
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();
        let b = create_project(&store, &CreateProjectRequest { name: "B".to_string() }).unwrap();

        // Add (claude_code, "<key>") to A — happy path.
        let key = "<key>".to_string();
        let a_view = add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some(key.clone()),
            },
        )
        .unwrap();
        assert_eq!(a_view.mappings.len(), 1);
        assert_eq!(a_view.mappings[0].provider, "claude_code");
        assert_eq!(a_view.mappings[0].native_project.as_deref(), Some("<key>"));

        // Add the same scope to B — 409 mapping_conflict naming A.
        let err = add_mapping(
            &store,
            &MappingRequest {
                project_id: b.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some(key.clone()),
            },
        )
        .unwrap_err();
        match err {
            ProjectError::MappingConflict {
                owning_project_name,
            } => {
                assert_eq!(owning_project_name, "A");
            }
            other => panic!("expected MappingConflict, got {other:?}"),
        }

        // GET /api/projects (list) shows <key> mapped ONLY to A (no row was
        // created for B).
        let listed = list_projects(&store).unwrap();
        for view in listed {
            if view.project_id == b.project_id {
                assert!(view.mappings.is_empty(), "B must have no mappings");
            } else {
                assert_eq!(view.mappings.len(), 1);
            }
        }

        // Idempotent re-add to A — same scope already owned by A returns the
        // unchanged view with exactly one entry for <key>.
        let a_again = add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some(key),
            },
        )
        .unwrap();
        assert_eq!(a_again.mappings.len(), 1);
    }

    #[test]
    fn codex_null_scope_is_unique_across_projects() {
        let (_conn, store) = fresh_store();
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();
        let b = create_project(&store, &CreateProjectRequest { name: "B".to_string() }).unwrap();

        // (codex, null) on A — happy.
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap();

        // (codex, null) on B — 409 mapping_conflict (AD-27 with NULL
        // collapsed).
        let err = add_mapping(
            &store,
            &MappingRequest {
                project_id: b.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::MappingConflict { .. }));

        // A whitespace `Some("  ")` cannot smuggle into the Codex-global
        // scope — validation rejects it.
        assert!(matches!(
            add_mapping(
                &store,
                &MappingRequest {
                    project_id: b.project_id.clone(),
                    provider: "codex".to_string(),
                    native_project: Some("  ".to_string()),
                }
            ),
            Err(ProjectError::BadRequest)
        ));
    }

    #[test]
    fn remove_mapping_distinguishes_missing_project_and_missing_mapping() {
        let (_conn, store) = fresh_store();
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap();

        // Unknown project → ProjectNotFound.
        let unknown = ProjectId("proj_99999".to_string());
        assert!(matches!(
            remove_mapping(
                &store,
                &MappingRequest {
                    project_id: unknown,
                    provider: "codex".to_string(),
                    native_project: None,
                }
            ),
            Err(ProjectError::ProjectNotFound)
        ));

        // Existing project, missing mapping → MappingNotFound.
        assert!(matches!(
            remove_mapping(
                &store,
                &MappingRequest {
                    project_id: a.project_id.clone(),
                    provider: "claude_code".to_string(),
                    native_project: Some("not-mapped".to_string()),
                }
            ),
            Err(ProjectError::MappingNotFound)
        ));

        // Existing mapping → removed; view no longer carries it.
        let after = remove_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap();
        assert!(after.mappings.is_empty());
    }

    #[test]
    fn unknown_provider_and_invalid_name_are_bad_request() {
        let (_conn, store) = fresh_store();
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();

        // Unknown provider on add_mapping.
        assert!(matches!(
            add_mapping(
                &store,
                &MappingRequest {
                    project_id: a.project_id.clone(),
                    provider: "not_a_provider".to_string(),
                    native_project: None,
                }
            ),
            Err(ProjectError::BadRequest)
        ));

        // Empty name on create.
        assert!(matches!(
            create_project(&store, &CreateProjectRequest { name: "   ".to_string() }),
            Err(ProjectError::BadRequest)
        ));
    }

    #[test]
    fn delete_cascades_mappings_and_reports_count() {
        let (conn, store) = fresh_store();
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();
        // Add 3 mappings.
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap();
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some("p1".to_string()),
            },
        )
        .unwrap();
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some("p2".to_string()),
            },
        )
        .unwrap();

        let outcome = delete_project(
            &store,
            &DeleteProjectRequest {
                project_id: a.project_id.clone(),
            },
        )
        .unwrap();
        assert_eq!(outcome.removed_mappings, 3);
        // Project is gone.
        assert!(list_projects(&store).unwrap().is_empty());
        // The cascade also removed every mapping row at the SQL layer.
        assert_eq!(count(conn, "project_mappings"), 0);
    }

    /// Story 5.1 non-destruction gate (I/O matrix + AC). A flurry of project
    /// operations must NOT modify the Source Registry or canonical
    /// `memory_records` rows.
    #[test]
    fn project_ops_never_modify_source_registry_or_memory_records() {
        // Apply migrations and seed a source_registry + memory_records row
        // manually (no scan runs — those are not the point of this test).
        let mut conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        migrations::apply(&mut conn).expect("migrations apply");
        conn.execute(
            "INSERT INTO source_registry (provider, source_kind, lifecycle_state, \
             health_state, coverage_level, normalized_root_path, fingerprint, \
             native_project, health_cause) VALUES \
             ('codex', 'agent_memory', 'confirmed', 'unknown', 'full', '/x', \
             'fp-1', NULL, NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO memory_records (record_id, source_id, generation, provider, \
             unit_kind, native_unit_id, native_locator, content_hash, parser_version, \
             title, body, native_project, provider_memory_type, coverage_level, \
             observed_at, source_revision, display_locator) VALUES \
             ('rec_1', 1, 'gen_1', 'codex', 'memory', 'u1', 'loc', 'hash', \
             'file-level/v1', 't', 'b', NULL, 'memory', 'full', 0, 'r1', 'd')",
            [],
        )
        .unwrap();
        let leaked: &'static rusqlite::Connection = Box::leak(Box::new(conn));
        let store = ProjectStore::new(leaked);

        // Snapshot the canonical counts BEFORE the project ops.
        let sources_before = count(leaked, "source_registry");
        let records_before = count(leaked, "memory_records");

        // Flurry: create / rename / add / remove / delete.
        let a = create_project(&store, &CreateProjectRequest { name: "A".to_string() }).unwrap();
        let b = create_project(&store, &CreateProjectRequest { name: "B".to_string() }).unwrap();
        rename_project(
            &store,
            &RenameProjectRequest {
                project_id: a.project_id.clone(),
                name: "A2".to_string(),
            },
        )
        .unwrap();
        add_mapping(
            &store,
            &MappingRequest {
                project_id: a.project_id.clone(),
                provider: "codex".to_string(),
                native_project: None,
            },
        )
        .unwrap();
        add_mapping(
            &store,
            &MappingRequest {
                project_id: b.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some("proj".to_string()),
            },
        )
        .unwrap();
        remove_mapping(
            &store,
            &MappingRequest {
                project_id: b.project_id.clone(),
                provider: "claude_code".to_string(),
                native_project: Some("proj".to_string()),
            },
        )
        .unwrap();
        delete_project(
            &store,
            &DeleteProjectRequest {
                project_id: a.project_id.clone(),
            },
        )
        .unwrap();

        // Counts UNCHANGED — project ops never delete or modify canonical
        // records or sources (non-destruction AC).
        assert_eq!(count(leaked, "source_registry"), sources_before);
        assert_eq!(count(leaked, "memory_records"), records_before);
    }
}
