import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import { discoverSources, type CandidateSource } from "../../api/discover";
import {
  confirmSource,
  disableSource,
  getSourceInventory,
  rejectSource,
  type HealthState,
  type SourceInventory,
} from "../../api/sources";
import { cancelRescan, getRescanProgress, startRescan, type RescanProgress } from "../../api/scan";
import { rebuildIndex } from "../../api/index";
import { readTesseraErrorMessage } from "../../api/errors";
import { providerDisplayName } from "../../components/providerDisplayName";

type LoadState<T> = { kind: "loading" } | { kind: "error"; message: string } | { kind: "ok"; value: T };

/**
 * Story 3.1 — the minimum facts App needs to swap into the Browse view for a
 * confirmed source. Passed up from the Inventory card when the user
 * activates "Browse"; App threads them into `<Browse>`'s heading so the user
 * sees which source they are browsing without re-fetching the row.
 */
export interface BrowseEntry {
  source_id: string;
  provider: string;
  native_project: string | null;
}

interface SourcesProps {
  /**
   * Story 3.1 — invoked when the user activates "Browse" on a confirmed
   * source's Inventory card. The parent (App) swaps in the Browse view for
   * that source id. Optional so existing callers that do not wire Browse
   * (e.g. unit tests) keep working unchanged.
   */
  onBrowse?: (source: BrowseEntry) => void;
}

/** Source Inventory is intentionally server-derived: the browser renders
 * coverage and health facts but never infers them from paths or scan output. */
