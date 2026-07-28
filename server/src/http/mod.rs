//! `http` — versioned HTTP handlers, SSE, and DTO mapping.
//!
//! Architecture invariants honoured here (Phase 0 establishes the contract
//! sample):
//!
//! - AD-9 / AD-17 / A-6: request/response payloads use a versioned envelope
//!   carrying `api_version`. Every endpoint DTO in later Stories mirrors this
//!   envelope shape. The transport is a loopback-only HTTP server (revised
//!   AD-9, 2026-07-22); the envelope contract itself is unchanged from the
//!   previous Tauri IPC transport.
//! - AD-13: core owns a shared structured error envelope (stable `code` + safe
//!   `message`, no body / query text / credentials). Phase 0 only fixes the
//!   shape; concrete error codes are added alongside the endpoints that emit
//!   them in Stories 1.2 – 1.6.
//! - The browser UI is the only API caller. UI never touches Providers, FS,
//!   or SQLite directly (AD-1).
//!
//! Phase 0 shipped exactly one endpoint — `ping` — as the contract sample.
//! Story 1.2 adds the first business endpoint, `discover_sources`, which
//! shares the same `Envelope<T>` shape. The TypeScript mirror for the new
//! endpoint lives in `src/api/discover.ts`; the original `ping.ts` mirror
//! remains the canonical reference for the envelope.
//!
//! Handler convention: handlers are plain synchronous functions taking
//! `&IndexState` (never a transport-specific handle). The mutex is held for
//! each whole request — the synchronous handler pattern serializes scans and
//! is the single-owner-per-Source guarantee (AD-5, Story 1.4 spec). tiny_http
//! is one-thread-per-connection, so handlers share the state via `Arc` at the
//! server layer.

pub mod envelope;
pub mod server;

pub use envelope::{Envelope, ErrorEnvelope, Pong, API_VERSION};

use std::sync::MutexGuard;

use crate::application;
use crate::application::query::QueryError;
use crate::application::{OpenError, ProjectError, SourceError, VaultPickerOutcome};
use crate::domain::open::{OpenRequest, OpenResult};
use crate::domain::project::{
    CreateProjectRequest, DeleteProjectRequest, DeleteProjectResponse, MappingRequest,
    RenameProjectRequest, TesseraProjectView,
};
use crate::domain::query::{BrowsePage, BrowseRequest, SearchPage, SearchRequest};
use crate::domain::scan::{ScanError, ScanOutcome, ScanStatus};
use crate::domain::source::{Source, SourceId};
use crate::domain::CandidateSource;
use crate::index::{scan_store::ScanStore, ProjectStore, SourceRegistry};
use crate::{IndexState, RescanEvent, RescanJob};

