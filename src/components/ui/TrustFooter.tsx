/**
 * Tessera — TrustFooter (Editorial Brutalism design system).
 *
 * Shared publication footer. The mono badge row is the persistent trust signal
 * the Trigger Map demands (BG-2/BG-3): read-only, loopback-bound, zero upload,
 * zero telemetry, sanitized logs, reconstructable. Presentational only.
 */
import type { ReactElement } from "react";

const BADGES: readonly string[] = [
  "Read-only",
  "127.0.0.1 bind",
  "Zero upload",
  "Zero telemetry",
  "Logs sanitized",
  "Reconstructable",
];

export function TrustFooter(): ReactElement {
  return (
    <footer className="tsr-trust-footer">
      <div className="tsr-trust-footer__badges">
        {BADGES.map((badge) => (
          <span key={badge}>{badge}</span>
        ))}
      </div>
      <div className="tsr-trust-footer__build">Build 0.1.0-alpha · Local</div>
    </footer>
  );
}
