//! `application::source` — orchestration of Source confirm / reject / disable /
//! list (Story 1.3).
//!
//! This is the only layer allowed to coordinate the policy (canonicalize) +
//! domain (fingerprint) + registry (persist) trio (AD-1). The IPC layer calls
//! these functions; it never touches the registry or policy directly.
//!
//! Idempotency model (Design Notes — "lifecycle 模型"):
//! - `confirm_source` = "ensure Confirmed". If the fingerprint matches an
//!   existing row, that row is flipped to `confirmed` (wake-up of a
//!   previously rejected / disabled Source — no separate re-enable command).
//!   Otherwise a new row is inserted with `lifecycle_state = confirmed`.
//! - `reject_source` = "ensure Rejected" (symmetric idempotency).
//! - `disable_source` (by `source_id`) = "ensure Disabled".
//!
//! Coverage single-source-of-truth (Design Notes — "coverage 单一事实源"):
//! the stored `coverage_level` on confirm is taken from
//! [`adapter_for`](provider)'s declaration, NOT from the candidate payload.
//! This partially mitigates the 1.2-deferred "candidate.coverage vs trait"
//! double-source risk by treating the adapter as authoritative at confirm
//! time.

use std::path::Path;

use crate::adapters::claude_code::ClaudeCodeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::adapters::opencode::OpenCodeAdapter;
use crate::domain::ports::provider_adapter::{CandidateSource, ProviderAdapter};
use crate::domain::source::{
    build_fingerprint, HealthCause, HealthState, Source, SourceId, SourceKind, SourceLifecycle,
    ROOT_KIND_DIR,
};
use crate::index::source_registry::{SourceInsert, SourceRegistry};
use crate::policy;

/// The error raised by the application confirm/reject/disable orchestration.
///
/// The IPC layer maps each variant onto a stable [`crate::ipc::envelope`]
/// code; this type keeps application concerns in the application layer (no
/// dependency from application → ipc).
#[derive(Debug)]
pub enum SourceError {
    /// Confirm failed because canonicalization or root validation failed
    /// (root missing / not a directory / not absolute — NFR-5/6). Maps to
    /// stable code `confirm_failed`.
    ConfirmFailed,
    /// A `source_id`-keyed operation targeted an id that does not match any
    /// row. Maps to stable code `source_not_found`.
    SourceNotFound,
    /// Confirm was blocked because the candidate root overlaps an already
    /// Confirmed Source root (Story 6.3 — contains / is-contained-by / equal).
    /// The caller must resolve ownership explicitly before re-confirming.
    /// Maps to stable code `root_overlap`.
    RootOverlap,
    /// An unexpected internal error from the registry (SQLite failure). Maps
    /// to the existing stable code `internal`.
    Internal,
}

/// Map a registry/transaction error (SQLite failure) to
/// [`SourceError::Internal`]. Required by
/// [`SourceRegistry::with_transaction`]'s `E: From<rusqlite::Error>` bound so
/// the transaction's begin/commit/rollback failures surface through the same
/// `Internal` mapping the rest of the application layer uses for DB errors.
impl From<rusqlite::Error> for SourceError {
    fn from(_: rusqlite::Error) -> Self {
        SourceError::Internal
    }
}

/// Resolve the provider adapter for a provider id. Story 2.1 widens this from
/// the mono-Codex match to a multi-provider registry so Claude Code candidates
/// route through the same confirm pipeline. Returns `None` for unknown
/// providers so confirm fails with `ConfirmFailed` rather than crashing.
///
/// This is the **single source of truth** for the provider→adapter registry.
/// `application::scan::adapter_for_scan` delegates here so scan dispatch and
/// confirm/dispatch can never drift (a provider added here is automatically
/// scannable; a provider missing here is rejected by both paths identically).
///
/// The Design Notes prefer `Option<Box<dyn ProviderAdapter>>` (matches the
/// architecture's "adapter registry" language"); each call returns a freshly
/// boxed unit struct, but the unit structs are zero-sized so the heap cost is
/// negligible. The boxed trait object preserves the existing confirm/reject
/// dispatch shape — only the registry width changes.
///
/// Story 4.1: exposed as `pub` (was `pub(crate)`) so integration tests can
/// assert that reconcile dispatches through the same registry as scan (no
/// drift between the two mutation paths). Production code outside this crate
/// has no business calling this directly; the surface is for tests.
pub fn adapter_for(provider: &str) -> Option<Box<dyn ProviderAdapter>> {
    match provider {
        // Reference Codex's canonical provider-id constant (single source of
        // truth) so a rename cannot desync the registry from the scan guard,
        // which uses the same constant. See `CodexAdapter::PROVIDER_ID`.
        CodexAdapter::PROVIDER_ID => Some(Box::new(CodexAdapter)),
        ClaudeCodeAdapter::PROVIDER_ID => Some(Box::new(ClaudeCodeAdapter)),
        OpenCodeAdapter::PROVIDER_ID => Some(Box::new(OpenCodeAdapter)),
        _ => None,
    }
}