/// `ping` — contract-sample endpoint (Phase 0).
///
/// Returns a versioned envelope wrapping [`Pong`]. The presence of
/// `api_version` on every response is the contract every later endpoint
/// follows (AD-17/A-6); `ping` exists primarily to prove the typed
/// UI → core → UI round-trip works on the locked stack, and to give the
/// accessibility smoke test (`tests/ui/accessibility.spec.ts`) a keyboard-
/// reachable target during Phase 0.
///
/// Phase 0 has no body content and therefore no error path expected; the
/// structured error envelope shape is still declared (see
/// [`ErrorEnvelope`]) so later Stories can return it without changing the
/// contract surface.
pub fn ping() -> Envelope<Pong> {
    Envelope {
        api_version: API_VERSION,
        payload: Pong {
            name: env!("CARGO_PKG_NAME").to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
    }
}

/// `discover_sources` — list local Candidate Sources (Story 1.2).
///
/// Versioned, **infallible** endpoint. The application service runs the
/// registered provider adapters' discovery slice and returns the union as a
/// vec of [`CandidateSource`]. Discovery only checks directory existence
/// (NFR-5) and returns an empty vec when no supported source exists on this
/// machine — "no candidate" is not an error (Design Notes / I/O matrix).
///
/// Boundaries honored here:
/// - AD-1: the HTTP layer calls only the application service, never adapters
///   or the filesystem.
/// - AD-17 / A-6: the response carries `api_version` on the envelope.
/// - No request payload: Story 1.2 has no parameters, so the deferred request
///   envelope deserialization work (see Phase 0 `deferred-work.md`) stays
///   deferred — it lands with the first versioned request payload (~1.6).
pub fn discover_sources() -> Envelope<Vec<CandidateSource>> {
    // Wrapping is isolated in `wrap_discover` so the HTTP layer's envelope
    // contract is testable with a known payload without depending on the host
    // filesystem (business behavior is pinned in the adapter seam tests).
    wrap_discover(application::discover_sources())
}

/// Wrap discovered candidates in the versioned envelope (AD-17/A-6). Extracted
/// from the handler so the envelope-wrapping is unit-testable with an injected
/// payload — `discover_sources()` itself depends on the host filesystem and may
/// return an empty vec, which would make a direct test vacuous on a host with
/// no memories directory.
fn wrap_discover(candidates: Vec<CandidateSource>) -> Envelope<Vec<CandidateSource>> {
    Envelope {
        api_version: API_VERSION,
        payload: candidates,
    }
}

/// Story 6.2 — Knowledge (Obsidian Vault) discovery response payload. Carries
/// the candidate vaults plus a stable diagnostic code when the Obsidian
/// registry was missing/corrupt/unreadable (AD-37), so the UI can distinguish
/// "no vaults registered" from "registry unreadable" without a silent empty
/// set.
#[derive(Debug, Clone, serde::Serialize)]
pub struct KnowledgeDiscoveryPayload {
    pub candidates: Vec<CandidateSource>,
    /// Stable snake_case wire string (`registry_missing` / `registry_unreadable`
    /// / `registry_corrupt`), or `None` when the registry parsed cleanly.
    pub diagnostic: Option<String>,
}

/// Story 6.2 — discover registered Obsidian Vaults. Routes through the
/// independent Knowledge pipeline (`discover_obsidian_vaults`), never through
/// `ProviderAdapter` (Story 6.1 AC).
pub fn discover_knowledge_sources() -> Envelope<KnowledgeDiscoveryPayload> {
    let result = application::discover_obsidian_vaults();
    let diagnostic = result.diagnostic.map(|d| {
        let code = match d {
            crate::adapters::obsidian::RegistryDiagnostic::RegistryMissing => "registry_missing",
            crate::adapters::obsidian::RegistryDiagnostic::RegistryUnreadable => {
                "registry_unreadable"
            }
            crate::adapters::obsidian::RegistryDiagnostic::RegistryCorrupt => "registry_corrupt",
        };
        code.to_string()
    });
    Envelope {
        api_version: API_VERSION,
        payload: KnowledgeDiscoveryPayload {
            candidates: result.candidates,
            diagnostic,
        },
    }
}

// --- Story 1.3: confirm / reject / disable / list -------------------------
//
// These four handlers share the same shape: they take a typed input (a
// CandidateSource for confirm/reject — the only "allowlist entry" actions; a
// source_id for disable; no args for list), acquire the IndexState mutex,
// construct a SourceRegistry view, and delegate to `application::source`.
// AD-4: confirm/reject are the ONLY endpoints that accept a path (via
// CandidateSource); disable/list accept a source_id / nothing.

/// `confirm_source` — confirm a Candidate Source (AD-4 "allowlist 入边界").
///
/// Idempotent: re-confirming the same root returns the same `source_id` and
/// flips any prior rejected/disabled state back to `confirmed` (wake-up). The
/// root is re-canonicalized at confirm time (NFR-5/6); a vanished or non-dir
/// root surfaces as `confirm_failed`.
///
/// Story 4.1: after a successful confirm, the file-change watcher is started
/// for the confirmed source's canonical root (mirroring the boot-time
/// `boot_start_watches` path). Best-effort: a watcher start failure is logged
/// and swallowed — the request still succeeds, and the periodic reconcile tick
/// still covers the source (AD-8 self-heal). This is the lifecycle hook the
/// spec I/O matrix row "Source confirmed at runtime" requires.
pub fn confirm_source(
    candidate: &CandidateSource,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_source(&registry, candidate)
        .map_err(|err| map_source_error(err, None))?;
    drop(conn);
    // Start the file-change watcher for this newly-confirmed source. The
    // supervisor slot may be `None` (no supervisor installed, e.g. in tests
    // that exercise only the scan surface); in that case the periodic tick is
    // also absent, so the caller is responsible for triggering reconciles.
    start_watch_best_effort(state, &source.source_id, &source.normalized_root_path);
    Ok(wrap_source(source))
}

/// `reject_source` — reject a Candidate Source. Persisted so the decision
/// survives restart. Idempotent by fingerprint.
///
/// Story 4.1: stopping the watcher on reject mirrors the confirm→start hook.
/// A previously-confirmed source being rejected (an unusual but legal
/// transition) would otherwise keep recording hints that the now-rejected
/// source cannot reconcile through.
pub fn reject_source(
    candidate: &CandidateSource,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source = application::reject_source(&registry, candidate)
        .map_err(|err| map_source_error(err, None))?;
    drop(conn);
    stop_watch_best_effort(state, &source.source_id);
    Ok(wrap_source(source))
}

// --- Story 6.3: Knowledge (Obsidian Vault) confirm / reject / picker --------
//
// Independent pipeline (AD-19): routes through confirm_knowledge_source /
// reject_knowledge_source, never through ProviderAdapter (6.1 AC). The picker
// is Rust-owned: the browser only triggers the action and receives a
// VaultPickerOutcome; it never submits a path, URI, or filesystem handle.

/// `confirm_knowledge_source` — confirm an Obsidian Vault as a
/// `local_knowledge` Source (Story 6.3). Blocks on root overlap with
/// `root_overlap`.
pub fn confirm_knowledge_source(
    candidate: &CandidateSource,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source = application::confirm_knowledge_source(&registry, candidate)
        .map_err(|err| map_source_error(err, None))?;
    drop(conn);
    start_watch_best_effort(state, &source.source_id, &source.normalized_root_path);
    Ok(wrap_source(source))
}

/// `reject_knowledge_source` — reject an Obsidian Vault Candidate (Story 6.3).
pub fn reject_knowledge_source(
    candidate: &CandidateSource,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source = application::reject_knowledge_source(&registry, candidate)
        .map_err(|err| map_source_error(err, None))?;
    drop(conn);
    stop_watch_best_effort(state, &source.source_id);
    Ok(wrap_source(source))
}

/// `request_vault_picker` — trigger the Rust-owned native OS folder picker
/// (Story 6.3). The browser receives a `VaultPickerOutcome`; it never supplies
/// a path. Returns the versioned envelope wrapping the outcome.
pub fn request_vault_picker() -> Envelope<VaultPickerOutcome> {
    Envelope {
        api_version: API_VERSION,
        payload: application::request_existing_vault_picker(),
    }
}

/// `disable_source` — disable a confirmed Source by `source_id` (AD-4: only
/// `source_id`, never an arbitrary path). Unknown id → `source_not_found`.
///
/// Story 4.1: disabling stops the watcher and clears any pending hint so a
/// stale hint cannot fire a reconcile for a source that is no longer
/// confirmed. The watcher can be re-started by re-confirming.
pub fn disable_source(
    source_id: &SourceId,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source = application::disable_source(&registry, source_id)
        .map_err(|err| map_source_error(err, Some(&source_id.0)))?;
    drop(conn);
    stop_watch_best_effort(state, &source.source_id);
    Ok(wrap_source(source))
}

/// Story 4.3 — `rebind_source`: the explicit recovery path for a Confirmed
/// Source whose root moved / lost permissions / changed filesystem identity.
/// Takes an old `source_id` (the degraded row 4.2 marked `Degraded +
/// PathMissing`) and a new `root_path` (where the user moved it), canonicalizes
/// and fingerprints the new root, and atomically disables the old row and
/// inserts (or wakes) the new row in ONE SQLite transaction (spec Boundaries —
/// "fail-closed on the new root BEFORE disabling the old, AND why disable+insert
/// must be ONE transaction").
///
/// Boundaries honored here:
/// - AD-1: HTTP calls only the application service; never the registry directly.
/// - AD-4: rebind takes a `source_id` + a path (the path is the only
///   allowlist-style input besides confirm/reject; disable/list still take only
///   `source_id`).
/// - The watcher lifecycle mirrors the row lifecycle: stop the watcher on the
///   OLD source id (its row just went Disabled), and start a fresh watcher on
///   the NEW source id at its new canonical root (Story 4.1's lifecycle hook
///   — a runtime-confirmed source gets a watcher; rebind produces a fresh
///   Confirmed row, so it gets the same hook). Best-effort: a watcher
///   start/stop failure is logged and swallowed.
pub fn rebind_source(
    request: RebindRequest,
    state: &IndexState,
) -> Result<Envelope<Source>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let new_source = application::rebind_source(&registry, &request.source_id, &request.root_path)
        .map_err(|err| map_source_error(err, Some(&request.source_id.0)))?;
    let old_source_id = request.source_id.clone();
    let new_source_id = new_source.source_id.clone();
    let new_root = new_source.normalized_root_path.clone();
    drop(conn);
    // Stop the watcher on the old (now-Disabled) row; start a fresh watcher
    // on the new Confirmed row at its new canonical root.
    stop_watch_best_effort(state, &old_source_id);
    start_watch_best_effort(state, &new_source_id, &new_root);
    Ok(wrap_source(new_source))
}

/// Story 4.3 — rebind request body. Carries the old `source_id` (the row to
/// disable) and the new `root_path` (where the user moved the root). The path
/// is canonicalized by the application layer's policy module, never trusted
/// verbatim.
#[derive(Debug, serde::Deserialize)]
pub struct RebindRequest {
    pub source_id: SourceId,
    pub root_path: String,
}

/// Best-effort `start_watch` on the reconcile supervisor, if one is installed.
/// Logs and swallows watcher errors: the HTTP request must not fail because a
/// notify backend could not register a kernel watch (the periodic reconcile
/// tick still covers the source — AD-8 self-heal).
fn start_watch_best_effort(state: &IndexState, source_id: &SourceId, canonical_root: &str) {
    let Ok(slot) = state.reconcile_supervisor.lock() else {
        eprintln!(
            "tessera: reconcile_supervisor mutex poisoned during start_watch for {source_id}"
        );
        return;
    };
    let Some(supervisor) = slot.as_ref() else {
        return;
    };
    if let Err(e) = supervisor.start_watch(source_id, std::path::Path::new(canonical_root)) {
        eprintln!(
            "tessera: watcher start failed for {source_id} ({canonical_root}): {e:?}; periodic reconcile still covers it"
        );
    }
}

/// Best-effort `stop_watch` on the reconcile supervisor, if one is installed.
/// Idempotent: a source that was never watched is a no-op. Locking errors are
/// logged — the lifecycle transition itself already succeeded.
fn stop_watch_best_effort(state: &IndexState, source_id: &SourceId) {
    let Ok(slot) = state.reconcile_supervisor.lock() else {
        eprintln!(
            "tessera: reconcile_supervisor mutex poisoned during stop_watch for {source_id}"
        );
        return;
    };
    let Some(supervisor) = slot.as_ref() else {
        return;
    };
    supervisor.stop_watch(source_id);
}

/// `list_sources` — list every registered Source (any lifecycle). Versioned
/// envelope, infallible at the contract layer (any DB failure surfaces as
/// `internal`).
pub fn list_sources(state: &IndexState) -> Result<Envelope<Vec<Source>>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let sources =
        application::list_sources(&registry).map_err(|err| map_source_error(err, None))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: sources,
    })
}

