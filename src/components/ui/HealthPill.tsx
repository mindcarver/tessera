/**
 * Tessera — HealthPill (Editorial Brutalism design system).
 *
 * The single shared health affordance. Signal color encodes Health ONLY
 * (design-tokens.md): green = healthy, red = degraded/error, mute = unknown.
 * Rendered as a mono uppercase label + a square dot, matching the refined
 * HTML anchors (`.r-health` / `.prov-status`).
 *
 * Presentational only — no fetch, no state. Maps the wire `HealthState`
 * (`unknown | healthy | degraded | error`) onto the three visual states. `error`
 * collapses onto the red "bad" vocabulary (it is the most-attention state and
 * already sorts first in the inventory), so no fourth decorative color is
 * introduced.
 */
import type { ReactElement } from "react";
import type { HealthState } from "../../api/sources";

type Variant = "ok" | "bad" | "unknown";

function variantFor(state: HealthState): Variant {
  switch (state) {
    case "healthy":
      return "ok";
    case "degraded":
    case "error":
      return "bad";
    case "unknown":
    default:
      return "unknown";
  }
}

/** The mono uppercase noun shown next to the dot (HEALTHY / DEGRADED / …). */
export function healthLabel(state: HealthState): string {
  switch (state) {
    case "healthy":
      return "Healthy";
    case "degraded":
      return "Degraded";
    case "error":
      return "Error";
    case "unknown":
    default:
      return "Unknown";
  }
}

interface HealthPillProps {
  state: HealthState;
  /** Optional localized label for a specific surface. */
  label?: string;
  /** Compact form renders just the dot + short label (row aside). Default false. */
  compact?: boolean;
}

export function HealthPill({ state, label, compact = false }: HealthPillProps): ReactElement {
  const variant = variantFor(state);
  return (
    <span
      className={`tsr-health ${compact ? "tsr-health--compact" : ""} tsr-health--${variant}`}
      data-health={state}
    >
      <span className="tsr-health__dot" aria-hidden="true" />
      {(label ?? healthLabel(state)).toUpperCase()}
    </span>
  );
}
