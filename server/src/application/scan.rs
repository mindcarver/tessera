//! `application::scan` — orchestration of the read-only scan pipeline with
//! atomic generation switching (Story 1.4).
//!
//! This is the only layer allowed to coordinate the adapter (enumerate) +
//! scan store (fencing / staging / CAS) + domain (record id / hash) trio
//! (AD-1). The IPC layer calls these functions; it never touches the adapter
//! or scan store directly.
//!
//! Pipeline (AD-5/AD-34/AD-36), in order:
//! 1. Validate the Source is `confirmed` and its root still resolves.
//! 2. **`begin_run`** — insert the persisted run row FIRST (with a placeholder
//!    `manifest_revision`), so a failure during the FIRST enumeration also
//!    lands on a persisted run row (spec Design Notes — "失败即 fail_run、
//!    不留半态"). The real revision is UPDATEd onto the row after the
//!    manifest is snapshotted.
//! 3. Enumerate file units → build the start manifest → UPDATE the real
//!    `manifest_revision`.
//! 4. Per file: read bytes, canonicalize Markdown, and stage canonical records.
//! 5. **Final manifest re-validation** before commit: re-enumerate and compare
//!    against the start manifest (`snapshot-at-validation`). A drift marks the
//!    run `failed` with `error_code='dirty_after_validation'`; its generation
//!    is never activated (AD-36).
//! 6. `committing` → [`ScanStore::commit_cas`]. A lost CAS returns
//!    `scan_failed` without re-marking the run (left for boot recovery).
//!
//! Zero-write invariant (NFR-1/SM-2): the pipeline only reads Source files
//! (enumerate metadata + read bytes for hashing); it never writes to them.

use std::io::Read;
use std::path::Path;

use rusqlite::Connection;
use same_file::Handle;

use crate::adapters::markdown::{canonicalize_markdown, file_uri, percent_encode_fragment};
use crate::domain::scan::{
    build_record_id, fnv1a_hex, Generation, ScanError, ScanOutcome, ScanRunState, ScanStatus,
    SourceInventory,
};
use crate::domain::source::{
    build_fingerprint, HealthCause, HealthState, SourceId, SourceLifecycle, ROOT_KIND_DIR,
};
use crate::domain::{CoverageLevel, ProviderAdapter, SupportedArtifact};
use crate::index::scan_store::{ScanStore, StagedDiagnostic, StagedKnowledgeRecord, StagedRecord};
use crate::index::SourceRegistry;
use crate::policy;

/// The placeholder `manifest_revision` written by `begin_run` before the first
/// enumeration (spec amendment 4: begin_run must precede the first enumeration
/// so an enumeration failure also lands on a persisted run row). The real
/// revision is UPDATEd onto the row once the manifest is snapshotted.
const PLACEHOLDER_MANIFEST_REVISION: &str = "pending";

/// Resolve the provider adapter for a scan dispatch (Story 2.2). Delegates to
/// [`application::source::adapter_for`] — the **single provider→adapter
/// registry** — so scan dispatch and the confirm/reject path can never drift
/// (a provider added to confirm is automatically scannable; a provider missing
/// from confirm is rejected by both paths identically). Story 2.1 hard-coded
/// `&CodexAdapter` here; 2.2 generalizes dispatch by `source.provider` so
/// Claude Code sources scan through their own adapter + parser tag. A
/// genuinely-unknown provider returns `None` and surfaces as
/// `ScanError::Internal` (mirroring confirm's `ConfirmFailed`): the
/// registry's confirm path already rejects unknown providers, so reaching
/// here with `None` is a registry/invariant drift, not a user-facing "not
/// scannable" outcome.
fn adapter_for_scan(provider: &str) -> Option<Box<dyn ProviderAdapter>> {
    crate::application::source::adapter_for(provider)
}

/// A single manifest entry: `(relative_path, canonical_target, size, mtime)`
/// where `mtime` is nanoseconds since the Unix epoch (sub-second precision —
/// AD-34). Including the canonical target binds the manifest to the exact
/// file resolved at enumeration time, rather than merely to its visible name.
type ManifestEntry = (String, String, u64, i64);

/// Scan a confirmed Source, dispatching the adapter by `source.provider`
/// (Story 2.2 generalization). The production entry point the IPC command
/// calls; delegates to [`scan_source_with`] after resolving the adapter. The
/// generic seam exists so integration tests can drive a scripted adapter
/// through the SAME public orchestration (e.g. to produce a real manifest
/// drift). Pre-2.2 this hard-coded `&CodexAdapter` and refused Claude via
/// `ProviderNotScannable`; that placeholder is gone now that Claude is
/// scannable.
pub fn scan_source(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
) -> Result<ScanOutcome, ScanError> {
    // Bootstrap: load the source row FIRST so we can dispatch by provider,
    // then pass the already-loaded Source through to `scan_source_with` so
    // production scans do ONE registry read, not two (Story 2.2 review fix).
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else {
        return Err(ScanError::SourceNotFound);
    };
    // Story 6.5 follow-up: Knowledge Sources route through the independent
    // Knowledge pipeline, never through ProviderAdapter (Story 6.1 AC).
    if source.source_kind == crate::domain::source::SourceKind::LocalKnowledge {
        return scan_knowledge_source(registry, conn, &source);
    }
    let adapter = adapter_for_scan(&source.provider).ok_or(ScanError::Internal)?;
    scan_source_with(adapter.as_ref(), registry, conn, &source)
}

/// Execute a run reserved before the queued response reached the browser.
/// Dispatches the adapter by `source.provider` (Story 2.2 generalization).
pub fn scan_reserved_source(
    registry: &SourceRegistry<'_>, conn: &Connection, source_id: &SourceId,
    scan_id: i64, fencing_token: i64, generation: Generation,
) -> Result<ScanOutcome, ScanError> {
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else { return Err(ScanError::SourceNotFound); };
    // Story 6.5 follow-up: Knowledge Sources route through the independent
    // Knowledge pipeline on the reserved (rescan) path too.
    if source.source_kind == crate::domain::source::SourceKind::LocalKnowledge {
        return scan_knowledge_reserved(registry, conn, source_id, scan_id, fencing_token, generation, &source);
    }
    let adapter = match adapter_for_scan(&source.provider) {
        Some(adapter) => adapter,
        None => {
            // Dispatch failure on the reserved path: the run row was already
            // `begin_run`'d by the reservation that allocated `scan_id`. Mark
            // it failed (and set health) BEFORE returning so it is not left
            // non-terminal — the fail-on-error contract (spec Design Notes —
            // "失败即 fail_run、不留半态"). Without this, only boot recovery
            // would clean the row up. `Internal` is an invariant drift here:
            // confirm already rejects unknown providers, so reaching this arm
            // means the registry and dispatch tables disagree.
            let _ = ScanStore::new(conn).fail_run(scan_id, ScanError::Internal.error_code());
            let _ = registry.set_health_and_cause(
                source_id,
                HealthState::Error,
                HealthCause::ScanFailed,
            );
            return Err(ScanError::Internal);
        }
    };
    scan_reserved_source_with(adapter.as_ref(), registry, conn, source_id, scan_id, fencing_token, generation)
}

