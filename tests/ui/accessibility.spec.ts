/**
 * Tessera — keyboard-driven scan accessibility regression (Story 1.4).
 *
 * Path is fixed by the architecture spine (AD-21):
 *   `tests/ui/accessibility.spec.ts`
 *
 * Uses an isolated Codex memory fixture and the real loopback HTTP server.
 * The flow must stay keyboard reachable: confirm the discovered source,
 * trigger Rescan with Enter, observe the scoped job request, then read the
 * polite progress announcement and rendered server-derived inventory.
 */

import { expect, test } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("keyboard rescan posts successfully and announces ordered progress", async ({ page }) => {
  let eventCalls = 0;
  await page.route("**/api/sources/discover", async (route) => route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }));
  await page.route("**/api/sources/inventory", async (route) => route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [{ source_id: "src_1", provider: "codex", lifecycle_state: "confirmed", root: "/fixture", native_project: null, coverage_level: "full", health_state: "healthy", last_successful_scan: 1, complete_record_count: 1, latest_error: null }] }) }));
  await page.route("**/api/sources/rescan/events?*", async (route) => {
    eventCalls += 1;
    const data = eventCalls === 1
      ? { api_version: "1", job_id: "job_1", source_id: "src_1", sequence: 2, state: "running", message: "Rescan running." }
      : { api_version: "1", job_id: "job_1", source_id: "src_1", sequence: 3, state: "cancelled", message: "Rescan cancelled." };
    await route.fulfill({ contentType: "text/event-stream", body: `event: progress\ndata: ${JSON.stringify(data)}\n\n` });
  });
  await page.route("**/api/sources/rescan/cancel", async (route) => route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: { api_version: "1", job_id: "job_1", source_id: "src_1", sequence: 3, state: "cancelled", message: "Rescan cancelled." } }) }));
  await page.route("**/api/sources/rescan", async (route) => route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: { api_version: "1", job_id: "job_1", source_id: "src_1", sequence: 1, state: "queued", message: "Rescan queued." } }) }));
  await page.goto("/");

  // Story 2.1: Sources UI copy must stay provider-agnostic — the empty-state
  // line carries "Agent Memory" and never "Codex" (or any other provider
  // name), so adding a second provider does not require a copy sweep. Pinned
  // by an assertion so a future Codex-only regression fails loudly.
  const candidateRegion = page.getByRole("region", { name: "Discovered candidate sources" });
  await expect(candidateRegion).toContainText("Agent Memory");
  await expect(candidateRegion).not.toContainText("Codex");

  const rescan = page.getByRole("button", { name: "Rescan", exact: true });
  await expect(rescan).toBeVisible();
  const rescanResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/sources/rescan") && response.request().method() === "POST",
  );
  await rescan.focus();
  await page.keyboard.press("Enter");

  expect((await rescanResponse).status()).toBe(200);
  await expect(page.getByRole("button", { name: "Cancel rescan" })).toBeVisible();
  await page.waitForTimeout(300);
  expect(eventCalls).toBeGreaterThan(0);
  const cancel = page.getByRole("button", { name: "Cancel rescan" });
  await cancel.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByTestId("rescan-progress")).toContainText("Rescan cancelled.");
  await expect(page.getByRole("region", { name: "Source inventory" }).getByText("Health", { exact: true })).toBeVisible();
});

