import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

/** Claims an unused code as an empty photo album. */
async function album(page: import("@playwright/test").Page, code: Code) {
  await page.goto(`/c/${code.id}#${code.secret}`);
  await page
    .getByRole("button", { name: /photograph what is in the box/i })
    .click();
  await expect(page.locator("main.album")).toBeVisible();
}

function uniqueTag(prefix: string): string {
  return `${prefix} ${Date.now()}-${Math.random().toString(36).slice(2, 6)}`;
}

async function openTags(page: import("@playwright/test").Page) {
  await page.locator(".code-controls").getByRole("button", { name: "Tags" }).click();
  await expect(page.getByRole("dialog", { name: "Tags" })).toBeVisible();
}

test.describe("tags on a sticker", () => {
  // The property: a tag typed on the phone is on the sticker, not only in the dialog.
  // Read the failure at the control bar after reload: the modal updating but the page
  // not is the difference between a client-side list and a stored tag.
  test("creates a tag from the picker and keeps it after a reload", async ({
    page,
  }) => {
    const code = codes()[18];
    const tag = uniqueTag("Loft");
    await album(page, code);

    await openTags(page);
    await page.getByLabel("Add a tag").fill(tag);
    await page.getByRole("button", { name: "Create tag" }).click();
    await expect(
      page.getByRole("dialog", { name: "Tags" }).getByRole("button", {
        name: new RegExp(tag),
      }),
    ).toBeVisible();
    await page.getByRole("button", { name: "Done" }).click();

    await expect(page.locator(".code-controls")).toContainText(tag);
    await page.reload();
    await expect(page.locator(".code-controls")).toContainText(tag);
  });

  // The catalog is shared, which is the reason for creating tags at all: the second
  // box in the loft should be one tap, not a second spelling.
  test("offers a tag created on another sticker", async ({ page }) => {
    const first = codes()[19];
    const second = codes()[20];
    const tag = uniqueTag("Bay");

    await album(page, first);
    await openTags(page);
    await page.getByLabel("Add a tag").fill(tag);
    await page.getByRole("button", { name: "Create tag" }).click();
    await page.getByRole("button", { name: "Done" }).click();

    await album(page, second);
    await openTags(page);
    await page.getByRole("button", { name: tag }).click();
    await page.getByRole("button", { name: "Done" }).click();

    await expect(page.locator(".code-controls")).toContainText(tag);
  });

  // Removing has to reach the server as well, or the chip disappears until the next
  // scan and then comes back.
  test("removes a tag from the picker", async ({ page }) => {
    const code = codes()[21];
    const tag = uniqueTag("Shelf");
    await album(page, code);

    await openTags(page);
    await page.getByLabel("Add a tag").fill(tag);
    await page.getByRole("button", { name: "Create tag" }).click();
    await page.getByRole("button", { name: "Done" }).click();
    await expect(page.locator(".code-controls")).toContainText(tag);

    await openTags(page);
    await page
      .getByRole("dialog", { name: "Tags" })
      .getByRole("button", { name: new RegExp(tag) })
      .click();
    await page.getByRole("button", { name: "Done" }).click();

    await expect(page.locator(".code-controls")).not.toContainText(tag);
    await page.reload();
    await expect(page.locator(".code-controls")).not.toContainText(tag);
  });
});
