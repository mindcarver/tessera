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

/**
 * Story 2.4 — keyboard-reachable filter controls (provider, memory-type)
 * narrow the result set with AND, the effective-range readout states the
 * active scope, and Clear-filters restores the full confirmed-source scope.
 * The sidecar stays unfiltered so the readout names both providers after
 * Clear even though the filtered query narrowed to one.
 */
test("filter controls narrow results by AND and Clear restores full scope", async ({ page }) => {
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
    excerpt: "claude keyword topic",
    provider: "claude_code",
    source_id: "src_2",
    native_project: "proj-claude",
    native_locator: "file:///claude#L1",
    display_locator: "file:///claude#L1-L2",
    observed_at: 2,
    coverage_level: "full",
    health_state: "healthy",
  };
  // Patch 8 — the mock ANDs (intersects) every active filter instead of
  // overwriting, so a cross-provider combination like provider=codex +
  // source=src_2 (a Claude source) correctly yields zero rows rather than
  // masking the empty intersection. The test exercises the wire-level
  // serialization (URLSearchParams) AND the UI's filter state in one pass.
  // The sidecar always lists both confirmed sources (Story 2.4 Design Notes:
  // sidecar stays unfiltered).
  await page.route("**/api/search?*", async (route) => {
    const url = new URL(route.request().url());
    const provider = url.searchParams.get("provider");
    const source = url.searchParams.get("source");
    const memoryType = url.searchParams.get("memory_type");
    let results = [codexResult, claudeResult];
    if (provider === "codex") results = results.filter((r) => r.provider === "codex");
    if (provider === "claude_code") results = results.filter((r) => r.provider === "claude_code");
    // A memory_type filter alone (no provider) narrows across providers.
    if (!provider && memoryType === "memory") results = results.filter((r) => r.record_id === "rec-codex");
    // Per-source filter (Spec Change Log 2026-07-25) narrows to one source.
    if (source === "src_1") results = results.filter((r) => r.source_id === "src_1");
    if (source === "src_2") results = results.filter((r) => r.source_id === "src_2");
    const sources = [
      { source_id: "src_1", provider: "codex", native_project: null, status: "available" },
      { source_id: "src_2", provider: "claude_code", native_project: "proj-claude", status: "available" },
    ];
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ api_version: "1", payload: { results, next_cursor: null, empty_state: null, sources } }),
    });
  });
  await page.goto("/");

  const input = page.getByLabel("Keyword");
  const searchRegion = page.getByRole("region", { name: "Memory search" });

  // Baseline: no filters → both providers.
  await input.fill("keyword");
  await input.press("Enter");
  await expect(searchRegion.getByRole("listitem")).toHaveCount(2);
  // The effective-range readout names both confirmed providers after the
  // first search populates the sidecar-derived provider list.
  await expect(page.getByTestId("search-effective-range")).toContainText("Codex + Claude Code");

  // Story 2.4 — the reserved Tessera-project slot is rendered DISABLED (not
  // merely absent), per the spec Boundaries/I/O matrix/AC.
  const tesseraSlot = page.getByLabel("Tessera project (reserved)");
  await expect(tesseraSlot).toBeVisible();
  await expect(tesseraSlot).toBeDisabled();

  // Keyboard-set the provider filter. `selectOption` focuses the `<select>`
  // and dispatches the change event — the keyboard-reachable contract.
  const providerSelect = page.getByLabel("Provider");
  await providerSelect.focus();
  await providerSelect.selectOption("codex");
  // Story 2.4 (Spec Change Log) — a filter change resets to idle, so the
  // previous results actually CLEAR (not just the readout updating). Pin this
  // with toHaveCount(0) so a regression that leaves stale results visible
  // fails loudly.
  await expect(searchRegion.getByRole("listitem")).toHaveCount(0);
  // The readout updates immediately to reflect the new filter state.
  await expect(page.getByTestId("search-effective-range")).toContainText("Codex");
  await expect(page.getByTestId("search-effective-range")).not.toContainText("Claude Code");

  // Keyboard-set the memory-type filter (AND combination).
  const typeSelect = page.getByLabel("Memory type");
  await typeSelect.focus();
  await typeSelect.selectOption("memory");
  await expect(page.getByTestId("search-effective-range")).toContainText("type=memory");

  // Run the filtered search (the filter changes reset to idle; pressing
  // Enter re-runs page 1 under the new filter combination).
  await input.press("Enter");
  // Narrowed to Codex memory only.
  await expect(searchRegion.getByRole("listitem")).toHaveCount(1);
  await expect(searchRegion.locator('[data-provider="codex"]')).toBeVisible();

  // Story 2.4 (Spec Change Log 2026-07-25) — the per-source filter narrows to
  // one source's records, distinct from the provider filter. Patch 3 scopes the
  // Source <select> by the active provider, so a cross-provider source (src_2 =
  // Claude) is only selectable when no provider filter is set. Clear the
  // provider + memory filters first to stay on a reachable user path — with the
  // patched AND mock, provider=codex + source=src_2 now correctly yields zero
  // rows instead of overwriting to the Claude record.
  const midClear = page.getByRole("button", { name: "Clear filters" });
  await midClear.focus();
  await midClear.press("Enter");
  await expect(page.getByTestId("search-effective-range")).toContainText("Codex + Claude Code");

  const sourceSelect = page.getByLabel("Source", { exact: true });
  await sourceSelect.focus();
  await sourceSelect.selectOption("src_2");
  // Filter change clears results (idle reset) and the readout names the source.
  await expect(searchRegion.getByRole("listitem")).toHaveCount(0);
  await expect(page.getByTestId("search-effective-range")).toContainText("source=src_2");
  await input.press("Enter");
  // Narrowed to the Claude (src_2) record.
  await expect(searchRegion.getByRole("listitem")).toHaveCount(1);
  await expect(searchRegion.locator('[data-provider="claude_code"]')).toBeVisible();

  // Clear filters — restores the full confirmed-source scope. The Clear
  // button is keyboard-reachable.
  const clearButton = page.getByRole("button", { name: "Clear filters" });
  await expect(clearButton).toBeEnabled();
  await clearButton.focus();
  await clearButton.press("Enter");
  // Readout reflects the cleared scope (both providers, no type).
  await expect(page.getByTestId("search-effective-range")).toContainText("Codex + Claude Code");
  await expect(page.getByTestId("search-effective-range")).not.toContainText("type=memory");

  // Run the unfiltered search — full scope returns.
  await input.press("Enter");
  await expect(searchRegion.getByRole("listitem")).toHaveCount(2);
  await expect(searchRegion.locator('[data-provider="codex"]')).toBeVisible();
  await expect(searchRegion.locator('[data-provider="claude_code"]')).toBeVisible();
});