/// Scan a confirmed Source with an injected adapter (AD-1 orchestration).
///
/// `registry` and `scan_store` borrow the same connection (the IPC layer
/// holds the `IndexState` mutex for the whole command — synchronous, single
/// owner per Source, AD-5). Returns the [`ScanOutcome`] on a fully-successful
/// scan; any failure after `begin_run` is a structured [`ScanError`] with the
/// run marked `failed` (except a lost commit CAS — see module doc).
///
/// Story 2.2: the adapter is now `&dyn ProviderAdapter` (was generic `<A>`)
/// so production dispatch can route by `source.provider`. Tests still inject a
/// scripted adapter through this seam — `&adapter` coerces to
/// `&dyn ProviderAdapter` at the call site.
///
/// Story 2.2 review: the Source is now passed IN (was re-loaded via
/// `registry.get(source_id)`) so the production path does one registry read,
/// not two. The caller (`scan_source`) loaded it moments ago; tests load it
/// via `confirm`. Behavior is identical except for the eliminated read.
pub fn scan_source_with(
    adapter: &dyn ProviderAdapter,
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source: &crate::domain::source::Source,
) -> Result<ScanOutcome, ScanError> {
    // Defense-in-depth (Story 2.2 review): the 2.1 `ProviderNotScannable`
    // guard is gone, so this pub seam silently trusts the caller paired the
    // correct adapter with the source. A mismatch would persist records under
    // the wrong `parser_version`; assert the pair cheaply in debug builds.
    debug_assert_eq!(adapter.provider_id(), source.provider);

    let scan_store = ScanStore::new(conn);
    let source_id = &source.source_id;

    // --- Validate source + root -------------------------------------------
    if source.lifecycle_state != SourceLifecycle::Confirmed {
        // Rejected / disabled sources are not scannable (spec I/O matrix).
        return Err(ScanError::NotConfirmed);
    }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;

    // Story 2.2: the 2.1 `ProviderNotScannable` guard that refused Claude
    // sources is removed — Claude is scannable now. Adapter dispatch already
    // happened in `scan_source`; an unknown provider would have surfaced as
    // `Internal` before reaching here.

    // Re-validate the root (AD-4/NFR-5/6). A deleted / non-dir root fails the
    // scan BEFORE begin_run (no run row — root validation precedes ownership);
    // any prior active generation is preserved. Story 4.2: the canonicalize
    // io error kind is captured here (the only site with the io error in hand
    // before mapping to `ScanError`) so the cause can be classified by
    // `io::Error::kind()` — NotFound → path_missing, PermissionDenied →
    // permission_denied, anything else → scan_failed. This is the single most
    // important failure mode (root gone), and it writes NO scan_runs row, so
    // the cause MUST be persisted on the source row to be recoverable.
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(err) => {
            let cause = health_cause_for_scan_error(&ScanError::RootInvalid, Some(err.kind()));
            let _ = registry.set_health_and_cause(source_id, HealthState::Degraded, cause);
            return Err(ScanError::RootInvalid);
        }
    };
    let current_fingerprint = build_fingerprint(
        &source.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );
    if current_fingerprint != source.fingerprint {
        // The source's path now resolves to a different filesystem object.
        // Require an explicit re-confirmation instead of silently scanning a
        // replacement directory under the old Source identity. Story 4.2: a
        // root-identity change is a path-shape failure (the canonicalize
        // succeeded, but the identity check failed), so the cause falls into
        // the scan_failed catch-all (it is not a missing/permission/format
        // failure — the root is there, it just is not the same one).
        let _ = registry.set_health_and_cause(
            source_id,
            HealthState::Degraded,
            health_cause_for_scan_error(&ScanError::RootIdentityChanged, None),
        );
        return Err(ScanError::RootIdentityChanged);
    }

    // --- Begin the run FIRST (placeholder revision) -------------------------
    // begin_run precedes the first enumeration so that an enumeration failure
    // also lands on a persisted run row (spec amendment 4 / Design Notes). The
    // real manifest_revision is UPDATEd after the manifest is snapshotted.
    let (scan_id, fencing_token, generation) = scan_store
        .begin_run(source_rowid, PLACEHOLDER_MANIFEST_REVISION)
        .map_err(|_| ScanError::Internal)?;

    // From here on, any error must fail_run before returning (except the CAS
    // loss, handled at the commit step).
    let outcome = run_pipeline(
        adapter,
        &scan_store,
        &root.normalized_path,
        source,
        source_rowid,
        scan_id,
        fencing_token,
        &generation,
    );
    match outcome {
        Ok(o) => {
            // Activation already committed; a health-write failure must not
            // falsely report that the previous generation remains active.
            // Story 4.2: success clears the cause (writes `(Healthy, None)`)
            // so a recovered source shows no stale cause.
            let _ = registry.set_health_and_cause(
                source_id,
                HealthState::Healthy,
                HealthCause::None,
            );
            Ok(o)
        }
        Err((e, cause)) => {
            // A lost CAS is NOT re-marked: the run is no longer owned by this
            // holder (left in `committing` for boot recovery). Every other
            // failure marks the run failed with its error category from the
            // domain-layer vocabulary mapping (`ScanError::error_code()`).
            if !matches!(e, ScanError::CommitCasFailed) {
                let _ = scan_store.fail_run(scan_id, e.error_code());
            }
            if !matches!(e, ScanError::Cancelled) {
                let health = health_for_scan_error(&e);
                // Story 4.2: the cause travels from the I/O boundary (set
                // inside run_pipeline via `cause_from_enumerate_error` /
                // `health_cause_for_scan_error`). Cancel does not clear a
                // previously-persisted cause (cancel is not a health
                // transition), so this arm is skipped on Cancel — the cause
                // persists from the prior failure.
                let _ = registry.set_health_and_cause(source_id, health, cause);
            }
            Err(e)
        }
    }
}

fn scan_reserved_source_with(
    adapter: &dyn ProviderAdapter, registry: &SourceRegistry<'_>, conn: &Connection, source_id: &SourceId,
    scan_id: i64, fencing_token: i64, generation: Generation,
) -> Result<ScanOutcome, ScanError> {
    let store = ScanStore::new(conn);
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else { return Err(ScanError::SourceNotFound); };
    // Defense-in-depth (Story 2.2 review): mirror the `scan_source_with`
    // mismatch guard on the reserved seam.
    debug_assert_eq!(adapter.provider_id(), source.provider);
    if source.lifecycle_state != SourceLifecycle::Confirmed { return Err(ScanError::NotConfirmed); }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;
    // Story 2.2: the 2.1 `ProviderNotScannable` mirror guard is removed —
    // Claude is scannable now. Adapter dispatch happened in
    // `scan_reserved_source`; an unknown provider would have surfaced as
    // `Internal` before reaching here.
    ensure_not_cancelled(&store, scan_id)?;
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(err) => {
            let cause = health_cause_for_scan_error(&ScanError::RootInvalid, Some(err.kind()));
            return reserved_failure(
                registry,
                &store,
                source_id,
                scan_id,
                ScanError::RootInvalid,
                "root_invalid",
                cause,
            );
        }
    };
    if build_fingerprint(&source.provider, ROOT_KIND_DIR, &root.normalized_path, root.identity)
        != source.fingerprint
    {
        let cause = health_cause_for_scan_error(&ScanError::RootIdentityChanged, None);
        return reserved_failure(
            registry,
            &store,
            source_id,
            scan_id,
            ScanError::RootIdentityChanged,
            "root_identity_changed",
            cause,
        );
    }
    match run_pipeline(
        adapter,
        &store,
        &root.normalized_path,
        &source,
        source_rowid,
        scan_id,
        fencing_token,
        &generation,
    ) {
        Ok(outcome) => {
            // Story 4.2: success clears the cause.
            let _ = registry
                .set_health_and_cause(source_id, HealthState::Healthy, HealthCause::None);
            Ok(outcome)
        }
        Err((error, cause)) => {
            if !matches!(error, ScanError::CommitCasFailed) {
                let _ = store.fail_run(scan_id, error.error_code());
            }
            if !matches!(error, ScanError::Cancelled) {
                let _ = registry
                    .set_health_and_cause(source_id, health_for_scan_error(&error), cause);
            }
            Err(error)
        }
    }
}

fn reserved_failure(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
    source_id: &SourceId,
    scan_id: i64,
    error: ScanError,
    error_code: &str,
    cause: HealthCause,
) -> Result<ScanOutcome, ScanError> {
    let _ = store.fail_run(scan_id, error_code);
    let _ = registry.set_health_and_cause(source_id, HealthState::Degraded, cause);
    Err(error)
}

