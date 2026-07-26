/**
 * Tessera — query-less Browse view (Story 3.1 + Story 3.2 + Story 3.3).
 *
 * The no-query browse entry surface: enter from a Source Inventory card,
 * fetch `browseMemories(sourceId, memoryType?, cursor?, limit?)`, render the
 * shared `ResultCard` list + `EmptyState` + `LoadMore`, with `aria-live`
 * status. Browse reuses Search's result-card / Provenance / Coverage / Health
 * / EmptyState / pagination components verbatim (Epic 3 Boundaries: "reuse,
 * do not re-implement").
 *
 * Story 3.2 adds the one in-source filter dimension that genuinely varies
 * within a single source (`memory_type`), and surfaces the existing
 * `observed_at DESC` ordering in the UI copy as "Recent scan first" (scan
 * recency — never implying content-change tracking; AD-7 no-disguise).
 *
 * Story 3.3 makes the Provider → Native Project hierarchy explicit as a
 * keyboard-reachable Breadcrumb at the top of the view (`Sources › <Provider>
 * › <Native Project | Global memory>`), where the Sources segment IS the back
 * affordance (one back action, not two), and consolidates the scattered
 * status readouts (3.2's "Recent scan first" + the current source's Source
 * Health from the existing sidecar) into one structured hierarchy-status
 * view. Pure front-end — no server contract change, no new data path.
 *
 * Accessibility (AD-21):
 * - The view is keyboard-reachable from the Inventory "Browse" button.
 * - The Breadcrumb's Sources segment is a `<button type="button">` (Tab-
 *   reachable; Enter/Space activates `onBack`) and is the first focusable
 *   element so a keyboard user can leave the view without tabbing through
 *   results. It auto-focuses on Browse entry so that contract is observable.
 * - The filter `<select>` is inside a `<fieldset aria-label="Browse filters">`
 *   with a `<label>`; the native `<select>` is keyboard-operable by default.
 * - The result list is an `<ol>` so the screen-reader semantics mirror
 *   Search's.
 * - The dynamic Source Health line lives inside the existing `aria-live`
 *   region so health transitions are spoken without moving focus; the static
 *   "Recent scan first" label lives OUTSIDE any `aria-live` ancestor so it is
 *   never re-announced.
 */

import { useCallback, useEffect, useRef, useState, type ReactElement, type RefObject } from "react";
import { readTesseraErrorMessage } from "../../api/errors";
import { openOriginalLocation } from "../../api/open";
import { browseMemories, PROVIDER_MEMORY_TYPES, type BrowseEmptyState, type ProviderMemoryType, type SearchResult, type SourceQueryStatus } from "../../api/browse";
import { EmptyState, type EmptyTone } from "../../components/EmptyState";
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
  // Story 3.3 (review PATCH 7) — the Breadcrumb's Sources segment is the first
  // focusable element and is auto-focused on Browse entry so a keyboard user
  // can leave the view without tabbing through results (preserves 3.1's
  // "first focusable = back" contract, now asserted in the a11y test).
  const sourcesButton = useRef<HTMLButtonElement>(null);

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

  // Story 3.3 (review PATCH 7) — focus the Sources segment on Browse entry so
  // the "first focusable = back" contract is observable, not just structural.
  useEffect(() => {
    sourcesButton.current?.focus();
  }, []);

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

  // Story 3.3 — the structure-status view needs the current source's Source
  // Health from the existing sidecar (no new fetch). PATCH 2/3: gate on STATE,
  // not on the truthiness of `state.sources`, so the health line never
  // mislabels a terminal state as "Loading…" (AD-7 no-disguise):
  // - `loading` is the only truly pending state → no sidecar yet ("Loading…").
  // - `error` may carry a partial sidecar (or none); either way the request
  //   has terminated, so the browsed source's health is "unknown" (NOT
  //   "Loading…").
  // - `loading_more` / `stale` / `ready` carry a sidecar; if the browsed
  //   `sourceId` is absent from it, that is a server-side omission, also
  //   "unknown" (never a fabricated status).
  //
  // `sidecarPending === true` means "no sidecar yet" (the loading state). A
  // present `sidecarSources` means "sidecar arrived"; the browsed-source
  // lookup follows.
  const sidecarPending = state.kind === "loading";
  const sidecarSources: SourceQueryStatus[] | undefined =
    sidecarPending ? undefined
      : state.kind === "error" ? (state.sources ?? [])
        : state.sources;
  // PATCH 10 — `.find()` returns the first match. The wire contract does not
  // guarantee `source_id` is unique within `sources`; a duplicate is a
  // server-side bug the UI tolerates by taking the first occurrence (the
  // status values are per-source, so duplicates carry the same status in
  // practice). Documented here so a future maintainer does not assume
  // uniqueness.
  const currentSourceStatus = sidecarSources?.find((s) => s.source_id === sourceId);

  return (
    <section aria-label="Memory browse" role="region" className="tsr-section">
      <h2 className="tsr-section__title">Browse memories</h2>
      {renderBreadcrumb(providerLabel, nativeProject, onBack, sourcesButton)}
      {renderFilterControls(memoryType, setMemoryType)}
      {/*
        PATCH 11 — the STATIC "Recent scan first" label lives OUTSIDE any
        `aria-live` ancestor. `role="status"` / `aria-live` re-announces on
        every React re-render that touches the subtree; a static label must
        never be re-announced (it would interrupt the user mid-list on every
        filter / pagination transition). The DYNAMIC health line stays inside
        the `<div aria-live="polite">` below so sidecar arrival / health
        changes are spoken without moving focus.
      */}
      {renderOrderReadout()}
      <div aria-live="polite">
        {renderHealthReadout(currentSourceStatus, sidecarPending)}
        {renderOpenState(openState)}
        {renderState(state, loadMore, openRecord, restartFromFreshSnapshot, alert, openState.kind === "opening" ? openState.recordId : null)}
      </div>
    </section>
  );
}