/**
 * Story 2.4 (Spec Change Log) pass-2 — the `resolvedSinceRef` fix resolves the
 * time preset to an absolute `since` ONCE on page 1 and reuses it for every
 * "Load more" in the session. A per-page recompute would bind a different
 * `since` into the cursor → `cursor_stale` → "Load more" breaks under a time
 * preset. This test has no UI coverage until now: it captures the `since` wire
 * param on the page-1 and page-2 requests and asserts they are equal, and that
 * no `cursor_stale` alert surfaces.
 */
test("since stays stable across Load more under a time preset", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  // Capture the `since` param on every search request so the page-1 and page-2
  // values can be compared.
  const sinceValues: (string | null)[] = [];
  const result = {
    record_id: "mock-record",
    excerpt: "mock excerpt",
    provider: "codex",
    source_id: "src_1",
    native_project: null,
    native_locator: "mock://semantic",
    display_locator: "mock://display",
    observed_at: 1,
    coverage_level: "full",
    health_state: "unknown",
  };
  await page.route("**/api/search?*", async (route) => {
    const params = new URL(route.request().url()).searchParams;
    sinceValues.push(params.get("since"));
    const cursor = params.get("cursor");
    // Page 1 (no cursor) returns one result + a cursor; page 2 returns one
    // more result and no cursor.
    if (!cursor) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ api_version: "1", payload: { results: [result], next_cursor: "v3.page2", empty_state: null, sources: [] } }),
      });
    }
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ api_version: "1", payload: { results: [{ ...result, record_id: "mock-record-2" }], next_cursor: null, empty_state: null, sources: [] } }),
    });
  });
  await page.goto("/");

  // Select the "Last 7 days" time preset.
  const timeSelect = page.getByLabel("Observed");
  await timeSelect.focus();
  await timeSelect.selectOption("7d");

  // Submit (page 1) — resolves `since` once and binds it into resolvedSinceRef.
  const input = page.getByLabel("Keyword");
  await input.fill("keyword");
  await input.press("Enter");
  await expect(page.getByRole("button", { name: "Load more" })).toBeVisible();

  // Load more (page 2) — reuses the session-stable `since`.
  await page.getByRole("button", { name: "Load more" }).press("Enter");
  await expect(page.getByRole("region", { name: "Memory search" }).getByRole("listitem")).toHaveCount(2);

  // Exactly two search requests fired (page 1 + page 2), both carrying a
  // non-null `since`, and the two values are EQUAL (the fix under test). A
  // per-page recompute would make them differ by the elapsed time.
  expect(sinceValues).toHaveLength(2);
  expect(sinceValues[0]).not.toBeNull();
  expect(sinceValues[0]).toBe(sinceValues[1]);

  // No cursor_stale alert surfaced: the page-2 cursor's bound `since` matches
  // the page-2 request's `since` (the fix), so pagination does not break under
  // a time preset.
  await expect(page.getByRole("alert")).toHaveCount(0);
});

