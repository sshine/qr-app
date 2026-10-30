import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

// A one-pixel PNG. Small enough to keep the test about the page rather than about
// image decoding, and real enough that the pipeline accepts and re-encodes it.
const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

/** Opens a code and claims it as a photo album, returning the album page. */
async function album(page: import("@playwright/test").Page, code: Code) {
  await page.goto(`/c/${code.id}#${code.secret}`);
  await page.getByRole("button", { name: /photograph what is in the box/i }).click();
  await expect(page.locator("main.album")).toBeVisible();
}

test.describe("photo album", () => {
  // The server answers 201 and the component updates its own array, but the grid is
  // rendered from the server's list. If the two are not the same list, the upload
  // succeeds and the page never shows it.
  test("an uploaded photo appears without reloading", async ({ page }) => {
    await album(page, codes()[2]);
    await expect(page.locator("ul.grid li.tile")).toHaveCount(0);

    await page
      .locator('form.capture input[type="file"]')
      .first()
      .setInputFiles({ name: "box.png", mimeType: "image/png", buffer: PNG });

    await expect(page.locator("ul.grid li.tile")).toHaveCount(1);
  });

  // Deleting has to be the component's job too. A plain form post navigates away from
  // the album, and a browser handed a body it will not render saves it instead, which
  // is what a phone reports as downloading a file called "delete".
  test("deleting a photo updates the page rather than navigating away", async ({
    page,
  }) => {
    await album(page, codes()[3]);
    await page
      .locator('form.capture input[type="file"]')
      .first()
      .setInputFiles({ name: "box.png", mimeType: "image/png", buffer: PNG });
    await expect(page.locator("ul.grid li.tile")).toHaveCount(1);

    const before = page.url();
    const download = page.waitForEvent("download", { timeout: 2000 }).catch(() => null);
    await page.locator("li.tile button[aria-label='Delete']").first().click();

    await expect(page.locator("ul.grid li.tile")).toHaveCount(0);
    expect(page.url(), "navigated away from the album").toBe(before);
    expect(await download, "the browser was handed a file to save").toBeNull();
  });
});

// The landing page is the only one not reached by scanning, and the list it shows comes
// from the browser's own storage: the service has no idea who is asking and a list of
// every code would be the inventory the capability scheme exists to withhold.
test("a code that has been opened is offered again on the landing page", async ({
  page,
}) => {
  const code = codes()[4];
  await page.goto(`/c/${code.id}#${code.secret}`);
  await expect(page.locator("main.chooser")).toBeVisible();

  await page.goto("/");
  await expect(page.getByRole("link", { name: code.id })).toBeVisible();
});
