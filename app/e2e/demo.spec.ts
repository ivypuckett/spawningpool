// Renders a deterministic visual representation of the spawningpool desktop UI
// from this environment — no display server, no Rust build, no Tauri webview.
//
// It drives the exact Svelte frontend that ships in the app, mocking only the
// Tauri IPC backend with fixed seed data (see tauri-mock.ts / seed.ts), so the
// output is reproducible. Each "scene" is captured as a PNG under
// app/media/screens/ (the artifacts an agent can view and a reviewer can see in
// the PR), and the whole walkthrough is also recorded to a .webm.
//
// Run it with `npm --prefix app run render`.

import { test, chromium, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { ensureChrome } from "./browser";
import { seed } from "./seed";
import { installTauriMock } from "./tauri-mock";

const here = path.dirname(fileURLToPath(import.meta.url));
const MEDIA = path.resolve(here, "../media");
const SCREENS = path.join(MEDIA, "screens");
const VIDEO_TMP = path.resolve(here, "../test-results/video");
const BASE_URL = process.env.PW_BASE_URL ?? "http://localhost:1420";
const SIZE = { width: 1280, height: 800 };
const BEAT = 600; // a pause between actions so the video is watchable

async function shot(page: Page, name: string): Promise<void> {
  await page.screenshot({ path: path.join(SCREENS, `${name}.png`) });
}

test("render desktop UI states", async () => {
  await mkdir(SCREENS, { recursive: true });

  const browser = await chromium.launch({ executablePath: await ensureChrome() });
  const context = await browser.newContext({
    viewport: SIZE,
    deviceScaleFactor: 2, // crisp screenshots
    recordVideo: { dir: VIDEO_TMP, size: SIZE },
  });
  await installTauriMock(context, seed);
  const page = await context.newPage();

  await page.goto(BASE_URL);
  await page.getByRole("button", { name: /Providers/ }).waitFor();
  await page.waitForTimeout(BEAT);
  await shot(page, "01-overview");

  // Providers: open a definition.
  await page.getByRole("button", { name: "anthropic" }).click();
  await page.waitForTimeout(BEAT);
  await shot(page, "02-provider");

  // Models: filter the list, then open a result.
  await page.getByRole("button", { name: /Models/ }).click();
  await page.getByLabel("Filter models").fill("gpt");
  await page.waitForTimeout(BEAT);
  await page.getByRole("button", { name: "gpt-4o" }).click();
  await page.waitForTimeout(BEAT);
  await shot(page, "03-model-filtered");

  // Specialists: open one with a richer definition.
  await page.getByRole("button", { name: /Specialists/ }).click();
  await page.getByRole("button", { name: "code-reviewer" }).click();
  await page.waitForTimeout(BEAT);
  await shot(page, "04-specialist");

  // Tools: open one and let the final frame settle.
  await page.getByRole("button", { name: /Tools/ }).click();
  await page.getByRole("button", { name: "web-search" }).click();
  await page.waitForTimeout(BEAT * 2);
  await shot(page, "05-tool");

  // Closing the context finalizes the recording. Save before closing the
  // browser — the Video handle needs the connection to still be open.
  const video = page.video();
  await context.close();
  if (!video) {
    throw new Error("no video was recorded");
  }
  await video.saveAs(path.join(MEDIA, "spawningpool-demo.webm"));
  await browser.close();
});
