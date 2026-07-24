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

use crate::adapters::codex::{
    canonicalize_markdown, file_uri, percent_encode_fragment, CodexAdapter,
    CODEX_MARKDOWN_PARSER_VERSION,
};
use crate::domain::scan::{
    build_record_id, fnv1a_hex, Generation, ScanError, ScanOutcome, ScanRunState, ScanStatus,
    SourceInventory,
};
use crate::domain::source::{
    build_fingerprint, HealthState, SourceId, SourceLifecycle, ROOT_KIND_DIR,
};
use crate::domain::{CoverageLevel, ProviderAdapter, SupportedArtifact};
use crate::index::scan_store::{ScanStore, StagedDiagnostic, StagedRecord};
use crate::index::SourceRegistry;
use crate::policy;

/// The placeholder `manifest_revision` written by `begin_run` before the first
/// enumeration (spec amendment 4: begin_run must precede the first enumeration
/// so an enumeration failure also lands on a persisted run row). The real
/// revision is UPDATEd onto the row once the manifest is snapshotted.
const PLACEHOLDER_MANIFEST_REVISION: &str = "pending";

/// A single manifest entry: `(relative_path, canonical_target, size, mtime)`
/// where `mtime` is nanoseconds since the Unix epoch (sub-second precision —
/// AD-34). Including the canonical target binds the manifest to the exact
/// file resolved at enumeration time, rather than merely to its visible name.
type ManifestEntry = (String, String, u64, i64);

/// Scan a confirmed Source using the Codex adapter (AD-1 orchestration).
///
/// This is the production entry point the IPC command calls. It delegates to
/// [`scan_source_with`] with the real [`CodexAdapter`]; the generic seam
/// exists so the integration tests can drive a scripted adapter through the
/// SAME public orchestration (e.g. to produce a real manifest drift).
pub fn scan_source(
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
) -> Result<ScanOutcome, ScanError> {
    scan_source_with(&CodexAdapter, registry, conn, source_id)
}

/// Execute a run reserved before the queued response reached the browser.
pub fn scan_reserved_source(
    registry: &SourceRegistry<'_>, conn: &Connection, source_id: &SourceId,
    scan_id: i64, fencing_token: i64, generation: Generation,
) -> Result<ScanOutcome, ScanError> {
    scan_reserved_source_with(&CodexAdapter, registry, conn, source_id, scan_id, fencing_token, generation)
}

/// Scan a confirmed Source with an injected adapter (AD-1 orchestration).
///
/// `registry` and `scan_store` borrow the same connection (the IPC layer
/// holds the `IndexState` mutex for the whole command — synchronous, single
/// owner per Source, AD-5). Returns the [`ScanOutcome`] on a fully-successful
/// scan; any failure after `begin_run` is a structured [`ScanError`] with the
/// run marked `failed` (except a lost commit CAS — see module doc).
///
/// The adapter is generic so tests can substitute a scripted
/// [`ProviderAdapter`] (the port exists for exactly this — see the amended
/// spec's tests task). Production callers use [`scan_source`].
pub fn scan_source_with<A: ProviderAdapter>(
    adapter: &A,
    registry: &SourceRegistry<'_>,
    conn: &Connection,
    source_id: &SourceId,
) -> Result<ScanOutcome, ScanError> {
    let scan_store = ScanStore::new(conn);

    // --- Validate source + root -------------------------------------------
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else {
        return Err(ScanError::SourceNotFound);
    };
    if source.lifecycle_state != SourceLifecycle::Confirmed {
        // Rejected / disabled sources are not scannable (spec I/O matrix).
        return Err(ScanError::NotConfirmed);
    }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;

    // Re-validate the root (AD-4/NFR-5/6). A deleted / non-dir root fails the
    // scan BEFORE begin_run (no run row — root validation precedes ownership);
    // any prior active generation is preserved.
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(_) => {
            let _ = registry.set_health(source_id, HealthState::Degraded);
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
        // replacement directory under the old Source identity.
        let _ = registry.set_health(source_id, HealthState::Degraded);
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
        &source,
        source_rowid,
        scan_id,
        fencing_token,
        &generation,
    );
    match outcome {
        Ok(o) => {
            // Activation already committed; a health-write failure must not
            // falsely report that the previous generation remains active.
            let _ = registry.set_health(source_id, HealthState::Healthy);
            Ok(o)
        }
        Err(e) => {
            // A lost CAS is NOT re-marked: the run is no longer owned by this
            // holder (left in `committing` for boot recovery). Every other
            // failure marks the run failed with its error category from the
            // domain-layer vocabulary mapping (`ScanError::error_code()`).
            if !matches!(e, ScanError::CommitCasFailed) {
                let _ = scan_store.fail_run(scan_id, e.error_code());
            }
            if !matches!(e, ScanError::Cancelled) {
                let health = health_for_scan_error(&e);
                let _ = registry.set_health(source_id, health);
            }
            Err(e)
        }
    }
}

