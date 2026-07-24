//! `policy` — canonical paths, allowlists, capability checks, and redaction.
//!
//! Phase 0 reserved the module. Story 1.3 lands the first concrete primitive:
//! [`canonicalize_root`], the AD-4 canonical-path policy that the confirm path
//! uses to normalize a Candidate Source's root and capture its filesystem
//! identity for fingerprinting (AD-35).
//!
//! Future Story additions:
//! - Root containment check (AD-4): every read re-validates that the target is
//!   still inside the confirmed, allowlisted root. Symlink and path-traversal
//!   escape is rejected here.
//! - Capability declaration surface (AD-3): the bridge between
//!   `ProviderAdapter::coverage_level` and the behavior the application core
//!   is willing to enable.
//! - Redaction helpers for logs and diagnostics (AD-12/AD-13): the policy
//!   layer owns the rule that body, query text, and credentials never enter
//!   logs or error envelopes.

use std::io;
use std::path::{Path, PathBuf};

use crate::domain::source::FilesystemIdentity;

/// A canonicalized Source root (AD-4). The normalized path is the real on-disk
/// absolute path with symlinks resolved; the filesystem identity is captured
/// when the platform exposes it (Unix) for fingerprinting (AD-35).
#[derive(Debug, Clone)]
pub struct CanonicalRoot {
    /// Canonicalized absolute path (symlinks resolved, `.`/`..` collapsed).
    pub normalized_path: PathBuf,
    /// `(device, file_id)` identity when available. `None` on non-Unix or when
    /// the metadata read failed — fingerprinting falls back to normalized path
    /// only (AD-35 explicit fallback).
    pub identity: Option<FilesystemIdentity>,
}

/// Canonicalize a Source root (AD-4) and capture its filesystem identity
/// (AD-35).
///
/// This is the policy primitive the confirm path calls before computing a
/// fingerprint and persisting a Source row. It:
///
/// 1. Calls `std::fs::canonicalize` — resolves symlinks, collapses `.`/`..`,
///    fails if the path does not exist (NFR-5/6: confirm must fail with
///    `confirm_failed` when the root vanished between discover and confirm).
/// 2. Verifies the canonicalized path is a directory (a regular file or
///    missing path is not a usable root).
/// 3. On Unix, reads `metadata().dev()` / `metadata().ino()` for the
///    fingerprint's identity segment. Non-Unix platforms or metadata failures
///    fall back to `identity: None`, which the fingerprint encoding marks with
///    an explicit `n` segment (AD-35).
///
/// Only metadata is read — directory contents are never opened (NFR-5).
pub fn canonicalize_root(root: &Path) -> io::Result<CanonicalRoot> {
    let normalized_path = std::fs::canonicalize(root)?;
    if !normalized_path.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source root is not a directory",
        ));
    }
    let identity = read_identity(&normalized_path);
    Ok(CanonicalRoot {
        normalized_path,
        identity,
    })
}

/// Read the `(device, file_id)` identity from the canonicalized path's
/// metadata on Unix. Returns `None` on non-Unix or when metadata cannot be
/// read — the fingerprint uses the normalized-path explicit fallback in that
/// case (AD-35).
#[cfg(unix)]
fn read_identity(canonical: &Path) -> Option<FilesystemIdentity> {
    use std::os::unix::fs::MetadataExt;
    let md = canonical.metadata().ok()?;
    Some(FilesystemIdentity {
        device: md.dev(),
        file_id: md.ino(),
    })
}

#[cfg(not(unix))]
fn read_identity(_canonical: &Path) -> Option<FilesystemIdentity> {
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocatorError {
    Invalid,
}

pub fn path_from_file_uri(locator: &str) -> Result<PathBuf, LocatorError> {
    let raw = locator
        .strip_prefix("file://")
        .ok_or(LocatorError::Invalid)?;
    let path_part = raw.split_once('#').map(|(path, _)| path).unwrap_or(raw);
    if path_part.is_empty() {
        return Err(LocatorError::Invalid);
    }
    let decoded = percent_decode_uri_path(path_part)?;
    if decoded.contains('\0') {
        return Err(LocatorError::Invalid);
    }
    let path = PathBuf::from(decoded);
    if !path.is_absolute() {
        return Err(LocatorError::Invalid);
    }
    Ok(path)
}

pub fn canonical_target_within_root(root: &Path, target: &Path) -> io::Result<PathBuf> {
    let root = canonicalize_root(root)?;
    let target = std::fs::canonicalize(target)?;
    if !target.starts_with(&root.normalized_path) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "target is outside source root",
        ));
    }
    Ok(target)
}

fn percent_decode_uri_path(value: &str) -> Result<String, LocatorError> {
    let mut bytes = Vec::with_capacity(value.len());
    let raw = value.as_bytes();
    let mut index = 0;
    while index < raw.len() {
        match raw[index] {
            b'%' if index + 2 < raw.len() => {
                let hi = (raw[index + 1] as char)
                    .to_digit(16)
                    .ok_or(LocatorError::Invalid)?;
                let lo = (raw[index + 2] as char)
                    .to_digit(16)
                    .ok_or(LocatorError::Invalid)?;
                bytes.push(((hi << 4) | lo) as u8);
                index += 3;
            }
            b'%' => return Err(LocatorError::Invalid),
            byte => {
                bytes.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(bytes).map_err(|_| LocatorError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonicalize_returns_err_for_missing_path() {
        let bogus = Path::new("/this/does/not/exist/tessera-1-3");
        assert!(canonicalize_root(bogus).is_err());
    }

    #[test]
    fn canonicalize_rejects_regular_file() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let file = tmp.path().join("not_a_dir");
        std::fs::write(&file, "x").expect("write");
        let err = canonicalize_root(&file).expect_err("file is not a dir");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_returns_identity_on_unix_for_real_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let root = canonicalize_root(tmp.path()).expect("canonicalize real dir");
        assert!(root.identity.is_some(), "Unix captures (dev, ino)");
        // Normalized path must be absolute and a dir.
        assert!(root.normalized_path.is_absolute());
        assert!(root.normalized_path.is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_resolves_symlinks() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let real = tmp.path().join("real");
        let link = tmp.path().join("link");
        std::fs::create_dir_all(&real).expect("mkdir real");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let via_link = canonicalize_root(&link).expect("canonicalize symlink");
        // Canonicalize resolves the symlink: normalized_path equals real, not link.
        assert_eq!(
            via_link.normalized_path,
            std::fs::canonicalize(&real).unwrap()
        );
    }
}
