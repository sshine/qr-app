import { defineConfig, devices } from "@playwright/test";

// Not the port the dev server uses: reusing it would let a test run mutate the
// state being clicked through by hand.
const PORT = 8081;
const BASE_URL = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./tests",
  outputDir: "./test-results",
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: [
    ["list"],
    ["html", { outputFolder: "playwright-report", open: "never" }],
  ],

  use: {
    baseURL: BASE_URL,
    trace: "on-first-retry",
  },

  projects: [
    {
      name: "chromium",
      use: {
        ...devices["Desktop Chrome"],
        // The default devshell carries only the headless shell. `nix develop
        // .#headed` sets PW_HEADED and carries full Chromium, where leaving the
        // channel unset is what allows --headed and --ui to work.
        channel: process.env.PW_HEADED ? undefined : "chromium-headless-shell",
      },
    },
  ],

  webServer: {
    command: "just serve-e2e",
    // Relative to this file; the command runs from the repository root.
    cwd: "..",
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    // Sized for a cold build, not for a server start.
    timeout: 300_000,
    stdout: "pipe",
    stderr: "pipe",
  },
});
