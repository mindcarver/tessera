/**
 * Tessera — shared empty-state component (Story 3.1).
 *
 * Extracted from the inlined `<p>{emptyCopy(...)}</p>` pattern in
 * `src/features/search/Search.tsx`. Both Search and Browse render an empty
 * state when the initial page yields zero results, and both want the same
 * polite `aria-live` announcement + accessible text. The component is
 * presentational: the parent computes the message string (so Search can keep
 * its filter-aware copy and Browse can carry its query-less three-state
 * copy).
 */

import type { ReactElement } from "react";

interface EmptyStateProps {
  /** The accessible empty-state message. Computed by the parent. */
  message: string;
}

export function EmptyState({ message }: EmptyStateProps): ReactElement {
  return <p>{message}</p>;
}