fn scan_reserved_source_with<A: ProviderAdapter>(
    adapter: &A, registry: &SourceRegistry<'_>, conn: &Connection, source_id: &SourceId,
    scan_id: i64, fencing_token: i64, generation: Generation,
) -> Result<ScanOutcome, ScanError> {
    let store = ScanStore::new(conn);
    let source = registry.get(source_id).map_err(|_| ScanError::Internal)?;
    let Some(source) = source else { return Err(ScanError::SourceNotFound); };
    if source.lifecycle_state != SourceLifecycle::Confirmed { return Err(ScanError::NotConfirmed); }
    let source_rowid = ScanStore::source_rowid(source_id).ok_or(ScanError::SourceNotFound)?;
    ensure_not_cancelled(&store, scan_id)?;
    let root = match policy::canonicalize_root(Path::new(&source.normalized_root_path)) {
        Ok(root) => root,
        Err(_) => return reserved_failure(registry, &store, source_id, scan_id, ScanError::RootInvalid, "root_invalid"),
    };
    if build_fingerprint(&source.provider, ROOT_KIND_DIR, &root.normalized_path, root.identity) != source.fingerprint {
        return reserved_failure(registry, &store, source_id, scan_id, ScanError::RootIdentityChanged, "root_identity_changed");
    }
    match run_pipeline(adapter, &store, &root.normalized_path, &source, source_rowid, scan_id, fencing_token, &generation) {
        Ok(outcome) => { let _ = registry.set_health(source_id, HealthState::Healthy); Ok(outcome) }
        Err(error) => {
            if !matches!(error, ScanError::CommitCasFailed) { let _ = store.fail_run(scan_id, error.error_code()); }
            if !matches!(error, ScanError::Cancelled) { let _ = registry.set_health(source_id, health_for_scan_error(&error)); }
            Err(error)
        }
    }
}

fn reserved_failure(
    registry: &SourceRegistry<'_>, store: &ScanStore<'_>, source_id: &SourceId,
    scan_id: i64, error: ScanError, error_code: &str,
) -> Result<ScanOutcome, ScanError> {
    let _ = store.fail_run(scan_id, error_code);
    let _ = registry.set_health(source_id, HealthState::Degraded);
    Err(error)
}