/// Search only confirmed Sources' active canonical generations. The HTTP
/// layer receives a validated DTO; it never sees source text or SQLite.
pub fn search(
    request: SearchRequest,
    state: &IndexState,
) -> Result<Envelope<SearchPage>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let page = application::search(&registry, &conn, request).map_err(|error| match error {
        QueryError::BadRequest => ErrorEnvelope::bad_request("search"),
        QueryError::CursorStale => ErrorEnvelope::cursor_stale("search"),
        QueryError::Internal => ErrorEnvelope::internal_for(None, "search"),
    })?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: page,
    })
}

/// Story 3.1 — Browse one confirmed Source's active generation. Query-less
/// entry: the request is scoped to a single `source_id` (validated as a
/// well-formed `src_<n>` upstream; non-confirmed/disabled/rejected/unknown →
/// `400 bad_request` per the I/O matrix). Cursor and empty-state mechanics
/// mirror `search`; `CursorStale → 409` so the UI's existing recovery path
/// re-runs page 1.
pub fn browse(
    request: BrowseRequest,
    state: &IndexState,
) -> Result<Envelope<BrowsePage>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let page = application::browse(&registry, &conn, request).map_err(|error| match error {
        QueryError::BadRequest => ErrorEnvelope::bad_request("browse"),
        QueryError::CursorStale => ErrorEnvelope::cursor_stale("browse"),
        QueryError::Internal => ErrorEnvelope::internal_for(None, "browse"),
    })?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: page,
    })
}

/// `GET /api/knowledge/browse?source=<src_n>&limit=<n>&cursor=<kb.xxx>` —
/// browse Knowledge notes for a single confirmed Vault (Story 6.9). Routes
/// through the independent Knowledge pipeline.
pub fn browse_knowledge(
    source_id: &str,
    limit: u32,
    cursor: Option<&str>,
    state: &IndexState,
) -> Result<Envelope<crate::application::query::KnowledgeBrowsePage>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let source_id = SourceId(source_id.to_string());
    let page =
        application::browse_knowledge(&registry, &conn, &source_id, limit, cursor).map_err(
            |error| match error {
                QueryError::BadRequest => ErrorEnvelope::bad_request("knowledge_browse"),
                QueryError::CursorStale => ErrorEnvelope::cursor_stale("knowledge_browse"),
                QueryError::Internal => ErrorEnvelope::internal_for(None, "knowledge_browse"),
            },
        )?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: page,
    })
}

pub fn open_original_location(
    request: OpenRequest,
    state: &IndexState,
) -> Result<Envelope<OpenResult>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let result = application::open_original_location(&conn, request).map_err(map_open_error)?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: result,
    })
}

/// Read all registered Sources as a server-derived Inventory. This is separate
/// from the lifecycle list because it combines registry facts with scan/index
/// state without asking the browser to infer health or counts.
pub fn source_inventory(
    state: &IndexState,
) -> Result<Envelope<Vec<crate::domain::scan::SourceInventory>>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_inventory(&registry, &conn)
        .map_err(|_| ErrorEnvelope::internal_for(None, "inventory"))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: inventory,
    })
}

/// `GET /api/knowledge/inventory` — Knowledge (Obsidian Vault) Inventory
/// (Story 6.6). One row per confirmed local_knowledge Source, with supported-
/// note count, coverage, health, last-success scan, stale state, and safe
/// latest error. Agent-Memory Sources are never included (AD-19).
pub fn knowledge_inventory(
    state: &IndexState,
) -> Result<Envelope<Vec<crate::domain::scan::KnowledgeInventory>>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let inventory = application::list_knowledge_inventory(&registry, &conn)
        .map_err(|_| ErrorEnvelope::internal_for(None, "knowledge_inventory"))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: inventory,
    })
}

// --- Story 5.1: Tessera Project mapping surface ---------------------------
//
// Six versioned, loopback-only endpoints under `/api/projects`. They all
// follow the same shape as the 1.3 handlers: acquire the IndexState mutex,
// construct a `ProjectStore` view over the guarded connection, and delegate
// to `application::project`. The mutex is held for the whole operation —
// synchronous handlers serialize writes and keep the cardinality pre-check
// + insert inside one transaction honest (the project store's
// `with_transaction` is the atomic boundary; the IndexState mutex guarantees
// no concurrent writer can observe a half-applied transaction).
//
// Boundaries honored here:
// - AD-1: HTTP calls only the application service; never the store directly.
// - AD-17/A-6: responses carry `api_version` on the envelope.
// - NFR-3: errors map to stable codes via `map_project_error`; no body /
//   query text / credentials surface (the project layer carries only
//   user-visible metadata: name + provider + native_project).
// - Zero-source-mutation: no handler here touches `source_registry` or
//   `memory_records`; the application layer never does either. The AC's
//   non-destruction gate is enforced by the application layer's project-only
//   SQL surface.

/// `POST /api/projects/create` — create a Tessera Project. AD-24: creating a
/// project creates zero mappings; the response carries an empty `mappings`
/// array.
pub fn create_project(
    request: CreateProjectRequest,
    state: &IndexState,
) -> Result<Envelope<TesseraProjectView>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let view =
        application::create_project(&store, &request).map_err(|err| map_project_error(err, None))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: view,
    })
}

/// `GET /api/projects` — list every Tessera Project with its mappings. Empty
/// array when none exist (the I/O matrix row "List projects" with no
/// projects).
pub fn list_projects(
    state: &IndexState,
) -> Result<Envelope<Vec<TesseraProjectView>>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let views = application::list_projects(&store).map_err(|err| map_project_error(err, None))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: views,
    })
}

/// `POST /api/projects/rename` — rename a Tessera Project. Advances
/// `updated_at`; unknown id → `project_not_found`.
pub fn rename_project(
    request: RenameProjectRequest,
    state: &IndexState,
) -> Result<Envelope<TesseraProjectView>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let project_id = request.project_id.clone();
    let view = application::rename_project(&store, &request)
        .map_err(|err| map_project_error(err, Some(&project_id.0)))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: view,
    })
}

/// `POST /api/projects/delete` — delete a Tessera Project. Cascades its
/// mappings (`ON DELETE CASCADE`); the response carries the cascade count via
/// [`DeleteProjectResponse`].
pub fn delete_project(
    request: DeleteProjectRequest,
    state: &IndexState,
) -> Result<Envelope<DeleteProjectResponse>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let project_id = request.project_id.clone();
    let outcome = application::delete_project(&store, &request)
        .map_err(|err| map_project_error(err, Some(&project_id.0)))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: outcome,
    })
}

/// `POST /api/projects/mappings/add` — add an explicit `(provider,
/// native_project)` mapping to a project. AD-27 cardinality: a scope already
/// owned by another project returns 409 `mapping_conflict` naming the owner;
/// re-adding the same scope to the same project is idempotent.
pub fn add_mapping(
    request: MappingRequest,
    state: &IndexState,
) -> Result<Envelope<TesseraProjectView>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let project_id = request.project_id.clone();
    let view = application::add_mapping(&store, &request)
        .map_err(|err| map_project_error(err, Some(&project_id.0)))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: view,
    })
}

/// `POST /api/projects/mappings/remove` — remove a mapping. Distinguishes
/// "no such project" (404 `project_not_found`) from "project exists, no such
/// mapping" (404 `mapping_not_found`).
pub fn remove_mapping(
    request: MappingRequest,
    state: &IndexState,
) -> Result<Envelope<TesseraProjectView>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let store = ProjectStore::new(&conn);
    let project_id = request.project_id.clone();
    let view = application::remove_mapping(&store, &request)
        .map_err(|err| map_project_error(err, Some(&project_id.0)))?;
    Ok(Envelope {
        api_version: API_VERSION,
        payload: view,
    })
}