/**
 * Story 2.4 pass-2 — the `emptyCopy` filter-aware branch names the active
 * filters instead of blaming the keyword. With a filter active and zero
 * results (`empty_state: "no_match"`), the copy must be the filter-aware
 * variant and must NOT contain the keyword-blaming "No indexed memory matched
 * this keyword.".
 */
test("filter-active empty state names active filters instead of blaming the keyword", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  // Zero results under the filter combination → empty_state: "no_match".
  await page.route("**/api/search?*", async (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ api_version: "1", payload: { results: [], next_cursor: null, empty_state: "no_match", sources: [] } }),
    }),
  );
  await page.goto("/");

  // Set a provider filter so filtersActive is true at empty-state render time.
  const providerSelect = page.getByLabel("Provider");
  await providerSelect.focus();
  await providerSelect.selectOption("codex");

  const input = page.getByLabel("Keyword");
  await input.fill("keyword");
  await input.press("Enter");

  // The copy is the filter-aware variant: it names the active filters.
  const searchRegion = page.getByRole("region", { name: "Memory search" });
  await expect(searchRegion).toContainText("No indexed memory matched within the active filters");
  // It must NOT fall back to the keyword-blaming copy.
  await expect(searchRegion).not.toContainText("No indexed memory matched this keyword.");
});

/**
 * Story 2.5 — the Source Inventory panorama groups cards by provider and
 * surfaces a health-summary header so cross-source health is comparable at a
 * glance. The backend has been multi-provider at the row level since 2.1; 2.5
 * adds the panorama affordance (grouping, summary, `data-provider` on each
 * card) and pins it here.
 *
 * The mock exercises every health branch the AC calls out:
 * - one `error`-health Codex source (the "one source down" AC) carrying a
 *   `latest_error`,
 * - a second Codex source that is `healthy`, so the `codex` bucket is a
 *   multi-card group AND the within-group health sort runs on it, and
 * - a `claude_code` `degraded` source so the panorama stays cross-provider.
 *
 * Assertions pin: the summary surfaces an "error" token (attention-first), the
 * error card's `latest_error` renders, within the same-provider group the
 * worse-health card precedes the healthy one in DOM order, the groups render in
 * stable alphabetical DOM order, and the summary precedes the grouped list.
 *
 * The existing single-source inventory coverage (the first test in this file)
 * stays intact; this test pins the multi-provider panorama contract.
 */