/// Discover local Candidate Sources (Story 1.2; moved here from
/// `application/mod.rs` per the spec's "application 内联"兑现 Note).
///
/// Stateless orchestrator over the registered provider adapters. Story 2.1
/// widens this from Codex-only to the union of every registered adapter's
/// `discover()`, so Codex and Claude Code candidates surface alongside each
/// other. An empty result is NOT an error and means "no supported source on
/// this machine right now".
pub fn discover_sources() -> Vec<CandidateSource> {
    let mut all = Vec::new();
    all.extend(CodexAdapter.discover());
    all.extend(ClaudeCodeAdapter.discover());
    all.extend(OpenCodeAdapter.discover());
    // Deterministic cross-provider ordering: stable by `(provider, root_path)`
    // so UI lists do not flicker between boots. Within a provider, each
    // adapter already emits sorted candidates.
    all.sort_by(|a, b| {
        (a.provider.as_str(), a.root_path.as_str()).cmp(&(b.provider.as_str(), b.root_path.as_str()))
    });
    all
}

/// Confirm a Candidate Source (AD-4 "allowlist 入边界" action).
///
/// Steps (AD-33/AD-35):
/// 1. Canonicalize the candidate root via [`policy::canonicalize_root`]
///    (NFR-5/6: fails with [`SourceError::ConfirmFailed`] if the root
///    vanished / is not a directory / is not absolute).
/// 2. Build the versioned fingerprint from
///    `(provider, root_kind="dir", normalized_path, identity)`.
/// 3. Coverage single-source-of-truth: take `coverage_level` from the adapter
///    for this provider, NOT from the candidate payload (Design Notes).
/// 4. Look up the fingerprint. If found, flip lifecycle to `confirmed`
///    (idempotent wake-up — same `source_id`). Otherwise insert a new row.
pub fn confirm_source(
    registry: &SourceRegistry<'_>,
    candidate: &CandidateSource,
) -> Result<Source, SourceError> {
    let adapter = adapter_for(&candidate.provider).ok_or(SourceError::ConfirmFailed)?;

    // Step 1: canonicalize (NFR-5/6). Root must still exist and be a directory.
    let root = policy::canonicalize_root(std::path::Path::new(&candidate.root_path))
        .map_err(|_| SourceError::ConfirmFailed)?;
    let normalized_str = root
        .normalized_path
        .to_str()
        .ok_or(SourceError::ConfirmFailed)?;

    // Step 2: build fingerprint.
    let fingerprint = build_fingerprint(
        &candidate.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );

    // Step 3: coverage from the adapter, not the payload (single source of
    // truth).
    let coverage = adapter.coverage_level();

    // Step 4: idempotent upsert by fingerprint.
    let existing = registry
        .find_by_fingerprint(&fingerprint)
        .map_err(|_| SourceError::Internal)?;
    if let Some(existing) = existing {
        // Wake-up path: flip to confirmed (no-op if already confirmed).
        return flip_lifecycle(registry, &existing.source_id, SourceLifecycle::Confirmed);
    }

    let inserted = registry
        .upsert_by_fingerprint(&SourceInsert {
            provider: &candidate.provider,
            source_kind: SourceKind::AgentMemory,
            lifecycle_state: SourceLifecycle::Confirmed,
            health_state: HealthState::Unknown,
            coverage_level: coverage,
            normalized_root_path: normalized_str,
            fingerprint: &fingerprint,
            native_project: candidate.native_project.as_deref(),
            health_cause: HealthCause::None,
        })
        .map_err(|_| SourceError::Internal)?;
    Ok(inserted)
}

