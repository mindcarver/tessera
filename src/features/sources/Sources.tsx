/**
 * Tessera — `<Sources />` (Story 1.3).
 *
 * Replaces the 1.2 `<DiscoverSources />` component. On mount it concurrently
 * fetches discovery candidates (`discover_sources`) AND the registered Source
 * inventory (`list_sources`), then renders two regions:
 *
 * 1. **Candidates** — each pre-confirmation candidate gets Confirm / Reject
 *    buttons. Confirming/rejecting persists a Source row and refreshes the
 *    inventory.
 * 2. **Registered sources** — every persisted Source (any lifecycle), grouped
 *    by lifecycle label. Confirmed sources have a Disable button.
 *
 * Honors:
 * - **AC + NFR-13 / AD-21:** semantic `<section aria-label>` regions, real
 *   `<ul>` lists, `aria-live="polite"` status announcements, and keyboard-
 *   reachable buttons (no mouse-only interaction).
 * - **MVP anti-goal (Epic 1 context):** NO "manually add directory" entry.
 *   When there are no candidates AND no registered sources, the empty state
 *   honestly says so.
 * - **AD-3 / AD-18:** coverage_level is rendered honestly; Codex is `full`.
 * - **AD-4:** confirm/reject are the only actions that take a candidate
 *   (which carries a path); disable takes only a `source_id`.
 */

import { useCallback, useEffect, useRef, useState, type ReactElement } from "react";
import {
  discoverSources,
  type CandidateSource,
} from "../../api/discover";
import {
  confirmSource,
  disableSource,
  listSources,
  rejectSource,
  type Source,
  type SourceLifecycle,
} from "../../api/sources";
import {
  getScanStatus,
  scanSource,
  type ScanStatus,
} from "../../api/scan";
import { readTesseraErrorMessage } from "../../api/errors";

type CandidatesState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; candidates: CandidateSource[] }
  | { kind: "error"; message: string };

type SourcesState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; sources: Source[] }
  | { kind: "error"; message: string };

type ActionStatus =
  | { kind: "idle" }
  | { kind: "error"; message: string };

/** Per-source scan status entry. `ok` carries the fetched status;
 * `unavailable` records that the status FETCH failed — the UI must say so
 * honestly rather than silently render "No scan yet" for a source that may
 * in fact have been scanned (AD-3 capability honesty). */
type ScanStatusEntry =
  | { kind: "ok"; status: ScanStatus }
  | { kind: "unavailable" };

/** Per-source scan status, keyed by `source_id`. A missing entry means the
 * status has not been fetched yet; an `unavailable` entry means the fetch
 * failed. Neither is ever rendered as a fabricated "No scan yet". */
type ScanStatusMap = Readonly<Record<string, ScanStatusEntry>>;

/**
 * Render the Sources feature. Fetches candidates + inventory on mount; each
 * confirm / reject / disable refreshes the inventory so the UI reflects
 * persisted state immediately.
 */
