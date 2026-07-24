import { useCallback, useEffect, useRef, useState, type FormEvent, type ReactElement, type RefObject } from "react";
import { readTesseraErrorMessage } from "../../api/errors";
import { searchMemories, type SearchEmptyState, type SearchResult } from "../../api/search";

const SEARCH_PAGE_SIZE = 2;

type State =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "loading_more"; results: SearchResult[]; cursor: string }
  | { kind: "error"; message: string; results?: SearchResult[]; cursor?: string | null }
  | { kind: "stale"; results: SearchResult[]; cursor: string | null; message: string }
  | { kind: "ready"; results: SearchResult[]; cursor: string | null; empty: SearchEmptyState | null };

export function Search(): ReactElement {
  const [query, setQuery] = useState("");
  const [state, setState] = useState<State>({ kind: "idle" });
  const request = useRef(0);
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
    setState({ kind: "loading" });
    searchMemories(query, undefined, SEARCH_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      setState({ kind: "ready", results: page.payload.results, cursor: page.payload.next_cursor, empty: page.payload.empty_state });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      setState({ kind: "error", message: readTesseraErrorMessage(error) });
    });
  }, [query]);
  const loadMore = useCallback(() => {
    if (state.kind !== "ready" || !state.cursor) return;
    const id = ++request.current;
    pendingResultFocus.current = state.results.length;
    setState({ kind: "loading_more", results: state.results, cursor: state.cursor });
    searchMemories(query, state.cursor, SEARCH_PAGE_SIZE).then((page) => {
      if (id !== request.current) return;
      setState({ kind: "ready", results: [...state.results, ...page.payload.results], cursor: page.payload.next_cursor, empty: null });
    }).catch((error: unknown) => {
      if (id !== request.current) return;
      if (hasErrorCode(error, "cursor_stale")) {
        setState({ kind: "stale", results: state.results, cursor: state.cursor, message: readTesseraErrorMessage(error) });
      } else {
        setState({ kind: "error", message: readTesseraErrorMessage(error), results: state.results, cursor: state.cursor });
      }
    });
  }, [query, state]);
  return <section aria-label="Memory search" role="region">
    <h2>Search memories</h2>
    <form onSubmit={submit}>
      <label htmlFor="memory-search">Keyword</label>
      <input id="memory-search" value={query} onChange={(event) => { ++request.current; setQuery(event.target.value); setState({ kind: "idle" }); }} />
      <button type="submit">Search</button>
    </form>
    <div aria-live="polite">{renderState(state, loadMore, resultList, alert)}</div>
  </section>;
}

function renderState(
  state: State,
  loadMore: () => void,
  resultList: RefObject<HTMLOListElement | null>,
  alert: RefObject<HTMLParagraphElement | null>,
): ReactElement | null {
  if (state.kind === "idle") return null;
  if (state.kind === "loading") return <p>Searching indexed memories…</p>;
  if (state.kind === "loading_more") return <>{renderResults(state.results, state.cursor, loadMore, resultList, true)}<p>Loading more results…</p></>;
  if (state.kind === "error") return <>{state.results ? renderResults(state.results, null, loadMore, resultList) : null}<p ref={alert} tabIndex={-1} role="alert">{state.message}</p></>;
  if (state.kind === "stale") return <><p ref={alert} tabIndex={-1} role="alert">{state.message}</p>{renderResults(state.results, null, loadMore, resultList)}</>;
  if (state.empty) return <p>{emptyCopy(state.empty)}</p>;
  return renderResults(state.results, state.cursor, loadMore, resultList);
}

function renderResults(results: SearchResult[], cursor: string | null, loadMore: () => void, resultList: RefObject<HTMLOListElement | null>, loadingMore = false): ReactElement {
  return <><p>{results.length} result{results.length === 1 ? "" : "s"}.</p><ol ref={resultList}>{results.map((result) => <li key={result.record_id} tabIndex={0}><p>{result.excerpt}</p><dl><dt>Provider</dt><dd>{result.provider}</dd><dt>Source</dt><dd>{result.source_id}</dd><dt>Native project</dt><dd>{result.native_project ?? "Unmapped"}</dd><dt>Semantic location</dt><dd>{result.native_locator}</dd><dt>Display location</dt><dd>{result.display_locator}</dd><dt>Last observed (scan)</dt><dd>{result.observed_at}</dd><dt>Coverage</dt><dd>{result.coverage_level}</dd><dt>Source health</dt><dd>{result.health_state}</dd></dl></li>)}</ol>{cursor ? <button type="button" onClick={loadMore} disabled={loadingMore}>Load more</button> : null}</>;
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
