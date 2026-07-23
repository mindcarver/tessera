import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tessera — Vite 8.1.x build for the React 19.2.7 frontend.
//
// The delivery form is a local web app (revised AD-9, 2026-07-22): the Rust
// core serves `dist/` and the versioned `/api/*` surface on 127.0.0.1:1420.
// In development the Vite dev server proxies `/api` to that loopback server,
// so `npm run dev` works next to `cargo run` with the same same-origin
// relative fetches the production build uses.
export default defineConfig({
  plugins: [react()],
  server: {
    // Dev UI port; the Rust core owns 1420. Loopback only (AD-12).
    port: 5173,
    strictPort: true,
    host: "127.0.0.1",
    proxy: {
      "/api": {
        target: "http://127.0.0.1:1420",
        changeOrigin: false,
      },
    },
  },
  // HMR must not depend on a remote websocket host.
  clearScreen: false,
  build: {
    target: "esnext",
    outDir: "dist",
    sourcemap: false,
  },
});
