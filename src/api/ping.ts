/**
 * Tessera — typed TS client mirroring the Rust API envelope (Phase 0).
 *
 * This module is the single source of truth on the UI side for the `ping`
 * endpoint's request/response shape. It must stay in lock-step with
 * `server/src/http/envelope.rs`. Bumping `API_VERSION` here requires the
 * matching bump on the Rust side (AD-17/A-6).
 *
 * The UI never touches Providers, the filesystem, or SQLite directly
 * (AD-1); every call goes through the loopback HTTP API served by the Rust
 * core (revised AD-9).
 */

import { apiGet, type Envelope, type TesseraApiError } from "./client";

export type { Envelope, TesseraApiError };
export { API_VERSION } from "./client";
import { API_VERSION } from "./client";

/** `ping` payload — Phase 0 contract sample. Mirrors Rust `Pong`. */
export interface Pong {
  /** Crate name from `CARGO_PKG_NAME` at build time. */
  name: string;
  /** Crate version from `CARGO_PKG_VERSION` at build time. */
  version: string;
}

/**
 * Call the `ping` endpoint and return the typed, versioned envelope.
 *
 * Phase 0 contract sample; later endpoints mirror this pattern. The Rust
 * `Envelope<Pong>` arrives here unchanged as `{ api_version, payload }`.
 *
 * On any shape drift we throw loudly so the React shell renders the error
 * state — Phase 0's whole purpose is to prove the typed round-trip, so we
 * never fabricate a fake `Pong` to mask a broken contract.
 */
export async function ping(): Promise<Envelope<Pong>> {
  const envelope = (await apiGet("/api/ping")) as Envelope<Pong> | null;
  if (
    envelope &&
    envelope.api_version === API_VERSION &&
    envelope.payload !== null &&
    typeof envelope.payload === "object" &&
    typeof envelope.payload.name === "string" &&
    typeof envelope.payload.version === "string"
  ) {
    return envelope;
  }
  // Contract drift: surface it as a structured Tessera error instead of
  // inventing a successful pong. App.tsx only honours messages carrying a
  // known stable code (AD-12/AD-13 redaction).
  throw {
    code: "api_contract",
    message: "Tessera core ping response did not match the versioned envelope contract.",
    source_id: null,
    phase: "transport",
  } satisfies TesseraApiError;
}
