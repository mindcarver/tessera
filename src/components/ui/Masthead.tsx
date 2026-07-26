/**
 * Tessera — Masthead (Editorial Brutalism design system).
 *
 * Shared publication header for every view. Brand (the page's single h1) +
 * primary nav (in-page anchors to the stacked sections — there is no router,
 * per the spec's "no router" constraint) + a local-first status line whose
 * green dot conveys BG-3 (loopback / read-only) at a glance.
 *
 * The nav is presentational anchoring only — it does not swap views. The brand
 * is the h1 so the document keeps exactly one top-level heading.
 */
import type { ReactElement } from "react";

type NavId = "inventory" | "projects" | "search";

interface NavItem {
  id: NavId;
  label: string;
  href: string;
}

const NAV: readonly NavItem[] = [
  { id: "inventory", label: "Inventory", href: "#tessera-sources" },
  { id: "projects", label: "Projects", href: "#tessera-projects" },
  { id: "search", label: "Search", href: "#tessera-search" },
];

interface MastheadProps {
  /** Section the user is currently in (renders the active nav treatment). */
  active?: NavId;
}

export function Masthead({ active }: MastheadProps = {}): ReactElement {
  return (
    <header className="tsr-masthead">
      <div className="tsr-brand">
        <h1 className="tsr-brand__name">Tessera</h1>
        <span className="tsr-brand__tag">Local-first memory federation</span>
      </div>
      <nav className="tsr-nav" aria-label="Primary">
        {NAV.map((item) => (
          <a
            key={item.id}
            className={`tsr-nav__item${item.id === active ? " tsr-nav__item--on" : ""}`}
            href={item.href}
            aria-current={item.id === active ? "page" : undefined}
          >
            {item.label}
          </a>
        ))}
      </nav>
      <div className="tsr-status">
        <span className="tsr-status__dot" aria-hidden="true" />
        127.0.0.1 · Local · Read-only
      </div>
    </header>
  );
}
