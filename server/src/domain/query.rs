//! Search-domain DTOs and validated request vocabulary.

use serde::{Deserialize, Serialize};

use crate::domain::ports::provider_adapter::ProviderMemoryType;
use crate::domain::source::{HealthState, SourceId};

pub const DEFAULT_SEARCH_LIMIT: usize = 20;
pub const MAX_SEARCH_LIMIT: usize = 100;
pub const MAX_QUERY_BYTES: usize = 1024;
/// Cursor input is opaque application state, but it must still be bounded
/// before decoding so a loopback request cannot force an unbounded allocation.
pub const MAX_CURSOR_BYTES: usize = 16 * 1024;
/// Upper bound on the textual filter values (`native_project`, the reserved
/// `tessera_project`) before they reach the SQL layer. Mirrors the bound style
/// used for `MAX_QUERY_BYTES` / `MAX_CURSOR_BYTES`: a loopback request cannot
/// force an unbounded allocation or SQL parameter.
pub const MAX_FILTER_BYTES: usize = 1024;
/// Upper bound on a `since` value. Unix-epoch seconds for the year 9999 are
/// well below 2^53; this just rejects absurd values without inventing a date
/// library dependency. `since >= 0` is the lower bound (Story 2.4 spec).
pub const MAX_SINCE: i64 = 999_999_999_999;

