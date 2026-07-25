/**
 * Tessera — typed TS client for the query-less Browse endpoint (Story 3.1 +
 * Story 3.2).
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
 *   `cursor`, `limit`, and (Story 3.2) an optional `memory_type` — no `q`.
 * - **Confirmed-source only:** a non-confirmed / unknown source surfaces as
 *   `bad_request` (phase `browse`) from the server. The client does not need
 *   to filter; it renders the structured error envelope like any other API
 *   failure.
 * - **Memory-type filter (Story 3.2):** the client validates the
 *   `memoryType` argument against `PROVIDER_MEMORY_TYPES` BEFORE sending, so
 *   an unknown value never crosses the wire — mirroring the server-side
 *   vocabulary check while giving the UI a single failure mode.
 * - **Cursor envelope prefix:** browse cursors carry a distinct `b4.<hex>`
 *   prefix (Story 3.2 bumped `b3.` → `b4.` so the memory_type could bind into
 *   the cursor) so a cross-type (search `v3.<hex>`) cursor or a 3.1-era `b3.`
 *   cursor is rejected as `cursor_stale` server-side. The client treats
 *   `cursor_stale` the same way search does (re-run page 1 from a fresh
 *   snapshot).
 * - **Response shape unchanged:** `BrowsePage` does NOT echo `memory_type`
 *   (it is a request param only), so `isBrowseEnvelope` is unchanged. A
 *   filter-narrows-to-zero page surfaces `empty_state = "no_indexable_memory"`
 *   on page 1 — the same state 3.1 uses for "scanned, zero records".
 */

import { API_VERSION, apiGet, type Envelope, type TesseraApiError } from "./client";
import {
  PROVIDER_MEMORY_TYPES,
  isSearchResult,
  isSourceQueryStatus,
  type ProviderMemoryType,
  type SearchResult,
  type SourceQueryStatus,
} from "./search";

// Re-export the shared row / sidecar shapes so feature code can import
// everything from one place. The browse contract reuses both verbatim — there
// is no BrowseResult type (Design Notes — "Why reuse `SearchResult` as-is").
// Story 3.2 re-exports `PROVIDER_MEMORY_TYPES` + `ProviderMemoryType` so the
// Browse filter UI imports the vocabulary from a single place.
export type { SearchResult, SourceQueryStatus, ProviderMemoryType };
export { isSearchResult, isSourceQueryStatus, PROVIDER_MEMORY_TYPES };

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
 * @param sourceId    `src_<n>` handle of the source to browse.
 * @param memoryType  Story 3.2 — optional memory-type filter. Validated
 *                    against `PROVIDER_MEMORY_TYPES` BEFORE sending so an
 *                    unknown value never crosses the wire (mirrors the
 *                    server-side vocabulary check from a single source of
 *                    truth). `undefined` restores the 3.1 default scope.
 * @param cursor      Continuation cursor from a prior page's `next_cursor`,
 *                    or `undefined` for page 1.
 * @param limit       Page size (defaults to 20, bounded by the server).
 */
export async function browseMemories(
  sourceId: string,
  memoryType?: ProviderMemoryType,
  cursor?: string,
  limit = 20,
): Promise<Envelope<BrowsePage>> {
  const params = new URLSearchParams({ source: sourceId });
  if (memoryType) {
    // Defense-in-depth: reject an unknown value before it crosses the wire.
    // The TypeScript type already constrains callers, but a runtime guard
    // keeps a buggy caller (e.g. one widening a string into the typed slot)
    // from smuggling an arbitrary value into the URL — the server would
    // reject it too, but failing here gives a single failure mode.
    if (!PROVIDER_MEMORY_TYPES.includes(memoryType)) {
      throw {
        code: "api_contract",
        message: "Tessera rejected an unknown memory type before sending.",
        source_id: null,
        phase: "browse",
      } satisfies TesseraApiError;
    }
    params.set("memory_type", memoryType);
  }
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
