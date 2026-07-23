/**
 * Tessera — loopback HTTP transport for the versioned API (revised AD-9,
 * 2026-07-22).
 *
 * Single source of truth for talking to the Rust core. The delivery form is
 * a local web app: the built UI is served by the Rust core itself, so all API
 * calls are same-origin relative fetches (`/api/...`). In development, Vite
 * proxies `/api` to the loopback server (see `vite.config.ts`).
 *
 * Contract rules shared by every client module:
 * - **Versioned envelopes (AD-17/A-6):** success responses carry
 *   `{ api_version, payload }`; error responses carry the structured
 *   `ErrorEnvelope` `{ code, message, source_id, phase }` (AD-13). A non-2xx response whose
 *   body does not match the error envelope is a contract violation and is
 *   surfaced as `api_contract` — never as a fabricated success.
 * - **No credentials, ever:** same-origin fetches send no cookies cross-site,
 *   and the server sets none. Nothing else to configure.
 */

/** API contract major version. Must match `envelope::API_VERSION` in Rust. */
export const API_VERSION = "1" as const;

/** Versioned success envelope wrapping a typed payload. Mirrors Rust `Envelope<T>`. */
export interface Envelope<T> {
  /** API contract major version (string, e.g. `"1"`). */
  api_version: string;
  /** Endpoint-specific typed payload. */
  payload: T;
}

/**
 * Structured error envelope (AD-13). Stable `code` + safe `message` +
 * source-scoped operation context; never carries body, query text, or
 * credentials.
 */
export interface TesseraApiError {
  code: string;
  message: string;
  source_id: string | null;
  phase: string;
}

/**
 * Perform a GET against the versioned API and return the raw parsed body.
 * Callers validate the success shape against `API_VERSION` and their own
 * narrow types; this helper only guarantees transport + error-envelope
 * semantics.
 */
export async function apiGet(path: string): Promise<unknown> {
  return request(path, { method: "GET" });
}

/**
 * Perform a POST with a JSON body against the versioned API. The body is the
 * endpoint's request DTO (e.g. `{ candidate }` / `{ source_id }`).
 */
export async function apiPost(path: string, body: unknown): Promise<unknown> {
  return request(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

async function request(path: string, init: RequestInit): Promise<unknown> {
  let response: Response;
  try {
    response = await fetch(path, init);
  } catch {
    // Network-level failure against a loopback server means the core is not
    // running — surface the same stable code the UI already renders.
    throw {
      code: "internal",
      message: "Tessera core did not respond. Try restarting the app.",
      source_id: null,
      phase: "transport",
    } satisfies TesseraApiError;
  }

  const text = await response.text();
  let parsed: unknown = null;
  if (text.length > 0) {
    try {
      parsed = JSON.parse(text);
    } catch {
      parsed = null;
    }
  }

  if (!response.ok) {
    // Error path: the body must be the structured ErrorEnvelope (AD-13).
    if (isErrorEnvelope(parsed)) {
      throw parsed satisfies TesseraApiError;
    }
    throw {
      code: "api_contract",
      message:
        "Tessera core error response did not match the structured error envelope contract.",
      source_id: null,
      phase: "transport",
    } satisfies TesseraApiError;
  }
  return parsed;
}

function isErrorEnvelope(value: unknown): value is TesseraApiError {
  if (!value || typeof value !== "object") return false;
  const v = value as Record<string, unknown>;
  return (
    typeof v.code === "string" &&
    typeof v.message === "string" &&
    (typeof v.source_id === "string" || v.source_id === null) &&
    typeof v.phase === "string"
  );
}