export function Sources(): ReactElement {
  const [candidates, setCandidates] = useState<CandidatesState>({ kind: "idle" });
  const [sources, setSources] = useState<SourcesState>({ kind: "idle" });
  const [action, setAction] = useState<ActionStatus>({ kind: "idle" });
  const [scanStatuses, setScanStatuses] = useState<ScanStatusMap>({});
  const [scanningId, setScanningId] = useState<string | null>(null);
  // Race guard: source_ids with an in-flight scan invocation. Multiple Scan
  // buttons could previously be clicked concurrently, starting overlapping
  // scans whose CAS loser surfaced a confusing failure; while ANY scan is in
  // flight, every Scan button is disabled (single-owner semantics, AD-5).
  const inFlightRef = useRef<Set<string>>(new Set());
  // Only the newest status refresh may write state. Confirm/disable/scan
  // callbacks can overlap, and an older request resolving last must not
  // overwrite the inventory's current status map.
  const scanStatusRequestRef = useRef(0);
  const [anyScanning, setAnyScanning] = useState(false);
  const [scanAnnouncement, setScanAnnouncement] = useState<string>("");

  // Fetch the scan status for every confirmed source. Best-effort per source:
  // a single failing status fetch must not blank the whole inventory — but it
  // must not be silently dropped either (that would render a scanned source as
  // "No scan yet"). Failures are recorded as `unavailable` and labeled as such.
  const refreshScanStatuses = useCallback((list: Source[]) => {
    const requestId = ++scanStatusRequestRef.current;
    const confirmed = list.filter((s) => s.lifecycle_state === "confirmed");
    if (confirmed.length === 0) {
      if (requestId === scanStatusRequestRef.current) {
        setScanStatuses({});
      }
      return;
    }
    void Promise.all(
      confirmed.map((s) =>
        getScanStatus(s.source_id)
          .then(
            (envelope): { id: string; entry: ScanStatusEntry } => ({
              id: s.source_id,
              entry: { kind: "ok", status: envelope.payload },
            }),
          )
          .catch((): { id: string; entry: ScanStatusEntry } => ({
            id: s.source_id,
            entry: { kind: "unavailable" },
          })),
      ),
    ).then((results) => {
      if (requestId !== scanStatusRequestRef.current) return;
      const map: Record<string, ScanStatusEntry> = {};
      for (const r of results) {
        map[r.id] = r.entry;
      }
      setScanStatuses(map);
    });
  }, []);

  const refreshSources = useCallback(() => {
    setSources({ kind: "loading" });
    listSources()
      .then((envelope) => {
        setSources({ kind: "ok", sources: envelope.payload });
        refreshScanStatuses(envelope.payload);
      })
      .catch((err: unknown) => {
        setSources({ kind: "error", message: readTesseraErrorMessage(err) });
      });
  }, [refreshScanStatuses]);

  useEffect(() => {
    let cancelled = false;
    setCandidates({ kind: "loading" });
    setSources({ kind: "loading" });
    // Concurrent: candidates (host FS) + inventory (SQLite). Independent.
    discoverSources()
      .then((envelope) => {
        if (cancelled) return;
        setCandidates({ kind: "ok", candidates: envelope.payload });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setCandidates({ kind: "error", message: readTesseraErrorMessage(err) });
      });
    listSources()
      .then((envelope) => {
        if (cancelled) return;
        setSources({ kind: "ok", sources: envelope.payload });
        refreshScanStatuses(envelope.payload);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        setSources({ kind: "error", message: readTesseraErrorMessage(err) });
      });
    return () => {
      cancelled = true;
    };
  }, [refreshScanStatuses]);

  const onConfirm = useCallback(
    (candidate: CandidateSource) => {
      confirmSource(candidate)
        .then(() => {
          setAction({ kind: "idle" });
          refreshSources();
        })
        .catch((err: unknown) => {
          setAction({ kind: "error", message: readTesseraErrorMessage(err) });
        });
    },
    [refreshSources],
  );

  const onReject = useCallback(
    (candidate: CandidateSource) => {
      rejectSource(candidate)
        .then(() => {
          setAction({ kind: "idle" });
          refreshSources();
        })
        .catch((err: unknown) => {
          setAction({ kind: "error", message: readTesseraErrorMessage(err) });
        });
    },
    [refreshSources],
  );

  const onDisable = useCallback(
    (sourceId: Source["source_id"]) => {
      disableSource(sourceId)
        .then(() => {
          setAction({ kind: "idle" });
          refreshSources();
        })
        .catch((err: unknown) => {
          setAction({ kind: "error", message: readTesseraErrorMessage(err) });
        });
    },
    [refreshSources],
  );

  const onScan = useCallback(
    (source: Source) => {
      // Single-owner guard (AD-5): ignore clicks while any scan is in flight.
      if (inFlightRef.current.size > 0) {
        return;
      }
      inFlightRef.current.add(source.source_id);
      setAnyScanning(true);
      setScanningId(source.source_id);
      setAction({ kind: "idle" });
      setScanAnnouncement(`Scanning ${source.provider} source…`);
      scanSource(source.source_id)
        .then((envelope) => {
          const n = envelope.payload.records_indexed;
          setScanAnnouncement(
            `Scan complete. Indexed ${n} ${n === 1 ? "record" : "records"} in generation ${envelope.payload.generation}.`,
          );
        })
        .catch((err: unknown) => {
          const message = readTesseraErrorMessage(err);
          setAction({ kind: "error", message });
          setScanAnnouncement(`Scan failed. ${message}`);
        })
        .finally(() => {
          inFlightRef.current.delete(source.source_id);
          setAnyScanning(inFlightRef.current.size > 0);
          setScanningId(null);
          // Refresh so the run's outcome is reflected in the label.
          refreshSources();
        });
    },
    [refreshSources],
  );

  return (
    <section aria-label="Tessera sources">
      <h2>Sources</h2>
      <div aria-live="polite">
        {scanAnnouncement !== "" ? (
          <p data-testid="scan-announcement" className="visually-hidden-text">
            {scanAnnouncement}
          </p>
        ) : null}
        {action.kind === "error" ? (
          <p role="alert" data-testid="sources-action-error">
            {action.message}
          </p>
        ) : null}
      </div>
      <CandidatesRegion state={candidates} onConfirm={onConfirm} onReject={onReject} />
      <RegisteredSourcesRegion
        state={sources}
        scanStatuses={scanStatuses}
        scanningId={scanningId}
        anyScanning={anyScanning}
        onDisable={onDisable}
        onScan={onScan}
      />
    </section>
  );
}

