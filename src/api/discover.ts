/**
 * Tessera — typed TS client for the `discover_sources` endpoint (Story 1.2).
 *
 * Mirrors the Rust types in `server/src/domain/ports/provider_adapter.rs`
 * (CandidateSource / CoverageLevel / DiscoveryBasis) and the versioned
 * envelope in `server/src/http/envelope.rs`. Must stay in lock-step with the
 * Rust side: any field rename / removal / type change here is a contract
 * break that requires an `api_version` bump (AD-17/A-6).
 *
 * The UI never touches Providers, the filesystem, or SQLite directly
 * (AD-1); every call goes through the loopback HTTP API (revised AD-9).
 *
 * Invariants honored by this module:
 * - **Infallible endpoint:** `discover_sources` has no error path. A failed
 *   fetch here means either (a) the server is down / the route is missing
 *   (a build/config error), or (b) the response shape drifted. Both are
 *   surfaced as a thrown `TesseraApiError` so the React shell renders the
 *   error state rather than masking it (Phase 0 review finding: never
 *   fabricate a fake success).
 * - **Honest narrow types:** `coverage_level` is the literal union of the
 *   four wire strings (`full | search_only | existence_only | unsupported`),
 *   matching Rust's `#[serde(rename_all = "snake_case")]`. The UI may NOT
 *   treat `search_only` results as complete enumeration (AD-18).
 */

import { apiGet, type Envelope, type TesseraApiError } from "./client";

export type { Envelope, TesseraApiError };
export { API_VERSION } from "./client";
import { API_VERSION } from "./client";

/**
 * Coverage level declared by the provider (AD-3 / AD-18). Mirrors Rust
 * `CoverageLevel` with `#[serde(rename_all = "snake_case")]`. A UI MUST NOT
 * display `search_only` / `existence_only` / `unsupported` as "fully synced"
 * (AD-3 capability-honesty).
 */
export type CoverageLevel =
  | "full"
  | "search_only"
  | "existence_only"
  | "unsupported";

/**
 * How a Candidate Source was discovered (AD-4). Mirrors Rust `DiscoveryBasis`
 * with `#[serde(rename_all = "snake_case")]`. UI-facing metadata only — not
 * part of source identity.
 */
export type DiscoveryBasis = "default_home" | "codex_home_env";

/**
 * Candidate Source metadata produced by discovery (AD-4 / Story 1.2).
 * Mirrors Rust `CandidateSource`. A Candidate is pre-confirmation: it has no
 * `source_id`, is not persisted, and is recomputed on every boot. Discovery
 * only checks directory existence (NFR-5) — this type never carries memory
 * body / transcript content.
 */
export interface CandidateSource {
  /** Stable lowercase provider id (e.g. `"codex"`). */
  provider: string;
  /** The provider root path discovery probed. NOT canonicalized (AD-4). */
  root_path: string;
  /** How discovery found this candidate. UI metadata only. */
  basis: DiscoveryBasis;
  /** Provider's declared coverage level (AD-3). */
  coverage_level: CoverageLevel;
  /**
   * Provider-native project identifier when discoverable from root metadata
   * alone. `null` for Codex (global store, no per-project split).
   */
  native_project: string | null;
}

const VALID_COVERAGE_LEVELS: ReadonlySet<string> = new Set([
  "full",
  "search_only",
  "existence_only",
  "unsupported",
]);

const VALID_DISCOVERY_BASES: ReadonlySet<string> = new Set([
  "default_home",
  "codex_home_env",
]);

/**
 * Narrow an unknown value to a `CoverageLevel`, or return `null` if it is not
 * one of the four stable wire strings. Used by `isCandidateSource` so the UI
 * never silently accepts an unknown variant.
 */
function asCoverageLevel(value: unknown): CoverageLevel | null {
  return typeof value === "string" && VALID_COVERAGE_LEVELS.has(value)
    ? (value as CoverageLevel)
    : null;
}

function asDiscoveryBasis(value: unknown): DiscoveryBasis | null {
  return typeof value === "string" && VALID_DISCOVERY_BASES.has(value)
    ? (value as DiscoveryBasis)
    : null;
}

/**
 * Runtime shape guard for a single `CandidateSource`. Returns the narrowed
 * value or `null` so the caller can throw a structured contract error rather
 * than passing bad data into React state.
 *
 * Pinning every field's type — not just `typeof === "object"` — means a
 * future Rust-side rename (e.g. `root_path` → `path`) surfaces as an explicit
 * contract error instead of an undefined-render bug in the UI.
 */
function asCandidateSource(value: unknown): CandidateSource | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    typeof v.provider !== "string" ||
    typeof v.root_path !== "string" ||
    asCoverageLevel(v.coverage_level) === null ||
    asDiscoveryBasis(v.basis) === null ||
    // native_project is `string | null`: a string is fine, null is fine,
    // undefined is NOT (the wire shape uses JSON null).
    (v.native_project !== null && typeof v.native_project !== "string")
  ) {
    return null;
  }
  return {
    provider: v.provider,
    root_path: v.root_path,
    basis: v.basis as DiscoveryBasis,
    coverage_level: v.coverage_level as CoverageLevel,
    native_project: v.native_project as string | null,
  };
}

/**
 * Call the `discover_sources` endpoint and return the typed, versioned
 * envelope.
 *
 * The Rust `Envelope<Vec<CandidateSource>>` arrives here as
 * `{ api_version, payload: CandidateSource[] }`. On any shape drift we throw
 * a structured `TesseraApiError` (`code: "api_contract"`) so the React shell
 * renders the error state — never fabricate an empty list to mask a broken
 * contract (Phase 0 review finding applies here too).
 */
export async function discoverSources(): Promise<Envelope<CandidateSource[]>> {
  const envelope = (await apiGet("/api/sources/discover")) as Envelope<CandidateSource[]> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    Array.isArray(envelope.payload) &&
    envelope.payload.every((c) => asCandidateSource(c) !== null)
  ) {
    return {
      api_version: envelope.api_version,
      payload: envelope.payload.map((c) => asCandidateSource(c) as CandidateSource),
    };
  }
  throw {
    code: "api_contract",
    message:
      "Tessera core discover_sources response did not match the versioned envelope contract.",
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}
