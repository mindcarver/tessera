/**
 * Tessera — query-less Browse view (Story 3.1 + Story 3.2).
 *
 * The no-query browse entry surface: enter from a Source Inventory card,
 * fetch `browseMemories(sourceId, memoryType?, cursor?, limit?)`, render the
 * shared `ResultCard` list + `EmptyState` + `LoadMore`, with `aria-live`
 * status and a "Back to inventory" button. Browse reuses Search's result-card
 * / Provenance / Coverage / Health / EmptyState / pagination components
 * verbatim (Epic 3 Boundaries: "reuse, do not re-implement").
 *
 * Story 3.2 adds the one in-source filter dimension that genuinely varies
 * within a single source (`memory_type`), and surfaces the existing
 * `observed_at DESC` ordering in the UI copy as "Recent scan first" (scan
 * recency — never implying content-change tracking; AD-7 no-disguise).
 *
 * Accessibility (AD-21):
 * - The view is keyboard-reachable from the Inventory "Browse" button.
 * - The filter `<select>` is inside a `<fieldset aria-label="Browse filters">`
 *   with a `<label>`, keyboard-operable via `selectOption`.
 * - The result list is an `<ol>` so the screen-reader semantics mirror
 *   Search's.
 * - `aria-live="polite"` on the status region announces load / error / empty
 *   transitions without moving focus.
 * - The "Back to inventory" button is the first focusable element so a
 *   keyboard user can leave the view without tabbing through results.
 */

import { useCallback, useEffect, useRef, useState, type ReactElement, type RefObject } from "react";
import { readTesseraErrorMessage } from "../../api/errors";
import { openOriginalLocation } from "../../api/open";
import { browseMemories, PROVIDER_MEMORY_TYPES, type BrowseEmptyState, type ProviderMemoryType, type SearchResult, type SourceQueryStatus } from "../../api/browse";
import { EmptyState } from "../../components/EmptyState";
import { LoadMore } from "../../components/LoadMore";
import { ResultCard } from "../../components/ResultCard";
import { providerDisplayName } from "../../components/providerDisplayName";

const BROWSE_PAGE_SIZE = 20;

interface BrowseProps {
  /** `src_<n>` handle of the confirmed source to browse. */
  sourceId: string;
  /**
   * Display name of the source's provider, used in the heading so the user
   * can tell which source they are browsing. The parent already knows this
   * from the Inventory row.
   */
  providerLabel: string;
  /** Native project label when known, for the heading sub-line. */
  nativeProject: string | null;
  /** Return to the Source Inventory. Keyboard-reachable (first focusable). */
  onBack: () => void;
}

type State =
  | { kind: "loading" }
  | { kind: "loading_more"; results: SearchResult[]; cursor: string; sources: SourceQueryStatus[] }
  | { kind: "error"; message: string; results?: SearchResult[]; cursor?: string | null; sources?: SourceQueryStatus[] }
  | { kind: "stale"; results: SearchResult[]; cursor: string | null; message: string; sources: SourceQueryStatus[] }
  | { kind: "ready"; results: SearchResult[]; cursor: string | null; empty: BrowseEmptyState | null; sources: SourceQueryStatus[] };

type OpenState =
  | { kind: "idle" }
  | { kind: "opening"; recordId: string }
  | { kind: "opened"; message: string }
  | { kind: "error"; message: string };