test("keyboard search renders provenance, empty states, pagination, and safe API errors", async ({ page }) => {
  await page.goto("/");
  const confirm = page.getByRole("button", { name: "Confirm" });
  await expect(confirm).toBeVisible();
  await confirm.press("Enter");
  await expect(page.getByRole("button", { name: "Rescan", exact: true })).toBeVisible();
  let stalePage = 0;
  let openCalls = 0;
  const openBodies: string[] = [];
  await page.route("**/api/open", async (route) => {
    openCalls += 1;
    openBodies.push(route.request().postData() ?? "");
    if (openCalls === 1) {
      return route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: { record_id: "mock-record", source_id: "src_1" } }) });
    }
    return route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ code: "open_failed", message: "Tessera could not open the original location.", source_id: "src_1", phase: "open" }) });
  });
  await page.route("**/api/search?*", async (route) => {
    const q = new URL(route.request().url()).searchParams.get("q");
    const payload = (empty_state: string | null, results: unknown[] = [], next_cursor: string | null = null) => ({ api_version: "1", payload: { results, next_cursor, empty_state, sources: [] } });
    const result = { record_id: "mock-record", excerpt: "mock excerpt", provider: "codex", source_id: "src_1", native_project: null, native_locator: "mock://semantic", display_locator: "mock://display", observed_at: 1, coverage_level: "full", health_state: "unknown" };
    if (q?.trim() === "") return route.fulfill({ status: 400, contentType: "application/json", body: JSON.stringify({ code: "bad_request", message: "The request did not match Tessera's search contract.", source_id: null, phase: "search" }) });
    if (q === "stale-page") {
      stalePage += 1;
      if (stalePage === 1) return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload(null, [result], "v1.cursor")) });
      return route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ code: "cursor_stale", message: "The index changed. Run the search again.", source_id: null, phase: "search" }) });
    }
    if (q === "not-indexed") return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload("source_not_indexed")) });
    if (q === "unavailable") return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload("source_unavailable")) });
    if (q === "does-not-match") return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload("no_match")) });
    return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload(null, [result], "v1.cursor")) });
  });
  const input = page.getByLabel("Keyword");
  const response = page.waitForResponse(
    (value) => value.url().includes("/api/search?") && value.request().method() === "GET",
  );
  await input.fill("keyword");
  await input.press("Enter");
  expect((await response).status()).toBe(200);
  await expect(page.getByText("Provider").first()).toBeVisible();
  await expect(page.getByText("Semantic location").first()).toBeVisible();
  await expect(page.getByText("Last observed (scan)").first()).toBeVisible();
  const openOriginal = page.getByRole("button", { name: "Open original location" }).first();
  await expect(openOriginal).toBeVisible();
  await openOriginal.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByText("Opened original location.")).toBeVisible();
  const openRequest = JSON.parse(openBodies[0]) as Record<string, unknown>;
  expect(Object.keys(openRequest)).toEqual(["record_id"]);
  expect(typeof openRequest.record_id).toBe("string");
  await openOriginal.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("alert")).toHaveText("Tessera could not open the original location.");
  await expect(page.getByText("Source health").first()).toBeVisible();

  const loadMore = page.getByRole("button", { name: "Load more" });
  await expect(loadMore).toBeVisible();
  await loadMore.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("region", { name: "Memory search" }).getByRole("listitem")).toHaveCount(2);

  await input.fill("stale-page");
  await input.press("Enter");
  await expect(page.getByText("mock excerpt")).toBeVisible();
  await page.getByRole("button", { name: "Load more" }).press("Enter");
  await expect(page.getByRole("alert")).toHaveText("The index changed. Run the search again.");
  await expect(page.getByText("mock excerpt")).toBeVisible();

  await input.fill("does-not-match");
  await input.press("Enter");
  await expect(page.getByText("No indexed memory matched this keyword.")).toBeVisible();

  await input.fill("not-indexed");
  await input.press("Enter");
  await expect(page.getByText("Confirmed sources have not been indexed yet.")).toBeVisible();

  await input.fill("unavailable");
  await input.press("Enter");
  await expect(page.getByText("A confirmed source is currently unavailable; its stored health was not changed.")).toBeVisible();

  await input.fill("   ");
  await input.press("Enter");
  await expect(page.getByRole("alert")).toHaveText("The request did not match Tessera's search contract.");
});

/**
 * Story 2.3 — multi-provider search renders provider badges (Codex + Claude
 * Code) so the two providers' memories are visually comparable, and the FR-14
 * partial-unavailability banner renders when the sidecar carries a non-
 * `available` source. Keeps the existing keyboard-reachability contract.
 */