test("multi-provider inventory groups cards by provider with a health summary", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  const inventory = [
    {
      source_id: "src_codex_down",
      provider: "codex",
      lifecycle_state: "confirmed",
      root: "/fixture/codex/down",
      native_project: null,
      coverage_level: "full",
      health_state: "error",
      last_successful_scan: null,
      complete_record_count: 0,
      latest_error: "Tessera could not read this source.",
    },
    {
      source_id: "src_codex_ok",
      provider: "codex",
      lifecycle_state: "confirmed",
      root: "/fixture/codex/ok",
      native_project: null,
      coverage_level: "full",
      health_state: "healthy",
      last_successful_scan: 100,
      complete_record_count: 2,
      latest_error: null,
    },
    {
      source_id: "src_claude",
      provider: "claude_code",
      lifecycle_state: "confirmed",
      root: "/fixture/claude",
      native_project: "proj-claude",
      coverage_level: "full",
      health_state: "degraded",
      last_successful_scan: 200,
      complete_record_count: 1,
      latest_error: "Tessera could not access this source.",
    },
  ];
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: inventory }) }),
  );
  await page.goto("/");

  const inventoryRegion = page.getByRole("region", { name: "Source inventory" });

  // Health-summary header states the totals — cross-source health comparable
  // at a glance. Attention-first ordering surfaces the actionable `error`
  // token (the "one source down" AC) alongside degraded/healthy.
  const summary = inventoryRegion.getByTestId("inventory-summary");
  await expect(summary).toBeVisible();
  await expect(summary).toContainText("3 sources");
  await expect(summary).toContainText("1 error");
  await expect(summary).toContainText("1 degraded");
  await expect(summary).toContainText("1 healthy");

  // The summary sits ABOVE the grouped list in DOM order (not just both
  // visible). compareDocumentPosition: the first group is "following" the
  // summary iff the summary precedes it.
  const summaryFirst = await inventoryRegion.evaluate((root) => {
    const head = root.querySelector('[data-testid="inventory-summary"]');
    const firstGroup = root.querySelector("[data-provider-group]");
    if (!head || !firstGroup) return false;
    return (head.compareDocumentPosition(firstGroup) & Node.DOCUMENT_POSITION_FOLLOWING) !== 0;
  });
  expect(summaryFirst).toBe(true);

  // The cards render with the correct `data-provider` attribute. Assert via
  // the stable attribute, NOT provider-name text — `getByText("codex")` would
  // also match the card's Provider <dd> and the mock root path. Pinning the
  // attribute makes a real grouping regression fail loudly. Two codex cards
  // (the multi-card bucket) plus one claude card.
  await expect(inventoryRegion.locator('[data-provider="codex"]')).toHaveCount(2);
  await expect(inventoryRegion.locator('[data-provider="claude_code"]')).toHaveCount(1);

  // Provider grouping: one section per provider, each with its group heading
  // (accessible region name) and a stable `data-provider-group` container.
  await expect(inventoryRegion.getByRole("region", { name: "Codex provider group" })).toBeVisible();
  await expect(inventoryRegion.getByRole("region", { name: "Claude Code provider group" })).toBeVisible();
  await expect(inventoryRegion.locator('[data-provider-group="codex"]')).toBeVisible();
  await expect(inventoryRegion.locator('[data-provider-group="claude_code"]')).toBeVisible();

  // Stable alphabetical group order is pinned in DOM order: `claude_code`
  // precedes `codex` (the panorama must not flicker on confirmation order).
  const groupOrder = await inventoryRegion
    .locator("[data-provider-group]")
    .evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.providerGroup));
  expect(groupOrder).toEqual(["claude_code", "codex"]);

  // All three cards' full status renders — the panorama must NOT narrow the AC
  // field set. Three "Health" `<dt>` labels means every card rendered its
  // status row.
  await expect(inventoryRegion.getByText("Health", { exact: true })).toHaveCount(3);

  // The `error`-health source (one source down) carries its `latest_error`;
  // the `degraded` source carries its own. Both render — the panorama reflects
  // real registry state.
  await expect(inventoryRegion.getByText("Tessera could not read this source.")).toBeVisible();
  await expect(inventoryRegion.getByText("Tessera could not access this source.")).toBeVisible();

  // Within-group attention-first health sort: in the multi-card `codex` group
  // the worse-health (`error`) card precedes the `healthy` card in DOM order.
  // Read each codex card's Health `<dd>` in DOM order and assert the sequence.
  const codexHealths = await inventoryRegion
    .locator('[data-provider-group="codex"] [data-provider="codex"]')
    .evaluateAll((cards) =>
      cards.map((card) => {
        for (const dt of card.querySelectorAll("dt")) {
          if (dt.textContent === "Health") {
            return dt.nextElementSibling?.textContent ?? "";
          }
        }
        return "";
      }),
    );
  expect(codexHealths).toEqual(["error", "healthy"]);

  // Honest per-Coverage record counts render and are pluralized correctly
  // (count === 1 → "record", otherwise "records"). The non-Full "unavailable"
  // copy is exercised by the dedicated inventory-coverage test below.
  await expect(inventoryRegion.getByText("2 complete indexed records.", { exact: true })).toBeVisible();
  await expect(inventoryRegion.getByText("1 complete indexed record.", { exact: true })).toBeVisible();
});

