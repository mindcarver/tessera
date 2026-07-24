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
    const payload = (empty_state: string | null, results: unknown[] = [], next_cursor: string | null = null) => ({ api_version: "1", payload: { results, next_cursor, empty_state } });
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