/// Reject a Candidate Source. Symmetric idempotency: if the fingerprint
/// matches an existing row, flip it to `rejected`; otherwise insert a new
/// `rejected` row so the decision persists across restarts.
pub fn reject_source(
    registry: &SourceRegistry<'_>,
    candidate: &CandidateSource,
) -> Result<Source, SourceError> {
    // Reject also canonicalizes + fingerprints so the rejection is keyed by
    // the same identity a future confirm would use (idempotent wake-up).
    let root = policy::canonicalize_root(std::path::Path::new(&candidate.root_path))
        .map_err(|_| SourceError::ConfirmFailed)?;
    let fingerprint = build_fingerprint(
        &candidate.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );

    if let Some(existing) = registry
        .find_by_fingerprint(&fingerprint)
        .map_err(|_| SourceError::Internal)?
    {
        return flip_lifecycle(registry, &existing.source_id, SourceLifecycle::Rejected);
    }

    // No existing row: persist a rejected row. Coverage still comes from the
    // adapter (single source of truth) even on reject, so a future confirm
    // wake-up sees the right coverage without re-reading the payload.
    let coverage = adapter_for(&candidate.provider)
        .map(|a| a.coverage_level())
        .unwrap_or(candidate.coverage_level);
    let normalized_str = root
        .normalized_path
        .to_str()
        .ok_or(SourceError::ConfirmFailed)?;
    let inserted = registry
        .upsert_by_fingerprint(&SourceInsert {
            provider: &candidate.provider,
            source_kind: SourceKind::AgentMemory,
            lifecycle_state: SourceLifecycle::Rejected,
            health_state: HealthState::Unknown,
            coverage_level: coverage,
            normalized_root_path: normalized_str,
            fingerprint: &fingerprint,
            native_project: candidate.native_project.as_deref(),
            health_cause: HealthCause::None,
        })
        .map_err(|_| SourceError::Internal)?;
    Ok(inserted)
}

/// Disable a confirmed Source by its `source_id` (AD-4: disable / list only
/// accept `source_id`, never an arbitrary path). Returns
/// [`SourceError::SourceNotFound`] when the id matches no row.
pub fn disable_source(
    registry: &SourceRegistry<'_>,
    source_id: &SourceId,
) -> Result<Source, SourceError> {
    flip_lifecycle(registry, source_id, SourceLifecycle::Disabled)
}

/// Story 4.3 — re-derive the provider-native project id from a root path.
/// This mirrors the project-key derivation [`ClaudeCodeAdapter::discover`]
/// performs, so rebind does not duplicate adapter parsing.
///
/// For Codex (global store) this is `None` — same value Codex's `discover()`
/// emits.
///
/// For Claude Code, the adapter emits TWO candidate shapes from the same
/// `discover()` call (see `adapters/claude_code.rs`):
/// - **Project-keyed shape** (`project_memory_dirs`): root path is
///   `<config>/projects/<project>/memory`, and the encoded `<project>` key is
///   the LEXICAL `entry.file_name()` of the `<project>` directory (NOT the
///   canonicalized target's name — so a symlinked project dir keeps the
///   symlink's name, not the target's).
/// - **`autoMemoryDirectory` shape** (`auto_memory_candidate`): root is
///   whatever absolute path the user configured, and `native_project = None`
///   regardless of what its parent directory happens to be named.
///
/// Re-derivation must be faithful to BOTH emission paths. The previous
/// implementation returned `Some(parent_dir_name)` for every Claude root,
/// silently mutating `native_project` from `None` to `Some(<garbage parent>)`
/// for an autoMemoryDirectory-shaped source — exactly the corruption F7 was
/// filed to prevent. The fix: only return `Some(project_key)` when the path
/// matches the project-keyed shape `<…>/projects/<project>/memory` (its
/// `file_name()` is `memory` AND its grandparent's name is `projects`); in
/// all other shapes return `None`, matching `auto_memory_candidate`'s
/// emission.
///
/// `lexical_root_path` is the USER-SUPPLIED (pre-canonicalize) path string
/// the adapter would have seen at discover time. We extract the project key
/// from it (NOT from the canonicalized path) so a symlinked project dir
/// yields the SAME project_key at rebind that the adapter emitted at confirm
/// — the adapter's `entry.file_name()` reads the lexical name. P2: this
/// closes the symlink divergence between adapter and rebind.
///
/// Re-derivation (NOT copying the old row's `native_project`) is the spec's
/// binding Always rule: copying the OLD project id to a DIFFERENT physical
/// root would mis-identify the new Source as belonging to a project it does
/// not belong to, corrupting any future Epic-5 mapping keyed off
/// `native_project`.
pub fn native_project_for_root(provider: &str, lexical_root_path: &str) -> Option<String> {
    if provider != ClaudeCodeAdapter::PROVIDER_ID {
        return None;
    }
    // Claude Code project-keyed shape: `<…>/projects/<project>/memory`. We
    // accept ONLY this exact trailing shape; an autoMemoryDirectory root
    // (any other absolute path) falls through and returns None, matching
    // `auto_memory_candidate`'s `native_project: None` emission. Operate on
    // the user-supplied lexical path so symlinked project dirs keep their
    // symlink's name (the adapter's `entry.file_name()` is lexical, too).
    let path = Path::new(lexical_root_path);
    if path.file_name().and_then(|n| n.to_str()) != Some("memory") {
        return None;
    }
    let project_dir = path.parent()?;
    let project_key = project_dir.file_name()?.to_str()?.to_string();
    let projects_dir = project_dir.parent()?;
    if projects_dir.file_name().and_then(|n| n.to_str()) != Some("projects") {
        return None;
    }
    Some(project_key)
}