/**
 * Story 3.3 — the keyboard-reachable Breadcrumb that makes the cross-view
 * drill-down path (Inventory provider group → single-source Browse) explicit.
 * Three segments inside a `<nav aria-label="Breadcrumb">` + `<ol>`:
 *
 * - **Sources** — a `<button type="button">` that IS the existing `onBack`
 *   action, surfaced as the breadcrumb's Sources segment (one back action,
 *   not two). It is the first focusable element and auto-focuses on Browse
 *   entry so a keyboard user can leave the view without tabbing through
 *   results (preserves 3.1's "first focusable = back" contract). PATCH 8:
 *   a visible `←` back cue prefixes the label so sighted users recognize
 *   the exit path; the glyph is `aria-hidden` so the accessible name stays
 *   "Sources" (name-based selectors keep working).
 * - **Provider** — a presentational `<span>` (no separate click target). The
 *   Inventory's own provider grouping already IS the provider layer, so a
 *   second click target here would be redundant chrome.
 * - **Native Project | "Global memory"** — the leaf, `aria-current="location"`
 *   (PATCH 4: this is the current in-app drill-down location, not a page
 *   among pages; "location" is the honest `aria-current` token). Honest
 *   about Codex's global store: a falsy `nativeProject` (Codex's adapter
 *   hard-codes `native_project: None`, `server/src/adapters/codex.rs`) renders
 *   "Global memory" (never a fake project name, never "All projects"); a
 *   Claude source renders its `native_project` string verbatim (no reverse-
 *   mapping to a repo path — that is Epic 5 federation, explicitly out of
 *   scope per epic-3-context.md:45).
 *
 * The Tessera-Project segment is reserved for Epic 5 and is NOT built here.
 */
function renderBreadcrumb(
  providerLabel: string,
  nativeProject: string | null,
  onBack: () => void,
  sourcesButtonRef: RefObject<HTMLButtonElement | null>,
): ReactElement {
  // PATCH 1 — treat ALL falsy native_project as global (the prop type is
  // `string | null` and the validator accepts `""`; `??` only catches
  // null/undefined, so an empty string would render a blank leaf). The
  // decision is made here (not in the parent) so the parent stays a pure
  // pass-through of the wire value.
  const leafLabel = nativeProject || "Global memory";
  // PATCH 13 — guard an empty-string providerLabel so the middle segment
  // never renders blank ("Sources › › <leaf>"). App.tsx resolves the label
  // via providerDisplayName (which always returns a non-empty string for the
  // known providers and falls back to the raw id), but an unknown future
  // caller could pass "" — the inline fallback keeps the hierarchy legible.
  const providerText = providerLabel || "(unknown provider)";
  return (
    <nav aria-label="Breadcrumb" className="tsr-crumb">
      <ol className="tsr-crumb__list">
        <li className="tsr-crumb__item">
          <button type="button" className="tsr-crumb__back" onClick={onBack} ref={sourcesButtonRef}>
            {/* PATCH 8 — visible back cue for sighted users; aria-hidden so the
                accessible name stays "Sources" (name-based selectors survive). */}
            <span aria-hidden="true">← </span>Sources
          </button>
        </li>
        {/* PATCH 5 — no aria-hidden here (the default is false; an explicit
            value is dead code that suggests an abandoned decision to hide the
            Provider segment). */}
        <li className="tsr-crumb__item"><span className="tsr-crumb__seg">{providerText}</span></li>
        <li className="tsr-crumb__item"><span className="tsr-crumb__leaf" aria-current="location">{leafLabel}</span></li>
      </ol>
    </nav>
  );
}