/// The staged body of the scan, split out so the caller can apply the
/// fail-run-on-error policy uniformly.
#[allow(clippy::too_many_arguments)]
fn run_pipeline<A: ProviderAdapter>(
    adapter: &A,
    scan_store: &ScanStore<'_>,
    canonical_root: &Path,
    source: &crate::domain::source::Source,
    source_rowid: i64,
    scan_id: i64,
    fencing_token: i64,
    generation: &Generation,
) -> Result<ScanOutcome, ScanError> {
    // running → staging.
    advance(scan_store, scan_id, ScanRunState::Running)?;
    ensure_not_cancelled(scan_store, scan_id)?;

    // --- First enumeration → start manifest → UPDATE real revision ----------
    let start_enumeration = adapter
        .enumerate_artifacts(canonical_root)
        .map_err(|_| ScanError::EnumerationFailed)?;
    ensure_not_cancelled(scan_store, scan_id)?;
    let active_generation = scan_store
        .active_generation(source_rowid)
        .map_err(|_| ScanError::Internal)?;
    if start_enumeration.supported.is_empty()
        && start_enumeration.diagnostics.is_empty()
        && scan_store
            .active_generation(source_rowid)
            .map_err(|_| ScanError::Internal)?
            .is_some()
    {
        // An intentionally empty first scan is valid. Replacing an existing
        // active generation with an empty result is not: unreadable roots can
        // otherwise masquerade as a successful destructive rescan.
        return Err(ScanError::EmptyScanWithActiveGeneration);
    }
    let start_manifest = build_manifest(&start_enumeration.supported);
    let manifest_revision = manifest_revision(&start_manifest);
    set_manifest_revision(scan_store, scan_id, &manifest_revision)?;

    advance(scan_store, scan_id, ScanRunState::Staging)?;
    ensure_not_cancelled(scan_store, scan_id)?;

    // Per file: read bytes, hash, build a staged file-level record. The
    // enumerated canonical target and metadata are re-validated before and
    // after every read so a retargeted symlink or replacement file cannot be
    // committed under a stale manifest (AD-4/AD-34).
    let mut staged = Vec::new();
    let mut source_digests = Vec::with_capacity(start_enumeration.supported.len());
    let observed_at = unix_seconds_now();
    for artifact in &start_enumeration.supported {
        ensure_not_cancelled(scan_store, scan_id)?;
        let bytes = read_verified(canonical_root, artifact)?;
        let source_revision = fnv1a_hex(&bytes);
        source_digests.push((artifact.clone(), source_revision.clone()));
        let file_locator =
            file_uri(&artifact.file.absolute_path).map_err(|_| ScanError::ParseFailed)?;
        let canonical_units = canonicalize_markdown(&bytes).map_err(|_| ScanError::ParseFailed)?;
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
                parser_version: CODEX_MARKDOWN_PARSER_VERSION.to_string(),
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
        .map_err(|_| ScanError::Internal)?;
    ensure_not_cancelled(scan_store, scan_id)?;
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
        .map_err(|_| ScanError::Internal)?;

    // Count while the generation is still staging. A database failure here
    // can still mark the run failed; no fallible work remains after CAS.
    let records_indexed = scan_store
        .count_generation_records(source_rowid, generation)
        .map_err(|_| ScanError::Internal)?;

    // --- Final manifest re-validation (AD-34/AD-36) ------------------------
    let final_enumeration = adapter
        .enumerate_artifacts(canonical_root)
        .map_err(|_| ScanError::EnumerationFailed)?;
    ensure_not_cancelled(scan_store, scan_id)?;
    let final_manifest = build_manifest(&final_enumeration.supported);
    if final_manifest != start_manifest || final_enumeration != start_enumeration {
        // Source changed during the scan: this generation is never activated.
        return Err(ScanError::DirtyAfterValidation);
    }
    // Re-read every staged source after final enumeration and compare the
    // whole-file byte digest. Metadata alone cannot detect a same-size write
    // with restored mtime; `read_verified` proves containment both before and
    // after the read so an escaping retarget is never read.
    for (artifact, expected_digest) in &source_digests {
        let final_bytes = read_verified(canonical_root, artifact)?;
        if fnv1a_hex(&final_bytes) != *expected_digest {
            return Err(ScanError::DirtyAfterValidation);
        }
    }

    // --- Commit under CAS (AD-32) ------------------------------------------
    advance(scan_store, scan_id, ScanRunState::Committing)?;
    ensure_not_cancelled(scan_store, scan_id)?;

    // A diagnostic-only observation may explain excluded artifacts but must
    // never replace a usable supported generation. This is deliberately after
    // complete enumeration equality and byte-digest validation.
    if start_enumeration.supported.is_empty() && !start_enumeration.diagnostics.is_empty() {
        let committed = scan_store
            .complete_without_activation(scan_id, fencing_token, generation, source_rowid)
            .map_err(|_| ScanError::Internal)?;
        if !committed {
            return Err(ScanError::CommitCasFailed);
        }
        let generation = active_generation.unwrap_or_else(|| generation.clone());
        let records_indexed = scan_store
            .count_active_records(source_rowid)
            .map_err(|_| ScanError::Internal)?;
        return Ok(ScanOutcome {
            source_id: source.source_id.clone(),
            scan_id,
            generation,
            records_indexed,
        });
    }
    let committed = scan_store
        .commit_cas(scan_id, fencing_token, generation, source_rowid)
        .map_err(|_| ScanError::Internal)?;
    if !committed {
        return Err(ScanError::CommitCasFailed);
    }

    Ok(ScanOutcome {
        source_id: source.source_id.clone(),
        scan_id,
        generation: generation.clone(),
        records_indexed,
    })
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
