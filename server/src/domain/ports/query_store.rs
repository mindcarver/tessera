//! `domain::ports::query_store` — read side of the Tessera Derived Index.
//!
//! Bounded, read-only search port over the Derived Index.

use crate::domain::query::{BrowseRequest, SearchRequest, SearchResult};

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

/// Browse cursor key (Story 3.1). Browse drops the `instr` predicate and the
/// `title_match` rank from search's relevance key, so the cursor carries only
/// the remaining three ORDER BY components: `observed_at DESC`,
/// `coverage_full`, `record_id ASC`. Mirrors [`SearchCursorKey`] exactly
/// minus the `title_match` field so the next-page predicate can perform the
/// same lexicographic "strictly-after" comparison and pagination stays stable
/// without a query-bound component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseCursorKey {
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

    /// Story 3.1 — Browse a single confirmed source's active generation. The
    /// request is scoped to one `source_id` (validated as a well-formed
    /// `src_<n>` upstream; the confirmed-source check is the SQL layer's
    /// `lifecycle_state = 'confirmed'` JOIN). Sort is the query-less form of
    /// search's key: `observed_at DESC → coverage_full → record_id ASC` (no
    /// `title_match`). The `after` cursor predicate mirrors the ORDER BY
    /// exactly so pagination stays stable across the three sort keys.
    fn browse_records(
        &self,
        request: &BrowseRequest,
        after: Option<&BrowseCursorKey>,
    ) -> rusqlite::Result<Vec<SearchResult>>;

    /// A deterministic digest of the current confirmed/active scope. It is
    /// state detection only: cursor clients cannot select any source or
    /// generation from it.
    fn current_index_revision(&self) -> rusqlite::Result<String>;
}