fn native_project_for_rebind<F>(
    provider: &str,
    lexical_root_path: &str,
    opencode_identity_resolver: &F,
) -> Result<Option<String>, SourceError>
where
    F: Fn(&Path) -> Option<Option<String>>,
{
    if provider == OpenCodeAdapter::PROVIDER_ID {
        return opencode_identity_resolver(Path::new(lexical_root_path))
            .ok_or(SourceError::ConfirmFailed);
    }
    Ok(native_project_for_root(provider, lexical_root_path))
}

/// Story 4.3 — rebind a Confirmed Source whose root moved / lost permissions /
/// changed filesystem identity to a NEW root path. The explicit recovery path
/// for the "path/permission/identity change" AC: 4.2 marks the old Source
/// `Degraded + cause + last-success + stale` and preserves the previous
/// generation; rebind is the user-supplied action that points at the new
/// location.
///
/// Steps (spec Boundaries — "fail-closed on the new root BEFORE disabling the
/// old, AND why disable+insert must be ONE transaction"):
/// 1. Look up the old Source. Fail-closed: `SourceNotFound` if the id matches
///    no row; `ConfirmFailed` if the old row's lifecycle is not Confirmed
///    (rebind requires a confirmed-or-degraded old source — the I/O matrix
///    rejects Rejected/Disabled old rows with 409).
/// 2. Canonicalize + fingerprint the new root FIRST (fail-closed BEFORE
///    touching the old row): `ConfirmFailed` if the new root is missing /
///    not-a-dir / not-absolute, leaving the old Source UNCHANGED.
/// 3. No-op short-circuit: if the new fingerprint equals the old Source's
///    fingerprint, the move is a no-op. Leave the old row `Confirmed`, return
///    it, no new row.
/// 4. Disable-old + insert-or-wake-new INSIDE ONE SQLite transaction
///    ([`SourceRegistry::with_transaction`]). On a fingerprint collision with
///    an existing row, the wake-up branch sets that row to `Confirmed` AND
///    resets its `health_state`/`health_cause` to `Unknown`/`None` (a
///    resurrected previously-degraded row must surface as freshly-confirmed,
///    not stale-degraded — spec I/O matrix row 2). On any error between the
///    disable and the insert/wake, the transaction rolls the disable back so
///    the old row returns to its prior state (no catastrophic window).
/// 5. `native_project` for the new Source is RE-DERIVED from the new root
///    ([`native_project_for_root`]) — never copied from the old row.
///
/// Coverage for the new Source comes from the adapter (single source of
/// truth), matching `confirm_source`.
pub fn rebind_source(
    registry: &SourceRegistry<'_>,
    old_source_id: &SourceId,
    new_root_path: &str,
) -> Result<Source, SourceError> {
    rebind_source_with_opencode_identity_resolver(registry, old_source_id, new_root_path, |root| {
        OpenCodeAdapter.native_project_for_current_root(root)
    })
}

