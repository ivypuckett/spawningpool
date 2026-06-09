import { defineConfig } from "@playwright/test";

// Drives the screenshot render only — this is intentionally separate from the
// vitest unit suite (`npm test`). The demo spec launches its own Chromium and
// writes PNGs to app/media/screens/, so we don't declare browser `projects`
// here; we just need Playwright to start the Vite dev server and run the spec.
export default defineConfig({
  testDir: "./e2e",
  outputDir: "./test-results",
  fullyParallel: false,
  workers: 1,
  reporter: "list",
  // Generous: the first run may download Chrome (~150 MB) before rendering.
  timeout: 300_000,
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 60_000,
  },
});