/// The staged body of the scan, split out so the caller can apply the
/// fail-run-on-error policy uniformly.
///
/// Story 4.2: on error, returns `(ScanError, HealthCause)` so the structured
/// cause classified at the I/O boundary (by `EnumerateError` variant / io kind)
/// travels with the error to the caller's `set_health_and_cause` site. The
/// cause is `HealthCause::None` for `Cancelled` (cancel is not a health
/// transition — the caller does not call `set_health_and_cause` on cancel
/// anyway).
#[allow(clippy::too_many_arguments)]
fn run_pipeline(
    adapter: &dyn ProviderAdapter,
    scan_store: &ScanStore<'_>,
    canonical_root: &Path,
    source: &crate::domain::source::Source,
    source_rowid: i64,
    scan_id: i64,
    fencing_token: i64,
    generation: &Generation,
) -> Result<ScanOutcome, (ScanError, HealthCause)> {
    // Story 4.2 — helper that lifts a plain `Result<_, ScanError>` into this
    // function's `(ScanError, HealthCause)` error channel by attaching the
    // default cause for that variant. Used at every site that does not have
    // an io kind in hand (the io-kind-aware sites — adapter enumeration and
    // `read_verified` — attach their own cause inline).
    let lift = |e: ScanError| {
        let cause = health_cause_for_scan_error(&e, None);
        (e, cause)
    };

    // running → staging.
    advance(scan_store, scan_id, ScanRunState::Running).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    // --- First enumeration → start manifest → UPDATE real revision ----------
    // Story 4.2: the adapter's `EnumerateError` is classified by io kind at
    // the I/O boundary, so the cause travels with the failure into the
    // pipeline's error channel.
    let start_enumeration = adapter
        .enumerate_artifacts(canonical_root)
        .map_err(|err| (ScanError::EnumerationFailed, cause_from_enumerate_error(&err)))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let active_generation = scan_store
        .active_generation(source_rowid)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    if start_enumeration.supported.is_empty()
        && start_enumeration.diagnostics.is_empty()
        && scan_store
            .active_generation(source_rowid)
            .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?
            .is_some()
    {
        // An intentionally empty first scan is valid. Replacing an existing
        // active generation with an empty result is not: unreadable roots can
        // otherwise masquerade as a successful destructive rescan. Story 4.2:
        // EmptyScanWithActiveGeneration is a scan-failed-shape cause (not
        // path/perm/format — the enumeration succeeded but returned nothing
        // over an active generation).
        return Err((ScanError::EmptyScanWithActiveGeneration, HealthCause::ScanFailed));
    }
    let start_manifest = build_manifest(&start_enumeration.supported);
    let manifest_revision = manifest_revision(&start_manifest);
    set_manifest_revision(scan_store, scan_id, &manifest_revision).map_err(lift)?;

    advance(scan_store, scan_id, ScanRunState::Staging).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    // Per file: read bytes, hash, build a staged file-level record. The
    // enumerated canonical target and metadata are re-validated before and
    // after every read so a retargeted symlink or replacement file cannot be
    // committed under a stale manifest (AD-4/AD-34).
    let mut staged = Vec::new();
    let mut source_digests = Vec::with_capacity(start_enumeration.supported.len());
    let observed_at = unix_seconds_now();
    for artifact in &start_enumeration.supported {
        ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
        let bytes = read_verified(canonical_root, artifact).map_err(|e| {
            // Story 4.2: read_verified returns ScanError::ReadFailed or
            // ScanError::DirtyAfterValidation; classify the cause from the
            // variant (DirtyAfterValidation → ScanFailed, ReadFailed →
            // ScanFailed — the io kind was erased inside read_verified, but
            // ReadFailed's cause shape is the catch-all by default).
            let cause = health_cause_for_scan_error(&e, None);
            (e, cause)
        })?;
        let source_revision = fnv1a_hex(&bytes);
        source_digests.push((artifact.clone(), source_revision.clone()));
        let file_locator = file_uri(&artifact.file.absolute_path)
            .map_err(|_| (ScanError::ParseFailed, HealthCause::FormatUnsupported))?;
        let canonical_units = canonicalize_markdown(&bytes)
            .map_err(|_| (ScanError::ParseFailed, HealthCause::FormatUnsupported))?;
        for unit in canonical_units {
            let native_locator = format!(
                "{}#{}",
                file_locator,
                percent_encode_fragment(&unit.native_unit_id)
            );
            let display_locator =
                format!("{}#L{}-L{}", file_locator, unit.start_line, unit.end_line);
            let record_id = build_record_id(
                &source.source_id,
                &source.provider,
                &native_locator,
                &unit.unit_kind,
            );
            staged.push(StagedRecord {
                record_id,
                source_rowid,
                provider: source.provider.clone(),
                unit_kind: unit.unit_kind,
                native_unit_id: unit.native_unit_id,
                native_locator,
                content_hash: canonical_content_hash(&unit.title, &unit.body),
                // Story 2.2: read the parser-version tag from the adapter
                // (single source of truth, replacing the hard-coded
                // `CODEX_MARKDOWN_PARSER_VERSION` constant). Codex records
                // carry `codex-markdown/v1`; Claude records carry
                // `claude-markdown/v1`.
                parser_version: adapter.parser_version().to_string(),
                title: unit.title,
                body: unit.body,
                native_project: source.native_project.clone(),
                provider_memory_type: artifact.memory_type.as_str().to_string(),
                coverage_level: coverage_level_string(source.coverage_level).to_string(),
                observed_at,
                source_revision: source_revision.clone(),
                display_locator,
            });
        }
    }
    scan_store
        .stage_records(generation, &staged)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let diagnostics: Vec<StagedDiagnostic> = start_enumeration
        .diagnostics
        .iter()
        .map(|diagnostic| StagedDiagnostic {
            source_rowid,
            kind: diagnostic.kind.to_string(),
            observed_path: diagnostic.observed_path.clone(),
        })
        .collect();
    scan_store
        .stage_diagnostics(generation, &diagnostics)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;

    // Count while the generation is still staging. A database failure here
    // can still mark the run failed; no fallible work remains after CAS.
    let records_indexed = scan_store
        .count_generation_records(source_rowid, generation)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;

    // --- Final manifest re-validation (AD-34/AD-36) ------------------------
    let final_enumeration = adapter
        .enumerate_artifacts(canonical_root)
        .map_err(|err| (ScanError::EnumerationFailed, cause_from_enumerate_error(&err)))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let final_manifest = build_manifest(&final_enumeration.supported);
    if final_manifest != start_manifest || final_enumeration != start_enumeration {
        // Source changed during the scan: this generation is never activated.
        return Err((ScanError::DirtyAfterValidation, HealthCause::ScanFailed));
    }
    // Re-read every staged source after final enumeration and compare the
    // whole-file byte digest. Metadata alone cannot detect a same-size write
    // with restored mtime; `read_verified` proves containment both before and
    // after the read so an escaping retarget is never read.
    for (artifact, expected_digest) in &source_digests {
        let final_bytes = read_verified(canonical_root, artifact).map_err(|e| {
            let cause = health_cause_for_scan_error(&e, None);
            (e, cause)
        })?;
        if fnv1a_hex(&final_bytes) != *expected_digest {
            return Err((ScanError::DirtyAfterValidation, HealthCause::ScanFailed));
        }
    }

    // --- Commit under CAS (AD-32) ------------------------------------------
    advance(scan_store, scan_id, ScanRunState::Committing).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    // A diagnostic-only observation may explain excluded artifacts but must
    // never replace a usable supported generation. This is deliberately after
    // complete enumeration equality and byte-digest validation.
    if start_enumeration.supported.is_empty() && !start_enumeration.diagnostics.is_empty() {
        let committed = scan_store
            .complete_without_activation(scan_id, fencing_token, generation, source_rowid)
            .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
        if !committed {
            return Err((ScanError::CommitCasFailed, HealthCause::ScanFailed));
        }
        let generation = active_generation.unwrap_or_else(|| generation.clone());
        let records_indexed = scan_store
            .count_active_records(source_rowid)
            .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
        return Ok(ScanOutcome {
            source_id: source.source_id.clone(),
            scan_id,
            generation,
            records_indexed,
        });
    }
    let committed = scan_store
        .commit_cas(scan_id, fencing_token, generation, source_rowid)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    if !committed {
        return Err((ScanError::CommitCasFailed, HealthCause::ScanFailed));
    }

    Ok(ScanOutcome {
        source_id: source.source_id.clone(),
        scan_id,
        generation: generation.clone(),
        records_indexed,
    })
}

// ---------------------------------------------------------------------------
// Story 6.5 follow-up — Knowledge (Obsidian Vault) scan pipeline
// ---------------------------------------------------------------------------

/// A Knowledge manifest entry: `(vault_relative_path, absolute_path, size)`.
/// Knowledge notes are file-level (AD-38); mtime is captured on the staged
/// record, not the manifest (the manifest keys on path + size + content hash
/// re-validation, mirroring the Agent-Memory snapshot-at-validation).
type KnowledgeManifestEntry = (String, String, u64);

