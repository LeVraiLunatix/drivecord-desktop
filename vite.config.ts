import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/ — options tailored for Tauri development.
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Prevent Vite from obscuring Rust errors.
  clearScreen: false,
  server: {
    // Tauri expects a fixed port and fails if it is not available.
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Tauri handles rebuilds for `src-tauri` itself.
      ignored: ["**/src-tauri/**"],
    },
  },
}));
