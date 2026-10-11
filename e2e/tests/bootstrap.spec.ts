import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

// Read inside the test rather than at import: Playwright loads spec files before it
// starts the web server, and `just serve-e2e` mints these into a database made for
// this run. Reading at import would hand every run the previous run's codes, against
// a database that has never heard of them.
function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

test.describe("opening a code from its sticker", () => {
  // The whole capability flow in one assertion. The secret rides in the fragment,
  // which no browser sends to a server, so the page that arrives is a placeholder
  // whose script has to exchange the fragment for a cookie and reload. If that
  // exchange never starts, the visitor is left on the typed form with their secret
  // sitting unused in the address bar.
  test("a scanned link opens the code rather than asking for it to be typed", async ({
    page,
  }) => {
    const code = codes()[0];
    await page.goto(`/c/${code.id}#${code.secret}`);

    // The chooser is what an unclaimed code offers once the capability is held.
    await expect(
      page.getByRole("heading", { name: /what should this code be/i }),
    ).toBeVisible();

    // And the fragment is gone from the address bar, so the secret is not left in
    // history or in anything that reads location.
    await expect.poll(() => page.evaluate(() => location.hash)).toBe("");
  });

  // What a visitor without script gets, and the way back in when the exchange fails.
  test("the typed form is still reachable for a code with no secret", async ({
    page,
  }) => {
    const code = codes()[1];
    await page.goto(`/c/${code.id}`);

    await expect(
      page.getByRole("heading", { name: /opening this code/i }),
    ).toBeVisible();
    await expect(page.locator('input[name="secret"]')).toBeVisible();
  });
});