/**
 * Story 2.5 review — the non-Full honest-count copy ("Complete count
 * unavailable: coverage is limited.") renders at the UI layer when a source's
 * `complete_record_count` is null, and no per-source "N complete indexed
 * records." line appears for it. A `search_only` source never claims a complete
 * count (only Full coverage may), so its null count must surface honestly.
 */
test("non-full inventory source renders the honest unavailable count copy", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  const inventory = [
    {
      source_id: "src_search_only",
      provider: "codex",
      lifecycle_state: "confirmed",
      root: "/fixture/codex/search-only",
      native_project: null,
      coverage_level: "search_only",
      health_state: "healthy",
      last_successful_scan: 100,
      complete_record_count: null,
      latest_error: null,
    },
  ];
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: inventory }) }),
  );
  await page.goto("/");

  const inventoryRegion = page.getByRole("region", { name: "Source inventory" });

  // The honest unavailable-count copy renders for the non-Full source.
  await expect(inventoryRegion.getByText("Complete count unavailable: coverage is limited.")).toBeVisible();
  // No per-source complete-count line renders for it (only Full coverage claims
  // a count).
  await expect(inventoryRegion.getByText(/complete indexed records?\./)).toHaveCount(0);
});

/**
 * Story 3.1 — Browse is keyboard-reachable (AD-21): enter from a confirmed
 * source's Inventory card via the Browse button, paginate via Load more, read
 * the Provenance `<dl>` fields, and render each of the three query-less
 * empty states distinctly. Mirrors the existing keyboard-search test's shape
 * (focus + Enter, assert Provenance labels, exercise the cursor/pagination
 * contract).
 *
 * The mock exercises every Browse branch the AC calls out:
 * - happy path (one record on page 1, a second record on page 2 via Load more),
 * - the three distinct empty states (`not_yet_scanned`, `no_indexable_memory`,
 *   `source_unavailable`).
 *
 * Keyboard contract (AD-21): the user can enter Browse, paginate, read
 * Provenance, and return to the Inventory without ever using a pointer.
 */
