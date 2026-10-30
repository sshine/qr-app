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

function grouped(code: string): string {
  return code.replace(/(.{4})(?=.)/g, "$1 ");
}

function uniqueTag(prefix: string): string {
  return `${prefix} ${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

async function album(page: import("@playwright/test").Page, code: Code) {
  await page.goto(`/c/${code.id}#${code.secret}`);
  await page
    .getByRole("button", { name: /photograph what is in the box/i })
    .click();
  await expect(page.locator("main.album")).toBeVisible();
}

async function upload(page: import("@playwright/test").Page, code: Code) {
  const response = await page.request.post(
    `/c/${code.id}/api/v1/h/photo-album/photos`,
    {
      multipart: {
        photo: { name: "box.png", mimeType: "image/png", buffer: PNG },
      },
    },
  );
  expect(response.ok(), "the photo upload was refused").toBeTruthy();
}

async function tag(page: import("@playwright/test").Page, code: Code, name: string) {
  const response = await page.request.post(`/c/${code.id}/api/v1/tags`, {
    data: { name },
  });
  expect(response.ok(), "the tag was refused").toBeTruthy();
}

function card(page: import("@playwright/test").Page, code: Code) {
  return page.locator(".label-card", { hasText: grouped(code.id) });
}

test("filters by a tag and reveals a deleted label only when asked", async ({
  page,
}) => {
  const kept = codes()[22];
  const other = codes()[23];
  const tagName = uniqueTag("Overview");
  await album(page, kept);
  await upload(page, kept);
  await tag(page, kept, tagName);
  await album(page, other);

  await page.goto("/kiksmiks");
  await expect(card(page, kept)).toBeVisible();
  await expect(card(page, other)).toBeVisible();

  const filters = page.locator("form.filters");
  await filters.getByRole("checkbox", { name: tagName, exact: true }).check();
  await filters.getByRole("button", { name: "Filter" }).click();

  await expect(card(page, kept)).toBeVisible();
  await expect(card(page, other)).toHaveCount(0);

  // Hiding is the overview's own opinion, not a revoke: the card leaves the default
  // view and comes back under Show deleted, with a Restore control.
  await card(page, kept).getByRole("button", { name: "Delete" }).click();
  await expect(card(page, kept)).toHaveCount(0);

  await filters.getByRole("checkbox", { name: "Show deleted labels" }).check();
  await filters.getByRole("button", { name: "Filter" }).click();
  const deleted = card(page, kept);
  await expect(deleted).toBeVisible();
  await expect(deleted).toContainText("deleted");

  await deleted.getByRole("button", { name: "Restore" }).click();
  await expect(card(page, kept)).toBeVisible();
  await expect(card(page, kept)).not.toContainText("deleted");
});

test("a photo label shows its top photo and the extra ones as a count", async ({
  page,
}) => {
  const code = codes()[24];
  await album(page, code);
  await upload(page, code);
  await upload(page, code);
  await upload(page, code);

  await page.goto("/kiksmiks");
  const shown = card(page, code);
  await expect(shown).toBeVisible();
  await expect(shown.locator("img")).toBeVisible();
  await expect(shown.locator(".photo-count")).toHaveText("+2");
});

test.describe("label proportions", () => {
  test("is a list on a phone and a gallery of label-shaped boxes on a desktop", async ({
    page,
  }) => {
    const code = codes()[25];
    await album(page, code);
    await upload(page, code);
    await upload(page, code);

    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto("/kiksmiks");
    const mobile = card(page, code);
    await mobile.scrollIntoViewIfNeeded();
    const mobileBox = await mobile.boundingBox();
    const thumb = await mobile.locator(".thumb").boundingBox();
    const text = await mobile.locator(".card-text").boundingBox();
    expect(mobileBox, "the card is not laid out").not.toBeNull();
    expect(thumb!.x, "the thumbnail is not beside the text").toBeLessThan(text!.x);
    expect(
      Math.abs(thumb!.y - text!.y),
      "the mobile card is not one row",
    ).toBeLessThanOrEqual(2);
    expect(
      mobileBox!.width,
      "the mobile card is not the width the screen allows",
    ).toBeGreaterThan(300);

    await page.setViewportSize({ width: 1280, height: 900 });
    await page.reload();
    const desktop = card(page, code);
    await desktop.scrollIntoViewIfNeeded();
    const desktopBox = await desktop.boundingBox();
    expect(desktopBox, "the desktop card is not laid out").not.toBeNull();
    expect(
      desktopBox!.width / desktopBox!.height,
      "the desktop card is not shaped like a 63.5 x 38.1mm label",
    ).toBeCloseTo(63.5 / 38.1, 1);
    expect(desktopBox!.x, "the gallery has no desktop margin").toBeGreaterThan(24);
  });
});
