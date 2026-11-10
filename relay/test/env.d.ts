// The bindings `env` from "cloudflare:test" carries (wrangler.jsonc + vitest.config.ts).
declare namespace Cloudflare {
  interface Env {
    PAIRS: DurableObjectNamespace<import("../src/index").RelayPair>;
  }
}
