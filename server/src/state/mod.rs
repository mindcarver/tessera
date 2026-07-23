//! `state` — persisted scan runs, migration state, active generation markers.
//!
//! Phase 0 reserves the module. The concrete persistence lives alongside the
//! Derived Index (SQLite, owned by Tessera) and includes:
//!
//! - `scan_runs` state machine: `queued / running / staging / committing /
//!   succeeded / failed / retry` (AD-5/AD-16).
//! - Fencing token + generation intent for atomic compare-and-swap commit
//!   (AD-28/AD-32).
//! - Active generation marker — only a clean generation with matching source
//!   revision and fencing token may become active (AD-34/AD-36).
//!
//! All of these land in Story 1.4. Phase 0 only owns the migration meta row
//! (see [`crate::index::migrations`]).