/// Stable lowercase provider ids recognized by the filter vocabulary (Story
/// 2.4). This is an **explicit allowlist** kept in sync with the registered
/// adapters' `PROVIDER_ID` constants (`CodexAdapter::PROVIDER_ID`,
/// `ClaudeCodeAdapter::PROVIDER_ID`). It is a frozen literal rather than
/// derived from the adapter registry because `domain` must not depend on
/// `adapters` (hexagonal rule); a test (`known_provider_ids_match_registered_adapters`)
/// fails if the two drift apart. Validation is a single source of truth here so
/// `SearchRequest::new` rejects an unknown `provider` with a structured
/// `bad_request` instead of letting the SQL silently return zero rows.
pub const KNOWN_PROVIDER_IDS: &[&str] = &["codex", "claude_code"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchRequest {
    query: String,
    cursor: Option<String>,
    limit: usize,
    /// Story 2.4 cross-provider filters. `None` everywhere == today's default
    /// (full confirmed-source scope). Each `Some` narrows the WHERE with AND.
    provider: Option<String>,
    /// Per-source filter (Spec Change Log 2026-07-25): narrows to one specific
    /// confirmed source's `src_<n>` rowid, distinct from the coarser provider
    /// filter (a provider may own several confirmed sources, e.g. several Claude
    /// projects). AND-combined with the other dimensions.
    source: Option<SourceId>,
    memory_type: Option<ProviderMemoryType>,
    native_project: Option<String>,
    /// Absolute Unix-epoch seconds; the server stays stateless and never
    /// computes "now - N days" (Design Notes — client-side presets → absolute
    /// seconds). Predicate is `observed_at >= since`.
    since: Option<i64>,
    /// Reserved for Epic 5 (Tessera Project projection). Accepted on the wire
    /// and DTO, **ignored at the SQL layer** (no schema column, no predicate)
    /// so Epic 5 can fill it without a contract change here.
    tessera_project: Option<String>,
}

impl SearchRequest {
    pub fn new(query: String, cursor: Option<String>, limit: Option<usize>) -> Result<Self, SearchRequestError> {
        Self::new_with_filters(query, cursor, limit, SearchFilters::default())
    }

    /// Story 2.4 — construct a `SearchRequest` carrying optional cross-provider
    /// filters. Each `Some` narrows the result set with AND; `None` everywhere
    /// restores today's default scope. Validation:
    /// - `provider` must be a known provider id (`KNOWN_PROVIDER_IDS`).
    /// - `source` must be a well-formed `src_<n>` handle (`to_rowid().is_some()`).
    ///   Whether it is a *confirmed* source is enforced by the search SQL's
    ///   `lifecycle_state = 'confirmed'` JOIN — a non-confirmed/non-existent id
    ///   simply yields no rows, which is the honest behavior.
    /// - `memory_type` is already a typed enum (validated at the
    ///   `ProviderMemoryType::parse_str` boundary in the HTTP layer).
    /// - `native_project` is trimmed and bounded by `MAX_FILTER_BYTES` (a
    ///   user-typed value with stray whitespace should not silently fail to
    ///   match); the reserved `tessera_project` is bounded by `MAX_FILTER_BYTES`.
    /// - `since` must be `>= 0` and below `MAX_SINCE` (absurd-value guard).
    pub fn new_with_filters(
        query: String,
        cursor: Option<String>,
        limit: Option<usize>,
        filters: SearchFilters,
    ) -> Result<Self, SearchRequestError> {
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
        let provider = match filters.provider {
            Some(value) => {
                if !KNOWN_PROVIDER_IDS.contains(&value.as_str()) {
                    return Err(SearchRequestError::Invalid);
                }
                Some(value)
            }
            None => None,
        };
        // Per-source filter: validate the `src_<n>` shape. The confirmed-source
        // check is the SQL layer's job (the JOIN on lifecycle_state).
        let source = match filters.source {
            Some(value) => {
                if value.to_rowid().is_none() {
                    return Err(SearchRequestError::Invalid);
                }
                Some(value)
            }
            None => None,
        };
        // native_project is user-typed in a free-form input, so trim whitespace
        // and drop an all-whitespace value (it would never match). Bounded so a
        // loopback request cannot push an unbounded string into SQL.
        let native_project = filters
            .native_project
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(ref value) = native_project {
            if value.len() > MAX_FILTER_BYTES {
                return Err(SearchRequestError::Invalid);
            }
        }
        if let Some(ref value) = filters.tessera_project {
            if value.is_empty() || value.len() > MAX_FILTER_BYTES {
                return Err(SearchRequestError::Invalid);
            }
        }
        let since = match filters.since {
            Some(value) if (0..=MAX_SINCE).contains(&value) => Some(value),
            Some(_) => return Err(SearchRequestError::Invalid),
            None => None,
        };
        Ok(Self {
            query,
            cursor,
            limit,
            provider,
            source,
            memory_type: filters.memory_type,
            native_project,
            since,
            tessera_project: filters.tessera_project,
        })
    }

    pub fn query(&self) -> &str { &self.query }
    pub fn cursor(&self) -> Option<&str> { self.cursor.as_deref() }
    pub fn limit(&self) -> usize { self.limit }
    /// Story 2.4 filter accessors. `None` means "no predicate" (the dimension
    /// is unfiltered); `Some` narrows the WHERE with AND.
    pub fn provider(&self) -> Option<&str> { self.provider.as_deref() }
    /// Per-source filter handle (`src_<n>`), or `None` for no source predicate.
    pub fn source(&self) -> Option<&SourceId> { self.source.as_ref() }
    pub fn memory_type(&self) -> Option<ProviderMemoryType> { self.memory_type }
    pub fn native_project(&self) -> Option<&str> { self.native_project.as_deref() }
    pub fn since(&self) -> Option<i64> { self.since }
    /// Reserved for Epic 5 — accepted but never produces a SQL predicate.
    pub fn tessera_project(&self) -> Option<&str> { self.tessera_project.as_deref() }
}

/// Story 2.4 — optional cross-provider filter values carried into
/// [`SearchRequest::new_with_filters`]. `Default::default()` produces an
/// all-`None` filter set (the 2.3 default scope) so callers that have no
/// filters to apply name it concisely.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchFilters {
    pub provider: Option<String>,
    /// Per-source filter (Spec Change Log 2026-07-25): narrows to one specific
    /// confirmed source's `src_<n>` rowid. Distinct from the provider filter.
    pub source: Option<SourceId>,
    pub memory_type: Option<ProviderMemoryType>,
    pub native_project: Option<String>,
    pub since: Option<i64>,
    /// Reserved for Epic 5 (Tessera Project projection); accepted, ignored at
    /// the SQL layer.
    pub tessera_project: Option<String>,
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
