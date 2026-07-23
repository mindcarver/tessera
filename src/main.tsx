/**
 * Tessera — React 19.2.7 shell entry (Phase 0 scaffold).
 *
 * The shell only calls the versioned loopback HTTP API (AD-1); it never
 * touches Providers, the filesystem, or SQLite directly. Phase 0 renders a
 * single `<App />` that calls the `ping` command and displays the versioned
 * envelope response, proving the typed UI → core → UI round-trip works on
 * the locked stack.
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";

const rootElement = document.getElementById("root");
if (!rootElement) {
  throw new Error("Tessera root element #root not found in index.html");
}

createRoot(rootElement).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
