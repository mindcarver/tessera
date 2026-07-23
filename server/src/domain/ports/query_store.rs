//! `domain::ports::query_store` — read side of the Tessera Derived Index.
//!
//! Phase 0 declares the location only. Concrete read port
//! (BrowsePage/SearchPage with `cursor + limit`, stable sort, EmptyState
//! enum, snapshot-at-validation tokens per AD-23/AD-26/AD-31) lands in
//! Story 1.6.

/// Placeholder. Concrete trait body lands in Story 1.6 (search & browse).
pub trait QueryStore: std::fmt::Debug {}