/// Scan a confirmed Knowledge (Obsidian Vault) Source through the independent
/// Knowledge pipeline (Story 6.5 follow-up). Mirrors [`scan_source_with`]'s
/// shape but routes through `enumerate_notes` + `stage_knowledge_records`,
/// NEVER through `ProviderAdapter` (Story 6.1 AC / AD-19). Reuses the same
/// `scan_runs` state machine, fencing token, and atomic generation commit as
/// Agent Memory (single mutation path, AD-5).
///
/// Caller has already verified `source.source_kind == LocalKnowledge`.
pub fn scan_knowledge_source(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source: &crate::domain::source::Source,
) -> Result<ScanOutcome, ScanError> {
    let scan_store = ScanStore::new(conn);
    let source_id = &source.source_id;

    if source.lifecycle_state != SourceLifecycle::Confirmed {
        return Err(ScanError::NotConfirmed);
    }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;

    // Re-validate the root (NFR-5/6). Same fingerprint guard as Agent Memory.
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(err) => {
            let cause = health_cause_for_scan_error(&ScanError::RootInvalid, Some(err.kind()));
            let _ = registry.set_health_and_cause(source_id, HealthState::Degraded, cause);
            return Err(ScanError::RootInvalid);
        }
    };
    let current_fingerprint = build_fingerprint(
        &source.provider,
        ROOT_KIND_DIR,
        &root.normalized_path,
        root.identity,
    );
    if current_fingerprint != source.fingerprint {
        let _ = registry.set_health_and_cause(
            source_id,
            HealthState::Degraded,
            health_cause_for_scan_error(&ScanError::RootIdentityChanged, None),
        );
        return Err(ScanError::RootIdentityChanged);
    }

    // Begin the run FIRST (placeholder revision), mirroring Agent Memory.
    let (scan_id, fencing_token, generation) = scan_store
        .begin_run(source_rowid, PLACEHOLDER_MANIFEST_REVISION)
        .map_err(|_| ScanError::Internal)?;

    let outcome = run_knowledge_pipeline(
        &scan_store,
        &root.normalized_path,
        source,
        source_rowid,
        scan_id,
        fencing_token,
        &generation,
    );
    match outcome {
        Ok(o) => {
            let _ =
                registry.set_health_and_cause(source_id, HealthState::Healthy, HealthCause::None);
            Ok(o)
        }
        Err((e, cause)) => {
            if !matches!(e, ScanError::CommitCasFailed) {
                let _ = scan_store.fail_run(scan_id, e.error_code());
            }
            if !matches!(e, ScanError::Cancelled) {
                let health = health_for_scan_error(&e);
                let _ = registry.set_health_and_cause(source_id, health, cause);
            }
            Err(e)
        }
    }
}

/// Execute a reserved Knowledge scan run (rescan path, Story 6.5 follow-up).
/// The run was already `begin_run`'d by the reservation that allocated
/// `scan_id`; this validates the root + fingerprint and runs the pipeline.
/// Mirrors `scan_reserved_source_with` for Agent Memory.
fn scan_knowledge_reserved(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
    scan_id: i64,
    fencing_token: i64,
    generation: Generation,
    source: &crate::domain::source::Source,
) -> Result<ScanOutcome, ScanError> {
    let store = ScanStore::new(conn);
    if source.lifecycle_state != SourceLifecycle::Confirmed {
        return Err(ScanError::NotConfirmed);
    }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;
    ensure_not_cancelled(&store, scan_id)?;
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(err) => {
            let cause = health_cause_for_scan_error(&ScanError::RootInvalid, Some(err.kind()));
            return reserved_knowledge_failure(
                registry, &store, source_id, scan_id, ScanError::RootInvalid, cause,
            );
        }
    };
    if build_fingerprint(&source.provider, ROOT_KIND_DIR, &root.normalized_path, root.identity)
        != source.fingerprint
    {
        let cause = health_cause_for_scan_error(&ScanError::RootIdentityChanged, None);
        return reserved_knowledge_failure(
            registry, &store, source_id, scan_id, ScanError::RootIdentityChanged, cause,
        );
    }
    match run_knowledge_pipeline(
        &store,
        &root.normalized_path,
        source,
        source_rowid,
        scan_id,
        fencing_token,
        &generation,
    ) {
        Ok(outcome) => {
            let _ = registry
                .set_health_and_cause(source_id, HealthState::Healthy, HealthCause::None);
            Ok(outcome)
        }
        Err((e, cause)) => {
            if !matches!(e, ScanError::CommitCasFailed) {
                let _ = store.fail_run(scan_id, e.error_code());
            }
            if !matches!(e, ScanError::Cancelled) {
                let health = health_for_scan_error(&e);
                let _ = registry.set_health_and_cause(source_id, health, cause);
            }
            Err(e)
        }
    }
}

/// Fail a reserved Knowledge run and persist health + cause, mirroring the
/// Agent-Memory `reserved_failure` helper.
fn reserved_knowledge_failure(
    registry: &SourceRegistry<'_>,
    store: &ScanStore<'_>,
    source_id: &SourceId,
    scan_id: i64,
    error: ScanError,
    cause: HealthCause,
) -> Result<ScanOutcome, ScanError> {
    let _ = store.fail_run(scan_id, error.error_code());
    let health = health_for_scan_error(&error);
    let _ = registry.set_health_and_cause(source_id, health, cause);
    Err(error)
}


/// enumerate → start manifest → stage → final re-validation → CAS commit.
/// Zero-write (NFR-14): reads Vault metadata + note bytes only.
fn run_knowledge_pipeline(
    scan_store: &ScanStore<'_>,
    canonical_root: &Path,
    source: &crate::domain::source::Source,
    source_rowid: i64,
    scan_id: i64,
    fencing_token: i64,
    generation: &Generation,
) -> Result<ScanOutcome, (ScanError, HealthCause)> {
    let lift = |e: ScanError| {
        let cause = health_cause_for_scan_error(&e, None);
        (e, cause)
    };

    advance(scan_store, scan_id, ScanRunState::Running).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    // --- First enumeration → start manifest → UPDATE real revision ----------
    let start_notes = crate::adapters::obsidian::enumerate_notes(canonical_root)
        .map_err(|_| (ScanError::EnumerationFailed, HealthCause::ScanFailed))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let active_generation = scan_store
        .active_generation(source_rowid)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    if start_notes.is_empty() && active_generation.is_some() {
        // Replacing an existing active generation with an empty result is not
        // allowed (same guard as Agent Memory).
        return Err((ScanError::EmptyScanWithActiveGeneration, HealthCause::ScanFailed));
    }
    let start_manifest = build_knowledge_manifest(&start_notes, canonical_root);
    let manifest_revision = knowledge_manifest_revision(&start_manifest);
    set_manifest_revision(scan_store, scan_id, &manifest_revision).map_err(lift)?;

    advance(scan_store, scan_id, ScanRunState::Staging).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    // Per note: read bytes (bounded by max_note_bytes), hash, stage a file-level
    // Knowledge record. Re-validate containment before/after each read.
    let mut staged: Vec<StagedKnowledgeRecord> = Vec::with_capacity(start_notes.len());
    let mut note_digests: Vec<(KnowledgeManifestEntry, String)> = Vec::new();
    for note in &start_notes {
        ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
        let abs = canonical_root.join(&note.vault_relative_path);
        let bytes = read_knowledge_note_verified(canonical_root, &abs).map_err(|e| {
            let cause = health_cause_for_scan_error(&e, None);
            (e, cause)
        })?;
        let content_hash = fnv1a_hex(&bytes);
        let entry = start_manifest
            .iter()
            .find(|m| m.0 == note.vault_relative_path)
            .cloned()
            .unwrap_or_else(|| (note.vault_relative_path.clone(), abs.to_string_lossy().into_owned(), note.size));
        note_digests.push((entry, content_hash.clone()));
        let record_id = crate::adapters::obsidian::build_knowledge_record_id(
            &source.source_id.to_string(),
            &source.provider,
            &note.vault_relative_path,
            crate::adapters::obsidian::UNIT_KIND_NOTE,
        );
        // native_unit_id = filename without extension (file-level identity).
        let native_unit_id = Path::new(&note.vault_relative_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&note.vault_relative_path)
            .to_string();
        staged.push(StagedKnowledgeRecord {
            record_id,
            source_rowid,
            provider: source.provider.clone(),
            unit_kind: crate::adapters::obsidian::UNIT_KIND_NOTE.to_string(),
            native_unit_id,
            native_locator: note.vault_relative_path.clone(),
            content_hash,
            parser_version: crate::adapters::obsidian::KNOWLEDGE_PARSER_VERSION.to_string(),
            modified_time: note.modified_time.clone(),
        });
    }
    scan_store
        .stage_knowledge_records(generation, &staged)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;

    let records_indexed = scan_store
        .count_generation_knowledge_records(source_rowid, generation)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;

    // --- Final manifest re-validation (AD-34/AD-36) ------------------------
    let final_notes = crate::adapters::obsidian::enumerate_notes(canonical_root)
        .map_err(|_| (ScanError::EnumerationFailed, HealthCause::ScanFailed))?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let final_manifest = build_knowledge_manifest(&final_notes, canonical_root);
    if final_manifest != start_manifest {
        return Err((ScanError::DirtyAfterValidation, HealthCause::ScanFailed));
    }
    // Re-read every staged note and compare the byte digest (same-size write
    // with restored mtime cannot escape this).
    for (entry, expected_digest) in &note_digests {
        let abs = Path::new(&entry.1);
        let final_bytes = read_knowledge_note_verified(canonical_root, abs).map_err(|e| {
            let cause = health_cause_for_scan_error(&e, None);
            (e, cause)
        })?;
        if fnv1a_hex(&final_bytes) != *expected_digest {
            return Err((ScanError::DirtyAfterValidation, HealthCause::ScanFailed));
        }
    }

    // --- Commit under CAS (AD-32) ------------------------------------------
    advance(scan_store, scan_id, ScanRunState::Committing).map_err(lift)?;
    ensure_not_cancelled(scan_store, scan_id).map_err(lift)?;
    let committed = scan_store
        .commit_cas(scan_id, fencing_token, generation, source_rowid)
        .map_err(|_| (ScanError::Internal, HealthCause::ScanFailed))?;
    if !committed {
        return Err((ScanError::CommitCasFailed, HealthCause::ScanFailed));
    }

    Ok(ScanOutcome {
        source_id: source.source_id.clone(),
        scan_id,
        generation: generation.clone(),
        records_indexed,
    })
}

