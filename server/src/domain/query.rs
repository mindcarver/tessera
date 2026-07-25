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
    /// Whether `instr(title, query) > 0` at search time. Non-serialized: it is
    /// a search-internal sort-key component used by the application layer to
    /// encode the relevance-bound cursor (Story 2.3). It never crosses the
    /// wire — `#[serde(skip)]` keeps it out of the versioned DTO while still
    /// allowing `Serialize`/`Deserialize` derives for the rest of the struct.
    #[serde(skip)]
    title_match: bool,
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
        title_match: bool,
    ) -> Self {
        Self { record_id, excerpt, provider, source_id, native_project, native_locator, display_locator, observed_at, coverage_level, health_state, title_match }
    }

    pub fn record_id(&self) -> &str { &self.record_id }
    pub fn excerpt(&self) -> &str { &self.excerpt }
    pub fn provider(&self) -> &str { &self.provider }
    pub fn observed_at(&self) -> i64 { self.observed_at }
    pub fn coverage_level(&self) -> &str { &self.coverage_level }
    /// Search-internal relevance sort-key component (Story 2.3). `true` when
    /// the record's title contained the query substring at search time.
    pub fn title_match(&self) -> bool { self.title_match }
}

/// Per-source availability snapshot carried alongside every search response
/// (Story 2.3, FR-14 prototype). Each entry describes one confirmed source's
/// ability to answer this query, derived from `health_state` + active-
/// generation presence + `latest_run` state. A down source's already-indexed
/// records (if any) are **not** suppressed — the sidecar flag is informational
/// so the UI can surface a partial-unavailability banner without hiding data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceQueryStatus {
    pub source_id: SourceId,
    pub provider: String,
    pub native_project: Option<String>,
    /// `available` / `degraded` / `unavailable` — see [`SourceQueryStatusKind`].
    pub status: SourceQueryStatusKind,
}

/// Availability of a confirmed source for a given query (Story 2.3 I/O
/// matrix). Snake_case on the wire; the TS mirror in `src/api/search.ts` must
/// match exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceQueryStatusKind {
    /// Healthy (or never-probed `unknown`) with an active generation that
    /// completed successfully.
    Available,
    /// `health_state = degraded` or `error` while an active generation still
    /// serves records, or the latest run failed — the source has issues but
    /// its prior records remain queryable.
    Degraded,
    /// The source has no active generation (never scanned / failed scan with
    /// no prior generation), so it contributes no records to this query. An
    /// `Error` source WITH an active generation is [`Degraded`] instead.
    Unavailable,
}

impl SourceQueryStatusKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceQueryStatusKind::Available => "available",
            SourceQueryStatusKind::Degraded => "degraded",
            SourceQueryStatusKind::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchPage {
    results: Vec<SearchResult>,
    next_cursor: Option<String>,
    empty_state: Option<SearchEmptyState>,
    /// FR-14 per-query sidecar: one row per confirmed source describing its
    /// availability for this query. Present on every page (not only the empty
    /// case) so the UI can render the partial-unavailability banner without
    /// cross-referencing inventory.
    sources: Vec<SourceQueryStatus>,
}

impl SearchPage {
    pub(crate) fn new(
        results: Vec<SearchResult>,
        next_cursor: Option<String>,
        empty_state: Option<SearchEmptyState>,
        sources: Vec<SourceQueryStatus>,
    ) -> Self {
        Self { results, next_cursor, empty_state, sources }
    }

    pub fn results(&self) -> &[SearchResult] { &self.results }
    pub fn next_cursor(&self) -> Option<&str> { self.next_cursor.as_deref() }
    pub fn empty_state(&self) -> Option<SearchEmptyState> { self.empty_state.clone() }
    pub fn sources(&self) -> &[SourceQueryStatus] { &self.sources }
}
