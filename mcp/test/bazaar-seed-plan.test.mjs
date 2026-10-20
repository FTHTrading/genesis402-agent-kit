import test from "node:test";
import assert from "node:assert/strict";
import { planSeeds, budgetFromEnv, PER_CALL_MAX_USD, RUN_MAX_USD } from "../scripts/bazaar-seed.mjs";

const manifest = { services: [
  { endpoint: "https://twin.unykorn.org/price/bitcoin", price: { usd: 0.003 }, bazaar_listing: true },
  { endpoint: "https://twin.unykorn.org/hash/sha256", price: { usd: 0.001 } },
  { endpoint: "https://twin.unykorn.org/report/counterparty", price: { usd: 0.25 } }
] };

test("accepts cheap manifest paths and totals them", () => {
  const p = planSeeds(manifest, [{ path: "/price/bitcoin", method: "GET" }, { path: "/hash/sha256", body: {} }], 0.10);
  assert.deepEqual(p.items.map((i) => i.ok), [true, true]);
  assert.equal(p.totalUsd, 0.004);
});

test("refuses a price above the per-call cap, an unknown path and a hostile path", () => {
  const p = planSeeds(manifest, [{ path: "/report/counterparty" }, { path: "/nope" }, { path: "/../etc" }, { path: "https://evil.example/x" }], 0.10);
  assert.deepEqual(p.items.map((i) => i.reason), ["price_above_per_call_cap", "not_in_manifest", "path_not_allowed", "path_not_allowed"]);
  assert.equal(p.totalUsd, 0);
});

test("stops at the run budget", () => {
  const seeds = Array.from({ length: 50 }, () => ({ path: "/price/bitcoin", method: "GET" }));
  const p = planSeeds(manifest, seeds, RUN_MAX_USD);
  assert.equal(p.items.filter((i) => i.ok).length, 33);          // 33 x $0.003 = $0.099; a 34th would be $0.102
  assert.ok(p.totalUsd <= RUN_MAX_USD);
  assert.equal(p.items[33].reason, "over_budget");
});

test("rejects methods other than GET and POST", () => {
  assert.equal(planSeeds(manifest, [{ path: "/price/bitcoin", method: "DELETE" }], 0.1).items[0].reason, "method_not_allowed");
});

test("budget env can lower the per-run cap but never raise it", () => {
  assert.equal(budgetFromEnv({}), RUN_MAX_USD);
  assert.equal(budgetFromEnv({ SEED_MAX_USD: "0.02" }), 0.02);
  assert.throws(() => budgetFromEnv({ SEED_MAX_USD: "0.50" }), /ceiling/);
  assert.throws(() => budgetFromEnv({ SEED_MAX_USD: "-1" }), /positive/);
  assert.ok(PER_CALL_MAX_USD <= 0.005);
});
