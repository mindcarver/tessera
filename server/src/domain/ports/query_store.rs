//! `domain::ports::query_store` — read side of the Tessera Derived Index.
//!
//! Bounded, read-only search port over the Derived Index.

use crate::domain::query::{SearchRequest, SearchResult};

/// Relevance-bound cursor key (Story 2.3). The search ordering is no longer a
/// single `record_id ASC`: it is the multi-key relevance ordering
/// `(title_match, observed_at DESC, coverage_full, record_id ASC)`. A cursor
/// that only stored `last_record_id` would silently skip records whose
/// `record_id` is below the cursor but whose relevance rank is worse (they
/// would never satisfy `record_id > last`). This struct carries the full sort
/// key of the last record on the previous page so the next-page predicate can
/// perform a correct lexicographic "strictly-after" comparison and pagination
/// stays stable across relevance tiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchCursorKey {
    /// `true` when the cursor record's title contained the query substring.
    pub title_match: bool,
    /// `observed_at` of the cursor record (recency tiebreak, DESC).
    pub observed_at: i64,
    /// `true` when the cursor record's `coverage_level = "full"`.
    pub coverage_full: bool,
    /// `record_id` of the cursor record (final stable tiebreak, ASC).
    pub record_id: String,
}

pub trait QueryStore: std::fmt::Debug {
    fn search_records(
        &self,
        request: &SearchRequest,
        after: Option<&SearchCursorKey>,
    ) -> rusqlite::Result<Vec<SearchResult>>;

    /// A deterministic digest of the current confirmed/active scope. It is
    /// state detection only: cursor clients cannot select any source or
    /// generation from it.
    fn current_index_revision(&self) -> rusqlite::Result<String>;
}
