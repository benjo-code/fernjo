import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vitest/config";

export default defineConfig({
  root: "ui",
  plugins: [svelte()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG === "true" ? false : "oxc",
    sourcemap: process.env.TAURI_ENV_DEBUG === "true",
  },
  resolve: {
    conditions: ["browser"],
  },
  test: {
    environment: "jsdom",
  },
});
