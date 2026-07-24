import { defineConfig } from "@playwright/test";
import path from "node:path";

const root = process.cwd();

export default defineConfig({
  testDir: "./tests/ui",
  timeout: 30_000,
  use: { baseURL: "http://127.0.0.1:1422" },
  webServer: {
    command: "cargo run --quiet --manifest-path server/Cargo.toml",
    url: "http://127.0.0.1:1422/api/ping",
    reuseExistingServer: false,
    env: {
      TESSERA_PORT: "1422",
      TESSERA_STATIC_DIR: path.join(root, "dist"),
      TESSERA_DATA_DIR: path.join(root, "test-results", "tessera-e2e-story-1-8"),
      TESSERA_NO_BROWSER: "1",
      CODEX_HOME: path.join(root, "tests", "fixtures", "e2e-codex-home"),
    },
  },
});