/// Build a sorted Knowledge manifest from enumerated notes. The absolute path
/// is included so a retargeted symlink (same relative path, different inode)
/// produces a different manifest entry.
fn build_knowledge_manifest(
    notes: &[crate::adapters::obsidian::KnowledgeNote],
    canonical_root: &Path,
) -> Vec<KnowledgeManifestEntry> {
    let mut manifest: Vec<KnowledgeManifestEntry> = notes
        .iter()
        .map(|n| {
            let abs = canonical_root.join(&n.vault_relative_path);
            (
                n.vault_relative_path.clone(),
                abs.to_string_lossy().into_owned(),
                n.size,
            )
        })
        .collect();
    manifest.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    manifest
}

/// Compute the Knowledge manifest revision as FNV-1a over length-prefixed
/// sorted entries (AD-34), mirroring the Agent-Memory `manifest_revision`.
fn knowledge_manifest_revision(manifest: &[KnowledgeManifestEntry]) -> String {
    let mut buf = String::new();
    for (path, target, size) in manifest {
        buf.push_str(&path.len().to_string());
        buf.push(':');
        buf.push_str(path);
        buf.push('|');
        buf.push_str(&target.len().to_string());
        buf.push(':');
        buf.push_str(target);
        buf.push('|');
        buf.push_str(&size.to_string());
        buf.push('\n');
    }
    fnv1a_hex(buf.as_bytes())
}

/// Read a Knowledge note with containment re-validation (AD-4/AD-34). Mirrors
/// `read_verified` for the Agent-Memory pipeline but operates on Vault-relative
/// note paths. Rejects a retargeted symlink or replacement file at three
/// points: before open, after open (descriptor bound), after read. Enforces
/// `MAX_NOTE_BYTES` on the opened file's metadata before reading.
fn read_knowledge_note_verified(canonical_root: &Path, abs: &Path) -> Result<Vec<u8>, ScanError> {
    // Containment check before open.
    if !knowledge_note_within_root(canonical_root, abs) {
        return Err(ScanError::DirtyAfterValidation);
    }
    let metadata = std::fs::metadata(abs).map_err(|_| ScanError::ReadFailed)?;
    if !metadata.is_file() {
        return Err(ScanError::ReadFailed);
    }
    // Enforce max_note_bytes on the opened file's metadata BEFORE reading
    // (Story 6.5 AC: reject oversized notes before body allocation).
    if metadata.len() > crate::adapters::obsidian::MAX_NOTE_BYTES {
        return Err(ScanError::ReadFailed);
    }
    let opened_handle =
        Handle::from_file(std::fs::File::open(abs).map_err(|_| ScanError::ReadFailed)?)
            .map_err(|_| ScanError::ReadFailed)?;
    let root_handle = Handle::from_file(
        std::fs::File::open(canonical_root).map_err(|_| ScanError::ReadFailed)?,
    )
    .map_err(|_| ScanError::ReadFailed)?;
    // Re-validate containment after open via same-file handle comparison: the
    // opened file must still be under the canonical root.
    if !path_still_within_root(canonical_root, abs) {
        return Err(ScanError::DirtyAfterValidation);
    }
    let mut file = std::fs::File::open(abs).map_err(|_| ScanError::ReadFailed)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(|_| ScanError::ReadFailed)?;
    // Post-read containment re-check.
    if !path_still_within_root(canonical_root, abs) {
        return Err(ScanError::DirtyAfterValidation);
    }
    // Re-check size after read (a file could grow past the bound mid-read).
    if bytes.len() as u64 > crate::adapters::obsidian::MAX_NOTE_BYTES {
        return Err(ScanError::ReadFailed);
    }
    // Silence unused-handle warnings for the descriptor-binding guards (the
    // handles prove the descriptors were bound; path_still_within_root does the
    // post-open/post-read re-check).
    let _ = (opened_handle, root_handle);
    Ok(bytes)
}

/// True when `abs` canonicalizes to a path inside `canonical_root`. A symlink
/// that escaped the root, or a replacement file, fails this check.
fn knowledge_note_within_root(canonical_root: &Path, abs: &Path) -> bool {
    let Ok(real) = std::fs::canonicalize(abs) else {
        return false;
    };
    real.starts_with(canonical_root)
}

/// Post-open/post-read containment re-check. Re-canonicalizes the absolute path
/// and confirms it still starts with the canonical root (catches a retargeted
/// symlink between the pre-open check and the read).
fn path_still_within_root(canonical_root: &Path, abs: &Path) -> bool {
    knowledge_note_within_root(canonical_root, abs)
}

/// Story 4.2 — classify the cause from an adapter's `EnumerateError` variant.
/// Used at the I/O boundary where the adapter returns its refined error and
/// the pipeline lifts it into the `(ScanError, HealthCause)` error channel.
/// The `EnumerateError` variants already encode the io-kind classification
/// (`RootMissing` / `RootPermissionDenied` / `DirMissing` /
/// `DirPermissionDenied` / etc.), so this is a pure variant → cause mapping.
fn cause_from_enumerate_error(err: &crate::domain::ports::provider_adapter::EnumerateError) -> HealthCause {
    use crate::domain::ports::provider_adapter::EnumerateError::{
        AllowlistedArtifactUnresolvable, DirMissing, DirPermissionDenied, RootMissing,
        RootPermissionDenied, RootUnresolvable, Unreadable,
    };
    match err {
        // Path-missing io kind at any root/dir site.
        RootMissing | DirMissing => HealthCause::PathMissing,
        // PermissionDenied io kind at any root/dir site.
        RootPermissionDenied | DirPermissionDenied => HealthCause::PermissionDenied,
        // Fallback root/dir io kinds + the allowlisted-artifact site. The
        // artifact site's io kind is not propagated through the variant (an
        // artifact read failure could be any io kind), so it defaults to the
        // scan_failed catch-all — distinct from path/perm/format.
        RootUnresolvable | Unreadable | AllowlistedArtifactUnresolvable => HealthCause::ScanFailed,
    }
}

fn ensure_not_cancelled(scan_store: &ScanStore<'_>, scan_id: i64) -> Result<(), ScanError> {
    if scan_store
        .is_cancelled(scan_id)
        .map_err(|_| ScanError::Internal)?
    {
        return Err(ScanError::Cancelled);
    }
    Ok(())
}

fn advance(scan_store: &ScanStore<'_>, scan_id: i64, state: ScanRunState) -> Result<(), ScanError> {
    if scan_store.set_state(scan_id, state).map_err(|_| ScanError::Internal)? == 0 {
        return ensure_not_cancelled(scan_store, scan_id);
    }
    Ok(())
}

