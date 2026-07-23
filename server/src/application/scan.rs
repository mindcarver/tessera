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
//! 4. Per file: read bytes, compute content hash, stage a file-level record.
//! 5. **Final manifest re-validation** before commit: re-enumerate and compare
//!    against the start manifest (`snapshot-at-validation`). A drift marks the
//!    run `failed` with `error_code='dirty_after_validation'`; its generation
//!    is never activated (AD-36).
//! 6. `committing` → [`ScanStore::commit_cas`]. A lost CAS returns
//!    `scan_failed` without re-marking the run (left for boot recovery).
//!
//! Zero-write invariant (NFR-1/SM-2): the pipeline only reads Source files
//! (enumerate metadata + read bytes for hashing); it never writes to them.

use std::path::Path;

use rusqlite::Connection;

use crate::adapters::codex::CodexAdapter;
use crate::domain::scan::{
    build_record_id, fnv1a_hex, Generation, ScanError, ScanOutcome, ScanRunState, ScanStatus,
};
use crate::domain::source::{build_fingerprint, SourceId, SourceLifecycle, ROOT_KIND_DIR};
use crate::domain::ProviderAdapter;
use crate::index::scan_store::{ScanStore, StagedRecord};
use crate::index::SourceRegistry;
use crate::policy;

/// The constant `parser_version` written for every 1.4 record (spec Never:
/// file-level unit only, no section identity, no parsing semantics beyond
/// this marker).
const PARSER_VERSION_FILE_LEVEL_V1: &str = "file-level/v1";

/// The `unit_kind` for 1.4 file-level records (AD-30 baseline).
const UNIT_KIND_FILE: &str = "file";

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
    let root = policy::canonicalize_root(Path::new(&source.normalized_root_path))
        .map_err(|_| ScanError::RootInvalid)?;
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
        Ok(o) => Ok(o),
        Err(e) => {
            // A lost CAS is NOT re-marked: the run is no longer owned by this
            // holder (left in `committing` for boot recovery). Every other
            // failure marks the run failed with its error category from the
            // domain-layer vocabulary mapping (`ScanError::error_code()`).
            if !matches!(e, ScanError::CommitCasFailed) {
                let _ = scan_store.fail_run(scan_id, e.error_code());
            }
            Err(e)
        }
    }
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
    scan_store
        .set_state(scan_id, ScanRunState::Running)
        .map_err(|_| ScanError::Internal)?;

    // --- First enumeration → start manifest → UPDATE real revision ----------
    let start_units = adapter
        .enumerate_file_units(canonical_root)
        .map_err(|_| ScanError::EnumerationFailed)?;
    if start_units.is_empty()
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
    let start_manifest = build_manifest(&start_units);
    let manifest_revision = manifest_revision(&start_manifest);
    set_manifest_revision(scan_store, scan_id, &manifest_revision)?;

    scan_store
        .set_state(scan_id, ScanRunState::Staging)
        .map_err(|_| ScanError::Internal)?;

    // Per file: read bytes, hash, build a staged file-level record. The
    // enumerated canonical target and metadata are re-validated before and
    // after every read so a retargeted symlink or replacement file cannot be
    // committed under a stale manifest (AD-4/AD-34).
    let mut staged: Vec<StagedRecord> = Vec::with_capacity(start_units.len());
    for unit in &start_units {
        if !unit_matches_snapshot(canonical_root, unit) {
            return Err(ScanError::DirtyAfterValidation);
        }
        let bytes = std::fs::read(&unit.absolute_path).map_err(|_| ScanError::ReadFailed)?;
        if !unit_matches_snapshot(canonical_root, unit) {
            return Err(ScanError::DirtyAfterValidation);
        }
        let content_hash = fnv1a_hex(&bytes);
        let native_locator = format!("file://{}", unit.absolute_path.to_string_lossy());
        let record_id = build_record_id(
            &source.source_id,
            &source.provider,
            &native_locator,
            UNIT_KIND_FILE,
        );
        staged.push(StagedRecord {
            record_id,
            source_rowid,
            provider: source.provider.clone(),
            unit_kind: UNIT_KIND_FILE.to_string(),
            native_unit_id: unit.relative_path.clone(),
            native_locator,
            content_hash,
            parser_version: PARSER_VERSION_FILE_LEVEL_V1.to_string(),
        });
    }
    scan_store
        .stage_records(generation, &staged)
        .map_err(|_| ScanError::Internal)?;

    // Count while the generation is still staging. A database failure here
    // can still mark the run failed; no fallible work remains after CAS.
    let records_indexed = scan_store
        .count_generation_records(source_rowid, generation)
        .map_err(|_| ScanError::Internal)?;

    // --- Final manifest re-validation (AD-34/AD-36) ------------------------
    let final_units = adapter
        .enumerate_file_units(canonical_root)
        .map_err(|_| ScanError::EnumerationFailed)?;
    let final_manifest = build_manifest(&final_units);
    if final_manifest != start_manifest {
        // Source changed during the scan: this generation is never activated.
        return Err(ScanError::DirtyAfterValidation);
    }
    // Final enumeration establishes the file-set boundary; this additional
    // pass binds every file actually read to the same canonical target and
    // metadata snapshot before the generation can become visible.
    if start_units
        .iter()
        .any(|unit| !unit_matches_snapshot(canonical_root, unit))
    {
        return Err(ScanError::DirtyAfterValidation);
    }

    // --- Commit under CAS (AD-32) ------------------------------------------
    scan_store
        .set_state(scan_id, ScanRunState::Committing)
        .map_err(|_| ScanError::Internal)?;
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

    let latest = scan_store.latest_run(source_rowid).map_err(|_| ScanError::Internal)?;
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
fn build_manifest(units: &[crate::domain::FileUnit]) -> Vec<ManifestEntry> {
    let mut manifest: Vec<ManifestEntry> = units
        .iter()
        .map(|u| {
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
    let Some(mtime) = metadata_mtime(&metadata) else {
        return false;
    };
    metadata.is_file() && metadata.len() == unit.size && mtime == unit.mtime
}

fn metadata_mtime(metadata: &std::fs::Metadata) -> Option<i64> {
    let duration = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?;
    Some(duration.as_nanos() as i64)
}
