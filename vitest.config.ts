import { defineConfig } from "vitest/config";
import { fileURLToPath } from "node:url";

// Standalone vitest config for the pure-logic test tier. Deliberately does NOT
// wire the SvelteKit plugin: these tests import plain `.ts` modules only. The one
// thing the SvelteKit toolchain gives us that we DO need is the `$lib` path alias,
// which we mirror here by hand.
export default defineConfig({
  resolve: {
    alias: {
      $lib: fileURLToPath(new URL("./src/lib", import.meta.url)),
    },
  },
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },
});
