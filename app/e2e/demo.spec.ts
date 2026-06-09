// Records a narrated walkthrough of the spawningpool desktop UI as a .webm and
// saves it to app/media/spawningpool-demo.webm. The frontend is the exact
// Svelte app shipped in Tauri; only the IPC backend is mocked (see
// tauri-mock.ts) so the render is deterministic and needs no display server.
//
// Run it with `npm --prefix app run render-video`.

import { test, chromium } from "@playwright/test";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { seed } from "./seed";
import { installTauriMock } from "./tauri-mock";

const here = path.dirname(fileURLToPath(import.meta.url));
const OUTPUT = path.resolve(here, "../media/spawningpool-demo.webm");
const TMP_DIR = path.resolve(here, "../test-results/video");
const BASE_URL = process.env.PW_BASE_URL ?? "http://localhost:1420";
const SIZE = { width: 1280, height: 800 };

// A beat between actions so the recording is watchable rather than a blur.
const BEAT = 700;

test("render desktop UI walkthrough", async () => {
  const browser = await chromium.launch();
  const context = await browser.newContext({
    viewport: SIZE,
    recordVideo: { dir: TMP_DIR, size: SIZE },
  });
  await installTauriMock(context, seed);
  const page = await context.newPage();

  await page.goto(BASE_URL);

  // The left rail shows live counts once the registry loads.
  await page.getByRole("button", { name: /Providers/ }).waitFor();
  await page.waitForTimeout(BEAT);

  // Providers: open one definition.
  await page.getByRole("button", { name: "anthropic" }).click();
  await page.waitForTimeout(BEAT);

  // Models: browse, then filter down and pick a result.
  await page.getByRole("button", { name: /Models/ }).click();
  await page.waitForTimeout(BEAT);
  await page.getByRole("button", { name: "claude-opus-4" }).click();
  await page.waitForTimeout(BEAT);
  await page.getByLabel("Filter models").fill("gpt");
  await page.waitForTimeout(BEAT);
  await page.getByRole("button", { name: "gpt-4o" }).click();
  await page.waitForTimeout(BEAT);

  // Specialists: open one with a non-trivial definition.
  await page.getByRole("button", { name: /Specialists/ }).click();
  await page.waitForTimeout(BEAT);
  await page.getByRole("button", { name: "code-reviewer" }).click();
  await page.waitForTimeout(BEAT);

  // Tools: open one and let the final frame settle.
  await page.getByRole("button", { name: /Tools/ }).click();
  await page.waitForTimeout(BEAT);
  await page.getByRole("button", { name: "web-search" }).click();
  await page.waitForTimeout(BEAT * 2);

  // Closing the context finalizes the recording; the Video handle stays valid.
  const video = page.video();
  await context.close();
  await browser.close();
  if (!video) {
    throw new Error("no video was recorded");
  }
  await video.saveAs(OUTPUT);
});
