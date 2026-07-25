import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactElement, type RefObject } from "react";
import { readTesseraErrorMessage } from "../../api/errors";
import { openOriginalLocation } from "../../api/open";
import {
  KNOWN_PROVIDER_IDS,
  PROVIDER_MEMORY_TYPES,
  SEARCH_TIME_PRESETS,
  searchMemories,
  sinceFromPreset,
  type KnownProviderId,
  type ProviderMemoryType,
  type SearchEmptyState,
  type SearchFilters,
  type SearchResult,
  type SearchTimePreset,
  type SourceQueryStatus,
} from "../../api/search";
import { EmptyState } from "../../components/EmptyState";
import { LoadMore } from "../../components/LoadMore";
import { ResultCard } from "../../components/ResultCard";
import { providerDisplayName } from "../../components/providerDisplayName";

const SEARCH_PAGE_SIZE = 2;

/**
 * Story 2.4 — confirmed-source list derived from the most recent successful
 * search's sidecar, used to populate the per-source filter `<select>`. Stored
 * as state (not a ref) so updating it re-renders the controls. Persists across
 * the idle reset on filter change so the source options remain available
 * before the next search runs.
 */
interface ConfirmedSource {
  source_id: string;
  provider: string;
  native_project: string | null;
}

type State =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "loading_more"; results: SearchResult[]; cursor: string; sources: SourceQueryStatus[] }
  | { kind: "error"; message: string; results?: SearchResult[]; cursor?: string | null; sources?: SourceQueryStatus[] }
  | { kind: "stale"; results: SearchResult[]; cursor: string | null; message: string; sources: SourceQueryStatus[] }
  | { kind: "ready"; results: SearchResult[]; cursor: string | null; empty: SearchEmptyState | null; sources: SourceQueryStatus[] };

type OpenState =
  | { kind: "idle" }
  | { kind: "opening"; recordId: string }
  | { kind: "opened"; message: string }
  | { kind: "error"; message: string };

/**
 * Story 2.4 — UI-side filter state. Empty string everywhere == the 2.3 default
 * scope (all confirmed sources). Each non-empty value narrows the next search
 * with AND. Kept separate from {@link SearchFilters} so the UI can represent
 * "no filter" as "" (a natural `<select>` / `<input>` empty value) while the
 * API layer represents it as `undefined`.
 */
interface FilterState {
  provider: string;
  /** Per-source filter (Spec Change Log 2026-07-25): `src_<n>` or "". */
  source: string;
  memory_type: string;
  native_project: string;
  timePreset: string;
}

const EMPTY_FILTERS: FilterState = {
  provider: "",
  source: "",
  memory_type: "",
  native_project: "",
  timePreset: "all",
};

function isEmptyFilterState(filters: FilterState): boolean {
  return filters.provider === ""
    && filters.source === ""
    && filters.memory_type === ""
    && filters.native_project.trim() === ""
    && filters.timePreset === "all";
}

/**
 * Build the wire-level filters from the UI state plus the session-stable
 * resolved `since`. `since` is resolved ONCE on page 1 (see `resolvedSinceRef`)
 * and reused for every "Load more", so it is threaded in here rather than
 * recomputed from `timePreset` per call.
 */
function toSearchFilters(filters: FilterState, since: number | undefined): SearchFilters {
  const result: SearchFilters = {};
  if (filters.provider !== "") result.provider = filters.provider as KnownProviderId;
  if (filters.source !== "") result.source = filters.source;
  if (filters.memory_type !== "") result.memory_type = filters.memory_type as ProviderMemoryType;
  // Trim native_project so a whitespace-only value is treated as absent,
  // matching the server's trim-to-None (otherwise the UI would mark filters
  // active / show a readout / send a wire param the server drops).
  const nativeProject = filters.native_project.trim();
  if (nativeProject !== "") result.native_project = nativeProject;
  if (since !== undefined) result.since = since;
  return result;
}

