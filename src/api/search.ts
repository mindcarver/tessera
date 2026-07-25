import { API_VERSION, apiGet, type Envelope } from "./client";

export type SearchEmptyState = "no_match" | "source_not_indexed" | "source_unavailable";

/**
 * Per-source availability status (Story 2.3 FR-14 sidecar). Mirrors the Rust
 * `SourceQueryStatusKind` snake_case wire strings exactly.
 */
export type SourceQueryStatusKind = "available" | "degraded" | "unavailable";

/**
 * One row of the per-query availability sidecar. A down source's already-
 * indexed records (if any) are NOT suppressed — the flag is informational so
 * the UI can surface a partial-unavailability banner without hiding data.
 */
export interface SourceQueryStatus {
  source_id: string;
  provider: string;
  native_project: string | null;
  status: SourceQueryStatusKind;
}

export interface SearchResult {
  record_id: string;
  excerpt: string;
  provider: string;
  source_id: string;
  native_project: string | null;
  native_locator: string;
  display_locator: string;
  observed_at: number;
  coverage_level: string;
  health_state: string;
}

export interface SearchPage {
  results: SearchResult[];
  next_cursor: string | null;
  empty_state: SearchEmptyState | null;
  /**
   * FR-14 per-query sidecar — one row per confirmed source. Optional on the
   * wire for forward-compat: `sources` is an additive field on `api_version`
   * "1", so a newer client must tolerate an older server that omits it
   * (default `[]`). The Rust server always sends it.
   */
  sources?: SourceQueryStatus[];
}

export async function searchMemories(q: string, cursor?: string, limit = 20): Promise<Envelope<SearchPage>> {
  const params = new URLSearchParams({ q });
  if (cursor) params.set("cursor", cursor);
  params.set("limit", String(limit));
  const value = await apiGet(`/api/search?${params.toString()}`);
  if (!isSearchEnvelope(value)) throw apiContractError();
  return value;
}

function isSearchEnvelope(value: unknown): value is Envelope<SearchPage> {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  if (v.api_version !== API_VERSION || !v.payload || typeof v.payload !== "object") return false;
  const page = v.payload as Record<string, unknown>;
  if (!Array.isArray(page.results) || !page.results.every(isSearchResult)) return false;
  if (typeof page.next_cursor !== "string" && page.next_cursor !== null) return false;
  if (page.empty_state !== null && page.empty_state !== "no_match" && page.empty_state !== "source_not_indexed" && page.empty_state !== "source_unavailable") return false;
  // FR-14 sidecar is additive on `api_version` "1" (no bump), so a newer
  // client must tolerate an older server that omits `sources`. When present it
  // must still be well-formed; when absent the UI defaults to `[]`. `api_version`
  // stays "1" because adding a field is non-breaking.
  if (page.sources !== undefined && (!Array.isArray(page.sources) || !page.sources.every(isSourceQueryStatus))) return false;
  // Empty states describe an initial zero-result page. A wire payload must
  // never tell the UI to hide real results behind an empty-state message.
  return page.empty_state === null || (page.results.length === 0 && page.next_cursor === null);
}

function isSourceQueryStatus(value: unknown): value is SourceQueryStatus {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return typeof v.source_id === "string"
    && typeof v.provider === "string"
    && (typeof v.native_project === "string" || v.native_project === null)
    && (v.status === "available" || v.status === "degraded" || v.status === "unavailable");
}

function isSearchResult(value: unknown): value is SearchResult {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return ["record_id", "excerpt", "provider", "source_id", "native_locator", "display_locator", "coverage_level", "health_state"].every((key) => typeof v[key] === "string") &&
    (typeof v.native_project === "string" || v.native_project === null) && typeof v.observed_at === "number";
}

function apiContractError(): never {
  throw { code: "api_contract", message: "Tessera core returned an unsupported search response.", source_id: null, phase: "search" };
}
