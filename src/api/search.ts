import { API_VERSION, apiGet, type Envelope } from "./client";

export type SearchEmptyState = "no_match" | "source_not_indexed" | "source_unavailable";

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
  // Empty states describe an initial zero-result page. A wire payload must
  // never tell the UI to hide real results behind an empty-state message.
  return page.empty_state === null || (page.results.length === 0 && page.next_cursor === null);
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
