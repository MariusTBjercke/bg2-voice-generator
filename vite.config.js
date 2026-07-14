import path from "node:path";
import { fileURLToPath } from "node:url";
import { sveltekit } from "@sveltejs/kit/vite";
import { defineConfig } from "vite";

const root = path.dirname(fileURLToPath(import.meta.url));
const host = process.env.TAURI_DEV_HOST;
const e2eMock = process.env.VITE_E2E_MOCK === "1";

// Vite options tailored for Tauri development.
export default defineConfig(() => ({
  plugins: [sveltekit()],

  ...(e2eMock
    ? {
        resolve: {
          alias: {
            "@tauri-apps/plugin-dialog": path.resolve(root, "e2e/stubs/plugin-dialog.ts"),
            "@tauri-apps/plugin-opener": path.resolve(root, "e2e/stubs/plugin-opener.ts"),
            "@e2e": path.resolve(root, "e2e"),
          },
        },
      }
    : {}),

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : { port: 1421 },
    watch: {
      // Tell Vite to ignore watching `src-tauri`.
      ignored: ["**/src-tauri/**"],
    },
  },
}));
