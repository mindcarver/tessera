/**
 * Tessera — typed TS clients for the `scan_source` / `get_scan_status`
 * endpoints (Story 1.4).
 *
 * Mirrors the Rust types in `server/src/domain/scan.rs`
 * (ScanOutcome / ScanStatus / ScanRunState / Generation) and the versioned
 * envelope in `server/src/http/envelope.rs`. Must stay in lock-step with the
 * Rust side: any field rename / removal / type change here is a contract break
 * that requires an `api_version` bump (AD-17/A-6).
 *
 * The UI never touches Providers, the filesystem, or SQLite directly (AD-1);
 * every call goes through the loopback HTTP API (revised AD-9).
 *
 * Invariants honored by this module:
 * - **Versioned envelopes:** every response is validated against
 *   `API_VERSION`; any drift throws `TesseraApiError` with code `api_contract`
 *   (Phase 0 review finding: never fabricate a fake success).
 * - **Safe surface (AD-13):** `ScanOutcome` / `ScanStatus` carry only counts,
 *   generation identity, and the latest run state — never memory body, never
 *   query text, never path detail beyond what the user already confirmed.
 * - **Honest narrow types:** `ScanRunState` is the literal union of the stable
 *   wire strings, matching Rust's `#[serde(rename_all = "snake_case")]`.
 */

import { apiGet, apiPost } from "./client";
import type { Envelope, SourceId, TesseraApiError } from "./sources";
import { API_VERSION } from "./client";

// Re-export the shared shapes so feature code can import from one place.
export type { Envelope, SourceId, TesseraApiError };

// ---------------------------------------------------------------------------
// Scan domain mirror (Rust: domain::scan)
// ---------------------------------------------------------------------------

/**
 * The persisted scan state machine (AD-5/AD-16). Mirrors Rust `ScanRunState`
 * with `#[serde(rename_all = "snake_case")]`. `retry` exists in the enum but
 * is never written in Story 1.4 (bounded retry is Carver manually re-scanning).
 */
export type ScanRunState =
  | "queued"
  | "running"
  | "staging"
  | "committing"
  | "succeeded"
  | "failed"
  | "retry";

/**
 * Opaque generation handle (`gen_<n>`). Mirrors Rust `Generation`.
 */
export type Generation = string;

/**
 * The DTO returned by a successful `scan_source` call. Mirrors Rust
 * `ScanOutcome`. Carries only counts and generation identity (AD-13 safe
 * surface). `Ok` means success, so there is no redundant `outcome` field.
 */
export interface ScanOutcome {
  /** The scanned Source's stable handle. */
  source_id: SourceId;
  /** The `scan_runs` AUTOINCREMENT id of the run that committed. */
  scan_id: number;
  /** The generation that became active. */
  generation: Generation;
  /** Number of file-level records indexed into the active generation. */
  records_indexed: number;
}

/**
 * The DTO returned by `get_scan_status`. Mirrors Rust `ScanStatus`. Reports
 * the most recent run state plus the active generation and record count.
 */
export interface ScanStatus {
  /** The Source this status describes. */
  source_id: SourceId;
  /** State of the most recent run, or `null` when never scanned. */
  state: ScanRunState | null;
  /** The active generation, or `null` when none committed yet. */
  active_generation: Generation | null;
  /** Number of records in the active generation (`0` when none). */
  active_records: number;
}

// ---------------------------------------------------------------------------
// Runtime shape guards
// ---------------------------------------------------------------------------

const VALID_SCAN_STATES: ReadonlySet<string> = new Set([
  "queued",
  "running",
  "staging",
  "committing",
  "succeeded",
  "failed",
  "retry",
]);

function isString(v: unknown): v is string {
  return typeof v === "string";
}

function isNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

function isScanRunStateOrNull(v: unknown): v is ScanRunState | null {
  return v === null || (typeof v === "string" && VALID_SCAN_STATES.has(v));
}

function isGenerationOrNull(v: unknown): v is Generation | null {
  return v === null || typeof v === "string";
}

/** Narrow an unknown value to a `ScanOutcome`, or `null` on shape drift. */
function asScanOutcome(value: unknown): ScanOutcome | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.source_id) ||
    !isNumber(v.scan_id) ||
    !isString(v.generation) ||
    !isNumber(v.records_indexed)
  ) {
    return null;
  }
  return {
    source_id: v.source_id,
    scan_id: v.scan_id,
    generation: v.generation,
    records_indexed: v.records_indexed,
  };
}

/** Narrow an unknown value to a `ScanStatus`, or `null` on shape drift. */
function asScanStatus(value: unknown): ScanStatus | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (
    !isString(v.source_id) ||
    !isScanRunStateOrNull(v.state) ||
    !isGenerationOrNull(v.active_generation) ||
    !isNumber(v.active_records)
  ) {
    return null;
  }
  return {
    source_id: v.source_id,
    state: v.state as ScanRunState | null,
    active_generation: v.active_generation,
    active_records: v.active_records,
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
// Clients (mirror the two Rust endpoints)
// ---------------------------------------------------------------------------

/**
 * Run the read-only scan pipeline for a confirmed Source (AD-1). Resolves to
 * the versioned `ScanOutcome` on a fully-successful scan; rejects with a
 * structured `TesseraApiError` (`scan_failed` / `source_not_found` /
 * `confirm_failed` / `internal` / `api_contract`) otherwise. A failed scan
 * never activates a partial generation (NFR-9).
 */
export async function scanSource(sourceId: SourceId): Promise<Envelope<ScanOutcome>> {
  const envelope = (await apiPost("/api/scan", {
    source_id: sourceId,
  })) as Envelope<ScanOutcome> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asScanOutcome(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asScanOutcome(envelope.payload) as ScanOutcome,
    };
  }
  throwContractError(
    "Tessera core scan_source response did not match the versioned envelope contract.",
  );
}

/**
 * Report the scan status of a Source (latest run state + active generation +
 * record count). Resolves to the versioned `ScanStatus`; rejects with
 * `source_not_found` for an unknown id, or `api_contract` on shape drift.
 */
export async function getScanStatus(sourceId: SourceId): Promise<Envelope<ScanStatus>> {
  const envelope = (await apiGet(
    `/api/scan/status?source_id=${encodeURIComponent(sourceId)}`,
  )) as Envelope<ScanStatus> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asScanStatus(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asScanStatus(envelope.payload) as ScanStatus,
    };
  }
  throwContractError(
    "Tessera core get_scan_status response did not match the versioned envelope contract.",
  );
}