export function Search(): ReactElement {
  const [query, setQuery] = useState("");
  const [filters, setFilters] = useState<FilterState>(EMPTY_FILTERS);
  const [state, setState] = useState<State>({ kind: "idle" });
  const [openState, setOpenState] = useState<OpenState>({ kind: "idle" });
  const request = useRef(0);
  const openRequest = useRef(0);
  const pendingResultFocus = useRef<number | null>(null);
  const resultList = useRef<HTMLOListElement>(null);
  const alert = useRef<HTMLParagraphElement>(null);
  /**
   * Story 2.4 — confirmed-provider list derived from the most recent
   * successful search's sidecar. Stored as state (not a ref) so updating it
   * triggers a re-render of the effective-range readout. Persists across the
   * idle reset on filter change so the readout can still name the confirmed
   * providers before the next search runs.
   */
  const [confirmedProviders, setConfirmedProviders] = useState<string[]>([]);
  /**
   * Story 2.4 — per-source options derived from the sidecar, for the source
   * `<select>`. Persists across the idle reset on filter change (mirrors
   * {@link confirmedProviders}) so the options survive between searches.
   */
  const [confirmedSources, setConfirmedSources] = useState<ConfirmedSource[]>([]);
  /**
   * Story 2.4 (Spec Change Log) — the absolute `since` resolved ONCE on page 1
   * from the time preset, and reused for every "Load more" in the session. A
   * per-page recompute would change `since` between pages, binding a different
   * value into the cursor → `cursor_stale` → "Load more" breaks under a time
   * preset. Cleared on every filter change so the next page 1 resolves fresh.
   */
  const resolvedSinceRef = useRef<number | undefined>(undefined);
  useEffect(() => {
    const firstNewResult = pendingResultFocus.current;
    if (firstNewResult === null) return;
    pendingResultFocus.current = null;
    if (state.kind === "ready") {
      resultList.current?.children.item(firstNewResult)?.querySelector<HTMLElement>("[tabindex='0']")?.focus();
    } else if (state.kind === "stale" || state.kind === "error") {
      alert.current?.focus();
    }
  }, [state]);

  const submit = useCallback((event: FormEvent) => {
    event.preventDefault();
    const id = ++request.current;
    ++openRequest.current;
    setOpenState({ kind: "idle" });
    setState({ kind: "loading" });
    // Resolve the time preset to an absolute `since` ONCE on page 1. This value
    // is reused for every "Load more" via resolvedSinceRef so the cursor's bound
    // `since` does not drift across pages (Spec Change Log — since stability).
    resolvedSinceRef.current = sinceFromPreset(filters.timePreset as SearchTimePreset | undefined);
    searchMemories(query, undefined, SEARCH_PAGE_SIZE, toSearchFilters(filters, resolvedSinceRef.current)).then((page) => {
      if (id !== request.current) return;
      const sources = page.payload.sources ?? [];
      // Harmonized with loadMore: only refresh the derived lists when the
      // sidecar is non-empty so a transient empty sidecar does not wipe a
      // known-good readout. The server always sends sources, so the guard is a
      // safety net, not a normal path.
      if (sources.length > 0) {
        setConfirmedProviders(distinctProviders(sources));
        const nextConfirmed = distinctConfirmedSources(sources);
        setConfirmedSources(nextConfirmed);
        // Patch 4 — drop a source filter whose id disappeared from the
        // sidecar (unconfirmed between searches) so it cannot linger as a
        // dead handle yielding zero rows.
        setFilters((prev) => clearDanglingSource(prev, nextConfirmed));
      }
      setState({ kind: "ready", results: page.payload.results, cursor: page.payload.next_cursor, empty: page.payload.empty_state, sources });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      setState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, [query, filters]);
  const loadMore = useCallback(() => {
    if (state.kind !== "ready" || !state.cursor) return;
    const id = ++request.current;
    pendingResultFocus.current = state.results.length;
    const priorSources = state.sources;
    setState({ kind: "loading_more", results: state.results, cursor: state.cursor, sources: priorSources });
    // Reuse the session-stable resolved `since` — do NOT recompute the preset
    // here, or a time-preset filter would break pagination.
    searchMemories(query, state.cursor, SEARCH_PAGE_SIZE, toSearchFilters(filters, resolvedSinceRef.current)).then((page) => {
      if (id !== request.current) return;
      const nextSources = page.payload.sources ?? [];
      if (nextSources.length > 0) {
        setConfirmedProviders(distinctProviders(nextSources));
        const nextConfirmed = distinctConfirmedSources(nextSources);
        setConfirmedSources(nextConfirmed);
        // Patch 4 — mirror submit: drop a source filter whose id disappeared
        // from the sidecar between pages.
        setFilters((prev) => clearDanglingSource(prev, nextConfirmed));
      }
      setState({ kind: "ready", results: [...state.results, ...page.payload.results], cursor: page.payload.next_cursor, empty: null, sources: nextSources.length > 0 ? nextSources : priorSources });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      if (hasErrorCode(error, "cursor_stale")) {
        setState({ kind: "stale", results: state.results, cursor: state.cursor, message: readTesseraErrorMessage(error), sources: priorSources });
      } else {
        setState({ kind: "error", message: readTesseraErrorMessage(error), results: state.results, cursor: state.cursor, sources: priorSources });
      }
    });
  }, [query, filters, state]);
  const openRecord = useCallback((recordId: string) => {
    const id = ++openRequest.current;
    setOpenState({ kind: "opening", recordId });
    openOriginalLocation(recordId).then(() => {
      if (id !== openRequest.current) return;
      setOpenState({ kind: "opened", message: "Opened original location." });
    }).catch((error: unknown) => {
      if (id !== openRequest.current) return;
      setOpenState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, []);
  /**
   * Story 2.4 — any filter change invalidates the current result set and
   * held cursor: `++request.current` discards any in-flight response, and
   * `setState({kind:"idle"})` clears results + cursor so the next submit runs
   * a fresh page-1 query under the new filter combination. Mirrors the
   * existing query-input change handler.
   */
  const updateFilter = useCallback((patch: Partial<FilterState>) => {
    ++request.current;
    ++openRequest.current;
    setFilters((prev) => ({ ...prev, ...patch }));
    // A filter change invalidates the session-stable `since`: clear it so the
    // next page 1 resolves a fresh window under the new filter combination.
    resolvedSinceRef.current = undefined;
    setState({ kind: "idle" });
    setOpenState({ kind: "idle" });
  }, []);
  const clearFilters = useCallback(() => {
    updateFilter(EMPTY_FILTERS);
  }, [updateFilter]);
  const filtersActive = !isEmptyFilterState(filters);
  return <section aria-label="Memory search" role="region">
    <h2>Search memories</h2>
    <form onSubmit={submit}>
      <label htmlFor="memory-search">Keyword</label>
      <input id="memory-search" value={query} onChange={(event) => { ++request.current; ++openRequest.current; setQuery(event.target.value); setState({ kind: "idle" }); setOpenState({ kind: "idle" }); }} />
      <button type="submit">Search</button>
    </form>
    {renderFilterControls(filters, updateFilter, clearFilters, filtersActive, confirmedSources)}
    <p role="status" data-testid="search-effective-range">{effectiveRangeText(filters, confirmedProviders)}</p>
    <div aria-live="polite">{renderOpenState(openState)}{renderState(state, loadMore, openRecord, openState, resultList, alert, filters, filtersActive, confirmedProviders)}</div>
  </section>;
}

/**
 * Story 2.4 — keyboard-reachable filter controls. Each control carries a
 * readable `<label>`; the Provider, Source, Memory-type, and time-preset
 * filters are `<select>`s; Native-project is a free-form `<input>`
 * (exact-match on the wire). A Clear-filters button resets every filter to its
 * default. A disabled Tessera-project slot is rendered (reserved for Epic 5 —
 * the spec requires it "shown disabled in the UI", not merely absent). Changing
 * any control resets the result state to idle (clears any held cursor) so the
 * next submit is a fresh page-1 query under the new filter combination.
 */
function renderFilterControls(
  filters: FilterState,
  updateFilter: (patch: Partial<FilterState>) => void,
  clearFilters: () => void,
  filtersActive: boolean,
  confirmedSources: ConfirmedSource[],
): ReactElement {
  return <fieldset aria-label="Search filters">
    <legend>Filter memories</legend>
    <label htmlFor="memory-filter-provider">Provider</label>
    <select
      id="memory-filter-provider"
      value={filters.provider}
      onChange={(event) => updateFilter({ provider: event.target.value })}
    >
      <option value="">All providers</option>
      {KNOWN_PROVIDER_IDS.map((id) => <option key={id} value={id}>{providerDisplayName(id)}</option>)}
    </select>
    <label htmlFor="memory-filter-source">Source</label>
    <select
      id="memory-filter-source"
      value={filters.source}
      onChange={(event) => updateFilter({ source: event.target.value })}
    >
      <option value="">All sources</option>
      {confirmedSources
        // Patch 3 — scope the Source options by the active provider filter.
        // Without this, `provider=codex` + `source=src_2` (a Claude source)
        // is offered and always yields zero rows. Only show sources whose
        // provider matches when a provider filter is set.
        .filter((src) => filters.provider === "" || src.provider === filters.provider)
        .map((src) => (
          <option key={src.source_id} value={src.source_id}>{sourceLabel(src)}</option>
        ))}
    </select>
    <label htmlFor="memory-filter-type">Memory type</label>
    <select
      id="memory-filter-type"
      value={filters.memory_type}
      onChange={(event) => updateFilter({ memory_type: event.target.value })}
    >
      <option value="">All types</option>
      {PROVIDER_MEMORY_TYPES.map((id) => <option key={id} value={id}>{id}</option>)}
    </select>
    <label htmlFor="memory-filter-project">Native project</label>
    <input
      id="memory-filter-project"
      type="text"
      placeholder="Exact native project id"
      value={filters.native_project}
      onChange={(event) => updateFilter({ native_project: event.target.value })}
    />
    <label htmlFor="memory-filter-time">Observed</label>
    <select
      id="memory-filter-time"
      value={filters.timePreset}
      onChange={(event) => updateFilter({ timePreset: event.target.value })}
    >
      <option value="all">All time</option>
      {SEARCH_TIME_PRESETS.filter((preset) => preset !== "all").map((preset) => (
        <option key={preset} value={preset}>Last {preset === "7d" ? "7" : "30"} days</option>
      ))}
    </select>
    {/*
      Story 2.4 — reserved Tessera-project slot, rendered DISABLED (not merely
      absent) per the spec Boundaries/I/O matrix/AC. Epic 5 fills this without a
      contract change; until then it is visibly reserved so the user understands
      the dimension exists but is not yet active.
    */}
    <label htmlFor="memory-filter-tessera-project">Tessera project (reserved)</label>
    <select id="memory-filter-tessera-project" disabled aria-disabled="true">
      <option value="">Reserved for a future release</option>
    </select>
    <button type="button" onClick={clearFilters} disabled={!filtersActive}>Clear filters</button>
  </fieldset>;
}

/**
 * Story 2.4 — label for a confirmed-source `<option>`. Names the source id and
 * its provider (and native project when present) so several sources under one
 * provider are distinguishable (the source filter's whole purpose).
 */
function sourceLabel(src: ConfirmedSource): string {
  const project = src.native_project ? `, ${src.native_project}` : "";
  return `${src.source_id} (${providerDisplayName(src.provider)}${project})`;
}

/**
 * Story 2.4 — distinct confirmed sources from a sidecar, preserving first-seen
 * order and deduplicating by `source_id` so the source `<select>` has one
 * stable entry per source.
 */
function distinctConfirmedSources(sources: SourceQueryStatus[]): ConfirmedSource[] {
  const seen = new Set<string>();
  const out: ConfirmedSource[] = [];
  for (const src of sources) {
    if (seen.has(src.source_id)) continue;
    seen.add(src.source_id);
    out.push({ source_id: src.source_id, provider: src.provider, native_project: src.native_project });
  }
  return out;
}

/**
 * Patch 4 — drop a dangling source filter. If the selected `source` id is no
 * longer among the confirmed sources (e.g. unconfirmed between searches), the
 * filter yields zero rows and (combined with a provider filter) names an
 * unreachable combination, so clear it when the sidecar updates. Returns the
 * unchanged state when the source is still present or no source filter is set.
 */
function clearDanglingSource(prev: FilterState, confirmed: ConfirmedSource[]): FilterState {
  if (prev.source !== "" && !confirmed.some((src) => src.source_id === prev.source)) {
    return { ...prev, source: "" };
  }
  return prev;
}

/**
 * Story 2.4 — extract the distinct provider list from a sidecar, preserving
 * the {@link KNOWN_PROVIDER_IDS} declaration order so the readout is stable
 * across renders (Codex before Claude Code).
 */
function distinctProviders(sources: SourceQueryStatus[]): string[] {
  const present = new Set(sources.map((source) => source.provider));
  return KNOWN_PROVIDER_IDS.filter((id) => present.has(id));
}

/**
 * Story 2.4 — the effective-range readout. States the currently-applied scope
 * in plain text, derived from the active filter inputs + the confirmed
 * providers captured from the most recent successful search sidecar (persists
 * across the idle reset on filter change via a ref). Examples:
 * - "Codex + Claude Code" — no filter, both providers confirmed.
 * - "Codex, type=memory, last 7d" — provider + memory-type + time filters set.
 * - "All confirmed sources" — no search has run yet, so no sidecar.
 */
function effectiveRangeText(filters: FilterState, confirmedProviders: string[]): string {
  const providerText = filters.provider
    ? providerDisplayName(filters.provider)
    : (confirmedProviders.length > 0
      ? confirmedProviders.map(providerDisplayName).join(" + ")
      : "All confirmed sources");
  const parts: string[] = [providerText];
  if (filters.source) parts.push(`source=${filters.source}`);
  if (filters.memory_type) parts.push(`type=${filters.memory_type}`);
  // native_project is trimmed before display so a whitespace-only value does
  // not light up the readout (matches the server's trim-to-None).
  const nativeProject = filters.native_project.trim();
  if (nativeProject) {
    // Escape any double-quote in the project id so it cannot break the
    // readout's quoting or confuse a screen reader; replace with a visually-
    // similar single quote rather than inventing an HTML-escaping dependency.
    parts.push(`project="${nativeProject.replace(/"/g, "'")}"`);
  }
  if (filters.timePreset === "7d") parts.push("last 7d");
  else if (filters.timePreset === "30d") parts.push("last 30d");
  return parts.join(", ");
}

function renderState(
  state: State,
  loadMore: () => void,
  openRecord: (recordId: string) => void,
  openState: OpenState,
  resultList: RefObject<HTMLOListElement | null>,
  alert: RefObject<HTMLParagraphElement | null>,
  filters: FilterState,
  filtersActive: boolean,
  confirmedProviders: string[],
): ReactElement | null {
  const openingId = openingRecordId(openState);
  if (state.kind === "idle") return null;
  if (state.kind === "loading") return <p>Searching indexed memories…</p>;
  // Every remaining state carries results + the most recent sidecar sources,
  // so the partial-unavailability banner can render hoisted and survive
  // loading_more/error/stale transitions without flickering out. It is
  // suppressed entirely in the empty state (the empty-state copy is
  // authoritative there, including `source_not_indexed` for never-scanned
  // sources — the banner must not fabricate "absent from these results").
  if (state.kind === "loading_more") return <>{partialUnavailableBanner(state.sources, true)}{renderResults(state.results, state.cursor, loadMore, openRecord, openingId, resultList, true)}<p>Loading more results…</p></>;
  if (state.kind === "error") return <>{partialUnavailableBanner(state.sources ?? [], Boolean(state.results))}{state.results ? renderResults(state.results, null, loadMore, openRecord, openingId, resultList) : null}<p ref={alert} tabIndex={-1} role="alert">{state.message}</p></>;
  if (state.kind === "stale") return <>{partialUnavailableBanner(state.sources, true)}<p ref={alert} tabIndex={-1} role="alert">{state.message}</p>{renderResults(state.results, null, loadMore, openRecord, openingId, resultList)}</>;
  // ready
  if (state.empty) return <EmptyState message={emptyCopy(state.empty, filters, filtersActive, confirmedProviders)} />;
  return <>{partialUnavailableBanner(state.sources, state.results.length > 0)}{renderResults(state.results, state.cursor, loadMore, openRecord, openingId, resultList)}</>;
}

/**
 * FR-14 partial-unavailability banner — surface any confirmed source whose
 * status is not `available` while OTHER sources still answer. Informational
 * only: a down source's already-indexed records (if any) still render above.
 *
 * Render rules (patch 4): the banner appears ONLY for genuine partial
 * unavailability — `hasResults` is true (non-empty result set) AND ≥1 sidecar
 * source is non-`available`. It is hoisted above the per-state branches so it
 * does not flicker away during `loading_more`/`error`/`stale`, and it is
 * suppressed entirely when the empty-state copy is authoritative (no results to
 * be "absent from"). Keyboard-reachable via the polite `aria-live` region the
 * search output already lives in.
 *
 * Story 2.4: the sidecar stays UNFILTERED — it reports availability of all
 * confirmed sources regardless of result filters (Design Notes: availability
 * info, not result info).
 */
function partialUnavailableBanner(sources: SourceQueryStatus[], hasResults: boolean): ReactElement | null {
  if (!hasResults) return null;
  const flagged = sources.filter((source) => source.status !== "available");
  if (flagged.length === 0) return null;
  return <p role="status" data-testid="search-source-status">
    {flagged.map((source, index) => {
      const label = source.status === "unavailable"
        ? `Source ${source.provider} was unreachable at last scan; its memories may be absent from these results.`
        : `Source ${source.provider} is degraded; its prior memories still appear but may be stale.`;
      return <span key={source.source_id}>{index > 0 ? " " : ""}{label}</span>;
    })}
  </p>;
}

function renderResults(results: SearchResult[], cursor: string | null, loadMore: () => void, openRecord: (recordId: string) => void, openingRecordId: string | null, resultList: RefObject<HTMLOListElement | null>, loadingMore = false): ReactElement {
  // Story 3.1 — the result card, Provenance `<dl>`, Coverage/Health readout,
  // and Load-more button are shared with Browse via `src/components/`. Search
  // stays the canonical consumer of these components; the extraction does not
  // change the wire shape or the existing accessibility contract.
  return <><p>{results.length} result{results.length === 1 ? "" : "s"}.</p><ol ref={resultList}>{results.map((result) => (
    <ResultCard
      key={result.record_id}
      result={result}
      onOpen={openRecord}
      openInFlight={openingRecordId === result.record_id}
    />
  ))}</ol>{cursor ? <LoadMore onClick={loadMore} disabled={loadingMore} /> : null}</>;
}

function renderOpenState(state: OpenState): ReactElement | null {
  if (state.kind === "idle") return null;
  if (state.kind === "opening") return <p>Opening original location…</p>;
  if (state.kind === "opened") return <p>{state.message}</p>;
  return <p role="alert">{state.message}</p>;
}

function openingRecordId(state: OpenState): string | null {
  return state.kind === "opening" ? state.recordId : null;
}

function hasErrorCode(error: unknown, code: string): boolean {
  return Boolean(error && typeof error === "object" && "code" in error && (error as { code?: unknown }).code === code);
}

/**
 * Story 2.4 — empty-state copy. When filters are active and yield zero results,
 * the copy does NOT blame the keyword: it names the active-filter situation
 * (Spec Change Log — filter-aware empty states) so the user understands the
 * filters narrowed the set to nothing, rather than the keyword being a dead
 * end. `source_not_indexed` / `source_unavailable` are not keyword-blaming, so
 * they are unaffected by the filter-awareness guard.
 *
 * Patch 7 — `confirmedProviders` is threaded in (previously `[]`) so the
 * filter-aware readout names the known providers instead of the generic "All
 * confirmed sources" fallback.
 */
function emptyCopy(state: SearchEmptyState, filters: FilterState, filtersActive: boolean, confirmedProviders: string[]): string {
  switch (state) {
    case "no_match":
      // A filter-induced zero-result set must not claim the keyword matched
      // nothing — the filters are the cause. Name them.
      if (filtersActive) {
        const active = effectiveRangeText(filters, confirmedProviders);
        return `No indexed memory matched within the active filters (${active}).`;
      }
      return "No indexed memory matched this keyword.";
    case "source_not_indexed": return "Confirmed sources have not been indexed yet.";
    case "source_unavailable": return "A confirmed source is currently unavailable; its stored health was not changed.";
  }
}
