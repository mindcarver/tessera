/**
 * Tessera — shared API error-message helper (AD-12/AD-13 redaction).
 *
 * Single source of truth for coercing a thrown value into a safe user-facing
 * message. Only messages carrying a known Tessera stable `code` are surfaced;
 * everything else falls back to a generic string so raw diagnostics never
 * leak into the DOM. Shared by every feature that catches an API rejection
 * (`App.tsx` ping, `DiscoverSources`, …) so future error codes (1.4/1.5
 * source-scoped codes) land in one place instead of drifting between
 * call sites.
 */

/**
 * Stable Tessera error codes whose `message` is safe to display. Add new
 * stable codes here as the endpoints that emit them ship (AD-13).
 */
export const TESSERA_STABLE_ERROR_CODES: ReadonlySet<string> = new Set([
  "internal",
  "api_contract",
  // Transport hardening (AD-9): wrong Host / cross-origin caller.
  "forbidden_host",
  "forbidden_origin",
  "bad_request",
  "not_found",
  // Story 1.3: confirm/reject could not canonicalize the root (missing /
  // non-dir / non-absolute — NFR-5/6); disable/list surfaced a DB error.
  "confirm_failed",
  "source_not_found",
  // Story 1.4: a scan failed (mid-scan read failure, source changed during
  // scan, commit CAS loss, non-confirmed source). The previous index is
  // unchanged (NFR-9).
  "scan_failed",
  "cursor_stale",
]);

/**
 * Coerce a thrown value into a safe user-facing message. Honours AD-12/AD-13:
 * only surface a structured Tessera error's safe `message` (it must carry a
 * known stable `code`); any other thrown value (JS runtime error, raw fetch
 * rejection) gets the generic string so raw diagnostics never reach the DOM.
 */
export function readTesseraErrorMessage(err: unknown): string {
  if (err && typeof err === "object" && "code" in err && "message" in err) {
    const maybe = err as Partial<{
      code: string;
      message: string;
      source_id: string | null;
      phase: string;
    }>;
    if (
      typeof maybe.code === "string" &&
      TESSERA_STABLE_ERROR_CODES.has(maybe.code) &&
      typeof maybe.message === "string" &&
      maybe.message.length > 0 &&
      (typeof maybe.source_id === "string" || maybe.source_id === null) &&
      typeof maybe.phase === "string"
    ) {
      return maybe.message;
    }
  }
  return "Tessera core did not respond. Try restarting the app.";
}