export function Browse({ sourceId, providerLabel, nativeProject, onBack }: BrowseProps): ReactElement {
  const [state, setState] = useState<State>({ kind: "loading" });
  const [openState, setOpenState] = useState<OpenState>({ kind: "idle" });
  /**
   * Story 3.2 — the in-effect memory-type filter. Empty string == "All"
   * (3.1's default scope); a non-empty value (one of
   * `PROVIDER_MEMORY_TYPES`) narrows the browse WHERE with AND. Kept as a
   * string (not `ProviderMemoryType | undefined`) so the `<select>`'s empty
   * option is its natural zero value, mirroring Search's filter-state shape.
   */
  const [memoryType, setMemoryType] = useState<string>("");
  // Monotonic request id so an in-flight response that arrives after a newer
  // request is discarded (mirrors Search's pattern).
  const request = useRef(0);
  const openRequest = useRef(0);
  const alert = useRef<HTMLParagraphElement>(null);

  // Story 3.2 — the typed filter value passed to the API. Resolved from the
  // string state so the `<select>` can carry "" (no filter) naturally while
  // the API carries `undefined` (mirrors Search's `toSearchFilters`).
  const typedMemoryType: ProviderMemoryType | undefined =
    memoryType === "" ? undefined : (memoryType as ProviderMemoryType);

  // Page 1 fetch: re-run when the source id OR the memory_type filter changes.
  // A filter change clears results + cursor (mirrors Search's `++request` +
  // `idle` reset), so the next fetch is page 1 under the new filter
  // combination — the cursor's bound memory_type would otherwise mismatch
  // and surface `cursor_stale`.
  useEffect(() => {
    const id = ++request.current;
    ++openRequest.current;
    setOpenState({ kind: "idle" });
    setState({ kind: "loading" });
    browseMemories(sourceId, typedMemoryType, undefined, BROWSE_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      const sources = page.payload.sources ?? [];
      setState({
        kind: "ready",
        results: page.payload.results,
        cursor: page.payload.next_cursor,
        empty: page.payload.empty_state,
        sources,
      });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      setState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, [sourceId, typedMemoryType]);

  // Move focus to the alert on error/stale so screen-reader users hear the
  // transition without losing their place in the list.
  useEffect(() => {
    if (state.kind === "stale" || state.kind === "error") {
      alert.current?.focus();
    }
  }, [state]);

  const loadMore = useCallback(() => {
    if (state.kind !== "ready" || !state.cursor) return;
    const id = ++request.current;
    const priorSources = state.sources;
    setState({ kind: "loading_more", results: state.results, cursor: state.cursor, sources: priorSources });
    browseMemories(sourceId, typedMemoryType, state.cursor, BROWSE_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      const nextSources = page.payload.sources ?? [];
      setState({
        kind: "ready",
        results: [...state.results, ...page.payload.results],
        cursor: page.payload.next_cursor,
        // Empty state is computed only on page 1 — never carry it onto a
        // continuation page even if the server were to (defensively) send one.
        empty: null,
        sources: nextSources.length > 0 ? nextSources : priorSources,
      });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      if (hasErrorCode(error, "cursor_stale")) {
        setState({ kind: "stale", results: state.results, cursor: state.cursor, message: readTesseraErrorMessage(error), sources: priorSources });
      } else {
        setState({ kind: "error", message: readTesseraErrorMessage(error), results: state.results, cursor: state.cursor, sources: priorSources });
      }
    });
  }, [sourceId, typedMemoryType, state]);

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

  // A "Restart from the new snapshot" affordance for the stale-cursor case:
  // re-run page 1 under the current generation (the UI's existing
  // cursor_stale recovery path).
  const restartFromFreshSnapshot = useCallback(() => {
    const id = ++request.current;
    setState({ kind: "loading" });
    browseMemories(sourceId, typedMemoryType, undefined, BROWSE_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      const sources = page.payload.sources ?? [];
      setState({
        kind: "ready",
        results: page.payload.results,
        cursor: page.payload.next_cursor,
        empty: page.payload.empty_state,
        sources,
      });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      setState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, [sourceId, typedMemoryType]);

  const subheading = nativeProject
    ? `${providerLabel} · ${nativeProject}`
    : providerLabel;

  return (
    <section aria-label="Memory browse" role="region">
      <h2>Browse memories</h2>
      <p aria-live="polite">{subheading}</p>
      <button type="button" onClick={onBack}>Back to inventory</button>
      {renderFilterControls(memoryType, setMemoryType)}
      {/*
        Story 3.2 — surface the existing `observed_at DESC` ordering in the UI
        copy as "Recent scan first". The label names SCAN recency explicitly
        so it never implies content-change tracking (AD-7: never disguise
        Derived-Index state as source-data state). It is a label only — there
        is no new sort or data path. `observed_at` is set once per scan
        (scan.rs:330) and is constant across a source's active generation, so
        a time/date filter would be degenerate (always "all"); the readout is
        the only honest way to communicate "recent".
      */}
      <p role="status" data-testid="browse-effective-order">Recent scan first</p>
      <div aria-live="polite">
        {renderOpenState(openState)}
        {renderState(state, loadMore, openRecord, restartFromFreshSnapshot, alert, openState.kind === "opening" ? openState.recordId : null)}
      </div>
    </section>
  );
}

/**
 * Story 3.2 — keyboard-reachable memory-type filter, mirroring Search 2.4's
 * inline `<select>` pattern (consistency over extraction — Search's filter UI
 * was intentionally kept inline per the 2.4 survey). The control carries a
 * readable `<label>` and lives inside a `<fieldset aria-label="Browse
 * filters">` so the keyboard-reachable contract is explicit. Options come
 * from the shared `PROVIDER_MEMORY_TYPES` vocabulary so the two surfaces
 * cannot drift.
 *
 * A filter change is the `setMemoryType` setter — the parent's
 * `useEffect([sourceId, typedMemoryType])` re-fetches page 1 under the new
 * filter, mirroring Search's `++request.current + idle reset` pattern.
 */
function renderFilterControls(
  memoryType: string,
  setMemoryType: (next: string) => void,
): ReactElement {
  return (
    <fieldset aria-label="Browse filters">
      <legend>Filter memories</legend>
      <label htmlFor="browse-filter-type">Memory type</label>
      <select
        id="browse-filter-type"
        value={memoryType}
        onChange={(event) => setMemoryType(event.target.value)}
      >
        <option value="">All types</option>
        {PROVIDER_MEMORY_TYPES.map((id) => <option key={id} value={id}>{id}</option>)}
      </select>
    </fieldset>
  );
}

function renderState(
  state: State,
  loadMore: () => void,
  openRecord: (recordId: string) => void,
  restartFromFreshSnapshot: () => void,
  alert: RefObject<HTMLParagraphElement | null>,
  openingRecordId: string | null,
): ReactElement | null {
  if (state.kind === "loading") return <p>Loading browse…</p>;
  if (state.kind === "loading_more") {
    return (
      <>
        {partialUnavailableBanner(state.sources, true)}
        {renderResults(state.results, state.cursor, loadMore, openRecord, true, openingRecordId)}
        <p>Loading more results…</p>
      </>
    );
  }
  if (state.kind === "error") {
    return (
      <>
        {partialUnavailableBanner(state.sources ?? [], Boolean(state.results))}
        {state.results ? renderResults(state.results, null, loadMore, openRecord, false, openingRecordId) : null}
        <p ref={alert} tabIndex={-1} role="alert">{state.message}</p>
      </>
    );
  }
  if (state.kind === "stale") {
    return (
      <>
        {partialUnavailableBanner(state.sources, true)}
        <p ref={alert} tabIndex={-1} role="alert">{state.message}</p>
        <button type="button" onClick={restartFromFreshSnapshot}>Restart from the new snapshot</button>
        {renderResults(state.results, null, loadMore, openRecord, false, openingRecordId)}
      </>
    );
  }
  // ready
  if (state.empty) {
    return <EmptyState message={emptyCopy(state.empty)} />;
  }
  return (
    <>
      {partialUnavailableBanner(state.sources, state.results.length > 0)}
      {renderResults(state.results, state.cursor, loadMore, openRecord, false, openingRecordId)}
    </>
  );
}

/**
 * FR-14 partial-unavailability banner — surface any confirmed source whose
 * status is not `available`. Browse is scoped to a single source, so the copy
 * is strictly informational about OTHER sources' health; it must never imply
 * their records are (or could be) in this single-source list (the original
 * search-style copy was misleading under browse scope).
 */
function partialUnavailableBanner(sources: SourceQueryStatus[], hasResults: boolean): ReactElement | null {
  if (!hasResults) return null;
  const flagged = sources.filter((source) => source.status !== "available");
  if (flagged.length === 0) return null;
  return (
    <p role="status" data-testid="browse-source-status">
      {flagged.map((source, index) => {
        const label = source.status === "unavailable"
          ? `Source ${providerDisplayName(source.provider)} was unreachable at last scan.`
          : `Source ${providerDisplayName(source.provider)} is degraded; its memories may be stale.`;
        return <span key={source.source_id}>{index > 0 ? " " : ""}{label}</span>;
      })}
    </p>
  );
}

function renderResults(
  results: SearchResult[],
  cursor: string | null,
  loadMore: () => void,
  openRecord: (recordId: string) => void,
  loadingMore: boolean,
  openingRecordId: string | null,
): ReactElement {
  // The list ref is unused for browse (no auto-focus-on-new-results); the
  // Search component owns that affordance. Browse stays keyboard-reachable
  // via the natural tab order (list items carry `tabIndex={0}` in ResultCard).
  return (
    <>
      <p>{results.length} memor{results.length === 1 ? "y" : "ies"}.</p>
      <ol>
        {results.map((result) => (
          <ResultCard
            key={result.record_id}
            result={result}
            onOpen={openRecord}
            openInFlight={openingRecordId === result.record_id}
          />
        ))}
      </ol>
      {cursor ? <LoadMore onClick={loadMore} disabled={loadingMore} /> : null}
    </>
  );
}

function renderOpenState(state: OpenState): ReactElement | null {
  if (state.kind === "idle") return null;
  if (state.kind === "opening") return <p>Opening original location…</p>;
  if (state.kind === "opened") return <p>{state.message}</p>;
  return <p role="alert">{state.message}</p>;
}

function hasErrorCode(error: unknown, code: string): boolean {
  return Boolean(error && typeof error === "object" && "code" in error && (error as { code?: unknown }).code === code);
}

/**
 * Browse's three distinct empty-state messages. The three states are
 * distinct on the wire (`not_yet_scanned` / `no_indexable_memory` /
 * `source_unavailable`) so the copy can name each situation accurately — the
 * UI must never collapse them into a single "empty" (Boundaries: "Never
 * collapse Browse's three empty states into fewer").
 *
 * Story 3.2 reuses `no_indexable_memory` for a filter that narrows to zero:
 * at the contract level a filter narrowing to zero is indistinguishable from
 * a source with no records of any type (both are a zero-row first page on a
 * scanned-OK source), so no fourth state is added (Design Notes).
 *
 * The copy intentionally mirrors Search's `source_not_indexed` /
 * `source_unavailable` phrasing for the overlapping cases so the user hears
 * a consistent voice across the two surfaces; the new
 * `no_indexable_memory` copy names the query-less "scanned OK, zero
 * records" situation that Search has no analog for.
 */
function emptyCopy(state: BrowseEmptyState): string {
  switch (state) {
    case "not_yet_scanned":
      return "This source has not been scanned yet. Run a scan from the Source Inventory to populate its memories.";
    case "no_indexable_memory":
      return "This source scanned successfully but contains no indexable Agent Memory.";
    case "source_unavailable":
      return "This source is currently unavailable; its stored health was not changed.";
  }
}
