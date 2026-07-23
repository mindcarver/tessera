/**
 * Tessera — top-level React shell.
 *
 * Phase 0 established the shell and the `ping` contract sample. Story 1.2
 * composed the first business component; Story 1.3 replaces it with
 * `<Sources />` (discovery candidates + confirm/reject + registered inventory
 * with disable) alongside the ping section so the API contract round-trip
 * remains visible. Future Stories (1.4 scan, 1.6 search) append their feature
 * components into this same shell.
 *
 * Architecture invariants honored here:
 * - AD-1: the shell only calls the versioned loopback HTTP API. It never
 *   touches Providers, the filesystem, or SQLite directly.
 * - NFR-13 / AD-21: the page exposes a single coherent focus order via the
 *   underlying semantic regions (`<section aria-label>`), and shared
 *   `aria-live` announcements so status changes are spoken without moving
 *   focus.
 */

import { useEffect, useState, type ReactElement } from "react";
import { ping, type Pong } from "./api/ping";
import { Sources } from "./features/sources/Sources";
import { readTesseraErrorMessage } from "./api/errors";

type PingState =
  | { kind: "idle" }
  | { kind: "loading" }
  | { kind: "ok"; pong: Pong; apiVersion: string }
  | { kind: "error"; message: string };

export function App(): ReactElement {
  const [state, setState] = useState<PingState>({ kind: "idle" });

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

  return (
    <main aria-busy={state.kind === "loading"} aria-live="polite">
      <h1>Tessera</h1>
      <Sources />
      <section aria-label="API ping status">{renderPingState(state)}</section>
    </main>
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