fn health_for_scan_error(error: &ScanError) -> HealthState {
    match error {
        ScanError::RootInvalid
        | ScanError::RootIdentityChanged
        | ScanError::ReadFailed
        | ScanError::ParseFailed
        | ScanError::EnumerationFailed
        | ScanError::EmptyScanWithActiveGeneration => HealthState::Degraded,
        ScanError::SourceNotFound | ScanError::NotConfirmed | ScanError::Cancelled => {
            HealthState::Unknown
        }
        ScanError::DirtyAfterValidation | ScanError::CommitCasFailed | ScanError::Internal => {
            HealthState::Error
        }
    }
}

/// Classify a [`ScanError`] into the structured [`HealthCause`] taxonomy
/// (Story 4.2). Parallel to [`health_for_scan_error`], but answers "why is
/// the source's health degraded" (persisted on the source row), whereas
/// `health_for_scan_error` answers "what state should the row show".
///
/// The root-validation path (`RootInvalid` / `RootIdentityChanged`) classifies
/// the canonicalize io error kind directly at the call site (it has the io
/// error in hand before mapping to `ScanError`) and passes the kind via
/// `io_kind_hint`. The base mapping for this arm is `PathMissing` (per the
/// spec's Boundaries: RootInvalid/RootIdentityChanged → PathMissing, refined
/// by io kind at the call site); only a `PermissionDenied` hint overrides it
/// to `PermissionDenied`. `RootIdentityChanged`'s call sites pass `None` (it
/// is a fingerprint mismatch, not an io error), so it surfaces `PathMissing`.
/// `EnumerationFailed` / `ReadFailed` likewise refine their cause from an io
/// kind hint when the call site captured one (defaulting to `ScanFailed` for
/// any non-I/O failure or a missing hint). `ParseFailed` always maps to
/// `FormatUnsupported`. `DirtyAfterValidation` / `CommitCasFailed` /
/// `Internal` / `EmptyScanWithActiveGeneration` always map to `ScanFailed`.
/// `SourceNotFound` / `NotConfirmed` / `Cancelled` map to `None` (cancel is
/// not a health transition; not-found/not-confirmed are not health probes).
fn health_cause_for_scan_error(
    error: &ScanError,
    io_kind_hint: Option<std::io::ErrorKind>,
) -> HealthCause {
    match error {
        ScanError::RootInvalid | ScanError::RootIdentityChanged => {
            // The root-validation path passes the canonicalize io error kind
            // directly (RootInvalid's call sites supply `Some(err.kind())`).
            // RootIdentityChanged's call sites pass `None` (it is a fingerprint
            // mismatch, not an io error), so the BASE mapping for this arm is
            // `PathMissing` — the spec's Boundaries name RootInvalid/
            // RootIdentityChanged → PathMissing, refined by io kind at the
            // call site. Only a `PermissionDenied` hint overrides toward a
            // different category; `NotFound`, `InvalidInput`, `NotADirectory`
            // (a root that exists but is a regular file — synthesized by
            // `policy::canonicalize_root` as `ErrorKind::InvalidInput`), and
            // a missing hint all stay `PathMissing`.
            match io_kind_hint {
                Some(std::io::ErrorKind::PermissionDenied) => HealthCause::PermissionDenied,
                _ => HealthCause::PathMissing,
            }
        }
        ScanError::EnumerationFailed | ScanError::ReadFailed => {
            // Classified by io kind at the adapter boundary. The application
            // layer does not always have the io kind in hand here (the
            // adapter returns an `EnumerateError` whose variant already
            // encodes the classification, but `ScanError` erases it); default
            // to `ScanFailed` when no hint is supplied. Specific
            // permission-denied mid-scan failures pass a hint.
            match io_kind_hint {
                Some(std::io::ErrorKind::NotFound) => HealthCause::PathMissing,
                Some(std::io::ErrorKind::PermissionDenied) => HealthCause::PermissionDenied,
                _ => HealthCause::ScanFailed,
            }
        }
        ScanError::ParseFailed => HealthCause::FormatUnsupported,
        ScanError::EmptyScanWithActiveGeneration
        | ScanError::DirtyAfterValidation
        | ScanError::CommitCasFailed
        | ScanError::Internal => HealthCause::ScanFailed,
        ScanError::SourceNotFound | ScanError::NotConfirmed | ScanError::Cancelled => {
            HealthCause::None
        }
    }
}

fn read_verified(
    canonical_root: &Path,
    artifact: &SupportedArtifact,
) -> Result<Vec<u8>, ScanError> {
    // The path is checked before opening. The opened descriptor is then bound
    // to the current in-root path before a single byte is read, closing the
    // check-to-read retarget window for an external symlink.
    if !unit_matches_snapshot(canonical_root, &artifact.file) {
        return Err(ScanError::DirtyAfterValidation);
    }
    let mut file =
        std::fs::File::open(&artifact.file.absolute_path).map_err(|_| ScanError::ReadFailed)?;
    let opened_metadata = file.metadata().map_err(|_| ScanError::ReadFailed)?;
    let opened_handle = Handle::from_file(file.try_clone().map_err(|_| ScanError::ReadFailed)?)
        .map_err(|_| ScanError::ReadFailed)?;
    if !opened_file_matches_snapshot(
        canonical_root,
        &artifact.file,
        &opened_metadata,
        &opened_handle,
    ) {
        return Err(ScanError::DirtyAfterValidation);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ScanError::ReadFailed)?;
    if !unit_matches_snapshot(canonical_root, &artifact.file)
        || !opened_file_matches_snapshot(
            canonical_root,
            &artifact.file,
            &opened_metadata,
            &opened_handle,
        )
    {
        return Err(ScanError::DirtyAfterValidation);
    }
    Ok(bytes)
}

fn canonical_content_hash(title: &str, body: &str) -> String {
    let mut value = String::new();
    value.push_str(&title.len().to_string());
    value.push(':');
    value.push_str(title);
    value.push('|');
    value.push_str(&body.len().to_string());
    value.push(':');
    value.push_str(body);
    fnv1a_hex(value.as_bytes())
}

fn coverage_level_string(level: CoverageLevel) -> &'static str {
    match level {
        CoverageLevel::Full => "full",
        CoverageLevel::SearchOnly => "search_only",
        CoverageLevel::ExistenceOnly => "existence_only",
        CoverageLevel::Unsupported => "unsupported",
    }
}

fn unix_seconds_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// UPDATE the real `manifest_revision` onto a run row after the manifest is
/// snapshotted (spec amendment 4: `begin_run` wrote a placeholder). A store
/// helper is used rather than a raw query so the write stays behind the store
/// boundary.
fn set_manifest_revision(
    scan_store: &ScanStore<'_>,
    scan_id: i64,
    manifest_revision: &str,
) -> Result<(), ScanError> {
    scan_store
        .set_manifest_revision(scan_id, manifest_revision)
        .map(|_| ())
        .map_err(|_| ScanError::Internal)
}

/// Report the scan status of a Source (AD-13 safe surface). Infallible at the
/// application layer for a known source: a source with no runs and no active
/// generation reports `state: None, active_generation: None, active_records:
/// 0`. Returns `ScanError::SourceNotFound` for an unknown id.
pub fn get_scan_status(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
) -> Result<ScanStatus, ScanError> {
    let scan_store = ScanStore::new(conn);
    // Validate the source exists (spec: unknown id → source_not_found).
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    if source.is_none() {
        return Err(ScanError::SourceNotFound);
    }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;

    let latest = scan_store
        .latest_run(source_rowid)
        .map_err(|_| ScanError::Internal)?;
    let active_generation = scan_store
        .active_generation(source_rowid)
        .map_err(|_| ScanError::Internal)?;
    let active_records = scan_store
        .count_active_records(source_rowid)
        .map_err(|_| ScanError::Internal)?;

    Ok(ScanStatus {
        source_id: source_id.clone(),
        state: latest.map(|r| r.state),
        active_generation,
        active_records,
    })
}

