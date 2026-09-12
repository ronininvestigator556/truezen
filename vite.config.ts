import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";

export default defineConfig({
  plugins: [svelte()],
  // Tauri expects a fixed port and should fail rather than silently move.
  server: { port: 1420, strictPort: true },
  clearScreen: false,
  build: { target: "es2022", sourcemap: true },
});