// ---------------------------------------------------------------------------
// Candidates region
// ---------------------------------------------------------------------------

function CandidatesRegion({
  state,
  onConfirm,
  onReject,
}: {
  state: CandidatesState;
  onConfirm: (candidate: CandidateSource) => void;
  onReject: (candidate: CandidateSource) => void;
}): ReactElement {
  return (
    <section aria-label="Discovered candidate sources" aria-busy={state.kind === "loading"}>
      <h3>Candidates</h3>
      <div aria-live="polite">{renderCandidatesState(state, onConfirm, onReject)}</div>
    </section>
  );
}

function renderCandidatesState(
  state: CandidatesState,
  onConfirm: (candidate: CandidateSource) => void,
  onReject: (candidate: CandidateSource) => void,
): ReactElement {
  switch (state.kind) {
    case "idle":
      return <p>Looking for supported Codex sources…</p>;
    case "loading":
      return <p>Scanning this machine for supported Codex sources…</p>;
    case "ok":
      return state.candidates.length === 0
        ? renderCandidatesEmpty()
        : renderCandidatesList(state.candidates, onConfirm, onReject);
    case "error":
      return (
        <p role="alert" data-testid="candidates-error">
          {state.message}
        </p>
      );
  }
}

/**
 * Empty state — explicitly WITHOUT a "manually add directory" entry (MVP
 * anti-goal, Epic 1 context / AC2 of Story 1.2).
 */
function renderCandidatesEmpty(): ReactElement {
  return (
    <div data-testid="candidates-empty">
      <p>
        No supported Codex Agent Memory sources were found on this machine.
      </p>
      <p>
        Tessera looks for Codex memories at{" "}
        <code>$HOME/.codex/memories</code> by default, or at{" "}
        <code>$CODEX_HOME/memories</code> when <code>CODEX_HOME</code> is set.
      </p>
    </div>
  );
}

function renderCandidatesList(
  candidates: CandidateSource[],
  onConfirm: (candidate: CandidateSource) => void,
  onReject: (candidate: CandidateSource) => void,
): ReactElement {
  return (
    <ul data-testid="candidates-list">
      {candidates.map((candidate, index) => (
        // Pre-confirmation candidates have no stable id yet (source_id is
        // allocated at confirm). Index-based key is fine for this transient
        // render.
        <li key={`${candidate.provider}:${candidate.root_path}:${index}`}>
          <article>
            <h4>
              {candidate.provider} candidate
            </h4>
            <dl>
              <dt>Provider</dt>
              <dd>{candidate.provider}</dd>

              <dt>Path</dt>
              <dd>
                <code>{candidate.root_path}</code>
              </dd>

              <dt>Discovered via</dt>
              <dd>{describeBasis(candidate.basis)}</dd>

              <dt>Coverage</dt>
              <dd>{describeCoverage(candidate.coverage_level)}</dd>
            </dl>
            <div className="candidate-actions">
              <button type="button" onClick={() => onConfirm(candidate)}>
                Confirm
              </button>
              <button type="button" onClick={() => onReject(candidate)}>
                Reject
              </button>
            </div>
          </article>
        </li>
      ))}
    </ul>
  );
}

// ---------------------------------------------------------------------------
// Registered sources region
// ---------------------------------------------------------------------------

function RegisteredSourcesRegion({
  state,
  scanStatuses,
  scanningId,
  anyScanning,
  onDisable,
  onScan,
}: {
  state: SourcesState;
  scanStatuses: ScanStatusMap;
  scanningId: string | null;
  anyScanning: boolean;
  onDisable: (sourceId: Source["source_id"]) => void;
  onScan: (source: Source) => void;
}): ReactElement {
  return (
    <section aria-label="Registered sources" aria-busy={state.kind === "loading"}>
      <h3>Registered sources</h3>
      <div aria-live="polite">
        {renderSourcesState(state, scanStatuses, scanningId, anyScanning, onDisable, onScan)}
      </div>
    </section>
  );
}

function renderSourcesState(
  state: SourcesState,
  scanStatuses: ScanStatusMap,
  scanningId: string | null,
  anyScanning: boolean,
  onDisable: (sourceId: Source["source_id"]) => void,
  onScan: (source: Source) => void,
): ReactElement {
  switch (state.kind) {
    case "idle":
      return <p>Loading registered sources…</p>;
    case "loading":
      return <p>Loading registered sources…</p>;
    case "ok":
      return state.sources.length === 0
        ? renderSourcesEmpty()
        : renderSourcesList(state.sources, scanStatuses, scanningId, anyScanning, onDisable, onScan);
    case "error":
      return (
        <p role="alert" data-testid="sources-error">
          {state.message}
        </p>
      );
  }
}