export function Sources({ onBrowse }: SourcesProps = {}): ReactElement {
  const [candidates, setCandidates] = useState<LoadState<CandidateSource[]>>({ kind: "loading" });
  const [inventory, setInventory] = useState<LoadState<SourceInventory[]>>({ kind: "loading" });
  const [progress, setProgress] = useState<Record<string, RescanProgress>>({});
  const [message, setMessage] = useState("");
  // Story 4.4 — Rebuild confirm region + in-flight status. `rebuildConfirm`
  // toggles the inline confirm region (no modal infra exists). `rebuildStatus`
  // is the polite `aria-live` announcement ("Rebuilding…"); `rebuildError`
  // surfaces a `rebuild_failed` (409) envelope so the user knows to wait or
  // cancel the in-flight scan. `rebuilding` disables the button while in
  // flight so a double-click cannot race the dispatch.
  const [rebuildConfirm, setRebuildConfirm] = useState(false);
  const [rebuildStatus, setRebuildStatus] = useState<string>("");
  const [rebuildError, setRebuildError] = useState<string>("");
  const rebuilding = rebuildStatus !== "";
  // Patch UI — the rebuild settle-poll uses a `pollToken` to break the
  // setTimeout chain when the user is done. Lifted into a ref + cleared on
  // unmount so the per-tick inventory fetch + setTimeout chain stop when the
  // component unmounts (otherwise a slow post-unmount tick would call
  // setInventory/setRebuildStatus on an unmounted component, a React warning
  // + a leaked fetch).
  const rebuildPollTokenRef = useRef<{ stop: boolean }>({ stop: false });
  const rebuildConfirmRef = useRef<HTMLDivElement>(null);
  const timers = useRef<Record<string, number>>({});

  const refresh = useCallback(() => {
    getSourceInventory()
      .then((result) => setInventory({ kind: "ok", value: result.payload }))
      .catch((error: unknown) => setInventory({ kind: "error", message: readTesseraErrorMessage(error) }));
  }, []);

  useEffect(() => {
    discoverSources()
      .then((result) => setCandidates({ kind: "ok", value: result.payload }))
      .catch((error: unknown) => setCandidates({ kind: "error", message: readTesseraErrorMessage(error) }));
    refresh();
    return () => Object.values(timers.current).forEach((timer) => window.clearInterval(timer));
  }, [refresh]);

  const updateProgress = useCallback((sourceId: string, jobId: string) => {
    getRescanProgress(sourceId, jobId)
      .then((events) => {
        const latest = events.at(-1);
        if (!latest) return;
        setProgress((current) => ({ ...current, [sourceId]: latest }));
        setMessage(latest.message);
        if (["succeeded", "failed", "cancelled"].includes(latest.state)) {
          window.clearInterval(timers.current[sourceId]);
          delete timers.current[sourceId];
          refresh();
        }
      })
      .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [refresh]);

  const onRescan = useCallback((sourceId: string) => {
    startRescan(sourceId)
      .then((result) => {
        setProgress((current) => ({ ...current, [sourceId]: result.payload }));
        setMessage(result.payload.message);
        window.clearInterval(timers.current[sourceId]);
        timers.current[sourceId] = window.setInterval(() => updateProgress(sourceId, result.payload.job_id), 250);
      })
      .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [updateProgress]);

  const onCancel = useCallback((sourceId: string) => {
    cancelRescan(sourceId)
      .then((result) => {
        setProgress((current) => ({ ...current, [sourceId]: result.payload }));
        setMessage(result.payload.message);
        window.clearInterval(timers.current[sourceId]);
        delete timers.current[sourceId];
        refresh();
      })
      .catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [refresh]);

  const resolveCandidate = useCallback((candidate: CandidateSource, action: "confirm" | "reject") => {
    const request = action === "confirm" ? confirmSource(candidate) : rejectSource(candidate);
    request.then(() => refresh()).catch((error: unknown) => setMessage(readTesseraErrorMessage(error)));
  }, [refresh]);

  // Story 4.4 — open the rebuild confirm region (focus moves into it so the
  // keyboard user hears the warning before any destructive call). a11y
  // contract (AD-21): the warning is `role="alert"`, the region is
  // keyboard-reachable, and the destructive action is a separate explicit
  // "Rebuild now" activation (no implicit confirm).
  const openRebuildConfirm = useCallback(() => {
    setRebuildConfirm(true);
    setRebuildError("");
    // Move focus into the confirm region on the next tick (after render).
    window.setTimeout(() => rebuildConfirmRef.current?.focus(), 0);
  }, []);

  const cancelRebuildConfirm = useCallback(() => {
    setRebuildConfirm(false);
    setRebuildError("");
  }, []);

  // Story 4.4 — confirm the rebuild. The wipe runs server-side; once the
  // response arrives, the per-source re-scans have been dispatched. Clear
  // "Rebuilding…" status after a bounded number of inventory refreshes so a
  // slow re-scan cannot wedge the indicator forever. Per-source progress is
  // visible via the existing inventory row's `last_successful_scan` /
  // `latest_error` columns and via the per-source rescan SSE if the user
  // starts a separate rescan (the rebuild's worker threads emit progress
  // into the same `rescan_jobs` map, but the rebuild UI intentionally does
  // not subscribe to every source's SSE channel — the inventory refresh is
  // the cross-source "settled" signal the spec Design Names).
  const confirmRebuild = useCallback(() => {
    setRebuildError("");
    setRebuildStatus("Rebuilding…");
    setRebuildConfirm(false);
    // Patch UI — reset the poll token; the unmount effect (or the next
    // confirmRebuild) is responsible for stopping any prior poll chain.
    rebuildPollTokenRef.current = { stop: false };
    const pollToken = rebuildPollTokenRef.current;
    rebuildIndex()
      .then((outcome) => {
        // If the component unmounted (or a new rebuild started) while we
        // were waiting on the response, bail before touching state.
        if (pollToken.stop) return;
        const expected = outcome.payload.sources_rescanning;
        // No re-scans to wait for: clear status immediately after refreshing
        // inventory (the wipe still ran, clearing any leaked disabled /
        // rejected records — the inventory refresh shows the empty index).
        if (expected === 0) {
          refresh();
          // Keep the "Rebuilding…" announcement visible briefly so the
          // screen-reader user hears the operation ran, then clear.
          window.setTimeout(() => {
            if (!pollToken.stop) setRebuildStatus("");
          }, 800);
          return;
        }
        // Poll inventory at a bounded cadence. After a small number of
        // refreshes, clear the status indicator regardless of whether every
        // re-scan has reached a terminal state — the per-source rows surface
        // their own health / latest_error, so the user can see ongoing
        // failures there. This avoids the indicator wedging forever on a
        // slow / stuck re-scan.
        const POLL_INTERVAL_MS = 500;
        const POLL_MAX_TICKS = 20; // ~10 seconds at 500ms
        let ticks = 0;
        const tick = () => {
          if (pollToken.stop) return;
          ticks += 1;
          getSourceInventory()
            .then((result) => {
              if (pollToken.stop) return;
              setInventory({ kind: "ok", value: result.payload });
            })
            .catch(() => {
              // Inventory fetch failed mid-rebuild — keep the rebuild
              // status visible so the user knows the operation may still
              // be in flight; they can manually refresh.
            })
            .finally(() => {
              if (pollToken.stop) return;
              if (ticks >= POLL_MAX_TICKS) {
                pollToken.stop = true;
                setRebuildStatus("");
                return;
              }
              window.setTimeout(tick, POLL_INTERVAL_MS);
            });
        };
        // Refresh once immediately so the wipe is reflected, then poll.
        refresh();
        window.setTimeout(tick, POLL_INTERVAL_MS);
      })
      .catch((error: unknown) => {
        if (pollToken.stop) return;
        setRebuildStatus("");
        setRebuildError(readTesseraErrorMessage(error));
      });
  }, [refresh]);

  // Patch UI — stop the rebuild poll chain on unmount so a slow tick + its
  // inventory fetch do not fire after the component is gone (React warning +
  // leaked fetch).
  useEffect(() => {
    return () => {
      rebuildPollTokenRef.current.stop = true;
    };
  }, []);

  // Esc closes the rebuild confirm region (a11y contract — keyboard exit).
  useEffect(() => {
    if (!rebuildConfirm) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        setRebuildConfirm(false);
        setRebuildError("");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [rebuildConfirm]);

  return <section aria-label="Tessera sources">
    <h2>Sources</h2>
    <p aria-live="polite" data-testid="rescan-progress" className="visually-hidden-text">{message}</p>
    <section aria-label="Discovered candidate sources">
      <h3>Candidates</h3>
      {candidates.kind === "loading" ? <p>Looking for supported Agent Memory sources…</p> : null}
      {candidates.kind === "error" ? <p role="alert">{candidates.message}</p> : null}
      {candidates.kind === "ok" && candidates.value.length === 0 ? <p>No supported Agent Memory sources were found on this machine.</p> : null}
      {candidates.kind === "ok" ? <ul>{candidates.value.map((candidate) => <li key={`${candidate.provider}:${candidate.root_path}`}><strong>{candidate.provider}</strong> — {describeCoverage(candidate.coverage_level)} <button type="button" onClick={() => resolveCandidate(candidate, "confirm")}>Confirm</button> <button type="button" onClick={() => resolveCandidate(candidate, "reject")}>Reject</button></li>)}</ul> : null}
    </section>
    <section aria-label="Source inventory" aria-busy={inventory.kind === "loading"}>
      <h3>Inventory</h3>
      {/*
        Story 4.4 — keyboard-reachable "Rebuild index" button in the inventory
        header. Activating it opens an INLINE confirm region (no modal infra
        exists in the app — `grep -r 'role="dialog"' src/` returns zero). The
        region carries a `role="alert"` warning so the user is clearly told
        BEFORE the destructive call that only Tessera-derived data is deleted
        (Confirmed sources and project mappings are kept, source files are
        never modified). The destructive action is a separate explicit
        "Rebuild now" activation; Esc / "Cancel" closes the region.
        Disabled while a rebuild is in flight so a double-click cannot race
        the dispatch (the server's race guard already rejects concurrent
        rebuilds with 409 `rebuild_failed`; this is belt-and-suspenders).
      */}
      <p>
        <button
          type="button"
          onClick={openRebuildConfirm}
          disabled={rebuilding}
          aria-expanded={rebuildConfirm}
          aria-controls="rebuild-confirm-region"
        >Rebuild index</button>
      </p>
      {rebuildConfirm ? (
        <div
          id="rebuild-confirm-region"
          ref={rebuildConfirmRef}
          tabIndex={-1}
          role="group"
          aria-label="Rebuild index confirmation"
        >
          <p role="alert">
            Deletes only Tessera-derived index data. Confirmed sources and project mappings are kept. Source files are never modified.
          </p>
          <button type="button" onClick={confirmRebuild}>Rebuild now</button>
          {" "}
          <button type="button" onClick={cancelRebuildConfirm}>Cancel</button>
        </div>
      ) : null}
      {rebuildStatus ? (
        <p data-testid="rebuild-status" role="status" aria-live="polite">{rebuildStatus}</p>
      ) : null}
      {rebuildError ? (
        <p role="alert">{rebuildError}</p>
      ) : null}
      {inventory.kind === "loading" ? <p>Loading source inventory…</p> : null}
      {inventory.kind === "error" ? <p role="alert">{inventory.message}</p> : null}
      {inventory.kind === "ok" && inventory.value.length === 0 ? <p>No sources have been confirmed yet.</p> : null}
      {inventory.kind === "ok" && inventory.value.length > 0 ? <>
        <p data-testid="inventory-summary" role="status">{inventorySummary(inventory.value)}</p>
        <section data-testid="source-inventory">{groupInventoryByProvider(inventory.value).map((group) => {
          const name = providerDisplayName(group.provider);
          return <section key={group.provider} aria-label={`${name} provider group`} data-provider-group={group.provider}>
            <h4>{name}</h4>
            <ul>{group.items.map((item) => <InventoryCard key={item.source_id} item={item} progress={progress[item.source_id]} onRescan={onRescan} onCancel={onCancel} onDisable={(id) => disableSource(id).then(refresh).catch((error: unknown) => setMessage(readTesseraErrorMessage(error)))} onBrowse={onBrowse ? () => onBrowse({ source_id: item.source_id, provider: item.provider, native_project: item.native_project }) : undefined} />)}</ul>
          </section>;
        })}</section>
      </> : null}
    </section>
  </section>;
}

function InventoryCard({ item, progress, onRescan, onCancel, onDisable, onBrowse }: { item: SourceInventory; progress?: RescanProgress; onRescan: (id: string) => void; onCancel: (id: string) => void; onDisable: (id: string) => void; onBrowse?: () => void }): ReactElement {
  const running = progress?.state === "queued" || progress?.state === "running";
  return <li data-provider={item.provider}><article>
    <h5>{providerDisplayName(item.provider)} source</h5>
    <dl>
      <dt>Provider</dt><dd>{item.provider}</dd>
      <dt>Lifecycle</dt><dd>{item.lifecycle_state}</dd>
      <dt>Root</dt><dd><code>{item.root}</code></dd>
      <dt>Native project</dt><dd>{item.native_project ?? "Not mapped"}</dd>
      <dt>Coverage</dt><dd>{describeCoverage(item.coverage_level)}</dd>
      <dt>Health</dt><dd>{item.health_state}</dd>
      <dt>Last successful scan</dt><dd>{item.last_successful_scan === null ? "No successful scan yet." : new Date(item.last_successful_scan * 1000).toLocaleString()}</dd>
      <dt>Record count</dt><dd>{item.complete_record_count === null ? "Complete count unavailable: coverage is limited." : `${item.complete_record_count} complete indexed ${item.complete_record_count === 1 ? "record" : "records"}.`}</dd>
      {item.latest_error ? <><dt>Latest safe error</dt><dd>{item.latest_error}</dd></> : null}
      {progress ? <><dt>Rescan progress</dt><dd>{progress.state}: {progress.message}</dd></> : null}
    </dl>
    {item.health_state !== "unknown" || item.complete_record_count !== null ? null : <p>Inventory has not yet established source health.</p>}
    {item.lifecycle_state === "confirmed" ? <>
      <button type="button" onClick={() => onRescan(item.source_id)} disabled={running}>Rescan</button>
      {running ? <button type="button" onClick={() => onCancel(item.source_id)}>Cancel rescan</button> : null}
      <button type="button" onClick={() => onDisable(item.source_id)}>Disable</button>
      {/*
        Story 3.1 — the per-source Browse entry affordance. Rendered ONLY on
        confirmed sources (the I/O matrix forbids browse for disabled /
        rejected / unconfirmed). Activating it switches App's view state to
        `<Browse>` for this `source_id`. Keyboard-reachable by default
        (`<button type="button">`).
      */}
      {onBrowse ? <button type="button" onClick={onBrowse}>Browse</button> : null}
    </> : null}
  </article></li>;
}

function describeCoverage(level: CandidateSource["coverage_level"]): string {
  switch (level) {
    case "full": return "Full coverage";
    case "search_only": return "Search-only coverage; complete count unavailable";
    case "existence_only": return "Existence-only coverage; complete count unavailable";
    case "unsupported": return "Unsupported coverage; complete count unavailable";
  }
}

/**
 * Story 2.5 — multi-provider panorama helpers. The backend already returns
 * every confirmed source (any provider) on the inventory endpoint; these
 * pure functions shape that flat list into a comparable, grouped view:
 *
 * - `healthSeverityRank` — attention-first ordering within a provider group
 *   (`error` > `degraded` > `healthy` > `unknown`) so the worst card sorts to
 *     the top of its group.
 * - `groupInventoryByProvider` — one section per provider (stable alphabetical
 *   order so the panorama does not flicker on confirmation order), each
 *   group's cards sorted by health severity.
 * - `inventorySummary` — the cross-source health header counting sources by
 *   health state so Carver can compare providers' health/coverage at a glance.
 *
 * No DTO change, no server-side aggregation: grouping/sorting is a client-side
 * render decision (Boundaries: Never introduce server-side persistence of a
 * "panorama" or grouping state).
 */
function healthSeverityRank(state: HealthState): number {
  switch (state) {
    case "error": return 0;
    case "degraded": return 1;
    case "healthy": return 2;
    case "unknown": return 3;
    // Treat any unexpected (future-widened) HealthState as least-attention-
    // worthy so a new state can never yield `NaN` and scramble the sort.
    default: return Number.MAX_SAFE_INTEGER;
  }
}

interface InventoryGroup {
  provider: string;
  items: SourceInventory[];
}

function groupInventoryByProvider(items: SourceInventory[]): InventoryGroup[] {
  const groups = new Map<string, SourceInventory[]>();
  for (const item of items) {
    const bucket = groups.get(item.provider);
    if (bucket === undefined) {
      groups.set(item.provider, [item]);
    } else {
      bucket.push(item);
    }
  }
  for (const bucket of groups.values()) {
    bucket.sort((a, b) => healthSeverityRank(a.health_state) - healthSeverityRank(b.health_state));
  }
  return [...groups.entries()]
    .sort(([a], [b]) => a.localeCompare(b, "en", { sensitivity: "base" }))
    .map(([provider, bucket]) => ({ provider, items: bucket }));
}

function inventorySummary(items: SourceInventory[]): string {
  const total = items.length;
  const counts: Record<HealthState, number> = { healthy: 0, degraded: 0, error: 0, unknown: 0 };
  for (const item of items) counts[item.health_state] += 1;
  const parts: string[] = [`${total} source${total === 1 ? "" : "s"}`];
  // Attention-first: surface actionable states first (error > degraded >
  // healthy > unknown), matching the within-group health sort. Only non-zero
  // categories appear so an all-healthy inventory stays compact
  // ("3 sources · 3 healthy"). Each health noun pluralizes the same way as
  // "source" ("2 errors", "1 healthy").
  const order: HealthState[] = ["error", "degraded", "healthy", "unknown"];
  for (const state of order) {
    if (counts[state] > 0) parts.push(`${counts[state]} ${state}${counts[state] === 1 ? "" : "s"}`);
  }
  return parts.join(" · ");
}
