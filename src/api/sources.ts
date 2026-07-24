/**
 * Tessera — typed TS clients for Source confirm / reject / disable / list
 * (Story 1.3).
 *
 * Mirrors the Rust types in `server/src/domain/source.rs`
 * (Source / SourceLifecycle / SourceKind / HealthState / SourceId) and the
 * CandidateSource type from `server/src/domain/ports/provider_adapter.rs`.
 * Must stay in lock-step with the Rust side: any field rename / removal / type
 * change here is a contract break that requires an `api_version` bump
 * (AD-17/A-6).
 *
 * The UI never touches Providers, the filesystem, or SQLite directly (AD-1);
 * every call goes through the loopback HTTP API (revised AD-9).
 *
 * Invariants honored by this module:
 * - **AD-4 allowlist entry boundary:** `confirmSource` / `rejectSource` are
 *   the ONLY clients that accept a `CandidateSource` (which carries a path).
 *   `disableSource` / `listSources` accept a `source_id` / nothing — never an
 *   arbitrary path.
 * - **Versioned envelopes:** every response is validated against
 *   `API_VERSION`; any drift throws `TesseraApiError` with code `api_contract`
 *   (Phase 0 review finding: never fabricate a fake success).
 * - **No fingerprint on the wire:** the Rust `Source` DTO marks fingerprint
 *   `#[serde(skip)]`, so this TS type deliberately has no fingerprint field.
 *   Fingerprint is an internal matching key (AD-33/AD-35, Design Notes).
 * - **Honest narrow types:** enums are literal unions of the stable wire
 *   strings, matching Rust's `#[serde(rename_all = "snake_case")]`.
 */

import { apiGet, apiPost, type Envelope, type TesseraApiError } from "./client";
import type { CandidateSource } from "./discover";
import { API_VERSION } from "./client";

// Re-export the shared envelope / candidate / error shapes so feature code can
// import everything from one place. These come from client.ts / discover.ts.
export type { CandidateSource, Envelope, TesseraApiError };

// ---------------------------------------------------------------------------
// Source domain mirror (Rust: domain::source)
// ---------------------------------------------------------------------------

/**
 * Opaque stable Source handle (`src_<n>`). AD-33: independent of path/inode.
 * The numeric portion is the registry's AUTOINCREMENT rowid and is never
 * reused.
 */
export type SourceId = string;

/**
 * Lifecycle state of a Source (AD-7). Mirrors Rust `SourceLifecycle` with
 * `#[serde(rename_all = "snake_case")]`. All three states are persisted so
 * decisions (including rejections) survive restart.
 */
export type SourceLifecycle = "confirmed" | "disabled" | "rejected";

/**
 * Domain kind of a Source (AD-10/A-19). MVP only ships `agent_memory`.
 */
export type SourceKind = "agent_memory";

/**
 * Health state of a Source (AD-7). Story 1.3 always writes `unknown`; health
 * tracking is Story 1.8 / 4.x.
 */
export type HealthState = "unknown" | "healthy" | "degraded" | "error";

/**
 * A registered Source. Mirrors Rust `Source` DTO. The fingerprint field is
 * deliberately absent: Rust marks it `#[serde(skip)]` (Design Notes — "为何
 * Source DTO 隐藏 fingerprint").
 */
export interface Source {
  /** Opaque stable handle (`src_<n>`). */
  source_id: SourceId;
  /** Stable lowercase provider id (`codex`, ...). */
  provider: string;
  /** Domain kind (MVP: `agent_memory`). */
  source_kind: SourceKind;
  /** Lifecycle state. */
  lifecycle_state: SourceLifecycle;
  /** Health state (always `unknown` in 1.3). */
  health_state: HealthState;
  /**
   * Provider's declared coverage level (AD-3). On confirm, the stored value
   * comes from the adapter (single source of truth), not the candidate
   * payload (Design Notes — "coverage 单一事实源").
   */
  coverage_level: CandidateSource["coverage_level"];
  /** Canonicalized root path (AD-4). Shown to the user. */
  normalized_root_path: string;
  /** Provider-native project id when discoverable; `null` for Codex. */
  native_project: string | null;
}

// ---------------------------------------------------------------------------
// Runtime shape guards
// ---------------------------------------------------------------------------

const VALID_LIFECYCLES: ReadonlySet<string> = new Set([
  "confirmed",
  "disabled",
  "rejected",
]);
const VALID_SOURCE_KINDS: ReadonlySet<string> = new Set(["agent_memory"]);
const VALID_HEALTH_STATES: ReadonlySet<string> = new Set(["unknown", "healthy", "degraded", "error"]);
const VALID_COVERAGE_LEVELS: ReadonlySet<string> = new Set([
  "full",
  "search_only",
  "existence_only",
  "unsupported",
]);

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function isOptionalStringOrNull(v: unknown): boolean {
  return v === null || typeof v === "string";
}

/**
 * Runtime shape guard for a single `Source`. Returns the narrowed value or
 * `null` so callers throw a structured contract error rather than passing
 * bad data into React state.
 *
 * Pinning every field — not just `typeof === "object"` — means a Rust-side
 * rename (e.g. `source_id` → `id`) surfaces as a contract error instead of an
 * undefined-render bug.
 */
