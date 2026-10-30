import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

// The width of a phone, not of the desktop browser the other tests use. Every
// assertion here failed on the deployed app while the behaviour tests passed, because
// those never looked at a 390px page or at a cached stylesheet.
test.use({ viewport: { width: 390, height: 844 } });

test.describe("at phone width", () => {
  test("the link chooser is a camera card with versioned assets", async ({ page }) => {
    const code = codes()[16];
    await page.goto(`/c/${code.id}#${code.secret}`);

    const card = page.locator(".cards form.has-fields");
    await expect(card).toBeVisible();

    // The stylesheet's URL must change when its bytes do, or a returning phone keeps
    // the previous deploy's layout for as long as its service worker lives.
    const href = await page
      .locator('link[rel="stylesheet"]')
      .getAttribute("href");
    expect(href).toMatch(/^\/assets\/app\.css\?v=[0-9a-f]{16}$/);

    const icon = await card.locator(".card-body .icon").boundingBox();
    expect(icon, "the icon has no box").not.toBeNull();
    expect(icon!.width, "the SVG is not constrained to the icon column").toBeLessThanOrEqual(40);

    const title = await card.locator(".card-body .title").boundingBox();
    const blurb = await card.locator(".card-body .blurb").boundingBox();
    expect(blurb!.y, "the title and blurb are on one line").toBeGreaterThanOrEqual(
      title!.y + title!.height - 1,
    );

    // The typed fields are gone; the card's one control is the camera.
    await expect(card.locator('input[type="text"]')).toHaveCount(0);
    await expect(card.locator("video")).toBeHidden();

    const start = await card
      .getByRole("button", { name: "Scan the other sticker" })
      .boundingBox();
    const cardBox = await card.boundingBox();
    expect(start!.height, "the camera button is below a thumb target").toBeGreaterThanOrEqual(44);
    expect(start!.width, "the camera button is not the card's own width").toBeGreaterThan(
      cardBox!.width / 2,
    );
  });

  test("the album is full width, the capture controls line up, and the dialog is an overlay", async ({
    page,
  }) => {
    const code = codes()[17];
    await page.goto(`/c/${code.id}#${code.secret}`);
    await page
      .getByRole("button", { name: /photograph what is in the box/i })
      .click();
    await expect(page.locator("main.album")).toBeVisible();

    // The two controls are one row: same top and same height. Different padding made
    // the green one taller and the outlined one sit low.
    const take = await page.locator(".capture-button span").boundingBox();
    const pick = await page.locator(".pick-button span").boundingBox();
    expect(Math.abs(take!.y - pick!.y), "the capture buttons are not on one line").toBeLessThanOrEqual(1);
    expect(Math.abs(take!.height - pick!.height), "the capture buttons are not the same height").toBeLessThanOrEqual(1);

    await page
      .locator('form.capture input[type="file"]')
      .first()
      .setInputFiles({ name: "box.png", mimeType: "image/png", buffer: PNG });
    await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);

    // The tile exists as soon as the upload lands; the image inside it is still a
    // separate request. Measuring before it has decoded reads height:auto as zero.
    const photoImage = page.locator("ul.grid li.tile img").first();
    await expect
      .poll(() => photoImage.evaluate((img) => img.naturalWidth))
      .toBeGreaterThan(0);

    // A photo is the width of the screen less main's 1rem padding on each side.
    const photo = await photoImage.boundingBox();
    const available = await page.evaluate(
      () => document.documentElement.clientWidth - 32,
    );
    expect(Math.abs(photo!.width - available), "the photo is not full width").toBeLessThanOrEqual(1);

    // The delete control belongs on the image, not in a strip below it. Read the
    // failure: a tile taller than the image is the in-flow button; a delete button near
    // the bottom is the wrong selector.
    const tile = await page.locator("ul.grid li.tile").first().boundingBox();
    expect(
      Math.abs(tile!.height - photo!.height),
      "the tile has a strip below the photo",
    ).toBeLessThanOrEqual(1);
    const remove = await page
      .locator("button[aria-label='Delete']")
      .first()
      .boundingBox();
    expect(remove!.y, "the delete button is not at the top").toBeLessThan(
      photo!.y + 40,
    );
    expect(remove!.x, "the delete button is not at the right").toBeGreaterThan(
      photo!.x + photo!.width - 50,
    );

    // "Manage" and "Location" were adjacent words on the deployed app because the bar's
    // flex gap was in the stylesheet the phone had not fetched. The bar may wrap now that
    // Tags is a fifth control, so the property is reading order and separation, not one
    // row.
    const controls = await Promise.all(
      ["Manage", "Location", "Tags", "Release"].map((name) =>
        page.getByRole("link", { name }).or(page.getByRole("button", { name, exact: true })).boundingBox(),
      ),
    );
    for (let i = 1; i < controls.length; i++) {
      const previous = controls[i - 1]!;
      const current = controls[i]!;
      if (Math.abs(current.y - previous.y) <= 1) {
        expect(current.x, "two controls are one word").toBeGreaterThan(
          previous.x + previous.width + 4,
        );
      } else {
        expect(current.y, "the control bar wrapped out of reading order").toBeGreaterThanOrEqual(
          previous.y + previous.height - 1,
        );
      }
    }

    // The dialog is an overlay, and it is not in the document before the button is
    // pressed. Read the failure: a dialog already present is the leaked-inline-modal
    // failure; a dialog with the wrong box is a layout failure.
    await expect(page.getByRole("dialog")).toHaveCount(0);
    await page.getByRole("button", { name: "Release", exact: true }).click();
    const dialog = page.getByRole("dialog", { name: "Release this code?" });
    await expect(dialog).toBeVisible();

    const viewport = page.viewportSize()!;
    const backdrop = await page.locator(".modal-backdrop").boundingBox();
    expect(backdrop!.x, "the backdrop does not cover the screen").toBe(0);
    expect(backdrop!.y).toBe(0);
    expect(backdrop!.width).toBe(viewport.width);
    expect(backdrop!.height).toBe(viewport.height);

    const dialogBox = await dialog.boundingBox();
    expect(
      Math.abs(dialogBox!.x + dialogBox!.width / 2 - viewport.width / 2),
      "the dialog is not centred horizontally",
    ).toBeLessThanOrEqual(2);
    expect(
      Math.abs(dialogBox!.y + dialogBox!.height / 2 - viewport.height / 2),
      "the dialog is not centred vertically",
    ).toBeLessThanOrEqual(2);

    await dialog.getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("dialog")).toHaveCount(0);
  });
});