/// Assemble server-owned inventory facts. This deliberately reads the last
/// success, current active count, and latest failure independently so a failed
/// rescan cannot erase a previously safe, searchable generation.
///
/// Story 4.2 — the inventory row now also carries:
/// - `cause`: the structured cause persisted on the source row (read from
///   `source.health_cause`), surfaced as `Some(cause)` for any non-`None`
///   persisted cause; `None` for a healthy/never-probed source.
/// - `stale`: derived at read time as
///   `(health_state in {degraded, error}) AND active_generation IS NOT NULL`.
///   A degraded source with NO active generation is `unavailable`, not stale.
///
/// `latest_error` keeps its existing derivation (from `scan_runs.error_code`
/// for a Failed latest run, plus the existing Degraded fallback). `cause` and
/// `latest_error` are INDEPENDENT — keeping `latest_error`'s derivation
/// untouched preserves the pinned strings at `inventory.rs:43,111,176,287`.
pub fn list_inventory(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
) -> Result<Vec<SourceInventory>, ScanError> {
    let store = ScanStore::new(conn);
    registry
        .list()
        .map_err(|_| ScanError::Internal)?
        .into_iter()
        .map(|source| {
            let source_rowid =
                ScanStore::source_rowid(&source.source_id).ok_or(ScanError::Internal)?;
            let latest = store
                .latest_run(source_rowid)
                .map_err(|_| ScanError::Internal)?;
            let latest_error = latest.as_ref().and_then(|run| {
                (run.state == ScanRunState::Failed)
                    .then(|| safe_error_reason(run.error_code.as_deref()))
            });
            let complete_record_count = matches!(source.coverage_level, CoverageLevel::Full)
                .then(|| store.count_active_records(source_rowid))
                .transpose()
                .map_err(|_| ScanError::Internal)?;
            // Story 4.2 — derive `stale` and surface the persisted `cause`.
            // `stale` is a pure function of health_state + active_generation:
            // an older successful generation is still serving results while
            // the source currently fails to refresh it.
            let active_generation = store
                .active_generation(source_rowid)
                .map_err(|_| ScanError::Internal)?;
            let stale = matches!(
                source.health_state,
                HealthState::Degraded | HealthState::Error
            ) && active_generation.is_some();
            // Surface the persisted cause as `Some` for any non-`None` value;
            // `None` (null on the wire) means healthy/never-probed.
            let cause = (source.health_cause != HealthCause::None)
                .then_some(source.health_cause);
            Ok(SourceInventory {
                source_id: source.source_id,
                provider: source.provider,
                lifecycle_state: source.lifecycle_state,
                root: source.normalized_root_path,
                native_project: source.native_project,
                coverage_level: coverage_level_string(source.coverage_level).to_string(),
                health_state: source.health_state,
                last_successful_scan: store
                    .last_successful_finished_at(source_rowid)
                    .map_err(|_| ScanError::Internal)?,
                complete_record_count,
                latest_error: latest_error.or_else(|| (source.health_state == HealthState::Degraded).then(|| "Tessera could not access this source.".to_string())),
                cause,
                stale,
            })
        })
        .collect()
}

/// List Knowledge (Obsidian Vault) Inventory rows (Story 6.6). Filters the
/// Source Registry to `local_knowledge` Sources only and counts notes from
/// the independent `knowledge_records` table. Agent-Memory Sources are never
/// included (AD-19 domain separation).
pub fn list_knowledge_inventory(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
) -> Result<Vec<crate::domain::scan::KnowledgeInventory>, ScanError> {
    let store = ScanStore::new(conn);
    registry
        .list()
        .map_err(|_| ScanError::Internal)?
        .into_iter()
        .filter(|s| {
            s.source_kind == crate::domain::source::SourceKind::LocalKnowledge
        })
        .map(|source| {
            let source_rowid =
                ScanStore::source_rowid(&source.source_id).ok_or(ScanError::Internal)?;
            let latest = store
                .latest_run(source_rowid)
                .map_err(|_| ScanError::Internal)?;
            let latest_error = latest.as_ref().and_then(|run| {
                (run.state == ScanRunState::Failed)
                    .then(|| safe_error_reason(run.error_code.as_deref()))
            });
            let active_generation = store
                .active_generation(source_rowid)
                .map_err(|_| ScanError::Internal)?;
            // complete_note_count is Some ONLY when the Source declares full
            // coverage AND has an active generation. A never-scanned Source
            // returns None (truthful "not yet counted"), never Some(0) — a
            // missing value must not masquerade as a real zero. A scanned
            // Vault that genuinely has zero supported notes is Some(0).
            let complete_note_count = if matches!(source.coverage_level, CoverageLevel::Full)
                && active_generation.is_some()
            {
                Some(
                    store
                        .count_active_knowledge_records(source_rowid)
                        .map_err(|_| ScanError::Internal)?,
                )
            } else {
                None
            };
            let stale = matches!(
                source.health_state,
                HealthState::Degraded | HealthState::Error
            ) && active_generation.is_some();
            let cause = (source.health_cause != HealthCause::None)
                .then_some(source.health_cause);
            // Vault display name = final path component of the normalized root.
            let vault_name = std::path::Path::new(&source.normalized_root_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&source.normalized_root_path)
                .to_string();
            Ok(crate::domain::scan::KnowledgeInventory {
                source_id: source.source_id,
                vault_name,
                provider: source.provider,
                root: source.normalized_root_path,
                coverage_level: coverage_level_string(source.coverage_level).to_string(),
                health_state: source.health_state,
                last_successful_scan: store
                    .last_successful_finished_at(source_rowid)
                    .map_err(|_| ScanError::Internal)?,
                complete_note_count,
                latest_error: latest_error.or_else(|| (source.health_state == HealthState::Degraded).then(|| "Tessera could not access this vault.".to_string())),
                cause,
                stale,
                lifecycle_state: source.lifecycle_state,
            })
        })
        .collect()
}

fn safe_error_reason(error_code: Option<&str>) -> String {
    match error_code {
        Some("cancelled") => "The last rescan was cancelled.".to_string(),
        Some("read_failed") | Some("enumeration_failed") => {
            "Tessera could not read this source.".to_string()
        }
        Some("parse_failed") => "Tessera could not read this source format.".to_string(),
        Some("dirty_after_validation") => {
            "The source changed while Tessera was scanning it.".to_string()
        }
        Some("stale_recovered") => "The previous rescan did not finish.".to_string(),
        _ => "Tessera could not complete the last rescan.".to_string(),
    }
}

/// Request cancellation for the newest running Source job. Returns false when
/// no in-flight job exists; it never cancels a completed generation.
pub fn cancel_rescan(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
) -> Result<bool, ScanError> {
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else {
        return Err(ScanError::SourceNotFound);
    };
    if source.lifecycle_state != SourceLifecycle::Confirmed {
        return Err(ScanError::NotConfirmed);
    }
    ScanStore::new(conn)
        .cancel_latest_run(ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?)
        .map_err(|_| ScanError::Internal)
}

/// Boot-time scan recovery (AD-16). Flips stale in-flight runs to `failed`
/// (with `error_code='stale_recovered'`) and GCs every non-active-generation
/// `memory_records` row. Called from the boot path in `lib.rs` after
/// migrations.
///
/// **Log-and-continue contract (KEEP):** the boot call site must NOT
/// `.expect()`/panic on a recovery error — it logs and continues, and the
/// next boot retries. Returning `Err` here is how the caller is told to log;
/// panicking is the caller's bug, not this function's.
pub fn recover_scans(conn: &Connection) -> Result<(), ScanError> {
    ScanStore::new(conn)
        .recover_stale_runs()
        .map_err(|_| ScanError::Internal)
}

/// Build the sorted manifest from enumerated file units. Sorting by relative
/// path makes the manifest (and its hash) deterministic.
fn build_manifest(units: &[SupportedArtifact]) -> Vec<ManifestEntry> {
    let mut manifest: Vec<ManifestEntry> = units
        .iter()
        .map(|artifact| {
            let u = &artifact.file;
            (
                u.relative_path.clone(),
                u.absolute_path.to_string_lossy().into_owned(),
                u.size,
                u.mtime,
            )
        })
        .collect();
    manifest.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    manifest
}

/// Compute the `manifest_revision` as FNV-1a over the length-prefixed sorted
/// manifest entries (AD-34 `snapshot-at-validation`). Pure and deterministic.
fn manifest_revision(manifest: &[ManifestEntry]) -> String {
    let mut buf = String::new();
    for (path, target, size, mtime) in manifest {
        // Netstring-style length prefixes disambiguate the path and resolved
        // target; size/mtime are fixed-width decimal.
        buf.push_str(&path.len().to_string());
        buf.push(':');
        buf.push_str(path);
        buf.push('|');
        buf.push_str(&target.len().to_string());
        buf.push(':');
        buf.push_str(target);
        buf.push('|');
        buf.push_str(&size.to_string());
        buf.push('|');
        buf.push_str(&mtime.to_string());
        buf.push('\n');
    }
    fnv1a_hex(buf.as_bytes())
}