function renderSourcesEmpty(): ReactElement {
  return (
    <p data-testid="sources-empty">
      No sources have been confirmed yet. Confirm a candidate above to make it
      readable by Tessera.
    </p>
  );
}

function renderSourcesList(
  sources: Source[],
  scanStatuses: ScanStatusMap,
  scanningId: string | null,
  anyScanning: boolean,
  onDisable: (sourceId: Source["source_id"]) => void,
  onScan: (source: Source) => void,
): ReactElement {
  return (
    <ul data-testid="sources-list">
      {sources.map((source) => (
        <li key={source.source_id}>
          <article>
            <h4>
              {source.provider} source{" "}
              <span className="lifecycle-label">
                ({describeLifecycle(source.lifecycle_state)})
              </span>
            </h4>
            <dl>
              <dt>Source ID</dt>
              <dd>
                <code>{source.source_id}</code>
              </dd>

              <dt>Provider</dt>
              <dd>{source.provider}</dd>

              <dt>Lifecycle</dt>
              <dd>{describeLifecycle(source.lifecycle_state)}</dd>

              <dt>Root</dt>
              <dd>
                <code>{source.normalized_root_path}</code>
              </dd>

              <dt>Coverage</dt>
              <dd>{describeCoverage(source.coverage_level)}</dd>

              {source.lifecycle_state === "confirmed" ? (
                <>
                  <dt>Last scan</dt>
                  <dd data-testid={`scan-status-${source.source_id}`}>
                    {describeScanStatus(scanStatuses[source.source_id])}
                  </dd>
                </>
              ) : null}
            </dl>
            {source.lifecycle_state === "confirmed" ? (
              <div className="source-actions">
                <button
                  type="button"
                  onClick={() => onScan(source)}
                  disabled={anyScanning}
                >
                  {scanningId === source.source_id ? "Scanning…" : "Scan"}
                </button>
                <button type="button" onClick={() => onDisable(source.source_id)}>
                  Disable
                </button>
              </div>
            ) : null}
          </article>
        </li>
      ))}
    </ul>
  );
}

/**
 * Render the honest last-scan label for a confirmed source:
 * - `undefined` entry → the status has not been fetched yet ("Checking…").
 * - `unavailable` entry → the status FETCH failed; say so honestly instead of
 *   implying the source was never scanned (AD-3).
 * - `ok` + null state → the source has genuinely never been scanned.
 * - `ok` + state → the latest run state, with generation + record count for a
 *   succeeded scan. Never a fabricated success.
 */
function describeScanStatus(entry: ScanStatusEntry | undefined): string {
  if (entry === undefined) {
    return "Checking scan status…";
  }
  if (entry.kind === "unavailable") {
    return "Scan status unavailable.";
  }
  const status = entry.status;
  if (status.state === null) {
    return "No scan yet.";
  }
  const state = status.state;
  if (state === "succeeded") {
    const gen = status.active_generation ?? "none";
    return `Scan succeeded — generation ${gen}, ${status.active_records} ${
      status.active_records === 1 ? "record" : "records"
    } indexed.`;
  }
  if (state === "failed") {
    return "Last scan failed — the previous index is unchanged.";
  }
  // queued / running / staging / committing / retry: transient in-flight or
  // scheduled states. Report them honestly.
  return `Scan ${state}.`;
}

// ---------------------------------------------------------------------------
// Label helpers
// ---------------------------------------------------------------------------

function describeBasis(basis: CandidateSource["basis"]): string {
  switch (basis) {
    case "default_home":
      return "Default Codex home ($HOME/.codex/memories)";
    case "codex_home_env":
      return "CODEX_HOME environment override ($CODEX_HOME/memories)";
  }
}

function describeCoverage(level: CandidateSource["coverage_level"]): string {
  switch (level) {
    case "full":
      return "Full — local directory (Tessera will index it once scanning ships)";
    case "search_only":
      return "Search only — not fully enumerable";
    case "existence_only":
      return "Existence only — presence detected, contents not enumerable";
    case "unsupported":
      return "Unsupported at this coverage tier";
  }
}

function describeLifecycle(state: SourceLifecycle): string {
  switch (state) {
    case "confirmed":
      return "Confirmed";
    case "disabled":
      return "Disabled";
    case "rejected":
      return "Rejected";
  }
}