/// Test seam for metadata-backed OpenCode identity resolution.
///
/// Production calls [`rebind_source`], whose resolver always reads the
/// current OpenCode environment. Integration tests inject the exact
/// missing/ambiguous/current metadata result without mutating process-global
/// environment variables.
#[doc(hidden)]
pub fn rebind_source_with_opencode_identity_resolver<F>(
    registry: &SourceRegistry<'_>,
    old_source_id: &SourceId,
    new_root_path: &str,
    opencode_identity_resolver: F,
) -> Result<Source, SourceError>
where
    F: Fn(&Path) -> Option<Option<String>>,
{
    // Step 1: load + validate the old source's state. Fail-closed BEFORE
    // canonicalizing the new root: an unknown id or a bad old lifecycle yields
    // no state change.
    let old = registry
        .get(old_source_id)
        .map_err(|_| SourceError::Internal)?
        .ok_or(SourceError::SourceNotFound)?;
    if !matches!(old.lifecycle_state, SourceLifecycle::Confirmed) {
        // Rebind requires a Confirmed old source. A Degraded row is still
        // Confirmed (degraded is a HealthState, not a Lifecycle), so a
        // 4.2-marked Degraded+PathMissing row satisfies this. A Rejected or
        // already-Disabled row does not.
        return Err(SourceError::ConfirmFailed);
    }

    // Step 2: canonicalize + fingerprint the new root FIRST (fail-closed).
    let root = policy::canonicalize_root(std::path::Path::new(new_root_path))
        .map_err(|_| SourceError::ConfirmFailed)?;
    let normalized_str = root
        .normalized_path
        .to_str()
        .ok_or(SourceError::ConfirmFailed)?;
    let new_fingerprint = build_fingerprint(
        &old.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );

    // OpenCode identity is metadata-backed rather than path-encoded. Resolve
    // it before the no-op branch so missing/ambiguous current metadata cannot
    // silently preserve a stale project id on an unchanged filesystem root.
    let opencode_native_project = if old.provider == OpenCodeAdapter::PROVIDER_ID {
        Some(native_project_for_rebind(
            &old.provider,
            new_root_path,
            &opencode_identity_resolver,
        )?)
    } else {
        None
    };

    // Step 3: no-op short-circuit (same fingerprint → the "move" didn't
    // change identity). Leave the old row Confirmed, no new row.
    if old.fingerprint == new_fingerprint {
        if opencode_native_project
            .as_ref()
            .is_some_and(|identity| identity != &old.native_project)
        {
            return Err(SourceError::ConfirmFailed);
        }
        return Ok(old);
    }

    // Coverage + native_project are derived from the NEW root + provider (the
    // provider IS carried from old — same-provider-by-construction per the
    // spec's "KEEP" note on the F7 amendment; only the project id is
    // re-derived). `native_project` re-derivation uses the USER-SUPPLIED
    // (lexical) path string — NOT the canonicalized path — so a symlinked
    // project dir yields the SAME project_key at rebind that the adapter's
    // `entry.file_name()` emitted at confirm (P2: closes the adapter/rebind
    // symlink divergence).
    let adapter = adapter_for(&old.provider).ok_or(SourceError::ConfirmFailed)?;
    let coverage = adapter.coverage_level();
    let native_project = match opencode_native_project {
        Some(identity) => identity,
        None => {
            native_project_for_rebind(&old.provider, new_root_path, &opencode_identity_resolver)?
        }
    };
    let native_project_ref: Option<&str> = native_project.as_deref();

    // Capture fields the closure needs before moving into the transaction
    // (the closure borrows `registry` via the with_transaction API; these
    // owned values are captured by move).
    let provider = old.provider.clone();

    // Step 4: disable-old + insert-or-wake-new INSIDE ONE transaction. On any
    // error between the two writes, the transaction rolls the disable back so
    // the old row returns to Confirmed (the catastrophic state the fail-closed
    // design exists to prevent).
    registry.with_transaction(|tx| -> Result<Source, SourceError> {
        // Disable the old row.
        if tx
            .set_lifecycle(old_source_id, SourceLifecycle::Disabled)
            .map_err(|_| SourceError::Internal)?
            .is_none()
        {
            return Err(SourceError::SourceNotFound);
        }

        // Insert-or-wake the new row at the new fingerprint.
        if let Some(existing) = tx
            .find_by_fingerprint(&new_fingerprint)
            .map_err(|_| SourceError::Internal)?
        {
            // Wake-up branch: the new fingerprint already matches an
            // existing row. Wake it to Confirmed AND reset its
            // health_state/health_cause so a previously-degraded row
            // surfaces as freshly-confirmed, NOT stale-degraded (spec I/O
            // matrix row 2 + Design Notes "Why wake-up resets
            // health/cause").
            tx.set_lifecycle(&existing.source_id, SourceLifecycle::Confirmed)
                .map_err(|_| SourceError::Internal)?
                .ok_or(SourceError::Internal)?;
            tx.set_health_and_cause(&existing.source_id, HealthState::Unknown, HealthCause::None)
                .map_err(|_| SourceError::Internal)?
                .ok_or(SourceError::Internal)
        } else {
            // Insert branch: no row at the new fingerprint yet. Insert a
            // fresh Confirmed row. `native_project` was re-derived from the
            // new root (NOT copied from the old row) — see
            // [`native_project_for_root`].
            tx.upsert_by_fingerprint(&SourceInsert {
                provider: &provider,
                source_kind: SourceKind::AgentMemory,
                lifecycle_state: SourceLifecycle::Confirmed,
                health_state: HealthState::Unknown,
                coverage_level: coverage,
                normalized_root_path: normalized_str,
                fingerprint: &new_fingerprint,
                native_project: native_project_ref,
                health_cause: HealthCause::None,
            })
            .map_err(|_| SourceError::Internal)
        }
    })
}

