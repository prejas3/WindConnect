import { cloudflareTest } from "@cloudflare/vitest-plugin";
import { defineConfig } from "vitest/config";

export default defineConfig({
  plugins: [
    cloudflareTest({
      wrangler: { configPath: "./wrangler.jsonc" },
      // Small limits so the tests reach them quickly.
      miniflare: {
        bindings: { RATE_BYTES_PER_SEC: "4096", BURST_BYTES: "8192", AUTH_TIMEOUT_MS: "300" },
      },
    }),
  ],
});
