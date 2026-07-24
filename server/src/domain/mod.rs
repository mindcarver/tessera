//! `domain` — canonical records, ports, IDs, and state types.
//!
//! Phase 0 only declares the module surface; concrete canonical models
//! (`Source`, `CanonicalRecord`, `ScanRun`), opaque IDs (`src_`, `rec_`,
//! `proj_`), and the ProviderAdapter port land in Stories 1.2 – 1.6 per the
//! architecture spine (AD-3/AD-6/AD-15/AD-25/AD-30).
//!
//! Naming conventions (see ARCHITECTURE-SPINE "Consistency Conventions"):
//! - Domain IDs are opaque prefixed strings: `src_`, `rec_`, `proj_`.
//! - Provider names are stable lowercase ids: `codex`, `claude_code`.
//! - `record_id` is stable for the same
//!   `source_id + provider + native locator + unit kind`; content hash detects
//!   change but is not identity (AD-15/AD-30).

pub mod open;
pub mod ports;
pub mod query;
pub mod scan;
pub mod source;

// Re-export the most-used port types so application / adapter / IPC code can
// name them without a long path.
//
// Story 1.2 adds the discovery slice: `ProviderAdapter` (trait, was reserved
// in Phase 0), `CoverageLevel` (AD-3), `DiscoveryBasis` and `CandidateSource`
// (AD-4 — pre-confirmation metadata only, no source_id).
pub use ports::provider_adapter::{
    ArtifactDiagnostic, ArtifactEnumeration, CandidateSource, CoverageLevel, DiscoveryBasis,
    EnumerateError, FileUnit, ProviderAdapter, ProviderMemoryType, SupportedArtifact,
};

// Story 1.3 adds the persistent Source identity + lifecycle + fingerprint
// domain model (AD-33/AD-35). Re-exported so application / index / ipc can
// name them without a long path.
pub use open::{OpenRequest, OpenRequestError, OpenResult};

pub use source::{
    build_fingerprint, FilesystemIdentity, HealthState, Source, SourceFingerprint, SourceId,
    SourceKind, SourceLifecycle, ROOT_KIND_DIR,
};

// Story 1.4 adds the scan state machine + generation identity + DTOs + pure
// hashing helpers (AD-5/AD-15/AD-16/AD-30). Re-exported for application /
// index / ipc.
pub use scan::{
    build_record_id, fnv1a_hex, Generation, ScanError, ScanOutcome, ScanRunState, ScanStatus,
};
