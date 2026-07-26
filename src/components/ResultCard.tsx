/**
 * Tessera — shared search/browse result card (Story 3.1, restyled).
 *
 * Presentational only — no fetch, no state. Search and Browse render a record's
 * excerpt, Provenance, Coverage, and Source Health identically (Epic 3
 * Boundaries: "reuse, do not re-implement"). The `onOpen` callback lets each
 * feature wire its own open-original-location flow without duplicating markup.
 *
 * Accessibility (unchanged contract):
 * - The `<li>` carries `tabIndex={0}` (keyboard-focusable in AD-21's order) and
 *   the snake_case `data-provider` attribute the e2e tests locate cards by.
 * - The Provenance `<dl>` keeps the stable `<dt>` labels ("Provider", "Source",
 *   "Native project", "Semantic location", "Display location", "Last observed
 *   (scan)", "Coverage", "Source health") so screen-reader users hear the same
 *   vocabulary — now demoted to a muted L3 detail spine beneath the visual row.
 */

import type { ReactElement } from "react";
import type { SearchResult } from "../api/search";
import type { HealthState } from "../api/sources";
import { providerDisplayName } from "./providerDisplayName";
import { HealthPill } from "./ui/HealthPill";

interface ResultCardProps {
  result: SearchResult;
  /**
   * Invoked when the user activates the "Open original location" button.
   * `disabled` reflects an in-flight open so the parent can disable the
   * button while the open is being processed.
   */
  onOpen: (recordId: string) => void;
  /** When `true`, the open button renders disabled (an open is in flight). */
  openInFlight: boolean;
}

export function ResultCard({ result, onOpen, openInFlight }: ResultCardProps): ReactElement {
  // `degraded` + `error` both carry the red "needs attention" vocabulary.
  const attention = result.health_state === "degraded" || result.health_state === "error";
  return (
    <li key={result.record_id} tabIndex={0} className={`tsr-res${attention ? " tsr-res--deg" : ""}`} data-provider={result.provider}>
      <div className="tsr-res__row">
        <div className="tsr-res__main">
          <span className="tsr-res__prov">{providerDisplayName(result.provider)}</span>
          <p className="tsr-res__excerpt">{result.excerpt}</p>
          <div className="tsr-res__loc">{result.native_locator}</div>
        </div>
        <div className="tsr-res__aside">
          <HealthPill state={result.health_state as HealthState} compact />
          <button
            type="button"
            className="tsr-btn tsr-btn--link"
            onClick={() => onOpen(result.record_id)}
            disabled={openInFlight}
          >
            Open original location<span aria-hidden="true" className="tsr-arrow"> ▸</span>
          </button>
        </div>
      </div>

      <dl className="tsr-card__details">
        <dt>Provider</dt>
        <dd>{providerDisplayName(result.provider)}</dd>
        <dt>Source</dt>
        <dd>{result.source_id}</dd>
        <dt>Native project</dt>
        <dd>{result.native_project ?? "Unmapped"}</dd>
        <dt>Semantic location</dt>
        <dd>{result.native_locator}</dd>
        <dt>Display location</dt>
        <dd>{result.display_locator}</dd>
        <dt>Last observed (scan)</dt>
        <dd>{formatObserved(result.observed_at)}</dd>
        <dt>Coverage</dt>
        <dd>{result.coverage_level}</dd>
        <dt>Source health</dt>
        <dd>{result.health_state}</dd>
      </dl>
    </li>
  );
}

/** Format the scan-observed epoch (seconds) for the detail spine. */
function formatObserved(observedAt: number): string {
  return new Date(observedAt * 1000).toLocaleString();
}
