/**
 * Tessera — shared empty-state component (Story 3.1, restyled).
 *
 * Presentational: the parent computes the message string (Search keeps its
 * filter-aware copy; Browse carries its query-less three-state copy) and now
 * also selects a `tone` so the three empty-result states read distinctly per
 * the design tokens (mute = genuinely no match, blue = source present but not
 * yet indexed, red = source unavailable — reusing the degraded vocabulary).
 * Both surfaces keep the same polite `aria-live` announcement + accessible text.
 */

import type { ReactElement } from "react";

export type EmptyTone = "mute" | "blue" | "red";

interface EmptyStateProps {
  /** The accessible empty-state message. Computed by the parent. */
  message: string;
  /** Visual tone — encodes the empty-result reason (mute/blue/red). */
  tone?: EmptyTone;
}

export function EmptyState({ message, tone = "mute" }: EmptyStateProps): ReactElement {
  return <p className={`tsr-empty tsr-empty--${tone}`}>{message}</p>;
}
