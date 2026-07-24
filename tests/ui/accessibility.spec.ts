/**
 * Tessera — keyboard-driven scan accessibility regression (Story 1.4).
 *
 * Path is fixed by the architecture spine (AD-21):
 *   `tests/ui/accessibility.spec.ts`
 *
 * Uses an isolated Codex memory fixture and the real loopback HTTP server.
 * The flow must stay keyboard reachable: confirm the discovered source,
 * trigger Scan with Enter, observe the successful POST response, then read
 * the polite status announcement and persisted scan state.
 */

import { expect, test } from "@playwright/test";

test.describe.configure({ mode: "serial" });

test("keyboard scan posts successfully and announces completion", async ({ page }) => {
  await page.goto("/");

  const confirm = page.getByRole("button", { name: "Confirm" });
  await expect(confirm).toBeVisible();
  await confirm.focus();
  await page.keyboard.press("Enter");

  const scan = page.getByRole("button", { name: "Scan", exact: true });
  await expect(scan).toBeVisible();
  const scanResponse = page.waitForResponse(
    (response) =>
      response.url().endsWith("/api/scan") && response.request().method() === "POST",
  );
  await scan.focus();
  await page.keyboard.press("Enter");

  expect((await scanResponse).status()).toBe(200);
  await expect(page.getByTestId("scan-announcement")).toContainText("Scan complete.");
  await expect(page.getByText(/Scan succeeded — generation gen_\d+/)).toBeVisible();
});

test("keyboard search renders provenance, empty states, pagination, and safe API errors", async ({ page }) => {
  await page.goto("/");
  let stalePage = 0;
  await page.route("**/api/search?*", async (route) => {
    const q = new URL(route.request().url()).searchParams.get("q");
    const payload = (empty_state: string | null, results: unknown[] = [], next_cursor: string | null = null) => ({ api_version: "1", payload: { results, next_cursor, empty_state } });
    const result = { record_id: "mock-record", excerpt: "mock excerpt", provider: "codex", source_id: "src_1", native_project: null, native_locator: "mock://semantic", display_locator: "mock://display", observed_at: 1, coverage_level: "full", health_state: "unknown" };
    if (q === "stale-page") {
      stalePage += 1;
      if (stalePage === 1) return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload(null, [result], "v1.cursor")) });
      return route.fulfill({ status: 409, contentType: "application/json", body: JSON.stringify({ code: "cursor_stale", message: "The index changed. Run the search again.", source_id: null, phase: "search" }) });
    }
    if (q === "not-indexed") return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload("source_not_indexed")) });
    if (q === "unavailable") return route.fulfill({ contentType: "application/json", body: JSON.stringify(payload("source_unavailable")) });
    return route.continue();
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

  const loadMore = page.getByRole("button", { name: "Load more" });
  await expect(loadMore).toBeVisible();
  await loadMore.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("region", { name: "Memory search" }).getByRole("listitem")).toHaveCount(3);

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