function asSource(value: unknown): Source | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.source_id) ||
    !isString(v.provider) ||
    !(typeof v.source_kind === "string" && VALID_SOURCE_KINDS.has(v.source_kind)) ||
    !(typeof v.lifecycle_state === "string" && VALID_LIFECYCLES.has(v.lifecycle_state)) ||
    !(typeof v.health_state === "string" && VALID_HEALTH_STATES.has(v.health_state)) ||
    !(typeof v.coverage_level === "string" && VALID_COVERAGE_LEVELS.has(v.coverage_level)) ||
    !isString(v.normalized_root_path) ||
    !isOptionalStringOrNull(v.native_project)
  ) {
    return null;
  }
  return {
    source_id: v.source_id,
    provider: v.provider,
    source_kind: v.source_kind as SourceKind,
    lifecycle_state: v.lifecycle_state as SourceLifecycle,
    health_state: v.health_state as HealthState,
    coverage_level: v.coverage_level as CandidateSource["coverage_level"],
    normalized_root_path: v.normalized_root_path,
    native_project: v.native_project as string | null,
  };
}

/** Throw a structured API contract error (Phase 0 review finding). */
function throwContractError(message: string): never {
  throw {
    code: "api_contract",
    message,
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}

// ---------------------------------------------------------------------------
// Clients (mirror the four Rust endpoints)
// ---------------------------------------------------------------------------

/**
 * Confirm a Candidate Source (AD-4 "allowlist 入边界"). Idempotent: re-
 * confirming the same root returns the same `source_id` and wakes any prior
 * rejected/disabled state back to `confirmed`. The root is re-canonicalized
 * in Rust; a vanished / non-dir root surfaces as `confirm_failed`.
 */
export async function confirmSource(
  candidate: CandidateSource,
): Promise<Envelope<Source>> {
  const envelope = (await apiPost("/api/sources/confirm", { candidate })) as Envelope<Source> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asSource(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asSource(envelope.payload) as Source,
    };
  }
  throwContractError(
    "Tessera core confirm_source response did not match the versioned envelope contract.",
  );
}

/**
 * Reject a Candidate Source. Persisted so the decision survives restart.
 * Idempotent by fingerprint.
 */
export async function rejectSource(
  candidate: CandidateSource,
): Promise<Envelope<Source>> {
  const envelope = (await apiPost("/api/sources/reject", { candidate })) as Envelope<Source> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asSource(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asSource(envelope.payload) as Source,
    };
  }
  throwContractError(
    "Tessera core reject_source response did not match the versioned envelope contract.",
  );
}

/**
 * Disable a confirmed Source by `source_id` (AD-4: only `source_id`, never an
 * arbitrary path). Unknown id → `source_not_found`.
 */
export async function disableSource(sourceId: SourceId): Promise<Envelope<Source>> {
  const envelope = (await apiPost("/api/sources/disable", {
    source_id: sourceId,
  })) as Envelope<Source> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asSource(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asSource(envelope.payload) as Source,
    };
  }
  throwContractError(
    "Tessera core disable_source response did not match the versioned envelope contract.",
  );
}

/**
 * List every registered Source (any lifecycle), ordered by id. Versioned
 * envelope; any DB failure in Rust surfaces as `internal`.
 */
export async function listSources(): Promise<Envelope<Source[]>> {
  const envelope = (await apiGet("/api/sources")) as Envelope<Source[]> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    Array.isArray(envelope.payload) &&
    envelope.payload.every((s) => asSource(s) !== null)
  ) {
    return {
      api_version: envelope.api_version,
      payload: envelope.payload.map((s) => asSource(s) as Source),
    };
  }
  throwContractError(
    "Tessera core list_sources response did not match the versioned envelope contract.",
  );
}

/** Server-derived inventory facts. A null count is deliberately distinct from
 * zero: only full coverage may claim a complete record count. */
export interface SourceInventory {
  source_id: SourceId;
  provider: string;
  lifecycle_state: SourceLifecycle;
  root: string;
  native_project: string | null;
  coverage_level: CandidateSource["coverage_level"];
  health_state: HealthState;
  last_successful_scan: number | null;
  complete_record_count: number | null;
  latest_error: string | null;
}

function asInventory(value: unknown): SourceInventory | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (!isString(v.source_id) || !isString(v.provider) || !(typeof v.lifecycle_state === "string" && VALID_LIFECYCLES.has(v.lifecycle_state)) || !isString(v.root) ||
    !isOptionalStringOrNull(v.native_project) ||
    !(typeof v.coverage_level === "string" && VALID_COVERAGE_LEVELS.has(v.coverage_level)) ||
    !(typeof v.health_state === "string" && VALID_HEALTH_STATES.has(v.health_state)) ||
    !(v.last_successful_scan === null || isNumber(v.last_successful_scan)) ||
    !(v.complete_record_count === null || isNumber(v.complete_record_count)) ||
    !isOptionalStringOrNull(v.latest_error)) return null;
  return v as unknown as SourceInventory;
}

export async function getSourceInventory(): Promise<Envelope<SourceInventory[]>> {
  const envelope = (await apiGet("/api/sources/inventory")) as Envelope<SourceInventory[]> | null;
  if (envelope && envelope.api_version === API_VERSION && Array.isArray(envelope.payload) && envelope.payload.every((item) => asInventory(item) !== null)) {
    return { api_version: envelope.api_version, payload: envelope.payload.map((item) => asInventory(item) as SourceInventory) };
  }
  throwContractError("Tessera core Inventory response did not match the versioned envelope contract.");
}
