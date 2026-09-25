#!/usr/bin/env node
// Smoke test for genesis402-mcp against the live rail. Read-only: never sets
// GENESIS402_LIVE, so nothing is ever signed or paid.
const ORIGIN = (process.env.GENESIS402_ORIGIN || "https://twin.unykorn.org").replace(/\/$/, "");
let pass = 0, fail = 0;
const ok = (name, cond, detail) => {
  if (cond) { pass++; console.log(`  PASS  ${name}`); }
  else { fail++; console.log(`  FAIL  ${name}${detail ? " — " + detail : ""}`); }
};

console.log(`genesis402-mcp smoke — ${ORIGIN}\n`);

// 1. Manifest loads and is non-trivial.
const manifest = await (await fetch(ORIGIN + "/.well-known/x402")).json();
const services = manifest.services || [];
ok("manifest loads", services.length > 0, `got ${services.length}`);
ok("manifest has the full catalog", services.length >= 170, `got ${services.length}`);

// 2. Every service carries a resolvable USD price. This is the check that would
//    have caught the llm price going 10x stale in a hardcoded description.
const usd = (s) => {
  const p = s.price;
  if (p == null) return null;
  if (typeof p === "string" || typeof p === "number") return Number(p);
  const v = p.usd ?? p.amount ?? p.value;
  return v == null ? null : Number(v);
};
const unpriced = services.filter((s) => usd(s) == null || Number.isNaN(usd(s)));
ok("every endpoint has a numeric price", unpriced.length === 0, `${unpriced.length} unpriced`);

// 3. Prices sit in the expected band.
const prices = services.map(usd);
ok("prices within $0.001–$0.25", Math.min(...prices) >= 0.001 && Math.max(...prices) <= 0.25,
  `min ${Math.min(...prices)} max ${Math.max(...prices)}`);

// 4. The endpoints the named tools advertise all exist.
const byName = new Set(services.map((s) => s.name || String(s.endpoint || "").split("/").pop()));
const advertised = ["evm-multi-chain-scan", "screen-sanctions", "token-concentration", "contract-verified",
  "address-activity", "risk", "prove", "genesis-sim", "wallet-ops", "rwa-screen", "llm"];
const missing = advertised.filter((n) => !byName.has(n));
ok("all named tools map to live endpoints", missing.length === 0, `missing: ${missing.join(", ")}`);

// 5. A paid endpoint answers 402 with a Base USDC lane — the quote path works.
const probe = await fetch(ORIGIN + "/evm-multi-chain-scan", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ params: { address: "0x0000000000000000000000000000000000000000" } }),
});
ok("paid endpoint challenges with 402", probe.status === 402, `got ${probe.status}`);
const body = await probe.json().catch(() => null);
const hdr = probe.headers.get("payment-required");
let challenge = null;
if (hdr) { try { challenge = JSON.parse(Buffer.from(hdr, "base64").toString("utf8")); } catch {} }
challenge = challenge || body;
const lane = (challenge?.accepts || []).find(
  (a) => a.network === "eip155:8453" &&
    String(a.asset).toLowerCase() === "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913");
ok("402 offers a Base USDC lane", !!lane);
ok("quoted price matches the manifest", lane
  ? Math.abs(Number(lane.amount ?? lane.maxAmountRequired) / 1e6 - usd(services.find((s) => s.name === "evm-multi-chain-scan"))) < 1e-9
  : false);

// 6. Free endpoints stay free.
const health = await fetch(ORIGIN + "/health");
ok("/health is free", health.status === 200, `got ${health.status}`);

// 7. An unpaid request is answered with 402, NOT 400, even for malformed input.
//    This is deliberate on the rail's side: discovery crawlers probe with no
//    params, and a 400 would make the endpoint invisible to them. Validation runs
//    once a payment header is present, so bad input is never actually charged.
const bad = await fetch(ORIGIN + "/evm-multi-chain-scan", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ params: { address: "not-an-address" } }),
});
ok("unpaid probe stays discoverable (402, not 400)", bad.status === 402, `got ${bad.status}`);

// 8. The free validator is what catches bad params, and it does.
const v = await (await fetch(ORIGIN + "/__validate", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ name: "evm-multi-chain-scan", params: { address: "not-an-address" } }),
})).json();
ok("/__validate rejects malformed input for free", v.checked === true && v.valid === false, JSON.stringify(v));
ok("/__validate explains why", typeof v.message === "string" && v.message.length > 10, v.message);

// 9. And accepts good input.
const v2 = await (await fetch(ORIGIN + "/__validate", {
  method: "POST", headers: { "content-type": "application/json" },
  body: JSON.stringify({ name: "evm-multi-chain-scan", params: { address: "0x0000000000000000000000000000000000000000" } }),
})).json();
ok("/__validate accepts well-formed input", v2.valid === true, JSON.stringify(v2));

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
