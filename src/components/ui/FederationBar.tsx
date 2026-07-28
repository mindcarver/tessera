/**
 * Tessera — FederationBar (Editorial Brutalism design system).
 *
 * The single quiet ratio bar + legend used by the Source Inventory hero to
 * convey cross-source health at a glance (P1: unified visibility; N1: silent
 * loss made visible). Mirrors the refined HTML `.fed-bar`.
 *
 * Signal color encodes Health ONLY: green = healthy, red = attention
 * (degraded + error collapsed — both are "needs your attention"), mute =
 * unknown. Proportions are by source count. Presentational only.
 */
import type { ReactElement } from "react";

interface FederationBarProps {
  /** Healthy source count (green). */
  healthy: number;
  /** Attention count (red) — degraded + error sources. */
  attention: number;
  /** Unknown-health count (mute). */
  unknown?: number;
}

export function FederationBar({ healthy, attention, unknown = 0 }: FederationBarProps): ReactElement {
  const total = healthy + attention + unknown;
  const legend = [
    { key: "healthy", count: healthy, label: "个正常", mod: "ok" as const },
    { key: "attention", count: attention, label: "个需要处理", mod: "bad" as const },
    { key: "unknown", count: unknown, label: "个未知", mod: "unknown" as const },
  ].filter((entry) => entry.count > 0);

  return (
    <div className="tsr-fed">
      <div
        className="tsr-fed-bar"
        role="img"
        aria-label={`共 ${total} 个来源：${healthy} 个正常，${attention} 个需要处理${unknown ? `，${unknown} 个未知` : ""}`}
      >
        {total === 0 ? <span className="tsr-fed-bar__empty" /> : null}
        {healthy > 0 ? <span className="tsr-fed-bar__seg tsr-fed-bar__seg--ok" style={{ flexGrow: healthy }} /> : null}
        {attention > 0 ? <span className="tsr-fed-bar__seg tsr-fed-bar__seg--bad" style={{ flexGrow: attention }} /> : null}
        {unknown > 0 ? <span className="tsr-fed-bar__seg tsr-fed-bar__seg--unknown" style={{ flexGrow: unknown }} /> : null}
      </div>
      <div className="tsr-fed-legend">
        {legend.map((entry) => (
          <span key={entry.key} className={`tsr-fed-legend__item tsr-fed-legend__item--${entry.mod}`}>
            <i aria-hidden="true" />
            {entry.count}{entry.label}
          </span>
        ))}
      </div>
    </div>
  );
}
