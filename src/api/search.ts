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

/**
 * Story 2.4 — provider memory-type vocabulary, mirroring the Rust
 * `ProviderMemoryType::as_str` snake_case wire strings. Keep in lockstep with
 * `server/src/domain/ports/provider_adapter.rs`.
 */
export const PROVIDER_MEMORY_TYPES = [
  "memory",
  "memory_summary",
  "raw_memories",
  "rollout_summary",
  "topic_memory",
] as const;
export type ProviderMemoryType = (typeof PROVIDER_MEMORY_TYPES)[number];

/**
 * Story 2.4 — provider id vocabulary. Keep in lockstep with the Rust
 * `KNOWN_PROVIDER_IDS` constant in `server/src/domain/query.rs`.
 */
export const KNOWN_PROVIDER_IDS = ["codex", "claude_code"] as const;
export type KnownProviderId = (typeof KNOWN_PROVIDER_IDS)[number];

/**
 * Story 2.4 — time-preset filter options. The UI computes an absolute
 * `since = now - N*86400` client-side; the server stays stateless.
 */
export const SEARCH_TIME_PRESETS = ["7d", "30d", "all"] as const;
export type SearchTimePreset = (typeof SEARCH_TIME_PRESETS)[number];

/**
 * Story 2.4 — optional cross-provider filters. `undefined` everywhere is the
 * 2.3 default scope (all confirmed sources, relevance-ordered). Each set
 * value narrows the result set with AND. `since` is an ABSOLUTE Unix-epoch
 * seconds value, NOT a relative preset: the UI resolves a time preset to an
 * absolute `since` ONCE on page 1 and reuses that same value for every "Load
 * more" in the session (Spec Change Log — `since` stable across a pagination
 * session). Recomputing `now` per page would change `since` between page 1 and
 * a later page, binding a different value into the cursor → `cursor_stale` →
 * "Load more" breaks under a time preset. The server stays stateless and never
 * computes relative time.
 */
export interface SearchFilters {
  provider?: KnownProviderId;
  /**
   * Per-source filter (Spec Change Log 2026-07-25): narrows to one specific
   * confirmed source's `src_<n>` id, distinct from the coarser provider filter.
   */
  source?: string;
  memory_type?: ProviderMemoryType;
  native_project?: string;
  /**
   * Absolute Unix-epoch seconds (`observed_at >= since`). Resolved once per
   * pagination session by the UI; the server never computes relative time.
   */
  since?: number;
}

/**
 * Story 2.4 — build the URLSearchParams for a search request. Exposed so the
 * UI's effective-range readout can reason about exactly which params will be
 * sent without duplicating the serialization logic. `since` is serialized
 * verbatim (the caller resolves the preset to an absolute value once).
 */
export function buildSearchParams(
  q: string,
  cursor: string | undefined,
  limit: number,
  filters: SearchFilters | undefined,
): URLSearchParams {
  const params = new URLSearchParams({ q });
  if (cursor) params.set("cursor", cursor);
  params.set("limit", String(limit));
  if (filters) {
    if (filters.provider) params.set("provider", filters.provider);
    if (filters.source) params.set("source", filters.source);
    if (filters.memory_type) params.set("memory_type", filters.memory_type);
    if (filters.native_project) params.set("native_project", filters.native_project);
    if (filters.since !== undefined) params.set("since", String(filters.since));
  }
  return params;
}

/**
 * Resolve a UI time preset to an absolute Unix-epoch seconds value, or
 * `undefined` when the preset is `"all"` / absent (no `since` on the wire).
 * The UI calls this ONCE on page 1 and reuses the result for every "Load
 * more" in the session so the cursor's bound `since` does not drift across
 * pages (Spec Change Log — `since` stability).
 */
export function sinceFromPreset(preset: SearchTimePreset | undefined): number | undefined {
  if (!preset || preset === "all") return undefined;
  const nowSec = Math.floor(Date.now() / 1000);
  const days = preset === "7d" ? 7 : 30;
  return nowSec - days * 86400;
}

export async function searchMemories(
  q: string,
  cursor?: string,
  limit = 20,
  filters?: SearchFilters,
): Promise<Envelope<SearchPage>> {
  const params = buildSearchParams(q, cursor, limit, filters);
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
