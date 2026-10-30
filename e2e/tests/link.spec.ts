import { test, expect, type Page } from "@playwright/test";
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

/** A QR the app's own encoder rendered for the code at `index`. */
function qrPng(index: number): string {
  const name = `qr-${String(index).padStart(2, "0")}.png`;
  return readFileSync(join(__dirname, "..", "fixtures", "qr", name)).toString(
    "base64",
  );
}

/**
 * A camera that is a still frame of a real sticker.
 *
 * getUserMedia returns a canvas captureStream showing the same PNG a printer would put
 * on paper, so jsQR runs over the actual encoded bytes and not a mock of its result.
 */
async function useQrCamera(page: Page, index: number) {
  await page.addInitScript(
    ({ png }) => {
      let pending: Promise<MediaStream> | null = null;

      if (!navigator.mediaDevices) {
        Object.defineProperty(navigator, "mediaDevices", {
          value: {},
          configurable: true,
        });
      }

      navigator.mediaDevices.getUserMedia = () => {
        pending ??= (async () => {
          const image = new Image();
          image.src = `data:image/png;base64,${png}`;
          await image.decode();

          const canvas = document.createElement("canvas");
          canvas.width = 640;
          canvas.height = 480;
          const context = canvas.getContext("2d");
          if (!context) throw new Error("no 2d context");
          context.fillStyle = "#ffffff";
          context.fillRect(0, 0, canvas.width, canvas.height);
          const side = 440;
          context.drawImage(
            image,
            (canvas.width - side) / 2,
            (canvas.height - side) / 2,
            side,
            side,
          );
          // Kept alive for the lifetime of the page; a collected canvas stops producing
          // frames and the video element goes black.
          (window as unknown as { __qrCanvas?: HTMLCanvasElement }).__qrCanvas =
            canvas;

          return canvas.captureStream(10);
        })();
        return pending;
      };
    },
    { png: qrPng(index) },
  );
}

const PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==",
  "base64",
);

test.describe("linking two stickers", () => {
  // The property is that a phone that has only ever seen the second sticker ends up on
  // the first one's page with the first one's capability, without being asked for a
  // secret. Read the URL on failure; the three ways this breaks are distinct:
  //   /c/LINK            the confirmation never reached the claim route
  //   /c/ORIGINAL?hop=1  the hop counter was not stripped
  //   the scan result     jsQR never decoded the image the camera showed it
  test("a link opens the other sticker's album without asking for its secret again", async ({
    page,
  }) => {
    const original = codes()[5];
    const link = codes()[6];
    await useQrCamera(page, 5);

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

    await test.step("scan the other sticker and confirm it", async () => {
      await page.goto(`/c/${link.id}#${link.secret}`);
      await expect(page.locator("main.chooser")).toBeVisible();

      // The typed path is gone: the card only offers the camera.
      await expect(page.locator(".link-scan input[type='text']")).toHaveCount(0);

      await page
        .getByRole("button", { name: "Scan the other sticker" })
        .click();

      const result = page.locator(".scan-result");
      await expect(result).toBeVisible();
      await expect(result).toContainText(
        original.id.replace(/(.{4})(?=.)/g, "$1 "),
      );

      // Still on the link until the visitor confirms the decoded sticker.
      await expect(page).toHaveURL(`/c/${link.id}`);
      await result
        .getByRole("button", { name: "Use this sticker" })
        .click();
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

  // The card has no typed fallback, so a refused camera has to explain itself instead of
  // looking like a dead button.
  test("says what to do when camera permission is refused", async ({ page }) => {
    const link = codes()[7];

    await page.addInitScript(() => {
      if (!navigator.mediaDevices) {
        Object.defineProperty(navigator, "mediaDevices", {
          value: {},
          configurable: true,
        });
      }
      navigator.mediaDevices.getUserMedia = async () => {
        throw new DOMException("Permission denied", "NotAllowedError");
      };
    });

    await page.goto(`/c/${link.id}#${link.secret}`);
    await expect(page.locator("main.chooser")).toBeVisible();
    await page.getByRole("button", { name: "Scan the other sticker" }).click();

    await expect(page.getByRole("alert")).toContainText(
      /camera permission was refused/i,
    );
    await expect(page.locator(".link-scan input[type='text']")).toHaveCount(0);
    // The code is still unclaimed, so the visitor can retry or pick another card.
    await expect(page.locator("main.chooser")).toBeVisible();
  });
});
