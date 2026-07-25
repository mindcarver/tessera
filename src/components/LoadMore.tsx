/**
 * Tessera — shared Load-more button (Story 3.1).
 *
 * Extracted from the inlined `<button>Load more</button>` pattern in
 * `src/features/search/Search.tsx` so Browse paginates the same way (cursor
 * → Load more → next page). The button is keyboard-reachable by default
 * (`<button type="button">`); the `disabled` flag reflects an in-flight
 * pagination so the user cannot double-trigger a page load.
 */

import type { ReactElement } from "react";

interface LoadMoreProps {
  /** Called when the user activates the button. */
  onClick: () => void;
  /** When `true`, the button renders disabled (a page load is in flight). */
  disabled?: boolean;
}

export function LoadMore({ onClick, disabled = false }: LoadMoreProps): ReactElement {
  return (
    <button type="button" onClick={onClick} disabled={disabled}>
      Load more
    </button>
  );
}
