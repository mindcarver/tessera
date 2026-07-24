//! `domain::ports::query_store` — read side of the Tessera Derived Index.
//!
//! Bounded, read-only search port over the Derived Index.

use crate::domain::query::{SearchRequest, SearchResult};

pub trait QueryStore: std::fmt::Debug {
    fn search_records(
        &self,
        request: &SearchRequest,
        after_record_id: Option<&str>,
    ) -> rusqlite::Result<Vec<SearchResult>>;

    /// A deterministic digest of the current confirmed/active scope. It is
    /// state detection only: cursor clients cannot select any source or
    /// generation from it.
    fn current_index_revision(&self) -> rusqlite::Result<String>;
}
