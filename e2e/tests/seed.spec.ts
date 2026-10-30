import { test, expect } from "@playwright/test";

// The starting point for the Playwright test generator: a page it can drive while a
// test is being written, not a test of anything in particular. Left alone on purpose.
test("seed", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("main.landing")).toBeVisible();
});