/// Map an application-layer [`ProjectError`] onto the stable API error codes
/// (AD-13). The `project_id` is threaded in so the 404 envelopes can surface
/// the stable id the caller supplied (it is already user-visible).
fn map_project_error(err: ProjectError, project_id: Option<&str>) -> ErrorEnvelope {
    match err {
        ProjectError::BadRequest => ErrorEnvelope::bad_request("project"),
        ProjectError::ProjectNotFound => ErrorEnvelope::project_not_found(project_id),
        ProjectError::MappingConflict { owning_project_name } => {
            ErrorEnvelope::mapping_conflict(&owning_project_name)
        }
        ProjectError::MappingNotFound => ErrorEnvelope::mapping_not_found(project_id),
        ProjectError::Internal => ErrorEnvelope::internal_for(project_id, "project"),
    }
}

/// Start one background rescan. The worker opens its own SQLite connection so
/// an SSE observation request and cancel request never hold the main request
/// connection mutex for the duration of filesystem scanning.
///
/// The reservation block (validate confirmed + `begin_run`) is shared with the
/// watcher/reconcile path via [`application::reserve_run`]: there is ONE
/// canonical reservation path, and both surfaces reuse [`application::
/// scan_reserved_source`] for the FS work, so HTTP rescan and watcher reconcile
/// can never diverge into two mutation paths (spec task — "factor
/// `start_rescan`'s begin_run+spawn into a shared callable"). The HTTP path
/// layers transport job tracking (the SSE event log) on top; the watcher path
/// layers hint-queue tracking on top.
pub fn start_rescan(
    source_id: &SourceId,
    state: &std::sync::Arc<IndexState>,
) -> Result<Envelope<crate::RescanEvent>, ErrorEnvelope> {
    let mut jobs = state.rescan_jobs.lock().map_err(|_| ErrorEnvelope::internal_for(Some(&source_id.0), "rescan"))?;
    if let Some(existing) = jobs.get(&source_id.0) {
        if !existing.terminal {
            return Ok(Envelope { api_version: API_VERSION, payload: existing.events[0].clone() });
        }
    }
    // Shared reservation: validate confirmed + begin_run, holding the
    // synchronous request mutex only for that reservation.
    let (scan_id, fencing_token, generation) =
        match application::reserve_run(source_id, state) {
            Ok(triple) => triple,
            Err(application::TriggerError::AlreadyRunning { .. }) => {
                // AD-5/16/28/32 single-owner gate: another rescan/reconcile is
                // already in-flight for this source. Surface as `bad_request`
                // so the UI can show "a rescan is already running" and the
                // client can retry once the in-flight run finishes.
                return Err(ErrorEnvelope::bad_request("rescan"));
            }
            Err(application::TriggerError::ReservationFailed(reason)) => {
                // Map the reservation failure to the same stable codes the
                // pre-refactor path produced: source_not_found for an unknown
                // id, scan_failed for a non-confirmed source, internal for a
                // DB failure. The reason string comes from the shared
                // reservation code so the two surfaces cannot drift.
                if reason.contains("not found") || reason.contains("rowid") {
                    return Err(ErrorEnvelope::source_not_found(
                        Some(&source_id.0),
                        "rescan",
                    ));
                }
                if reason.contains("not confirmed") {
                    return Err(ErrorEnvelope::scan_failed_not_confirmed(&source_id.0));
                }
                return Err(ErrorEnvelope::internal_for(Some(&source_id.0), "rescan"));
            }
        };
    let job_id = format!("job_{scan_id}");
    let queued = RescanEvent { api_version: API_VERSION, job_id: job_id.clone(), source_id: source_id.0.clone(), sequence: 1, state: "queued".to_string(), message: "Rescan queued.".to_string() };
    jobs.insert(source_id.0.clone(), RescanJob { scan_id, job_id, events: vec![queued.clone()], terminal: false });
    drop(jobs);
    let worker_state = std::sync::Arc::clone(state);
    let worker_source = source_id.clone();
    std::thread::spawn(move || {
        append_rescan_event(&worker_state, &worker_source, scan_id, "running", "Rescan running.");
        let result = (|| -> Result<ScanOutcome, ScanError> {
            let conn = rusqlite::Connection::open(&worker_state.db_path)
                .map_err(|_| ScanError::Internal)?;
            conn.execute_batch("PRAGMA foreign_keys = ON;")
                .map_err(|_| ScanError::Internal)?;
            let registry = SourceRegistry::new(&conn);
            application::scan_reserved_source(&registry, &conn, &worker_source, scan_id, fencing_token, generation)
        })();
        match result {
            Ok(_) => {
                append_rescan_event(&worker_state, &worker_source, scan_id, "succeeded", "Rescan complete.");
            }
            Err(ScanError::Cancelled) => {
                append_rescan_event(&worker_state, &worker_source, scan_id, "cancelled", "Rescan cancelled.");
            }
            Err(_) => {
                append_rescan_event(&worker_state, &worker_source, scan_id, "failed", "Rescan failed. The previous index is unchanged.");
            }
        }
    });
    Ok(Envelope {
        api_version: API_VERSION,
        payload: queued,
    })
}

/// Story 4.4 — `POST /api/index/rebuild` payload. Carries the number of
/// Confirmed Sources being re-scanned (`0` when the registry has no
/// Confirmed Sources — the wipe still ran, clearing any leaked disabled /
/// rejected records). Mirrors the spec's `200 {sources_rescanning: N}`
/// response shape.
#[derive(Debug, serde::Serialize)]
pub struct RebuildOutcome {
    /// Count of Confirmed Sources the rebuild dispatched a re-scan for.
    pub sources_rescanning: u32,
}

