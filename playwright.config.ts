import { defineConfig } from "@playwright/test";
import path from "node:path";

const root = process.cwd();

export default defineConfig({
  testDir: "./tests/ui",
  timeout: 30_000,
  use: {
    baseURL: "http://127.0.0.1:1421",
  },
  webServer: {
    command: "cargo run --quiet --manifest-path server/Cargo.toml",
    url: "http://127.0.0.1:1421/api/ping",
    reuseExistingServer: false,
    env: {
      TESSERA_PORT: "1421",
      TESSERA_STATIC_DIR: path.join(root, "dist"),
      TESSERA_DATA_DIR: path.join(root, "test-results", "tessera-e2e-data"),
      TESSERA_NO_BROWSER: "1",
      CODEX_HOME: path.join(root, "tests", "fixtures", "e2e-codex-home"),
      // Story 2.1: Claude Code discovery is now wired in alongside Codex.
      // Keep the e2e run hermetic against the host's real ~/.claude/projects/
      // by pointing CLAUDE_CONFIG_DIR at an empty fixture path (mirrors how
      // CODEX_HOME above pins Codex discovery). The Claude fixture contract
      // itself lands in Story 2.2; for 2.1 the accessibility test only needs
      // discover/confirm/scan to stay keyboard-reachable, which the Codex
      // fixture still exercises.
      CLAUDE_CONFIG_DIR: path.join(root, "tests", "fixtures", "e2e-claude-empty"),
    },
  },
});
