//! `domain::ports::index_store` — write side of the Tessera Derived Index.
//!
//! Phase 0 declares the location only. Concrete write port (canonical record
//! upsert, FTS5 index write, staging generation commit/abort, scan_runs
//! state machine persistence) lands in Stories 1.4/1.5 alongside AD-5/AD-16/
//! AD-28/AD-32/AD-34/AD-36.

/// Placeholder. Concrete trait body lands in Story 1.4 (scan pipeline).
pub trait IndexStore: std::fmt::Debug {}
