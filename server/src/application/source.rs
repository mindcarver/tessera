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

use crate::adapters::claude_code::ClaudeCodeAdapter;
use crate::adapters::codex::CodexAdapter;
use crate::domain::ports::provider_adapter::{CandidateSource, ProviderAdapter};
use crate::domain::source::{
    build_fingerprint, HealthState, Source, SourceId, SourceKind, SourceLifecycle, ROOT_KIND_DIR,
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
    /// An unexpected internal error from the registry (SQLite failure). Maps
    /// to the existing stable code `internal`.
    Internal,
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
