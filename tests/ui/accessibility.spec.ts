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
