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

test.describe("releasing and recovering a code", () => {
  // Release is a soft delete, so the property is that the photos are still there
  // afterwards. The distinct failure modes: no Recover section means the history was
  // lost; an empty album after recovery means the handler rows were deleted rather than
  // kept behind the released binding.
  test("an album keeps its photos through release and recovery", async ({ page }) => {
    const code = codes()[10];

    await page.goto(`/c/${code.id}#${code.secret}`);
    await page
      .getByRole("button", { name: /photograph what is in the box/i })
      .click();
    await page
      .locator('form.capture input[type="file"]')
      .first()
      .setInputFiles({ name: "box.png", mimeType: "image/png", buffer: PNG });
    await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);

    const photo = await page.locator("ul.grid li.tile img").first().getAttribute("src");
    expect(photo).not.toBeNull();

    await page.getByRole("button", { name: "Release…" }).click();
    const dialog = page.getByRole("dialog", { name: "Release this code?" });
    await expect(dialog).toBeVisible();
    await expect(dialog).toContainText(/photos, link or destination stay/i);
    await dialog.getByRole("button", { name: "Release QR code" }).click();

    // The chooser is what an unused code offers; a used one also offers its history.
    await expect(page.getByRole("heading", { name: "Recover QR code" })).toBeVisible();
    const previous = page.locator(".recover-list li").first();
    await expect(previous).toContainText("Photos");
    await expect(previous).toContainText("1 photo");

    await previous.getByRole("button", { name: "Recover" }).click();

    await expect(page.locator("main.album")).toBeVisible();
    await expect(page.locator("ul.grid li.tile:not(.pending)")).toHaveCount(1);
    expect(
      await page.locator("ul.grid li.tile img").first().getAttribute("src"),
      "the recovered album is not the one that was released",
    ).toBe(photo);
  });

  // The dialog is the confirmation: closing it must do nothing at all, not release and
  // rely on the recovery to undo it.
  test("keeping a code closes the dialog without releasing it", async ({ page }) => {
    const code = codes()[13];

    await page.goto(`/c/${code.id}#${code.secret}`);
    await page
      .getByRole("button", { name: /photograph what is in the box/i })
      .click();
    await expect(page.locator("main.album")).toBeVisible();

    await page.getByRole("button", { name: "Release…" }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Keep it" }).click();
    await expect(page.getByRole("dialog")).toBeHidden();

    await page.reload();
    await expect(page.locator("main.album")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Recover QR code" })).toBeHidden();
  });
});

test.describe("a code that redirects", () => {
  /** Claims a forward code and gives it a destination, returning the destination. */
  async function forward(
    page: import("@playwright/test").Page,
    code: Code,
  ): Promise<string> {
    await page.goto(`/c/${code.id}#${code.secret}`);
    const destination = new URL("/go", page.url()).href;

    await page
      .getByRole("button", { name: /redirect to a website/i })
      .click();

    await page.getByLabel("Destination").fill(destination);
    const saved = page.waitForResponse(
      (response) =>
        response.request().method() === "PUT" && response.url().endsWith("/target"),
    );
    await page.getByRole("button", { name: "Save" }).click();
    await saved;

    await page.goto(`/c/${code.id}`);
    return destination;
  }

  // The countdown is the one part of a redirect that is easy to get wrong in both
  // directions: leaving before it is read, or never leaving at all. Staying is checked
  // by waiting past the deadline, which is the only way to observe an absence.
  test("shows where it goes, and staying wins over the countdown", async ({ page }) => {
    const destination = await forward(page, codes()[11]);

    await expect(page.locator(".interstitial .destination")).toContainText(destination);
    await expect(page.getByRole("link", { name: "Continue now" })).toBeVisible();

    await page.getByRole("button", { name: "Stay here" }).click();
    await page.waitForTimeout(6000); // past the five-second deadline
    await expect(page).toHaveURL(new RegExp(`/c/${codes()[11].id}$`));
  });

  test("leaves for its destination on its own", async ({ page }) => {
    const destination = await forward(page, codes()[12]);
    await expect(page).toHaveURL(destination, { timeout: 8000 });
    await expect(page.getByRole("heading", { name: "Open a code" })).toBeVisible();
  });
});
