/**
 * Tessera — top-level React shell.
 *
 * Phase 0 established the shell and the `ping` contract sample. Story 1.2
 * composed the first business component; Story 1.3 replaces it with
 * `<Sources />` (discovery candidates + confirm/reject + registered inventory
 * with disable) alongside the ping section so the API contract round-trip
 * remains visible. Story 1.6 adds `<Search />` and Story 3.1 adds `<Browse />`
 * — the latter entered from a Source Inventory card via hand-rolled view
 * state (no router, per the spec's "no router" constraint). Story 5.1 adds
 * `<Projects />` (Tessera Project create / rename / delete + explicit
 * `(provider, native_project)` add-mapping / remove-mapping) as a peer
 * section; the reserved `tessera_project` filter in `<Search />` stays
 * disabled until Story 5.2 fills the projection slot.
 *
 * Architecture invariants honored here:
 * - AD-1: the shell only calls the versioned loopback HTTP API. It never
 *   touches Providers, the filesystem, or SQLite directly.
 * - NFR-13 / AD-21: the page exposes a single coherent focus order via the
 *   underlying semantic regions (`<section aria-label>`), and shared
 *   `aria-live` announcements so status changes are spoken without moving
 *   focus.
 * - Story 3.1: Browse is a single-source view entered from the Source
 *   Inventory. The App holds the active view in hand-rolled state (no router
 *   dependency): the `Sources` component notifies App via `onBrowse` when the
 *   user activates "Browse" on a confirmed source's card, and App swaps
 *   `<Browse>` in until the user activates the Breadcrumb's Sources segment
 *   (Story 3.3 restated the `onBack` action as navigation).
 */

import { useEffect, useState, type ReactElement } from "react";
import { ping, type Pong } from "./api/ping";
import { Sources } from "./features/sources/Sources";
import { Obsidian } from "./features/obsidian/Obsidian";
import { Search } from "./features/search/Search";
import { Browse } from "./features/browse/Browse";
import { Projects } from "./features/projects/Projects";
import { readTesseraErrorMessage } from "./api/errors";
import { providerDisplayName } from "./components/providerDisplayName";
import { Masthead } from "./components/ui/Masthead";
import { TrustFooter } from "./components/ui/TrustFooter";

type PingState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; pong: Pong; apiVersion: string }
  | { kind: "error"; message: string };

/**
 * Story 3.1 — hand-rolled view state (no router). The shell swaps between
 * the Source Inventory / Search default composition and the single-source
 * Browse view. Entered from the Inventory's "Browse" button (only rendered
 * for confirmed sources); exited via Browse's Breadcrumb Sources segment
 * (Story 3.3 — the back action surfaced as the breadcrumb's Sources segment;
 * the prop stays `onBack`).
 *
 * `providerLabel` / `nativeProject` are passed in at swap time so the Browse
 * heading can name the source without re-fetching the inventory row from the
 * Browse component. The fields come from the Inventory row the user
 * activated, so they always match what the user just saw.
 */
type View =
  | { kind: "default" }
  | {
      kind: "browse";
      sourceId: string;
      providerLabel: string;
      nativeProject: string | null;
    };

export function App(): ReactElement {
  const [state, setState] = useState<PingState>({ kind: "idle" });
  const [view, setView] = useState<View>({ kind: "default" });

  // Trigger the ping round-trip on mount so the Phase 0 accessibility smoke
  // (`tests/ui/accessibility.spec.ts`) has a stable keyboard-reachable
  // target without extra wiring.
  useEffect(() => {
    let cancelled = false;
    setState({ kind: "loading" });
    ping()
      .then((envelope) => {
        if (cancelled) return;
        setState({
          kind: "ok",
          pong: envelope.payload,
          apiVersion: envelope.api_version,
        });
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const message = readTesseraErrorMessage(err);
        setState({ kind: "error", message });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (view.kind === "browse") {
    return (
      <div className="tsr-page">
        <Masthead active="inventory" />
        <main aria-live="polite" className="tsr-main">
          <Browse
            sourceId={view.sourceId}
            providerLabel={view.providerLabel}
            nativeProject={view.nativeProject}
            onBack={() => setView({ kind: "default" })}
          />
          <section aria-label="API ping status">{renderPingState(state)}</section>
        </main>
        <TrustFooter />
      </div>
    );
  }

  return (
    <div className="tsr-page">
      <Masthead active="inventory" />
      <main aria-busy={state.kind === "loading"} aria-live="polite" className="tsr-main">
        <Sources
          onBrowse={(source) =>
            setView({
              kind: "browse",
              sourceId: source.source_id,
              providerLabel: providerDisplayName(source.provider),
              nativeProject: source.native_project,
            })
          }
        />
        <Obsidian />
        <Projects />
        <Search />
        <section aria-label="API ping status">{renderPingState(state)}</section>
      </main>
      <TrustFooter />
    </div>
  );
}

function renderPingState(state: PingState): ReactElement {
  switch (state.kind) {
    case "idle":
      return <p>Waiting to ping Tessera core…</p>;
    case "loading":
      return <p>Pinging Tessera core…</p>;
    case "ok":
      return (
        <p data-testid="ping-ok">
          core responded via the loopback API (api_version {state.apiVersion}):{" "}
          {state.pong.name} {state.pong.version}
        </p>
      );
    case "error":
      return (
        <p role="alert" data-testid="ping-error">
          {state.message}
        </p>
      );
  }
}
