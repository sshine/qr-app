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
    await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);

    const before = page.url();
    const download = page.waitForEvent("download", { timeout: 2000 }).catch(() => null);
    await page.locator("li.tile button[aria-label='Delete']").first().click();

    await expect(page.locator("ul.grid li.tile")).toHaveCount(0);
    expect(page.url(), "navigated away from the album").toBe(before);
    expect(await download, "the browser was handed a file to save").toBeNull();
  });

  // A photo is the page. Read the failure: a width near zero means the tile kept its
  // old square-grid sizing; a width that changes between the two orientations but not
  // with the viewport means the image was sized in pixels rather than percentages.
  test("a photo fills the width of the screen in either orientation", async ({
    page,
  }) => {
    await album(page, codes()[9]);

    await page
      .locator('form.capture input[type="file"]')
      .nth(1)
      .setInputFiles([
        { name: "one.png", mimeType: "image/png", buffer: PNG },
        { name: "two.png", mimeType: "image/png", buffer: PNG },
      ]);
    await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(2);

    /** The image width, and what the content box allows: the viewport minus main's 1rem
     * padding on each side. Scrollbars are why this cannot use `viewportSize()`. */
    async function widths() {
      const tiles = page.locator("ul.grid li.tile:not(.pending)");
      const first = await tiles.nth(0).boundingBox();
      const second = await tiles.nth(1).boundingBox();
      const available = await page.evaluate(
        () => document.documentElement.clientWidth - 32,
      );
      return { first, second, available };
    }

    const portrait = await widths();
    expect(portrait.first, "the first photo is not laid out").not.toBeNull();
    expect(portrait.second, "the second photo is not laid out").not.toBeNull();
    for (const box of [portrait.first, portrait.second]) {
      expect(
        Math.abs(box!.width - portrait.available),
        `a photo is ${box!.width}px wide, not the ${portrait.available}px the screen allows`,
      ).toBeLessThanOrEqual(1);
    }
    expect(
      portrait.second!.y,
      "the photos are side by side; they should be rows",
    ).toBeGreaterThanOrEqual(portrait.first!.y + portrait.first!.height - 1);

    const size = page.viewportSize();
    expect(size).not.toBeNull();
    await page.setViewportSize({ width: size!.height, height: size!.width });

    const landscape = await widths();
    for (const box of [landscape.first, landscape.second]) {
      expect(
        Math.abs(box!.width - landscape.available),
        `after turning the phone a photo is ${box!.width}px wide, not ${landscape.available}px`,
      ).toBeLessThanOrEqual(1);
    }
    expect(landscape.available, "the viewport did not change").not.toBe(
      portrait.available,
    );
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