test("keyboard browse enters from inventory paginates and renders three empty states", async ({ page }) => {
  // No candidates; one confirmed source so the Browse button renders.
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  const inventory = [
    {
      source_id: "src_1",
      provider: "codex",
      lifecycle_state: "confirmed",
      root: "/fixture/codex",
      native_project: null,
      coverage_level: "full",
      health_state: "healthy",
      last_successful_scan: 100,
      complete_record_count: 2,
      latest_error: null,
    },
  ];
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: inventory }) }),
  );
  let browseCalls = 0;
  const result = {
    record_id: "rec-mock",
    excerpt: "browse mock excerpt",
    provider: "codex",
    source_id: "src_1",
    native_project: null,
    native_locator: "file:///fixture#semantic",
    display_locator: "file:///fixture#L1-L2",
    observed_at: 1,
    coverage_level: "full",
    health_state: "healthy",
  };
  // The browse mock returns one result + a cursor on page 1 and a different
  // result with no cursor on page 2. When the test later swaps the `source`
  // param to `src_*`, it returns the matching empty state (no cursor).
  await page.route("**/api/browse?*", async (route) => {
    browseCalls += 1;
    const params = new URL(route.request().url()).searchParams;
    const source = params.get("source");
    const cursor = params.get("cursor");
    if (source === "src_not_yet_scanned") {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ api_version: "1", payload: { results: [], next_cursor: null, empty_state: "not_yet_scanned", sources: [] } }),
      });
    }
    if (source === "src_no_memory") {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ api_version: "1", payload: { results: [], next_cursor: null, empty_state: "no_indexable_memory", sources: [] } }),
      });
    }
    if (source === "src_unavailable") {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ api_version: "1", payload: { results: [], next_cursor: null, empty_state: "source_unavailable", sources: [] } }),
      });
    }
    // src_1 (happy path).
    if (!cursor) {
      return route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ api_version: "1", payload: { results: [result], next_cursor: "b3.page2", empty_state: null, sources: [] } }),
      });
    }
    // Page 2: a distinct result, no cursor.
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({ api_version: "1", payload: { results: [{ ...result, record_id: "rec-mock-2" }], next_cursor: null, empty_state: null, sources: [] } }),
    });
  });

  await page.goto("/");

  // The Inventory card for the confirmed source carries a keyboard-reachable
  // Browse button. Story 3.1 AC: "enter from inventory via keyboard".
  const browseButton = page.getByRole("button", { name: "Browse", exact: true });
  await expect(browseButton).toBeVisible();
  await browseButton.focus();
  await page.keyboard.press("Enter");

  // The Browse view swapped in (App's hand-rolled view state, no router).
  const browseRegion = page.getByRole("region", { name: "Memory browse" });
  await expect(browseRegion).toBeVisible();
  // The Provenance fields render (Search's shared ResultCard is reused, so
  // the labels are identical: Provider, Source, Native project, Semantic
  // location, Display location, Last observed (scan), Coverage, Source
  // health).
  await expect(browseRegion.getByText("Provider").first()).toBeVisible();
  await expect(browseRegion.getByText("Semantic location").first()).toBeVisible();
  await expect(browseRegion.getByText("Source health").first()).toBeVisible();
  await expect(browseRegion.getByText("browse mock excerpt")).toBeVisible();

  // Load more (page 2): keyboard-reachable.
  const loadMore = browseRegion.getByRole("button", { name: "Load more" });
  await expect(loadMore).toBeVisible();
  await loadMore.focus();
  await page.keyboard.press("Enter");
  await expect(browseRegion.getByRole("listitem")).toHaveCount(2);

  // Back to inventory is keyboard-reachable (first focusable in the Browse
  // view).
  const backButton = browseRegion.getByRole("button", { name: "Back to inventory" });
  await expect(backButton).toBeVisible();
  await backButton.focus();
  await page.keyboard.press("Enter");
  // The Inventory region is visible again.
  await expect(page.getByRole("region", { name: "Source inventory" })).toBeVisible();

  // Three-state empty coverage: swap the inventory's source_id between
  // confirmed sources so the same Browse button activates Browse for each
  // distinct empty state. The mock keys off `source`, so we re-route the
  // inventory to return the next confirmed source's row and reload the
  // view.
  for (const [sourceId, expectedText] of [
    ["src_not_yet_scanned", "This source has not been scanned yet."],
    ["src_no_memory", "This source scanned successfully but contains no indexable Agent Memory."],
    ["src_unavailable", "This source is currently unavailable; its stored health was not changed."],
  ] as const) {
    await page.route("**/api/sources/inventory", async (route) =>
      route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          api_version: "1",
          payload: [{ ...inventory[0], source_id: sourceId }],
        }),
      }),
    );
    await page.reload();
    // The Browse button is keyboard-reachable after reload too.
    const reloadBrowse = page.getByRole("button", { name: "Browse", exact: true });
    await reloadBrowse.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("region", { name: "Memory browse" })).toBeVisible();
    await expect(page.getByText(expectedText)).toBeVisible();
    // No results render alongside the empty state — the three states describe
    // the browsed source's initial zero-result page.
    await expect(page.getByRole("region", { name: "Memory browse" }).getByRole("listitem")).toHaveCount(0);
    // Back to inventory to reset for the next iteration (or finish).
    await page.getByRole("button", { name: "Back to inventory" }).press("Enter");
  }

  // Smoke: at least the page-1 browse call fired during the test.
  expect(browseCalls).toBeGreaterThan(0);
});

