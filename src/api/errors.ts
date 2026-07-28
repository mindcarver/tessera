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
  // Story 6.3: a Knowledge (Obsidian Vault) confirm was rejected because its
  // root overlaps an already-confirmed Source. Surfacing this safe message is
  // what lets a failed confirm show visible feedback instead of falling back
  // to the generic "core did not respond" string (which reads like a crash and
  // contributed to the "点击确认没反应" symptom).
  "root_overlap",
  "source_not_found",
  // Story 1.4: a scan failed (mid-scan read failure, source changed during
  // scan, commit CAS loss, non-confirmed source). The previous index is
  // unchanged (NFR-9).
  "scan_failed",
  "cursor_stale",
  "record_not_found",
  "open_failed",
  "rescan_not_running",
  // Story 4.4: rebuild was rejected because a scan is currently in-flight
  // across any source. The previous index is unchanged; the user should wait
  // for or cancel the in-flight scan, then retry the rebuild.
  "rebuild_failed",
  // Story 5.1: project + mapping errors. `project_not_found` / `mapping_not_found`
  // surface when a `project_id`-keyed or `(provider, native_project)`-keyed
  // operation targets nothing. `mapping_conflict` surfaces when `addMapping`
  // rejects because the scope is already owned by another project (AD-27
  // cardinality); the safe message names the owning project so the user can
  // see who owns the scope.
  "project_not_found",
  "mapping_not_found",
  "mapping_conflict",
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
