import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

// Read inside the test: the web server mints these fixtures when it starts, which is
// after Playwright has loaded the spec files.
function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

test.describe("linking two stickers", () => {
  // The property is that a phone that has only ever seen the second sticker ends up on
  // the first one's page with the first one's capability, without being asked for a
  // secret. Read the URL on failure; the two ways this breaks are distinct:
  //   /c/LINK         the delegation never ran
  //   /c/ORIGINAL?hop=1  the hop counter was not stripped
  test("a link opens the other sticker's album without asking for its secret again", async ({
    page,
  }) => {
    const original = codes()[5];
    const link = codes()[6];

    await test.step("make the original an album with one photo", async () => {
      await page.goto(`/c/${original.id}#${original.secret}`);
      await page
        .getByRole("button", { name: /photograph what is in the box/i })
        .click();
      await expect(page.locator("main.album")).toBeVisible();

      await page
        .locator('form.capture input[type="file"]')
        .first()
        .setInputFiles({ name: "box.png", mimeType: "image/png", buffer: PNG });
      // Not merely `li.tile`: the pending tile is drawn from the local file before the
      // upload has landed, and navigating away now would abort the request and leave an
      // album that never had a photo.
      await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);
    });

    await test.step("claim the second sticker as a link to it", async () => {
      await page.goto(`/c/${link.id}#${link.secret}`);
      await expect(page.locator("main.chooser")).toBeVisible();

      await page.getByLabel("Code on the other sticker").fill(original.id);
      await page.getByLabel("Secret under it").fill(original.secret);
      await page.getByRole("button", { name: "Use this" }).click();
    });

    await test.step("land on the original, with its photo", async () => {
      await expect(page).toHaveURL(`/c/${original.id}`);
      await expect(page.locator("main.album")).toBeVisible();
      await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);

      const src = await page
        .locator("ul.grid li.tile img")
        .first()
        .getAttribute("src");
      expect(src, "the album did not render under the original's path").toContain(
        `/c/${original.id}/`,
      );
    });
  });

  // Linking is a capability handover, so the proof must be the real secret and not
  // merely the printed code id.
  test("refuses a link when the other sticker's secret is wrong", async ({ page }) => {
    const original = codes()[7];
    const link = codes()[8];

    await page.goto(`/c/${original.id}#${original.secret}`);
    await page
      .getByRole("button", { name: /photograph what is in the box/i })
      .click();
    await expect(page.locator("main.album")).toBeVisible();

    await page.goto(`/c/${link.id}#${link.secret}`);
    await page.getByLabel("Code on the other sticker").fill(original.id);
    await page.getByLabel("Secret under it").fill("X".repeat(16));
    await page.getByRole("button", { name: "Use this" }).click();

    await expect(page.locator("body")).toContainText(/do not open anything/i);
  });
});