/**
 * Story 3.2 — the STATIC "Recent scan first" ordering readout. PATCH 11: this
 * label lives OUTSIDE any `aria-live` ancestor (the caller renders it before
 * the `<div aria-live="polite">`), so React re-renders never re-announce it.
 * It is a label naming SCAN recency — never content-change tracking (AD-7:
 * never disguise Derived-Index state as source-data state). No new sort or
 * data path. `observed_at` is set once per scan (scan.rs:330) and is constant
 * across a source's active generation, so a time/date filter would be
 * degenerate (always "all"); the readout is the only honest way to
 * communicate "recent".
 */
function renderOrderReadout(): ReactElement {
  return <p data-testid="browse-effective-order" className="tsr-readout">Recent scan first</p>;
}

/**
 * Story 3.3 — the DYNAMIC Source Health readout for the browsed source,
 * derived from the existing `sources` sidecar filtered by `sourceId` (no new
 * fetch). Lives inside the `<div aria-live="polite">` so health transitions
 * (sidecar arrival, status change between pages) are spoken without moving
 * focus. PATCH 2/3: the label is state-gated so a terminal state is never
 * mislabeled as "Loading…":
 *
 * - `sidecarPending === true` (loading, no sidecar yet) → "Loading…".
 * - sidecar arrived but the browsed source is absent, OR the request errored
 *   → "unknown" (never a fabricated status; AD-7 no-disguise).
 * - sidecar arrived with the browsed source → its `status` verbatim.
 *
 * No `role="status"` on this `<p>` (PATCH 11): the outer `aria-live` already
 * covers the announcement; a second live region would double-announce.
 */
function renderHealthReadout(
  currentSourceStatus: SourceQueryStatus | undefined,
  sidecarPending: boolean,
): ReactElement {
  const healthLabel = sidecarPending
    ? "Loading…"
    : (currentSourceStatus?.status ?? "unknown");
  return (
    <p className="tsr-readout">
      <span data-testid="browse-source-health">Source health: {healthLabel}</span>
    </p>
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
    <fieldset aria-label="Browse filters" className="tsr-filters">
      <legend className="tsr-filters__legend">Filter memories</legend>
      <div className="tsr-filter">
        <label htmlFor="browse-filter-type">Memory type</label>
        <select
          id="browse-filter-type"
          value={memoryType}
          onChange={(event) => setMemoryType(event.target.value)}
        >
          <option value="">All types</option>
          {PROVIDER_MEMORY_TYPES.map((id) => <option key={id} value={id}>{id}</option>)}
        </select>
      </div>
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
        <button type="button" className="tsr-btn" onClick={restartFromFreshSnapshot}>Restart from the new snapshot</button>
        {renderResults(state.results, null, loadMore, openRecord, false, openingRecordId)}
      </>
    );
  }
  // ready
  if (state.empty) {
    return <EmptyState message={emptyCopy(state.empty)} tone={browseEmptyTone(state.empty)} />;
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
    <p role="status" data-testid="browse-source-status" className="tsr-banner tsr-banner--bad">
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
      <p className="tsr-res-hero">{results.length} memor{results.length === 1 ? "y" : "ies"}</p>
      {/*
        Story 3.3 — the results `<ol>` carries an `aria-label` so it is
        distinguishable from the Breadcrumb's `<ol>` (two lists now coexist in
        the browse region). PATCH 9: the label is "Browse results", NOT
        "Browse memories" (the `<h2>` already exposes that name in the same
        region; duplicating it would make SR users hear it twice and make
        by-name navigation ambiguous).
      */}
      <ol aria-label="Browse results" className="tsr-res-list">
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

/** Map the browse empty-state reason onto the three-state tone (mute/blue/red). */
function browseEmptyTone(state: BrowseEmptyState): EmptyTone {
  if (state === "source_unavailable") return "red";
  if (state === "not_yet_scanned") return "blue";
  return "mute";
}