/// Story 4.4 — full Derived Index rebuild (AD-29 reset boundary promoted to a
/// repeatable runtime operation).
///
/// Critical section (spec Boundaries — "Reserve the rebuild's per-source runs
/// UNDER the IndexState mutex ... so reconcile cannot grab any Confirmed
/// source between the wipe and the rebuild's dispatch"):
/// 1. Acquire the synchronous request mutex.
/// 2. `application::rebuild_index(&conn)` — race guard (`any_in_flight_run` →
///    409 `rebuild_failed`), atomic four-target wipe (`memory_records` +
///    `scan_runs` + `scan_diagnostics` + `tessera_meta` rows matching
///    `active_generation:%`), enumerate Confirmed Source ids. Preserves
///    `source_registry`, `tessera_meta.schema_version`,
///    `tessera_migrations_applied`, and any other `tessera_meta` key.
/// 3. Reserve a run per Confirmed Source via `ScanStore::begin_run` UNDER the
///    same mutex (the post-wipe `scan_runs` is empty so the single-owner gate
///    is trivially satisfied; reconcile cannot grab any of these sources
///    between the wipe and the dispatch).
/// 4. Release the mutex BEFORE spawning workers (NFR-12: queries stay
///    available during the rebuild — the synchronous request mutex is held
///    only for the wipe + reservation, not for the FS work).
///
/// Worker fan-out: each reservation spawns a thread that opens its own
/// `rusqlite::Connection` and reuses [`application::scan_reserved_source`] —
/// exactly the [`start_rescan`] pattern. Per-source progress flows through the
/// existing `rescan_jobs` map so the existing `GET /api/sources/rescan/events`
/// SSE surface works with no new transport (spec Design Notes — "Why reuse
/// `rescan_jobs` + existing SSE, not a new channel"). Existing 4.2 source-
/// scoped error isolation applies: a source that fails to re-scan is marked
/// degraded/error + cause + last-success + stale per 4.2; other sources still
/// rebuild.
pub fn start_rebuild(
    state: &std::sync::Arc<IndexState>,
) -> Result<Envelope<RebuildOutcome>, ErrorEnvelope> {
    // Critical section: hold the synchronous request mutex for the wipe + the
    // per-source begin_run reservations. Releasing the mutex before spawning
    // workers keeps queries available during the per-source FS work (NFR-12).
    let reservations: Vec<(SourceId, i64, i64, crate::domain::scan::Generation)> = {
        let conn = lock_conn(state)?;
        // Race guard + atomic wipe + collect Confirmed ids. The race guard is
        // the primary correctness check: any in-flight scan would race with
        // the wipe, so we reject up front with a 409 the UI can act on.
        let confirmed = application::rebuild_index(&conn).map_err(|err| match err {
            application::RebuildError::InFlight => ErrorEnvelope::rebuild_failed(),
            application::RebuildError::Internal => {
                ErrorEnvelope::internal_for(None, "rebuild")
            }
        })?;
        // Reserve a run for each Confirmed Source UNDER the same mutex. The
        // registry was just enumerated by `rebuild_index`, so each id is
        // known to exist and be Confirmed; `has_in_flight_run` is trivially
        // false after the wipe (`scan_runs` is empty), so we skip the choke-
        // point check and call `begin_run` directly. `reserve_run` would re-
        // acquire the mutex (deadlock under std::Mutex), so we inline its
        // body minus the redundant checks.
        //
        // Patch A — on `begin_run` Err, BEFORE returning the error, clean up
        // every previously-reserved `queued` row in this same critical
        // section. Without this, the wipe already committed but the
        // already-reserved rows stay `queued` with no worker ever spawned →
        // `any_in_flight_run()` returns true forever → every later rebuild
        // rejects 409 until process reboot (`recover_stale_runs` is boot-
        // only). Cleanup is best-effort (a clean failure here just leaves
        // the row for the next boot's recovery, matching the existing
        // "transient failures stay queued until boot" posture in
        // `application::reconcile::fail_reserved_run_from_main_conn`).
        let store = ScanStore::new(&conn);
        let mut reserved = Vec::with_capacity(confirmed.len());
        let mut reservation_failed: Option<SourceId> = None;
        for source_id in &confirmed {
            let Some(rowid) = ScanStore::source_rowid(source_id) else {
                continue;
            };
            match store.begin_run(rowid, "pending") {
                Ok((scan_id, fencing_token, generation)) => {
                    reserved.push((source_id.clone(), scan_id, fencing_token, generation));
                }
                Err(_) => {
                    reservation_failed = Some(source_id.clone());
                    break;
                }
            }
        }
        if let Some(failed_id) = reservation_failed {
            // Best-effort cleanup of every `queued` row this loop reserved.
            // `fail_run` is the right primitive (vs. raw DELETE) because it
            // also clears any in-flight staging rows the reservation may have
            // produced — but post-wipe `scan_runs` is empty before
            // `begin_run`, so a queued reservation has not yet staged any
            // rows; `fail_run` is still the canonical "this run is dead"
            // transition and stays correct if a future change adds staging
            // before spawn. The accumulated scan_ids are the only ones ever
            // produced by this rebuild dispatch; no other run is touched.
            for (_, scan_id, _, _) in &reserved {
                let _ = store.fail_run(*scan_id, "internal");
            }
            return Err(ErrorEnvelope::internal_for(
                Some(&failed_id.0),
                "rebuild",
            ));
        }
        reserved
    }; // Synchronous request mutex released here.

    let sources_rescanning = u32::try_from(reservations.len()).unwrap_or(u32::MAX);

    // Pre-populate `rescan_jobs` with a `queued` event per Confirmed Source so
    // the existing SSE surface works with no new transport. Workers append
    // `running`/`succeeded`/`failed`/`cancelled` events as they run. Matches
    // the `start_rescan` pattern.
    let mut jobs = state
        .rescan_jobs
        .lock()
        .map_err(|_| ErrorEnvelope::internal_for(None, "rebuild"))?;
    for (source_id, scan_id, _, _) in &reservations {
        let job_id = format!("job_{scan_id}");
        let queued = RescanEvent {
            api_version: API_VERSION,
            job_id: job_id.clone(),
            source_id: source_id.0.clone(),
            sequence: 1,
            state: "queued".to_string(),
            message: "Rebuild rescan queued.".to_string(),
        };
        jobs.insert(
            source_id.0.clone(),
            RescanJob {
                scan_id: *scan_id,
                job_id,
                events: vec![queued],
                terminal: false,
            },
        );
    }
    drop(jobs);

    // Fan out: spawn one worker per reservation. Each opens its OWN connection
    // and reuses `application::scan_reserved_source` — exactly the
    // `start_rescan` worker pattern. The synchronous request mutex is NOT held
    // for the FS work (NFR-12).
    for (source_id, scan_id, fencing_token, generation) in reservations {
        let worker_state = std::sync::Arc::clone(state);
        std::thread::spawn(move || {
            append_rescan_event(
                &worker_state,
                &source_id,
                scan_id,
                "running",
                "Rebuild rescan running.",
            );
            let result = (|| -> Result<ScanOutcome, ScanError> {
                let conn = rusqlite::Connection::open(&worker_state.db_path)
                    .map_err(|_| ScanError::Internal)?;
                conn.execute_batch("PRAGMA foreign_keys = ON;")
                    .map_err(|_| ScanError::Internal)?;
                let registry = SourceRegistry::new(&conn);
                application::scan_reserved_source(
                    &registry,
                    &conn,
                    &source_id,
                    scan_id,
                    fencing_token,
                    generation,
                )
            })();
            match result {
                Ok(_) => append_rescan_event(
                    &worker_state,
                    &source_id,
                    scan_id,
                    "succeeded",
                    "Rebuild rescan complete.",
                ),
                Err(ScanError::Cancelled) => append_rescan_event(
                    &worker_state,
                    &source_id,
                    scan_id,
                    "cancelled",
                    "Rebuild rescan cancelled.",
                ),
                Err(_) => append_rescan_event(
                    &worker_state,
                    &source_id,
                    scan_id,
                    "failed",
                    // Patch C — rebuild-specific failure message. The
                    // start_rescan worker's "The previous index is
                    // unchanged." is FALSE in the rebuild context: the wipe
                    // already cleared every derived row, so a failed re-scan
                    // leaves this source with NO indexed records until a
                    // later scan succeeds. Honest about the post-rebuild
                    // state; the failure cause surfaces on the source's
                    // inventory row via 4.2's `cause + last_success`.
                    "Rebuild rescan failed. This source has no indexed records until a later scan succeeds.",
                ),
            }
        });
    }

    Ok(Envelope {
        api_version: API_VERSION,
        payload: RebuildOutcome { sources_rescanning },
    })
}

pub fn cancel_rescan_request(
    source_id: &SourceId,
    state: &IndexState,
) -> Result<Envelope<crate::RescanEvent>, ErrorEnvelope> {
    let mut jobs = state.rescan_jobs.lock().map_err(|_| ErrorEnvelope::internal_for(Some(&source_id.0), "rescan"))?;
    let Some(job) = jobs.get(&source_id.0) else { return Err(ErrorEnvelope::bad_request("rescan")); };
    if job.terminal { return Err(ErrorEnvelope::bad_request("rescan")); }
    let scan_id = job.scan_id;
    let conn = lock_conn(state)?;
    let source_rowid = ScanStore::source_rowid(source_id).ok_or_else(|| ErrorEnvelope::source_not_found(Some(&source_id.0), "rescan"))?;
    let cancelled = ScanStore::new(&conn).cancel_run(scan_id, source_rowid).map_err(|_| ErrorEnvelope::internal_for(Some(&source_id.0), "rescan"))?;
    if !cancelled {
        return Err(ErrorEnvelope::bad_request("rescan"));
    }
    drop(conn);
    let event = append_rescan_event_locked(&mut jobs, source_id, scan_id, "cancelled", "Rescan cancelled.");
    Ok(Envelope { api_version: API_VERSION, payload: event })
}

pub fn rescan_events(
    source_id: &SourceId,
    job_id: &str,
    after: u64,
    state: &IndexState,
) -> Result<Vec<crate::RescanEvent>, ErrorEnvelope> {
    let jobs = state
        .rescan_jobs
        .lock()
        .map_err(|_| ErrorEnvelope::internal_for(Some(&source_id.0), "rescan"))?;
    let Some(job) = jobs.get(&source_id.0) else { return Ok(Vec::new()); };
    if job.job_id != job_id { return Err(ErrorEnvelope::bad_request("rescan")); }
    Ok(job.events.iter().filter(|event| event.sequence > after).cloned().collect())
}

