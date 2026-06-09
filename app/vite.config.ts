import { defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    // Unit tests live in src/. The Playwright video render under e2e/ is a
    // separate suite (`npm run render-video`) and must not be picked up here.
    include: ["src/**/*.{test,spec}.{ts,js}"],
    environment: "jsdom",
    globals: true,
    setupFiles: ["./src/test-setup.ts"],
  },
});