/// List every registered Source (any lifecycle), ordered by id. Infallible at
/// the application layer; the IPC layer surfaces any registry error as
/// `internal`.
pub fn list_sources(registry: &SourceRegistry<'_>) -> Result<Vec<Source>, SourceError> {
    registry.list().map_err(|_| SourceError::Internal)
}

/// Helper: flip a Source's lifecycle, mapping "no row matched" to
/// [`SourceError::SourceNotFound`] and DB errors to [`SourceError::Internal`].
fn flip_lifecycle(
    registry: &SourceRegistry<'_>,
    source_id: &SourceId,
    target: SourceLifecycle,
) -> Result<Source, SourceError> {
    match registry.set_lifecycle(source_id, target) {
        Ok(Some(updated)) => Ok(updated),
        Ok(None) => Err(SourceError::SourceNotFound),
        Err(_) => Err(SourceError::Internal),
    }
}

/// Discover local Obsidian Vault Candidates (Story 6.2 / Phase C.0).
///
/// This is the Knowledge-pipeline counterpart to [`discover_sources`], but it
/// deliberately does NOT route through [`adapter_for`] / `ProviderAdapter`
/// (Story 6.1 AC: "source-kind dispatch cannot route Knowledge through
/// `ProviderAdapter`"). Knowledge Sources use an independent canonical table,
/// identity prefix, and parser version (AD-19/AD-38); discovery is the one
/// surface they safely share with Agent Memory because [`CandidateSource`] is
/// generic pre-confirmation metadata, not an Agent-Memory canonical model.
///
/// Returns candidates plus an optional diagnostic when the Obsidian registry
/// was missing/corrupt/unreadable (AD-37). A registry problem never blocks
/// Agent Memory — this function is called from a separate discovery path and
/// any error is source-scoped (AD-13).
pub fn discover_obsidian_vaults() -> crate::adapters::obsidian::DiscoveryResult {
    crate::adapters::obsidian::discover()
}

// ---------------------------------------------------------------------------
// Story 6.3 — Knowledge (Obsidian Vault) confirm / reject / disable + overlap
// ---------------------------------------------------------------------------

/// The outcome of a Rust-owned native folder-picker request (Story 6.3). The
/// browser invokes the action and receives one of these; it never submits a
/// path, URI, or filesystem handle (AD-37).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum VaultPickerOutcome {
    /// The user selected an existing directory. Rust has validated it is a
    /// directory but has NOT yet confirmed it is an Obsidian Vault (no
    /// `.obsidian` check is required for confirmation — the registry/fallback
    /// path is the source of truth). The candidate is returned for the UI to
    /// show a Candidate card; it is NOT auto-confirmed.
    Selected(CandidateSource),
    /// The user cancelled the native dialog. The UI restores focus and
    /// persists nothing.
    Cancelled,
    /// The selected directory is unreadable or outside the policy boundary.
    /// The error is safe (no path leakage beyond the already user-visible
    /// selection).
    Invalid,
}