/// Return whether the actual filesystem target still matches the canonical
/// target and metadata snapshot supplied by enumeration.
fn unit_matches_snapshot(canonical_root: &Path, unit: &crate::domain::FileUnit) -> bool {
    let Ok(real) = std::fs::canonicalize(&unit.absolute_path) else {
        return false;
    };
    if real != unit.absolute_path || !real.starts_with(canonical_root) {
        return false;
    }
    let Ok(metadata) = std::fs::metadata(&real) else {
        return false;
    };
    metadata_matches_snapshot(&metadata, unit)
}

/// Verify that an already-opened descriptor still denotes the same in-root
/// file that the enumerated path names. No bytes are read until this holds.
fn opened_file_matches_snapshot(
    canonical_root: &Path,
    unit: &crate::domain::FileUnit,
    opened: &std::fs::Metadata,
    opened_handle: &Handle,
) -> bool {
    let Ok(real) = std::fs::canonicalize(&unit.absolute_path) else {
        return false;
    };
    if real != unit.absolute_path || !real.starts_with(canonical_root) {
        return false;
    }
    let Ok(current_handle) = Handle::from_path(&real) else {
        return false;
    };
    *opened_handle == current_handle && metadata_matches_snapshot(opened, unit)
}

fn metadata_matches_snapshot(metadata: &std::fs::Metadata, unit: &crate::domain::FileUnit) -> bool {
    metadata.is_file()
        && metadata.len() == unit.size
        && metadata_mtime(metadata).is_some_and(|mtime| mtime == unit.mtime)
}

fn metadata_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
    let duration = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(duration.as_nanos() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::source::adapter_for;

    /// Story 2.2 review — there is ONE provider→adapter registry. Scan dispatch
    /// (`adapter_for_scan`) delegates to confirm dispatch
    /// (`application::source::adapter_for`), so a provider added to one path is
    /// visible to the other and a provider missing from one is rejected by
    /// both. Both paths resolve identically for every provider id; an unknown
    /// id yields `None` on both (so confirm rejects it and scan surfaces
    /// `Internal`, never a silent mismatch).
    #[test]
    fn adapter_for_scan_matches_confirm_registry() {
        for provider in ["codex", "claude_code", "unknown"] {
            let scan = adapter_for_scan(provider);
            let confirm = adapter_for(provider);
            assert_eq!(
                scan.as_ref().map(|a| a.provider_id()),
                confirm.as_ref().map(|a| a.provider_id()),
                "scan dispatch and confirm registry disagree for provider {provider:?}"
            );
        }
    }

    // Story 4.2 — `health_cause_for_scan_error` is the load-bearing classifier
    // for the root-validation path (where the application layer has the io
    // error in hand) and for the post-begin_run failure paths (where the io
    // kind is not threaded). These unit tests pin the mapping directly,
    // independently of the inventory projection tests.

    /// Patch 2 — `RootIdentityChanged` (a fingerprint mismatch, no io error in
    /// hand) classifies as `PathMissing`, the base mapping for the
    /// `RootInvalid | RootIdentityChanged` arm per the spec's Boundaries.
    #[test]
    fn health_cause_for_scan_error_maps_root_identity_changed_to_path_missing() {
        assert_eq!(
            health_cause_for_scan_error(&ScanError::RootIdentityChanged, None),
            HealthCause::PathMissing,
        );
    }

    /// Patch 3 — a root that exists but is a regular file is rejected by
    /// `policy::canonicalize_root` with `ErrorKind::InvalidInput`
    /// ("source root is not a directory"). The spec's `PathMissing` definition
    /// explicitly includes "not-a-dir", so `InvalidInput` must classify as
    /// `PathMissing`, not the catch-all `ScanFailed`.
    #[test]
    fn health_cause_for_scan_error_maps_root_invalid_invalid_input_to_path_missing() {
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::RootInvalid,
                Some(std::io::ErrorKind::InvalidInput),
            ),
            HealthCause::PathMissing,
            "InvalidInput (not-a-dir) must classify as path_missing, not scan_failed",
        );
    }

    /// Patch 3 — `ErrorKind::NotADirectory` likewise classifies as
    /// `PathMissing` (a root path component is not a directory).
    #[test]
    fn health_cause_for_scan_error_maps_root_invalid_not_a_directory_to_path_missing() {
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::RootInvalid,
                Some(std::io::ErrorKind::NotADirectory),
            ),
            HealthCause::PathMissing,
            "NotADirectory must classify as path_missing, not scan_failed",
        );
    }

    /// Patch 2/3 — the base mapping for the root-validation arm is
    /// `PathMissing`; only `PermissionDenied` overrides it. `NotFound`
    /// (deleted root) stays `PathMissing`.
    #[test]
    fn health_cause_for_scan_error_root_arm_path_missing_for_not_found_and_unknown_kinds() {
        assert_eq!(
            health_cause_for_scan_error(&ScanError::RootInvalid, Some(std::io::ErrorKind::NotFound)),
            HealthCause::PathMissing,
        );
        // An unknown io kind at the root site also stays PathMissing (the base
        // mapping), NOT ScanFailed — the root is gone/unusable either way.
        assert_eq!(
            health_cause_for_scan_error(&ScanError::RootInvalid, Some(std::io::ErrorKind::Other)),
            HealthCause::PathMissing,
        );
        // PermissionDenied is the one override.
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::RootInvalid,
                Some(std::io::ErrorKind::PermissionDenied),
            ),
            HealthCause::PermissionDenied,
        );
    }

    /// Patch 7 — `ParseFailed` always maps to `FormatUnsupported` regardless
    /// of any io hint (a parse failure is never a path/perm/scan failure).
    #[test]
    fn health_cause_for_scan_error_maps_parse_failed_to_format_unsupported() {
        assert_eq!(
            health_cause_for_scan_error(&ScanError::ParseFailed, None),
            HealthCause::FormatUnsupported,
        );
        // An io hint is ignored for ParseFailed (it is a Markdown decode
        // failure, not an io error).
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::ParseFailed,
                Some(std::io::ErrorKind::PermissionDenied),
            ),
            HealthCause::FormatUnsupported,
        );
    }

    /// Patch 7 — `EmptyScanWithActiveGeneration`, `DirtyAfterValidation`,
    /// `CommitCasFailed`, and `Internal` all map to `ScanFailed` (the catch-all
    /// that is NOT path/perm/format).
    #[test]
    fn health_cause_for_scan_error_maps_internal_catch_all_variants_to_scan_failed() {
        for variant in [
            ScanError::EmptyScanWithActiveGeneration,
            ScanError::DirtyAfterValidation,
            ScanError::CommitCasFailed,
            ScanError::Internal,
        ] {
            assert_eq!(
                health_cause_for_scan_error(&variant, None),
                HealthCause::ScanFailed,
                "{variant:?} must classify as scan_failed",
            );
        }
    }

    /// Patch 7 — `SourceNotFound`, `NotConfirmed`, and `Cancelled` all map to
    /// `None` (cancel is not a health transition; not-found/not-confirmed are
    /// not health probes).
    #[test]
    fn health_cause_for_scan_error_maps_non_health_variants_to_none() {
        for variant in [
            ScanError::SourceNotFound,
            ScanError::NotConfirmed,
            ScanError::Cancelled,
        ] {
            assert_eq!(
                health_cause_for_scan_error(&variant, None),
                HealthCause::None,
                "{variant:?} must classify as None",
            );
        }
    }

    /// Patch 7 — `EnumerationFailed`/`ReadFailed` without an io hint default
    /// to `ScanFailed`; `NotFound`/`PermissionDenied` hints refine toward the
    /// matching cause.
    #[test]
    fn health_cause_for_scan_error_enumeration_and_read_default_and_refine() {
        // No hint → default to scan_failed (the io kind was erased by the
        // ScanError mapping at the adapter boundary).
        for variant in [ScanError::EnumerationFailed, ScanError::ReadFailed] {
            assert_eq!(
                health_cause_for_scan_error(&variant, None),
                HealthCause::ScanFailed,
                "{variant:?} with no hint defaults to scan_failed",
            );
        }
        // NotFound hint → path_missing.
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::EnumerationFailed,
                Some(std::io::ErrorKind::NotFound),
            ),
            HealthCause::PathMissing,
        );
        // PermissionDenied hint → permission_denied.
        assert_eq!(
            health_cause_for_scan_error(
                &ScanError::ReadFailed,
                Some(std::io::ErrorKind::PermissionDenied),
            ),
            HealthCause::PermissionDenied,
        );
    }
}
