// Provisions the Chromium that the render harness drives. This cloud sandbox
// blocks Playwright's own browser CDN, but the Chrome for Testing bucket on
// storage.googleapis.com IS reachable, and @puppeteer/browsers downloads from
// there. So we fetch Chrome once into a gitignored cache and point Playwright at
// it via `executablePath` — no Playwright browser install needed.

import { install, computeExecutablePath, Browser } from "@puppeteer/browsers";
import { existsSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));

// Pinned so every render — local, CI, or sandbox — uses the identical engine.
// Bump deliberately; screenshots will change when you do.
export const CHROME_BUILD = "148.0.7778.96";

// Kept inside app/ (gitignored) so it survives between runs in the same clone.
export const CACHE_DIR = path.resolve(here, "../.browser");

/** Absolute path to the pinned Chrome binary (whether or not it exists yet). */
export function chromeExecutablePath(): string {
  return computeExecutablePath({
    browser: Browser.CHROME,
    buildId: CHROME_BUILD,
    cacheDir: CACHE_DIR,
  });
}

/** Ensure the pinned Chrome is present, downloading it on first use. */
export async function ensureChrome(): Promise<string> {
  const exe = chromeExecutablePath();
  if (existsSync(exe)) {
    return exe;
  }
  await install({
    browser: Browser.CHROME,
    buildId: CHROME_BUILD,
    cacheDir: CACHE_DIR,
  });
  return exe;
}
