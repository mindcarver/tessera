/**
 * Tessera — typed TS client for the `POST /api/index/rebuild` endpoint
 * (Story 4.4).
 *
 * Mirrors the Rust types in `server/src/application/rebuild.rs`
 * (RebuildError → envelope code) and `server/src/http/mod.rs`
 * (RebuildOutcome). Must stay in lock-step with the Rust side: any field
 * rename / removal / type change here is a contract break that requires an
 * `api_version` bump (AD-17/A-6).
 *
 * Invariants honored by this module (mirrors `./scan.ts`):
 * - **Versioned envelopes:** every response is validated against
 *   `API_VERSION`; any drift throws `TesseraApiError` with code `api_contract`.
 * - **Safe surface (AD-13):** `RebuildOutcome` carries only a count of the
 *   Confirmed Sources being re-scanned — never memory body, query text, or
 *   any path detail.
 * - **Stable error codes:** a 409 response carries `rebuild_failed` (a scan
 *   is currently in-flight); any other non-2xx is surfaced via the existing
 *   `readTesseraErrorMessage` helper in `./errors.ts`.
 */

import { apiPost } from "./client";
import type { Envelope, TesseraApiError } from "./sources";
import { API_VERSION } from "./client";

// Re-export the shared shapes so feature code can import from one place.
export type { Envelope, TesseraApiError };

// ---------------------------------------------------------------------------
// Rebuild domain mirror (Rust: application::rebuild + http::RebuildOutcome)
// ---------------------------------------------------------------------------

/**
 * The DTO returned by a successful `POST /api/index/rebuild`. Mirrors Rust
 * `RebuildOutcome`. Carries only a count of the Confirmed Sources being
 * re-scanned (AD-13 safe surface). A count of `0` means the registry had no
 * Confirmed Sources — the wipe still ran, clearing any leaked disabled /
 * rejected records.
 */
export interface RebuildOutcome {
  /** Count of Confirmed Sources the rebuild dispatched a re-scan for. */
  sources_rescanning: number;
}

// ---------------------------------------------------------------------------
// Runtime shape guards
// ---------------------------------------------------------------------------

function isNumber(v: unknown): v is number {
  return typeof v === "number" && Number.isFinite(v);
}

/** Narrow an unknown value to a `RebuildOutcome`, or `null` on shape drift. */
function asRebuildOutcome(value: unknown): RebuildOutcome | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Record<string, unknown>;
  if (!isNumber(v.sources_rescanning) || v.sources_rescanning < 0) {
    return null;
  }
  return { sources_rescanning: v.sources_rescanning };
}

/** Throw a structured API contract error (mirrors `./scan.ts`). */
function throwContractError(message: string): never {
  throw {
    code: "api_contract",
    message,
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}

// ---------------------------------------------------------------------------
// Client
// ---------------------------------------------------------------------------

/**
 * Trigger a full Derived Index rebuild (Story 4.4). The wipe is server-side
 * and atomic; this call returns once the wipe has committed and the rebuild's
 * per-source re-scans have been DISPATCHED (not finished). Per-source progress
 * flows through the existing rescan SSE/inventory surfaces, keyed by
 * `source_id`.
 *
 * Rejects with a structured `TesseraApiError`:
 * - `rebuild_failed` (409) — a scan is currently in-flight across any
 *   source. The user should wait for or cancel the in-flight scan, then
 *   retry.
 * - `internal` (500) — wipe / DB failure.
 * - `api_contract` — the response did not match the versioned envelope.
 */
export async function rebuildIndex(): Promise<Envelope<RebuildOutcome>> {
  const envelope = (await apiPost("/api/index/rebuild", {})) as
    | Envelope<RebuildOutcome>
    | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    asRebuildOutcome(envelope.payload) !== null
  ) {
    return {
      api_version: envelope.api_version,
      payload: asRebuildOutcome(envelope.payload) as RebuildOutcome,
    };
  }
  throwContractError(
    "Tessera core rebuild response did not match the versioned envelope contract.",
  );
}
