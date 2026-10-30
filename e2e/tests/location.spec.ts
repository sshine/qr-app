import { test, expect } from "@playwright/test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

type Code = { id: string; secret: string };

function codes(): Code[] {
  return JSON.parse(
    readFileSync(join(__dirname, "..", "fixtures", "codes.json"), "utf8"),
  );
}

// 127.0.0.1 is a secure context, so the browser exposes geolocation here just as it
// does to a phone over HTTPS. The coordinates are a real point in Copenhagen.
test.use({
  permissions: ["geolocation"],
  geolocation: { latitude: 55.6761, longitude: 12.5683 },
});

/** Claims an unused code as an empty photo album. */
async function album(page: import("@playwright/test").Page, code: Code) {
  await page.goto(`/c/${code.id}#${code.secret}`);
  await page
    .getByRole("button", { name: /photograph what is in the box/i })
    .click();
  await expect(page.locator("main.album")).toBeVisible();
}

test.describe("tagging where a box is", () => {
  // The property: the tag is on the sticker, so it is still there after a reload. It
  // also has to survive the round trip through the browser's geolocation prompt, which
  // is why the button stays disabled until coordinates actually arrive.
  test("takes the place from the device and keeps it", async ({ page }) => {
    const code = codes()[14];
    await album(page, code);

    await page.getByRole("link", { name: "Location" }).click();
    await expect(
      page.getByRole("heading", { name: "Where is this code?" }),
    ).toBeVisible();

    await page.getByLabel("Name").fill("Loft, shelf 3");
    await page.getByRole("button", { name: "Use my location" }).click();
    await expect(page.getByText(/Got it: 55\.676100, 12\.568300/)).toBeVisible();

    await page.getByRole("button", { name: "Tag this place" }).click();
    await expect(page).toHaveURL(new RegExp(`/c/${code.id}$`));
    await expect(page.locator(".code-controls")).toContainText("Loft, shelf 3");

    await page.reload();
    await expect(page.locator(".code-controls")).toContainText("Loft, shelf 3");
  });

  // Choosing from the catalog is the half that works without geolocation, and the
  // reason a second box in the same place does not need a second name.
  test("chooses an existing place by name", async ({ page }) => {
    const code = codes()[15];
    await album(page, code);

    const stamp = Date.now();
    const first = `Depot ${stamp}`;
    const second = `Bay ${stamp}`;
    for (const name of [first, second]) {
      const response = await page.request.post(`/c/${code.id}/api/v1/location`, {
        form: { name, lat: "55.7", lon: "12.5" },
      });
      expect(response.ok(), `could not seed ${name}`).toBeTruthy();
    }

    await page.goto(`/c/${code.id}/location`);
    await page.getByLabel("Place").selectOption({ label: first });
    await page.getByRole("button", { name: "Tag", exact: true }).click();

    await expect(page).toHaveURL(new RegExp(`/c/${code.id}$`));
    await expect(page.locator(".code-controls")).toContainText(first);
  });
});