/// Request the native OS folder picker for an existing Obsidian Vault
/// (Story 6.3 AC). Rust-owned: the browser only triggers the action and
/// receives a [`VaultPickerOutcome`]; it never supplies a path. The selected
/// path is canonicalized and validated as an existing directory before being
/// returned as a Candidate. No Source is persisted here — confirmation is a
/// separate explicit step.
pub fn request_existing_vault_picker() -> VaultPickerOutcome {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Select an existing Obsidian Vault")
        .pick_folder()
    else {
        return VaultPickerOutcome::Cancelled;
    };
    // Canonicalize + validate as an existing directory (NFR-5/6). A selection
    // that vanished between pick and validate, or is not a directory, is
    // Invalid — cancellation is a distinct, user-intentional outcome.
    let root = match policy::canonicalize_root(&path) {
        Ok(r) => r,
        Err(_) => return VaultPickerOutcome::Invalid,
    };
    let Some(path_str) = root.normalized_path.to_str() else {
        return VaultPickerOutcome::Invalid;
    };
    VaultPickerOutcome::Selected(CandidateSource {
        provider: crate::adapters::obsidian::PROVIDER_ID.to_string(),
        root_path: path_str.to_string(),
        basis: crate::domain::ports::provider_adapter::DiscoveryBasis::ObsidianVaultRegistry,
        coverage_level: crate::domain::ports::provider_adapter::CoverageLevel::Full,
        native_project: None,
    })
}

/// Confirm an Obsidian Vault Candidate as a `local_knowledge` Source
/// (Story 6.3). Mirrors [`confirm_source`]'s canonicalize → fingerprint →
/// idempotent-upsert shape, but:
/// - persists `source_kind = LocalKnowledge` (never `AgentMemory`);
/// - does NOT route through [`adapter_for`] (Story 6.1 AC);
/// - blocks when the candidate root overlaps an already-Confirmed Source root
///   (Story 6.3 AC) until the user resolves ownership.
///
/// Overlap is detected against EVERY Confirmed Source (Knowledge and
/// Agent-Memory alike), because two confirmed roots must never own overlapping
/// filesystem trees (AD-4 read-boundary uniqueness).
pub fn confirm_knowledge_source(
    registry: &SourceRegistry<'_>,
    candidate: &CandidateSource,
) -> Result<Source, SourceError> {
    // Only Obsidian/Knowledge candidates may be confirmed through this path.
    if candidate.provider != crate::adapters::obsidian::PROVIDER_ID {
        return Err(SourceError::ConfirmFailed);
    }

    // Step 1: canonicalize (NFR-5/6).
    let root = policy::canonicalize_root(std::path::Path::new(&candidate.root_path))
        .map_err(|_| SourceError::ConfirmFailed)?;
    let normalized_str = root
        .normalized_path
        .to_str()
        .ok_or(SourceError::ConfirmFailed)?;

    // Step 2: build fingerprint.
    let fingerprint = build_fingerprint(
        &candidate.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );

    // Step 3: idempotent wake-up — if this exact fingerprint is already a row,
    // flip it to confirmed (no overlap check needed; it IS the owner).
    if let Some(existing) = registry
        .find_by_fingerprint(&fingerprint)
        .map_err(|_| SourceError::Internal)?
    {
        return flip_lifecycle(registry, &existing.source_id, SourceLifecycle::Confirmed);
    }

    // Step 4: overlap guard. Block if the normalized root contains, is
    // contained by, or equals any OTHER Confirmed Source's root.
    let confirmed = registry.list().map_err(|_| SourceError::Internal)?;
    for other in &confirmed {
        if other.lifecycle_state != SourceLifecycle::Confirmed {
            continue;
        }
        if roots_overlap(&root.normalized_path, &other.normalized_root_path) {
            return Err(SourceError::RootOverlap);
        }
    }

    // Step 5: insert new local_knowledge Source.
    let inserted = registry
        .upsert_by_fingerprint(&SourceInsert {
            provider: &candidate.provider,
            source_kind: SourceKind::LocalKnowledge,
            lifecycle_state: SourceLifecycle::Confirmed,
            health_state: HealthState::Unknown,
            coverage_level: candidate.coverage_level,
            normalized_root_path: normalized_str,
            fingerprint: &fingerprint,
            native_project: None,
            health_cause: HealthCause::None,
        })
        .map_err(|_| SourceError::Internal)?;
    Ok(inserted)
}

