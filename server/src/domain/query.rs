//! Search-domain DTOs and validated request vocabulary.

use serde::{Deserialize, Serialize};

use crate::domain::source::{HealthState, SourceId};

pub const DEFAULT_SEARCH_LIMIT: usize = 20;
pub const MAX_SEARCH_LIMIT: usize = 100;
pub const MAX_QUERY_BYTES: usize = 1024;
/// Cursor input is opaque application state, but it must still be bounded
/// before decoding so a loopback request cannot force an unbounded allocation.
pub const MAX_CURSOR_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    query: String,
    cursor: Option<String>,
    limit: usize,
}

impl SearchRequest {
    pub fn new(query: String, cursor: Option<String>, limit: Option<usize>) -> Result<Self, SearchRequestError> {
        let query = query.trim().to_string();
        if query.is_empty() || query.len() > MAX_QUERY_BYTES {
            return Err(SearchRequestError::Invalid);
        }
        if cursor.as_ref().is_some_and(|value| value.len() > MAX_CURSOR_BYTES) {
            return Err(SearchRequestError::Invalid);
        }
        let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT);
        if !(1..=MAX_SEARCH_LIMIT).contains(&limit) {
            return Err(SearchRequestError::Invalid);
        }
        Ok(Self { query, cursor, limit })
    }

    pub fn query(&self) -> &str { &self.query }
    pub fn cursor(&self) -> Option<&str> { self.cursor.as_deref() }
    pub fn limit(&self) -> usize { self.limit }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchRequestError { Invalid }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchEmptyState { NoMatch, SourceNotIndexed, SourceUnavailable }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    record_id: String,
    excerpt: String,
    provider: String,
    source_id: SourceId,
    native_project: Option<String>,
    native_locator: String,
    display_locator: String,
    observed_at: i64,
    coverage_level: String,
    health_state: HealthState,
}

impl SearchResult {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        record_id: String,
        excerpt: String,
        provider: String,
        source_id: SourceId,
        native_project: Option<String>,
        native_locator: String,
        display_locator: String,
        observed_at: i64,
        coverage_level: String,
        health_state: HealthState,
    ) -> Self {
        Self { record_id, excerpt, provider, source_id, native_project, native_locator, display_locator, observed_at, coverage_level, health_state }
    }

    pub fn record_id(&self) -> &str { &self.record_id }
    pub fn excerpt(&self) -> &str { &self.excerpt }
    pub fn provider(&self) -> &str { &self.provider }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPage {
    results: Vec<SearchResult>,
    next_cursor: Option<String>,
    empty_state: Option<SearchEmptyState>,
}

impl SearchPage {
    pub(crate) fn new(results: Vec<SearchResult>, next_cursor: Option<String>, empty_state: Option<SearchEmptyState>) -> Self {
        Self { results, next_cursor, empty_state }
    }

    pub fn results(&self) -> &[SearchResult] { &self.results }
    pub fn next_cursor(&self) -> Option<&str> { self.next_cursor.as_deref() }
    pub fn empty_state(&self) -> Option<SearchEmptyState> { self.empty_state.clone() }
}
