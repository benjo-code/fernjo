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
    // Never inline assets as data: URIs: the CSP (src-tauri/tauri.conf.json) only allows fonts from 'self'.
    assetsInlineLimit: 0,
    // The terminal (xterm.js and its addons) lives in the main chunk on purpose: the bundle is loaded from disk,
    // so lazy loading gains nothing (Docs/DECISIONS.md, "M4 terminal"). About 632 kB is the known size.
    chunkSizeWarningLimit: 700,
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG === "true" ? false : "oxc",
    sourcemap: process.env.TAURI_ENV_DEBUG === "true",
  },
  // Vitest needs the browser build of Svelte; elsewhere the Vite defaults must stay (a user value replaces them).
  resolve: process.env.VITEST ? { conditions: ["browser"] } : undefined,
  test: {
    environment: "jsdom",
    // Vitest replaces CSS with an empty string by default; tokens.test.ts reads the token files via ?raw.
    css: true,
  },
});