/// Reject a Knowledge Vault Candidate. Symmetric idempotency with
/// [`reject_source`]: persists a `rejected` row keyed by fingerprint so the
/// decision survives restart. Never routes through `ProviderAdapter`.
pub fn reject_knowledge_source(
    registry: &SourceRegistry<'_>,
    candidate: &CandidateSource,
) -> Result<Source, SourceError> {
    if candidate.provider != crate::adapters::obsidian::PROVIDER_ID {
        return Err(SourceError::ConfirmFailed);
    }
    let root = policy::canonicalize_root(std::path::Path::new(&candidate.root_path))
        .map_err(|_| SourceError::ConfirmFailed)?;
    let fingerprint = build_fingerprint(
        &candidate.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );
    if let Some(existing) = registry
        .find_by_fingerprint(&fingerprint)
        .map_err(|_| SourceError::Internal)?
    {
        return flip_lifecycle(registry, &existing.source_id, SourceLifecycle::Rejected);
    }
    let normalized_str = root
        .normalized_path
        .to_str()
        .ok_or(SourceError::ConfirmFailed)?;
    let inserted = registry
        .upsert_by_fingerprint(&SourceInsert {
            provider: &candidate.provider,
            source_kind: SourceKind::LocalKnowledge,
            lifecycle_state: SourceLifecycle::Rejected,
            health_state: HealthState::Unknown,
            coverage_level: candidate.coverage_level,
            normalized_root_path: normalized_str,
            fingerprint: &fingerprint,
            native_project: None,
            health_cause: HealthCause::None,
        })
        .map_err(|_| SourceError::Internal)?;
    Ok(inserted)
}

/// Detect whether two normalized root paths overlap (one contains the other,
/// or they are equal). Story 6.3 AC: overlapping roots cannot both be
/// confirmed until ownership is resolved.
///
/// Uses component-wise comparison on the normalized paths so trailing-slash
/// differences do not cause false negatives. Pure function — testable without
/// a registry.
fn roots_overlap(a: &std::path::Path, b: &str) -> bool {
    let pa: Vec<_> = a.components().collect();
    let pb: Vec<_> = std::path::Path::new(b).components().collect();
    if pa == pb {
        return true;
    }
    // a contains b, or b contains a.
    contains_prefix(&pa, &pb) || contains_prefix(&pb, &pa)
}

/// True when `longer` starts with all components of `shorter` (shorter is an
/// ancestor of longer). Equal-length prefixes that differ return false.
fn contains_prefix(shorter: &[std::path::Component<'_>], longer: &[std::path::Component<'_>]) -> bool {
    longer.len() > shorter.len() && longer[..shorter.len()] == *shorter
}

#[cfg(test)]
mod tests {
    use super::*;

    /// adapter_for returns the Codex and Claude Code adapters for their known
    /// provider ids and None for an unknown one. This is the precondition for
    /// coverage being a single source of truth (Design Notes — "coverage 单一
    /// 事实源") and for the multi-provider confirm/dispatch path Story 2.1
    /// ships.
    #[test]
    fn adapter_for_returns_codex_for_known_provider() {
        let codex = adapter_for("codex").expect("codex registered");
        assert_eq!(codex.provider_id(), "codex");
        let claude = adapter_for("claude_code").expect("claude_code registered");
        assert_eq!(claude.provider_id(), "claude_code");
        let opencode = adapter_for("opencode").expect("opencode registered");
        assert_eq!(opencode.provider_id(), "opencode");
        assert!(adapter_for("unknown").is_none());
    }

    #[test]
    fn confirm_failed_error_is_constructible_and_debug() {
        // Compile-check that SourceError variants exist and are Debug, so the
        // IPC mapping can name them without surprises.
        let e = SourceError::ConfirmFailed;
        let _ = format!("{e:?}");
        let e = SourceError::SourceNotFound;
        let _ = format!("{e:?}");
        let e = SourceError::Internal;
        let _ = format!("{e:?}");
    }

    /// Story 2.1 review fix — `CodexAdapter::PROVIDER_ID` is the canonical
    /// provider id used in BOTH the registry match arm (`adapter_for`) and the
    /// scan guard (`application::scan`). Pin that the constant equals the
    /// trait's `provider_id()` so a rename cannot desync the two surfaces.
    #[test]
    fn codex_provider_id_constant_matches_trait_provider_id() {
        assert_eq!(CodexAdapter::PROVIDER_ID, CodexAdapter.provider_id());
        // The match arm in `adapter_for` references the same constant; verify
        // it routes the canonical id to the Codex adapter.
        let adapter = adapter_for(CodexAdapter::PROVIDER_ID).expect("codex registered");
        assert_eq!(adapter.provider_id(), CodexAdapter::PROVIDER_ID);
    }
}
