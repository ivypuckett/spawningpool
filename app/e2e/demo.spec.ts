// Renders a deterministic visual representation of the spawningpool desktop UI
// from this environment — no display server, no Rust build, no Tauri webview.
//
// It drives the exact Svelte frontend the app ships, mocking only the Tauri IPC
// backend with fixed seed data (see tauri-mock.ts / seed.ts), so the output is
// reproducible. Each "scene" is captured as a PNG under app/media/screens/.
// Those PNGs are the deliverable: an agent reads them back to verify its own
// work, and the pre-commit hook publishes them to a PR (see .githooks/pre-commit).
//
// Run it with `npm --prefix app run render`.

import { test, chromium, type Page } from "@playwright/test";
import { mkdir, rm } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensureChrome } from "./browser";
import { seed } from "./seed";
import { installTauriMock } from "./tauri-mock";

const here = path.dirname(fileURLToPath(import.meta.url));
const SCREENS = path.resolve(here, "../media/screens");
const BASE_URL = process.env.PW_BASE_URL ?? "http://localhost:1420";
const SIZE = { width: 1280, height: 800 };
const BEAT = 250; // let the UI settle before each capture

async function shot(page: Page, name: string): Promise<void> {
  await page.waitForTimeout(BEAT);
  await page.screenshot({ path: path.join(SCREENS, `${name}.png`) });
}

test("render desktop UI states", async () => {
  // Start clean so a removed scene never leaves a stale PNG behind.
  await rm(SCREENS, { recursive: true, force: true });
  await mkdir(SCREENS, { recursive: true });

  const browser = await chromium.launch({
    executablePath: await ensureChrome(),
    args: ["--no-sandbox"], // CI runners run as root
  });
  const context = await browser.newContext({
    viewport: SIZE,
    deviceScaleFactor: 2, // crisp screenshots
  });
  await installTauriMock(context, seed);
  const page = await context.newPage();

  await page.goto(BASE_URL);
  await page.getByRole("button", { name: /Providers/ }).waitFor();
  await shot(page, "01-overview");

  // Providers: open a definition.
  await page.getByRole("button", { name: "anthropic" }).click();
  await shot(page, "02-provider");

  // Models: filter the list, then open a result.
  await page.getByRole("button", { name: /Models/ }).click();
  await page.getByLabel("Filter models").fill("gpt");
  await page.getByRole("button", { name: "gpt-4o" }).click();
  await shot(page, "03-model-filtered");

  // Specialists: open one with a richer definition.
  await page.getByRole("button", { name: /Specialists/ }).click();
  await page.getByRole("button", { name: "code-reviewer" }).click();
  await shot(page, "04-specialist");

  // Tools: open one.
  await page.getByRole("button", { name: /Tools/ }).click();
  await page.getByRole("button", { name: "web-search" }).click();
  await shot(page, "05-tool");

  await context.close();
  await browser.close();
});
