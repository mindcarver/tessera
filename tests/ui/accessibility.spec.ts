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