fn append_rescan_event(
    state: &IndexState,
    source_id: &SourceId,
    scan_id: i64,
    event_state: &str,
    message: &str,
) -> crate::RescanEvent {
    let mut jobs = state.rescan_jobs.lock().expect("rescan job mutex must not be poisoned");
    append_rescan_event_locked(&mut jobs, source_id, scan_id, event_state, message)
}

fn append_rescan_event_locked(
    jobs: &mut std::collections::HashMap<String, RescanJob>, source_id: &SourceId, scan_id: i64,
    event_state: &str, message: &str,
) -> RescanEvent {
    const MAX_EVENTS: usize = 32;
    let Some(job) = jobs.get_mut(&source_id.0) else { return RescanEvent { api_version: API_VERSION, job_id: format!("job_{scan_id}"), source_id: source_id.0.clone(), sequence: 0, state: event_state.to_string(), message: message.to_string() }; };
    if job.scan_id != scan_id || job.terminal { return job.events.last().cloned().expect("job has queued event"); }
    let event = RescanEvent { api_version: API_VERSION, job_id: job.job_id.clone(), source_id: source_id.0.clone(), sequence: job.events.last().map_or(1, |last| last.sequence + 1), state: event_state.to_string(), message: message.to_string() };
    job.events.push(event.clone());
    if job.events.len() > MAX_EVENTS { job.events.remove(0); }
    job.terminal = matches!(event_state, "succeeded" | "failed" | "cancelled");
    event
}

/// Acquire the IndexState mutex and return a guarded connection reference.
/// Maps poisoning / lock failure to the generic `internal` error envelope.
fn lock_conn(state: &IndexState) -> Result<MutexGuard<'_, rusqlite::Connection>, ErrorEnvelope> {
    state.conn.lock().map_err(|_| ErrorEnvelope::internal())
}

/// Map an application-layer [`SourceError`] onto the stable API error codes
/// (AD-13). Keeps the application → http mapping in one place.
fn map_open_error(err: OpenError) -> ErrorEnvelope {
    match err {
        OpenError::RecordNotFound => ErrorEnvelope::record_not_found(),
        OpenError::OpenFailed { source_id } => {
            let source_id = source_id.as_ref().map(|id| id.0.as_str());
            ErrorEnvelope::open_failed(source_id)
        }
        OpenError::Internal => ErrorEnvelope::internal_for(None, "open"),
    }
}

fn map_source_error(err: SourceError, source_id: Option<&str>) -> ErrorEnvelope {
    match err {
        SourceError::ConfirmFailed => ErrorEnvelope::confirm_failed(source_id, "source"),
        SourceError::SourceNotFound => ErrorEnvelope::source_not_found(source_id, "source"),
        SourceError::RootOverlap => ErrorEnvelope::root_overlap(source_id, "source"),
        SourceError::Internal => ErrorEnvelope::internal_for(source_id, "source"),
    }
}

/// Wrap a Source in the versioned envelope (AD-17/A-6). Extracted as a seam
/// so the wire shape — including the `#[serde(skip)]` on fingerprint — is
/// unit-testable without a live DB.
fn wrap_source(source: Source) -> Envelope<Source> {
    Envelope {
        api_version: API_VERSION,
        payload: source,
    }
}

// --- Story 1.4: scan_source / get_scan_status ------------------------------
//
// Both handlers follow the same shape as the 1.3 handlers: acquire the
// IndexState mutex, construct registry + scan-store views over the guarded
// connection, and delegate to `application::scan`. The mutex is held for the
// whole scan + commit — the synchronous handler pattern serializes scans and
// is the single-owner-per-Source guarantee (AD-5). No async, no tokio (spec
// Never); tiny_http's one-thread-per-connection model keeps handlers
// synchronous.

/// `scan_source` — run the read-only scan pipeline for a confirmed Source
/// (AD-1). Synchronous; returns `Envelope<ScanOutcome>` on a fully-successful
/// scan (staging generation committed as active under CAS). Failures surface
/// as a structured `ErrorEnvelope` (`scan_failed` / `source_not_found` /
/// `confirm_failed` / `internal`) and never activate a partial generation
/// (NFR-9).
pub fn scan_source(
    source_id: &SourceId,
    state: &IndexState,
) -> Result<Envelope<ScanOutcome>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let outcome = application::scan_source(&registry, &conn, source_id)
        .map_err(|err| map_scan_error(err, source_id))?;
    Ok(wrap_scan_outcome(outcome))
}

/// `get_scan_status` — report the most recent run state + active generation +
/// active record count for a Source (AD-13 safe surface). Unknown id →
/// `source_not_found`.
pub fn get_scan_status(
    source_id: &SourceId,
    state: &IndexState,
) -> Result<Envelope<ScanStatus>, ErrorEnvelope> {
    let conn = lock_conn(state)?;
    let registry = SourceRegistry::new(&conn);
    let status = application::get_scan_status(&registry, &conn, source_id)
        .map_err(|err| map_scan_error(err, source_id))?;
    Ok(wrap_scan_status(status))
}

/// Wrap a ScanOutcome in the versioned envelope (AD-17/A-6). Extracted as a
/// seam so the wire shape is unit-testable without a live DB.
fn wrap_scan_outcome(outcome: ScanOutcome) -> Envelope<ScanOutcome> {
    Envelope {
        api_version: API_VERSION,
        payload: outcome,
    }
}

/// Wrap a ScanStatus in the versioned envelope (AD-17/A-6). Extracted as a
/// seam so the wire shape is unit-testable without a live DB.
fn wrap_scan_status(status: ScanStatus) -> Envelope<ScanStatus> {
    Envelope {
        api_version: API_VERSION,
        payload: status,
    }
}

/// Map an application-layer [`ScanError`] onto the stable API error codes
/// (AD-13). Keeps the application → http mapping in one place.
fn map_scan_error(err: ScanError, source_id: &SourceId) -> ErrorEnvelope {
    let source_id = source_id.0.as_str();
    match err {
        ScanError::SourceNotFound => ErrorEnvelope::source_not_found(Some(source_id), "scan"),
        ScanError::RootInvalid | ScanError::RootIdentityChanged => {
            ErrorEnvelope::confirm_failed(Some(source_id), "scan")
        }
        ScanError::NotConfirmed => ErrorEnvelope::scan_failed_not_confirmed(source_id),
        ScanError::EnumerationFailed
        | ScanError::ReadFailed
        | ScanError::ParseFailed
        | ScanError::EmptyScanWithActiveGeneration
        | ScanError::CommitCasFailed
        | ScanError::Cancelled => ErrorEnvelope::scan_failed(source_id),
        ScanError::DirtyAfterValidation => ErrorEnvelope::scan_failed_source_changed(source_id),
        ScanError::Internal => ErrorEnvelope::internal_for(Some(source_id), "scan"),
    }
}

/// Convenience marker so the HTTP module can be named in tests without pulling
/// a handler symbol.
#[derive(Debug, serde::Serialize)]
pub struct HttpMarker;

#[cfg(test)]
mod tests {
    use super::*;

    /// I/O matrix row 1 (happy round-trip): `ping` returns a versioned envelope
    /// carrying `api_version` and a populated `Pong` payload — the contract
    /// every later endpoint mirrors (AD-17/A-6). The full UI→core→UI wiring is
    /// exercised end-to-end only once Playwright is activated (see
    /// `tests/ui/accessibility.spec.ts`); this test pins the core half of the
    /// round-trip on the locked stack.
    #[test]
    fn ping_returns_versioned_envelope() {
        let env = ping();
        assert_eq!(env.api_version, API_VERSION);
        assert_eq!(env.payload.name, env!("CARGO_PKG_NAME"));
        assert!(!env.payload.version.is_empty());
    }