test("multi-provider search renders provider badges and partial-unavailability banner", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  const codexResult = {
    record_id: "rec-codex",
    excerpt: "codex keyword memory",
    provider: "codex",
    source_id: "src_1",
    native_project: null,
    native_locator: "file:///codex#L1",
    display_locator: "file:///codex#L1-L2",
    observed_at: 1,
    coverage_level: "full",
    health_state: "healthy",
  };
  const claudeResult = {
    record_id: "rec-claude",
    excerpt: "claude keyword memory",
    provider: "claude_code",
    source_id: "src_2",
    native_project: "proj-claude",
    native_locator: "file:///claude#L1",
    display_locator: "file:///claude#L1-L2",
    observed_at: 2,
    coverage_level: "full",
    health_state: "healthy",
  };
  await page.route("**/api/search?*", async (route) => {
    const sources = [
      { source_id: "src_1", provider: "codex", native_project: null, status: "available" },
      { source_id: "src_2", provider: "claude_code", native_project: "proj-claude", status: "unavailable" },
    ];
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ api_version: "1", payload: { results: [codexResult, claudeResult], next_cursor: null, empty_state: null, sources } }),
    });
  });
  await page.goto("/");

  const input = page.getByLabel("Keyword");
  await input.fill("keyword");
  await input.press("Enter");

  // Both provider badges render so the cards are comparable at a glance.
  // Assert via the stable `data-provider` attribute, NOT `getByText("Codex")`,
  // which would match the mock excerpt ("codex keyword memory") via case-
  // insensitive substring instead of the badge — a badge regression would
  // silently pass. Pinning the attribute makes a real badge removal fail.
  await expect(page.locator('[data-provider="codex"]')).toBeVisible();
  await expect(page.locator('[data-provider="claude_code"]')).toBeVisible();

  // The partial-unavailability banner surfaces the degraded/unavailable source.
  const banner = page.getByTestId("search-source-status");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("claude_code");

  // The unavailable source's results are NOT suppressed — both records render.
  await expect(page.getByRole("region", { name: "Memory search" }).getByRole("listitem")).toHaveCount(2);
});

/**
 * Story 2.1 pass-2 review fix — exercise the TS basis path for Claude Code
 * discovery. Mocks `/api/sources/discover` to return candidates with each
 * `claude_*` basis (`claude_default_home`, `claude_config_dir_env`,
 * `claude_auto_memory_dir`) and asserts every candidate renders in the
 * "Discovered candidate sources" region.
 *
 * This exercises `asDiscoveryBasis` / `asCandidateSource` /
 * `VALID_DISCOVERY_BASES` in `src/api/discover.ts` so a dropped `claude_*`
 * entry in the runtime Set causes `discoverSources()` to throw an
 * `api_contract` error (rejected by `.every((c) => asCandidateSource(c) !== null)`),
 * which the Sources UI renders as an error state rather than a candidate
 * list — the test would fail at the `claude_code` text assertion in that
 * case.
 *
 * The run stays hermetic: `playwright.config.ts` already pins
 * `CLAUDE_CONFIG_DIR` at an empty fixture; the route mock overrides discover
 * regardless of what the server-side adapter finds.
 */
test("renders Claude Code candidates with each claude_* discovery basis", async ({ page }) => {
  const claudeCandidates = {
    api_version: "1",
    payload: [
      {
        provider: "claude_code",
        root_path: "/fixture/claude/auto",
        basis: "claude_auto_memory_dir",
        coverage_level: "full",
        native_project: null,
      },
      {
        provider: "claude_code",
        root_path: "/fixture/claude/default",
        basis: "claude_default_home",
        coverage_level: "full",
        native_project: "proj-a",
      },
      {
        provider: "claude_code",
        root_path: "/fixture/claude/env",
        basis: "claude_config_dir_env",
        coverage_level: "full",
        native_project: "proj-b",
      },
    ],
  };
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify(claudeCandidates) }),
  );
  // Inventory can stay empty — this test only exercises the candidate path.
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  await page.goto("/");

  const candidateRegion = page.getByRole("region", { name: "Discovered candidate sources" });

  // Each candidate must render with the provider name and its full-coverage
  // description. A dropped `claude_*` basis in `VALID_DISCOVERY_BASES` would
  // make `asCandidateSource` reject the whole payload, `discoverSources()`
  // would throw an `api_contract` error, and the Sources UI would render an
  // error `role="alert"` instead of a candidate list — every assertion below
  // would fail in that case.
  await expect(candidateRegion).toContainText("claude_code");
  await expect(candidateRegion).toContainText("Full coverage");

  // The load-bearing assertion: all three candidates render as list items.
  // The list only renders when candidates.kind === "ok" AND value.length > 0,
  // and the count matches the three basis variants we injected.
  const items = candidateRegion.getByRole("listitem");
  await expect(items).toHaveCount(3);

  // Belt-and-suspenders: assert no error alert rendered. If `asDiscoveryBasis`
  // rejected any `claude_*` basis, the discover promise would reject and the
  // UI would render `<p role="alert">` with the api_contract message.
  await expect(candidateRegion.getByRole("alert")).toHaveCount(0);
});
