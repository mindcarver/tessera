/**
 * Tessera — shared search/browse result card (Story 3.1).
 *
 * Extracted from `src/features/search/Search.tsx` so Search and Browse render
 * a record's Provenance, Coverage, and Source Health identically (Epic 3
 * Boundaries: "reuse, do not re-implement"). The card is presentational only
 * — no fetch, no state. The `onOpen` callback lets each feature wire its own
 * open-original-location flow without duplicating the card markup.
 *
 * Accessibility:
 * - The `<li>` carries `tabIndex={0}` so the whole card is keyboard-focusable
 *   in AD-21's focus order.
 * - The Provenance `<dl>` uses stable `<dt>` labels ("Provider", "Source",
 *   "Native project", "Semantic location", "Display location", "Last observed
 *   (scan)", "Coverage", "Source health") so screen-reader users hear the
 *   same vocabulary Search has always spoken — Browse is "the same surface,
 *   query-less", not a new shape.
 */

import type { ReactElement } from "react";
import type { SearchResult } from "../api/search";
import { providerDisplayName } from "./providerDisplayName";

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
  return (
    <li key={result.record_id} tabIndex={0}>
      <p>{result.excerpt}</p>
      <dl>
        <dt>Provider</dt>
        <dd>{providerBadge(result.provider)}</dd>
        <dt>Source</dt>
        <dd>{result.source_id}</dd>
        <dt>Native project</dt>
        <dd>{result.native_project ?? "Unmapped"}</dd>
        <dt>Semantic location</dt>
        <dd>{result.native_locator}</dd>
        <dt>Display location</dt>
        <dd>{result.display_locator}</dd>
        <dt>Last observed (scan)</dt>
        <dd>{result.observed_at}</dd>
        <dt>Coverage</dt>
        <dd>{result.coverage_level}</dd>
        <dt>Source health</dt>
        <dd>{result.health_state}</dd>
      </dl>
      <button
        type="button"
        onClick={() => onOpen(result.record_id)}
        disabled={openInFlight}
      >
        Open original location
      </button>
    </li>
  );
}

/**
 * Provider badge — renders a short label so Codex vs Claude Code cards are
 * visually comparable at a glance. Mirrors the badge Search has always
 * rendered; extracted so Browse cards share it without duplication.
 */
function providerBadge(provider: string): ReactElement {
  return (
    <span className="tessera-provider-badge" data-provider={provider}>
      {providerDisplayName(provider)}
    </span>
  );
}
