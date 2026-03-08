import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST;

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  optimizeDeps: {
    entries: ["index.html"],
  },
  envPrefix: ["VITE_", "TAURI_"],
  server: {
    host: host || "127.0.0.1",
    port: 1420,
    strictPort: true,
  },
  preview: {
    host: host || "127.0.0.1",
    port: 4173,
    strictPort: true,
  },
  build: {
    target: process.env.TAURI_ENV_PLATFORM === "windows" ? "chrome105" : "safari13",
    minify: process.env.TAURI_ENV_DEBUG ? false : "esbuild",
    sourcemap: !!process.env.TAURI_ENV_DEBUG,
  },
});