    /// `wrap_discover` carries an injected payload through the versioned
    /// envelope non-vacuously — this is the HTTP layer's actual job (envelope
    /// wrapping per AD-17/A-6). `discover_sources()`'s business behavior is
    /// pinned in the adapter seam tests (`codex_discover.rs`); testing it here
    /// would be vacuous on a host with no memories dir, so we inject instead.
    #[test]
    fn wrap_discover_carries_injected_payload_in_versioned_envelope() {
        let known = vec![CandidateSource {
            provider: "codex".to_string(),
            root_path: "/tmp/codex/memories".to_string(),
            basis: crate::domain::DiscoveryBasis::CodexHomeEnv,
            coverage_level: crate::domain::CoverageLevel::Full,
            native_project: None,
        }];
        let env = wrap_discover(known);
        assert_eq!(env.api_version, API_VERSION);
        assert_eq!(env.payload.len(), 1, "injected payload carried through");
        assert_eq!(env.payload[0].provider, "codex");
        assert_eq!(
            env.payload[0].basis,
            crate::domain::DiscoveryBasis::CodexHomeEnv
        );
        assert_eq!(
            env.payload[0].coverage_level,
            crate::domain::CoverageLevel::Full
        );
    }

    /// The handler itself is wired and infallible: it always returns a Vec
    /// (never errors) against the real environment, and any candidate it
    /// happens to find declares a known provider id with Full coverage. Count
    /// and provider mix are host-dependent and intentionally not asserted
    /// (covered by adapter seam tests); Story 2.1 widens discovery from
    /// Codex-only to the union of every registered adapter (Codex + Claude
    /// Code), so a host with Claude Code projects under `~/.claude/projects/`
    /// will see both providers' candidates here.
    #[test]
    fn discover_sources_handler_is_infallible_and_versioned() {
        let env = discover_sources();
        assert_eq!(env.api_version, API_VERSION);
        for c in &env.payload {
            assert!(
                matches!(c.provider.as_str(), "codex" | "claude_code"),
                "unknown provider on the wire: {}",
                c.provider
            );
            assert_eq!(
                c.coverage_level,
                crate::domain::CoverageLevel::Full,
                "registered adapters must carry Full coverage"
            );
        }
    }