/**
 * Story 3.1 (review pass) — the Browse view surfaces the FR-14 partial-
 * unavailability banner when the sidecar carries a non-`available` source,
 * and recovers from a mid-pagination `cursor_stale` (409) via the "Restart
 * from the new snapshot" affordance. Both paths were dead code under the
 * happy-path mock in the test above.
 */
test("browse surfaces partial-unavailability banner and recovers from cursor_stale", async ({ page }) => {
  await page.route("**/api/sources/discover", async (route) =>
    route.fulfill({ contentType: "application/json", body: JSON.stringify({ api_version: "1", payload: [] }) }),
  );
  await page.route("**/api/sources/inventory", async (route) =>
    route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        api_version: "1",
        payload: [
          {
            source_id: "src_1",
            provider: "codex",
            lifecycle_state: "confirmed",
            root: "/fixture/codex",
            native_project: null,
            coverage_level: "full",
            health_state: "healthy",
            last_successful_scan: 100,
            complete_record_count: 2,
            latest_error: null,
          },
        ],
      }),
    }),
  );
  const result = {
    record_id: "rec-mock",
    excerpt: "browse mock excerpt",
    provider: "codex",
    source_id: "src_1",
    native_project: null,
    native_locator: "file:///fixture#semantic",
    display_locator: "file:///fixture#L1-L2",
    observed_at: 1,
    coverage_level: "full",
    health_state: "healthy",
  };
  // Page 1 (no cursor): one result + a sidecar where another source is
  // `unavailable` (so the banner renders), plus a continuation cursor. Page 2
  // (cursor present): the generation changed → 409 cursor_stale.
  await page.route("**/api/browse?*", async (route) => {
    const cursor = new URL(route.request().url()).searchParams.get("cursor");
    if (cursor) {
      return route.fulfill({
        status: 409,
        contentType: "application/json",
        body: JSON.stringify({ code: "cursor_stale", message: "The index changed. Run the browse again.", source_id: null, phase: "browse" }),
      });
    }
    return route.fulfill({
      contentType: "application/json",
      body: JSON.stringify({
        api_version: "1",
        payload: {
          results: [result],
          next_cursor: "b3.page2",
          empty_state: null,
          sources: [{ source_id: "src_2", provider: "claude_code", native_project: null, status: "unavailable" }],
        },
      }),
    });
  });

  await page.goto("/");

  // Enter Browse via keyboard.
  const browseButton = page.getByRole("button", { name: "Browse", exact: true });
  await browseButton.focus();
  await page.keyboard.press("Enter");
  const browseRegion = page.getByRole("region", { name: "Memory browse" });
  await expect(browseRegion).toBeVisible();

  // V6 — the partial-unavailability banner renders for the unavailable
  // sidecar source (single-source browse: informational about OTHER sources).
  const banner = browseRegion.getByTestId("browse-source-status");
  await expect(banner).toBeVisible();
  await expect(banner).toContainText("Claude Code");

  // V5 — Load more hits the 409 cursor_stale; the stale message + Restart
  // button render (keyboard-reachable).
  const loadMore = browseRegion.getByRole("button", { name: "Load more" });
  await loadMore.focus();
  await page.keyboard.press("Enter");
  await expect(browseRegion.getByRole("alert")).toContainText("The index changed.");
  const restart = browseRegion.getByRole("button", { name: "Restart from the new snapshot" });
  await expect(restart).toBeVisible();

  // Activating Restart re-fetches page 1 (no cursor) → the result renders.
  await restart.focus();
  await page.keyboard.press("Enter");
  await expect(browseRegion.getByText("browse mock excerpt")).toBeVisible();
});
