import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactElement, type RefObject } from "react";
import { readTesseraErrorMessage } from "../../api/errors";
import { openOriginalLocation } from "../../api/open";
import { searchMemories, type SearchEmptyState, type SearchResult, type SourceQueryStatus } from "../../api/search";

const SEARCH_PAGE_SIZE = 2;

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

export function Search(): ReactElement {
  const [query, setQuery] = useState("");
  const [state, setState] = useState<State>({ kind: "idle" });
  const [openState, setOpenState] = useState<OpenState>({ kind: "idle" });
  const request = useRef(0);
  const openRequest = useRef(0);
  const pendingResultFocus = useRef<number | null>(null);
  const resultList = useRef<HTMLOListElement>(null);
  const alert = useRef<HTMLParagraphElement>(null);
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
    searchMemories(query, undefined, SEARCH_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      setState({ kind: "ready", results: page.payload.results, cursor: page.payload.next_cursor, empty: page.payload.empty_state, sources: page.payload.sources ?? [] });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      setState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, [query]);
  const loadMore = useCallback(() => {
    if (state.kind !== "ready" || !state.cursor) return;
    const id = ++request.current;
    pendingResultFocus.current = state.results.length;
    const priorSources = state.sources;
    setState({ kind: "loading_more", results: state.results, cursor: state.cursor, sources: priorSources });
    searchMemories(query, state.cursor, SEARCH_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      const nextSources = page.payload.sources ?? [];
      setState({ kind: "ready", results: [...state.results, ...page.payload.results], cursor: page.payload.next_cursor, empty: null, sources: nextSources.length > 0 ? nextSources : priorSources });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      if (hasErrorCode(error, "cursor_stale")) {
        setState({ kind: "stale", results: state.results, cursor: state.cursor, message: readTesseraErrorMessage(error), sources: priorSources });
      } else {
        setState({ kind: "error", message: readTesseraErrorMessage(error), results: state.results, cursor: state.cursor, sources: priorSources });
      }
    });
  }, [query, state]);
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
  return <section aria-label="Memory search" role="region">
    <h2>Search memories</h2>
    <form onSubmit={submit}>
      <label htmlFor="memory-search">Keyword</label>
      <input id="memory-search" value={query} onChange={(event) => { ++request.current; ++openRequest.current; setQuery(event.target.value); setState({ kind: "idle" }); setOpenState({ kind: "idle" }); }} />
      <button type="submit">Search</button>
    </form>
    <div aria-live="polite">{renderOpenState(openState)}{renderState(state, loadMore, openRecord, openState, resultList, alert)}</div>
  </section>;
}

function renderState(
  state: State,
  loadMore: () => void,
  openRecord: (recordId: string) => void,
  openState: OpenState,
  resultList: RefObject<HTMLOListElement | null>,
  alert: RefObject<HTMLParagraphElement | null>,
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
  if (state.empty) return <p>{emptyCopy(state.empty)}</p>;
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
  return <><p>{results.length} result{results.length === 1 ? "" : "s"}.</p><ol ref={resultList}>{results.map((result) => <li key={result.record_id} tabIndex={0}><p>{result.excerpt}</p><dl><dt>Provider</dt><dd>{providerBadge(result.provider)}</dd><dt>Source</dt><dd>{result.source_id}</dd><dt>Native project</dt><dd>{result.native_project ?? "Unmapped"}</dd><dt>Semantic location</dt><dd>{result.native_locator}</dd><dt>Display location</dt><dd>{result.display_locator}</dd><dt>Last observed (scan)</dt><dd>{result.observed_at}</dd><dt>Coverage</dt><dd>{result.coverage_level}</dd><dt>Source health</dt><dd>{result.health_state}</dd></dl><button type="button" onClick={() => openRecord(result.record_id)} disabled={openingRecordId === result.record_id}>Open original location</button></li>)}</ol>{cursor ? <button type="button" onClick={loadMore} disabled={loadingMore}>Load more</button> : null}</>;
}

/**
 * Provider badge — renders a short label so Codex vs Claude Code cards are
 * visually comparable at a glance. The data layer needs no change for
 * comparison; this is purely a layout affordance (Design Notes: YAGNI for a
 * split-view compare component).
 */
function providerBadge(provider: string): ReactElement {
  return <span className="tessera-provider-badge" data-provider={provider}>{providerDisplayName(provider)}</span>;
}

function providerDisplayName(provider: string): string {
  switch (provider) {
    case "codex": return "Codex";
    case "claude_code": return "Claude Code";
    default: return provider;
  }
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

function emptyCopy(state: SearchEmptyState): string {
  switch (state) {
    case "no_match": return "No indexed memory matched this keyword.";
    case "source_not_indexed": return "Confirmed sources have not been indexed yet.";
    case "source_unavailable": return "A confirmed source is currently unavailable; its stored health was not changed.";
  }
}