    /// Round-trip a candidate through serde to pin the wire shape — the TS
    /// mirror in `src/api/discover.ts` must match exactly.
    ///
    /// NOTE: `Envelope.api_version` is `&'static str`, so the *envelope* is
    /// serialize-only on the Rust side. The payload itself round-trips
    /// cleanly. Full envelope deserialization is deferred (Design Notes) — it
    /// becomes relevant once Story 1.6 ships the first versioned request
    /// payload, at which point `api_version` will move to an owned type.
    #[test]
    fn candidate_source_wire_shape_round_trips() {
        let candidate = CandidateSource {
            provider: "codex".to_string(),
            root_path: "/tmp/codex/memories".to_string(),
            basis: crate::domain::DiscoveryBasis::CodexHomeEnv,
            coverage_level: crate::domain::CoverageLevel::Full,
            native_project: None,
        };
        let env = Envelope {
            api_version: API_VERSION,
            payload: vec![candidate.clone()],
        };
        let json = serde_json::to_string(&env).expect("serialize");
        // Stable wire strings — must match the TS client's narrow type.
        assert!(json.contains("\"api_version\":\"1\""), "json was: {json}");
        assert!(json.contains("\"provider\":\"codex\""));
        assert!(json.contains("\"basis\":\"codex_home_env\""));
        assert!(json.contains("\"coverage_level\":\"full\""));
        assert!(json.contains("\"native_project\":null"));

        // Payload round-trips into the same value.
        let payload_json = serde_json::to_string(&vec![candidate]).expect("serialize payload");
        let back: Vec<CandidateSource> =
            serde_json::from_str(&payload_json).expect("deserialize payload");
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].provider, "codex");
        assert_eq!(back[0].basis, crate::domain::DiscoveryBasis::CodexHomeEnv);
        assert_eq!(back[0].coverage_level, crate::domain::CoverageLevel::Full);
        assert!(back[0].native_project.is_none());
    }

    /// `wrap_source` carries a Source through the versioned envelope with the
    /// fingerprint hidden (AD-17/A-6 + Design Notes "为何 Source DTO 隐藏
    /// fingerprint"). Extracted as a seam so this contract is pinned without
    /// a live DB.
    #[test]
    fn wrap_source_carries_versioned_envelope_with_fingerprint_hidden() {
        use crate::domain::source::{
            HealthCause, HealthState, SourceFingerprint, SourceId, SourceKind, SourceLifecycle,
        };
        let src = Source {
            source_id: SourceId::from_rowid(7),
            provider: "codex".to_string(),
            source_kind: SourceKind::AgentMemory,
            lifecycle_state: SourceLifecycle::Confirmed,
            health_state: HealthState::Unknown,
            coverage_level: crate::domain::CoverageLevel::Full,
            normalized_root_path: "/x/memories".to_string(),
            native_project: None,
            fingerprint: SourceFingerprint("root-fingerprint/v1|internal".to_string()),
            health_cause: HealthCause::None,
        };
        let env = wrap_source(src);
        assert_eq!(env.api_version, API_VERSION);
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(json.contains("\"api_version\":\"1\""));
        assert!(json.contains("\"source_id\":\"src_7\""));
        assert!(json.contains("\"lifecycle_state\":\"confirmed\""));
        // Fingerprint must NOT cross the wire.
        assert!(!json.contains("fingerprint"));
        assert!(!json.contains("root-fingerprint"));
        // Story 4.2: health_cause is also hidden on the bare-Source wire.
        assert!(!json.contains("health_cause"));
    }

    /// Source round-trips through serde into the same value (payload only —
    /// the envelope's `api_version` is `&'static str` so the full envelope is
    /// serialize-only on the Rust side, same as Phase 0). Pins the wire shape
    /// the TS mirror in `src/api/sources.ts` must match.
    #[test]
    fn source_wire_shape_round_trips() {
        use crate::domain::source::{
            HealthCause, HealthState, SourceFingerprint, SourceId, SourceKind, SourceLifecycle,
        };
        let src = Source {
            source_id: SourceId("src_3".to_string()),
            provider: "codex".to_string(),
            source_kind: SourceKind::AgentMemory,
            lifecycle_state: SourceLifecycle::Disabled,
            health_state: HealthState::Unknown,
            coverage_level: crate::domain::CoverageLevel::Full,
            normalized_root_path: "/y/memories".to_string(),
            native_project: None,
            fingerprint: SourceFingerprint("skipped-on-wire".to_string()),
            health_cause: HealthCause::None,
        };
        let json = serde_json::to_string(&src).expect("serialize");
        // Stable wire strings — must match the TS client's narrow type.
        assert!(json.contains("\"source_id\":\"src_3\""));
        assert!(json.contains("\"provider\":\"codex\""));
        assert!(json.contains("\"source_kind\":\"agent_memory\""));
        assert!(json.contains("\"lifecycle_state\":\"disabled\""));
        assert!(json.contains("\"health_state\":\"unknown\""));
        assert!(json.contains("\"coverage_level\":\"full\""));
        assert!(json.contains("\"normalized_root_path\":\"/y/memories\""));
        assert!(json.contains("\"native_project\":null"));
        // Fingerprint field must not appear on the wire.
        assert!(!json.contains("fingerprint"));

        let back: Source = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.source_id.0, "src_3");
        assert_eq!(back.provider, "codex");
        assert_eq!(back.source_kind, SourceKind::AgentMemory);
        assert_eq!(back.lifecycle_state, SourceLifecycle::Disabled);
        assert_eq!(back.health_state, HealthState::Unknown);
        assert_eq!(back.coverage_level, crate::domain::CoverageLevel::Full);
        assert_eq!(back.normalized_root_path, "/y/memories");
        assert!(back.native_project.is_none());
        // Fingerprint was skipped on the wire → default-constructed on
        // deserialize. The internal key is never re-read from JSON.
        assert_eq!(back.fingerprint.0, "");
    }

    /// `map_source_error` routes each application-layer `SourceError` to the
    /// stable API error code the TS client depends on (AD-13). This pins the
    /// handler `Err` return path — which `wrap_source`/wire-shape tests above
    /// do not otherwise exercise — so a typo'd mapping can't ship silently.
    #[test]
    fn map_source_error_routes_to_stable_api_codes() {
        let cf = map_source_error(SourceError::ConfirmFailed, None);
        assert_eq!(cf.code, "confirm_failed");
        let snf = map_source_error(SourceError::SourceNotFound, Some("src_7"));
        assert_eq!(snf.code, "source_not_found");
        let int = map_source_error(SourceError::Internal, None);
        assert_eq!(int.code, "internal");
        // AD-13: safe messages are non-empty and never carry body/query/creds.
        for env in [cf, snf, int] {
            assert!(!env.message.is_empty(), "non-empty safe message");
            let lower = env.message.to_lowercase();
            assert!(!lower.contains("body"));
            assert!(!lower.contains("credential"));
        }
    }

    /// `map_scan_error` routes each application-layer `ScanError` to the
    /// stable API error code the TS client depends on (AD-13). Pins the
    /// handler `Err` return path for both scan endpoints.
    #[test]
    fn map_scan_error_routes_to_stable_api_codes() {
        use crate::domain::scan::ScanError;
        let source_id = SourceId("src_7".to_string());
        assert_eq!(
            map_scan_error(ScanError::SourceNotFound, &source_id).code,
            "source_not_found"
        );
        assert_eq!(
            map_scan_error(ScanError::RootInvalid, &source_id).code,
            "confirm_failed"
        );
        // NotConfirmed keeps the stable `scan_failed` code but carries an
        // accurate, distinct message (the generic one wrongly implies a
        // previous index exists).
        let not_confirmed = map_scan_error(ScanError::NotConfirmed, &source_id);
        assert_eq!(not_confirmed.code, "scan_failed");
        assert!(not_confirmed.message.contains("not confirmed"));
        assert_ne!(
            not_confirmed.message,
            map_scan_error(ScanError::ReadFailed, &source_id).message
        );
        assert_eq!(
            map_scan_error(ScanError::EnumerationFailed, &source_id).code,
            "scan_failed"
        );
        assert_eq!(
            map_scan_error(ScanError::ReadFailed, &source_id).code,
            "scan_failed"
        );
        assert_eq!(
            map_scan_error(ScanError::ParseFailed, &source_id).code,
            "scan_failed"
        );
        assert_eq!(
            map_scan_error(ScanError::DirtyAfterValidation, &source_id).code,
            "scan_failed"
        );
        assert_eq!(
            map_scan_error(ScanError::CommitCasFailed, &source_id).code,
            "scan_failed"
        );
        assert_eq!(
            map_scan_error(ScanError::Internal, &source_id).code,
            "internal"
        );
        // AD-13: safe messages are non-empty and never carry body/query/creds.
        for env in [
            map_scan_error(ScanError::ReadFailed, &source_id),
            map_scan_error(ScanError::SourceNotFound, &source_id),
        ] {
            assert!(!env.message.is_empty());
            let lower = env.message.to_lowercase();
            assert!(!lower.contains("body"));
            assert!(!lower.contains("credential"));
        }
    }

    /// `map_open_error` routes each application-layer `OpenError` to the stable
    /// API error code the TS client depends on (AD-13). The `open_failed` 409
    /// path is currently exercised only at the application layer
    /// (`tests/open.rs`) and over the wire (`tests/http_api.rs`), never through
    /// `map_open_error` directly — this pins the handler `Err` return path so
    /// a typo'd mapping can't ship silently. Mirrors
    /// `map_source_error_routes_to_stable_api_codes` /
    /// `map_scan_error_routes_to_stable_api_codes`.
    #[test]
    fn map_open_error_routes_to_stable_api_codes() {
        let rnf = map_open_error(OpenError::RecordNotFound);
        assert_eq!(rnf.code, "record_not_found");
        let of = map_open_error(OpenError::OpenFailed {
            source_id: Some(SourceId("src_7".to_string())),
        });
        assert_eq!(of.code, "open_failed");
        assert_eq!(of.source_id.as_deref(), Some("src_7"));
        // `OpenFailed` may surface without a source attribution; the stable
        // code is unchanged.
        let of_anon = map_open_error(OpenError::OpenFailed { source_id: None });
        assert_eq!(of_anon.code, "open_failed");
        assert!(of_anon.source_id.is_none());
        let int = map_open_error(OpenError::Internal);
        assert_eq!(int.code, "internal");
        // AD-13/NFR-3: the safe messages are static strings — no source path,
        // memory body, or credential is interpolated into either the message
        // or the serialized envelope. The `native_locator` / filesystem root
        // never cross this boundary; assert that via the serialized form too.
        for env in [rnf, of, of_anon, int] {
            assert!(!env.message.is_empty(), "non-empty safe message");
            let lower = env.message.to_lowercase();
            assert!(!lower.contains("body"));
            assert!(!lower.contains("credential"));
            assert!(
                !lower.contains("native_locator"),
                "internal field name leaked into message: {}",
                env.message
            );
            let json = serde_json::to_string(&env).expect("serialize");
            assert!(!json.contains("body"));
            assert!(!json.contains("credential"));
            assert!(
                !json.contains("native_locator"),
                "internal field name leaked onto the wire: {json}"
            );
            assert!(
                !json.contains('\\'),
                "no filesystem path separator leaks onto the wire: {json}"
            );
        }
    }

    /// `wrap_scan_outcome` carries a ScanOutcome through the versioned
    /// envelope; the wire shape matches the TS mirror (`src/api/scan.ts`).
    #[test]
    fn wrap_scan_outcome_carries_versioned_envelope() {
        use crate::domain::scan::{Generation, ScanOutcome};
        use crate::domain::source::SourceId;
        let outcome = ScanOutcome {
            source_id: SourceId("src_1".to_string()),
            scan_id: 5,
            generation: Generation("gen_5".to_string()),
            records_indexed: 3,
        };
        let env = wrap_scan_outcome(outcome);
        assert_eq!(env.api_version, API_VERSION);
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(json.contains("\"api_version\":\"1\""));
        assert!(json.contains("\"source_id\":\"src_1\""));
        assert!(json.contains("\"generation\":\"gen_5\""));
        assert!(json.contains("\"records_indexed\":3"));
        // No body / path detail and no redundant outcome field (Ok = success).
        assert!(!json.contains("body"));
        assert!(!json.contains("\"outcome\""));
    }

    /// `wrap_scan_status` carries a ScanStatus through the versioned envelope,
    /// including the never-scanned null shape.
    #[test]
    fn wrap_scan_status_carries_versioned_envelope() {
        use crate::domain::scan::{Generation, ScanRunState, ScanStatus};
        use crate::domain::source::SourceId;
        let status = ScanStatus {
            source_id: SourceId("src_2".to_string()),
            state: Some(ScanRunState::Succeeded),
            active_generation: Some(Generation("gen_2".to_string())),
            active_records: 7,
        };
        let env = wrap_scan_status(status);
        assert_eq!(env.api_version, API_VERSION);
        let json = serde_json::to_string(&env).expect("serialize");
        assert!(json.contains("\"state\":\"succeeded\""));
        assert!(json.contains("\"active_generation\":\"gen_2\""));
        assert!(json.contains("\"active_records\":7"));

        let never = ScanStatus {
            source_id: SourceId("src_3".to_string()),
            state: None,
            active_generation: None,
            active_records: 0,
        };
        let json = serde_json::to_string(&wrap_scan_status(never)).expect("serialize");
        assert!(json.contains("\"state\":null"));
        assert!(json.contains("\"active_generation\":null"));
    }
}
