/**
 * Tessera — typed TS client for the query-less Browse endpoint (Story 3.1).
 *
 * Mirrors the Rust types in `server/src/domain/query.rs`
 * (`BrowseRequest` / `BrowsePage` / `BrowseEmptyState`) and reuses the
 * SearchResult / SourceQueryStatus shapes from `./search` verbatim so the
 * browse UI can share Search's result-card components without duplication
 * (Design Notes / Boundaries: "reuse, do not re-implement").
 *
 * Invariants honored:
 * - **Versioned envelope:** response is validated against `API_VERSION`; any
 *   drift throws `TesseraApiError` with code `api_contract`.
 * - **No query:** browse is query-less. The wire params are exactly `source`,
 *   `cursor`, `limit` — no `q`.
 * - **Confirmed-source only:** a non-confirmed / unknown source surfaces as
 *   `bad_request` (phase `browse`) from the server. The client does not need
 *   to filter; it renders the structured error envelope like any other API
 *   failure.
 * - **Cursor envelope prefix:** browse cursors carry a distinct `b3.<hex>`
 *   prefix so a cross-type (search `v3.<hex>`) cursor is rejected as
 *   `cursor_stale` server-side. The client treats `cursor_stale` the same way
 *   search does (re-run page 1 from a fresh snapshot).
 */

import { API_VERSION, apiGet, type Envelope, type TesseraApiError } from "./client";
import {
  isSearchResult,
  isSourceQueryStatus,
  type SearchResult,
  type SourceQueryStatus,
} from "./search";

// Re-export the shared row / sidecar shapes so feature code can import
// everything from one place. The browse contract reuses both verbatim — there
// is no BrowseResult type (Design Notes — "Why reuse `SearchResult` as-is").
export type { SearchResult, SourceQueryStatus };
export { isSearchResult, isSourceQueryStatus };

/**
 * Browse's three distinct empty-collection states (Epic 3 / FR-16). Mirrors
 * the Rust `BrowseEmptyState` snake_case wire strings exactly. Computed by
 * the server ONLY on page 1 when results are empty; never on a continuation
 * page. The UI never collapses these into a single "empty": each names a
 * different recovery path.
 */
export type BrowseEmptyState =
  /**
   * Confirmed source with no active generation and no successful scan.
   * Distinct from "scanned OK, zero records" so the user can tell "not yet"
   * from "empty after scan".
   */
  | "not_yet_scanned"
  /**
   * Confirmed source whose latest scan succeeded and activated a generation,
   * but that generation contains zero records. The source IS indexed; it
   * just has no Agent Memory in scope. Search has no analog (`no_match` is
   * query-bound and meaningless for a query-less browse).
   */
  | "no_indexable_memory"
  /**
   * Confirmed source whose latest run is Failed/Retry with no usable active
   * generation. Mirrors `SearchEmptyState.source_unavailable` semantically
   * but is a separate wire string so the browse UI's copy can name the
   * query-less situation accurately.
   */
  | "source_unavailable";

/**
 * A browse page. Same shape as `SearchPage` (results + cursor + empty_state +
 * per-source sidecar) so the UI can reuse Search's result-card / Provenance /
 * Coverage / Health / EmptyState / pagination components without duplication.
 *
 * `empty_state` is present ONLY on page 1 when results are empty. The
 * `isBrowseEnvelope` runtime guard rejects a payload that pairs a non-null
 * `empty_state` with non-empty results (the server never sends that shape,
 * but a forward-compat guard keeps the UI honest).
 */
export interface BrowsePage {
  results: SearchResult[];
  next_cursor: string | null;
  empty_state: BrowseEmptyState | null;
  /**
   * FR-14 per-query sidecar — one row per confirmed source. Optional on the
   * wire for forward-compat (mirrors `SearchPage.sources`).
   */
  sources?: SourceQueryStatus[];
}

/**
 * Browse the query-less list of records for a single confirmed source.
 *
 * @param sourceId  `src_<n>` handle of the source to browse.
 * @param cursor    Continuation cursor from a prior page's `next_cursor`, or
 *                  `undefined` for page 1.
 * @param limit     Page size (defaults to 20, bounded by the server).
 */
export async function browseMemories(
  sourceId: string,
  cursor?: string,
  limit = 20,
): Promise<Envelope<BrowsePage>> {
  const params = new URLSearchParams({ source: sourceId });
  if (cursor) params.set("cursor", cursor);
  params.set("limit", String(limit));
  const value = await apiGet(`/api/browse?${params.toString()}`);
  if (!isBrowseEnvelope(value)) throw apiContractError();
  return value;
}

export function isBrowseEnvelope(value: unknown): value is Envelope<BrowsePage> {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  if (v.api_version !== API_VERSION || !v.payload || typeof v.payload !== "object") return false;
  const page = v.payload as Record<string, unknown>;
  if (!Array.isArray(page.results) || !page.results.every(isSearchResult)) return false;
  if (typeof page.next_cursor !== "string" && page.next_cursor !== null) return false;
  // empty_state must be one of the three browse strings or null.
  if (
    page.empty_state !== null
    && page.empty_state !== "not_yet_scanned"
    && page.empty_state !== "no_indexable_memory"
    && page.empty_state !== "source_unavailable"
  ) {
    return false;
  }
  // Sidecar is additive on `api_version` "1" (mirrors search): a newer
  // client must tolerate an older server that omits `sources`. When present
  // it must still be well-formed.
  if (page.sources !== undefined && (!Array.isArray(page.sources) || !page.sources.every(isSourceQueryStatus))) {
    return false;
  }
  // Empty states describe an initial zero-result page. A wire payload must
  // never tell the UI to hide real results behind an empty-state message.
  return page.empty_state === null || (page.results.length === 0 && page.next_cursor === null);
}

function apiContractError(): never {
  throw {
    code: "api_contract",
    message: "Tessera core returned an unsupported browse response.",
    source_id: null,
    phase: "browse",
  } satisfies TesseraApiError;
}
