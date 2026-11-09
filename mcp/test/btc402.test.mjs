// Offline tests for the btc402 tools (genesis402_btc402_*). Every network call goes to a stub
// installed on globalThis.fetch, so nothing reaches btc402.genesis402.com, Base RPC or the rail,
// and nothing is ever signed or paid.
import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { InMemoryTransport } from "@modelcontextprotocol/sdk/inMemory.js";
import { createServer } from "../core.mjs";

const here = (p) => fileURLToPath(new URL(p, import.meta.url));
const fx = (n) => JSON.parse(readFileSync(here(`./fixtures/${n}`), "utf8"));
const REC = fx("att-v2-live-c469.json");
const KEY = fx("key.json");
const RECEIPT = fx("receipt-c469.json");
const DIGEST = "db03f51b95bd74956f9337a0068de244018fa189c2037070db0c04d5f52515f4";
const GATE = "https://btc402.genesis402.com";
const PAY_TO = "0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB";
const USDC = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const HISTORY = { keys: [{ key_id: KEY.key_id, alg: "Ed25519", public_key_hex: KEY.public_key_hex, valid_from: "2026-11-08T11:10:19Z", valid_to: null }] };

const challenge = (path) => ({
  x402Version: 2,
  error: "PAYMENT-SIGNATURE header is required",
  resource: { url: GATE + path, description: path === "/blockcount" ? "Current Bitcoin mainnet block height" : "Bitcoin fee estimate", mimeType: "application/json" },
  accepts: [{ scheme: "exact", network: "eip155:8453", amount: "3000", asset: USDC, payTo: PAY_TO, maxTimeoutSeconds: 60, extra: { name: "USD Coin", version: "2" } }]
});
const b64 = (o) => Buffer.from(JSON.stringify(o)).toString("base64");
const json = (status, body, headers = {}) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json", ...headers } });
const V1_BODY = { x402Version: 1, accepts: [{ scheme: "exact", network: "base", maxAmountRequired: "3000", payTo: PAY_TO, asset: USDC }] };

// routes: { "<METHOD> <url>" | "<url>": (req) => Response | throws }. Unmatched URLs fail loudly.
let routes, calls;
const realFetch = globalThis.fetch;
function install(extra = {}) {
  routes = {
    // the rail manifest the server loads at boot: keep it tiny so only catalog/call/receipt register
    "https://twin.unykorn.org/.well-known/x402": () => json(200, { services: [{ name: "health", endpoint: "https://twin.unykorn.org/health", price: 0.001 }] }),
    [GATE + "/blockcount"]: () => json(402, V1_BODY, { "payment-required": b64(challenge("/blockcount")) }),
    [GATE + "/feeestimate"]: () => json(402, V1_BODY, { "payment-required": b64(challenge("/feeestimate")) }),
    [GATE + "/attestations/" + DIGEST]: () => json(200, REC),
    [GATE + "/attestations/key"]: () => json(200, KEY),
    [GATE + "/attestations/keys"]: () => json(200, HISTORY),
    "https://mainnet.base.org": () => json(200, RECEIPT),
    ...extra
  };
  calls = [];
  globalThis.fetch = async (input, init = {}) => {
    const url = String(input instanceof Request ? input.url : input);
    const method = (init.method || (input instanceof Request ? input.method : "GET")).toUpperCase();
    const headers = new Headers(init.headers || (input instanceof Request ? input.headers : undefined));
    calls.push({ url, method, headers });
    const h = routes[`${method} ${url}`] || routes[url];
    if (!h) throw new Error(`unstubbed fetch ${method} ${url}`);
    return h({ url, method, headers });
  };
}
beforeEach(() => install());
afterEach(() => { globalThis.fetch = realFetch; });

async function connect(opts = {}) {
  const server = await createServer(opts);
  const client = new Client({ name: "btc402-test", version: "1" });
  const [a, b] = InMemoryTransport.createLinkedPair();
  await Promise.all([server.connect(a), client.connect(b)]);
  return client;
}
const call = async (client, name, args) => client.callTool({ name, arguments: args });
const body = (res) => JSON.parse(res.content[0].text);
const byName = (checks) => Object.fromEntries(checks.map((c) => [c.name, c]));
const isGate = (u) => { try { return new URL(u).origin === GATE; } catch { return false; } };
const sentPayment = () => calls.some((c) => c.headers.has("payment-signature") || c.headers.has("x-payment"));

// ---------------------------------------------------------------------------
test("the four btc402 tools are listed, each with an object outputSchema", async () => {
  const c = await connect();
  const { tools } = await c.listTools();
  const names = tools.map((t) => t.name);
  for (const n of ["genesis402_btc402_quote", "genesis402_btc402_verify_attestation", "genesis402_btc402_blockcount", "genesis402_btc402_feeestimate"]) {
    assert.ok(names.includes(n), `${n} missing from ${names.join(", ")}`);
    const t = tools.find((x) => x.name === n);
    assert.equal(t.outputSchema?.type, "object", `${n} has no object outputSchema`);
    assert.ok(n.length + "kevanbtc-genesis402-mcp_".length <= 64, `${n} too long once Smithery prefixes it`);
  }
  const free = tools.filter((t) => ["genesis402_btc402_quote", "genesis402_btc402_verify_attestation"].includes(t.name));
  for (const t of free) assert.equal(t.annotations?.readOnlyHint, true, `${t.name} should be read-only`);
});

// ---------------------------------------------------------------------------
// genesis402_btc402_quote
test("quote: decodes the live v2 PAYMENT-REQUIRED header for /blockcount", async () => {
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, undefined, JSON.stringify(res));
  const q = res.structuredContent;
  assert.deepEqual(q.price, { atomic: "3000", usd: 0.003, network: "eip155:8453", asset: USDC, payTo: PAY_TO });
  assert.equal(q.resource, GATE + "/blockcount");
  assert.equal(q.route, "/blockcount");
  assert.equal(q.x402_version, 2);
  assert.equal(q.scheme, "exact");
  assert.equal(typeof q.payment_required_b64, "string");
  assert.deepEqual(body(res), q);
  assert.equal(sentPayment(), false, "a quote must never send a payment header");
  assert.ok(calls.some((x) => x.url === GATE + "/blockcount" && x.method === "GET"));
});

test("quote: works for /feeestimate", async () => {
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/feeestimate" });
  assert.equal(res.structuredContent.resource, GATE + "/feeestimate");
  assert.equal(res.structuredContent.price.atomic, "3000");
});

test("quote: a route outside the allow-list is refused before any fetch", async () => {
  const c = await connect();
  const before = calls.length;
  const res = await call(c, "genesis402_btc402_quote", { route: "/lightning" });
  assert.equal(res.isError, true);
  assert.match(res.content[0].text, /invalid/i, "must be an input-validation refusal, not a missing tool");
  assert.equal(calls.slice(before).some((x) => isGate(x.url)), false);
});

test("quote: a malformed PAYMENT-REQUIRED header is an error, never a quote from the v1 body", async () => {
  install({ [GATE + "/blockcount"]: () => json(402, V1_BODY, { "payment-required": "%%%not-base64-json%%%" }) });
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "malformed_payment_required");
});

test("quote: a 402 without the v2 header is an error", async () => {
  install({ [GATE + "/blockcount"]: () => json(402, V1_BODY) });
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "malformed_payment_required");
});

test("quote: a v2 header with no Base USDC lane is an error", async () => {
  const ch = challenge("/blockcount"); ch.accepts[0].network = "eip155:1";
  install({ [GATE + "/blockcount"]: () => json(402, V1_BODY, { "payment-required": b64(ch) }) });
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "no_base_usdc_lane");
});

test("quote: gate down (network error) is reported as gate_unreachable", async () => {
  install({ [GATE + "/blockcount"]: () => { throw new TypeError("fetch failed"); } });
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "gate_unreachable");
});

test("quote: gate answering 503 instead of 402 is an error with the status", async () => {
  install({ [GATE + "/blockcount"]: () => json(503, { error: "down" }) });
  const c = await connect();
  const res = await call(c, "genesis402_btc402_quote", { route: "/blockcount" });
  assert.equal(res.isError, true);
  assert.equal(body(res).status, 503);
});

// ---------------------------------------------------------------------------
// genesis402_btc402_verify_attestation
test("verify: the live c469 record verifies: every check PASS, verdict VERIFIED", async () => {
  const c = await connect();
  const res = await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST });
  assert.equal(res.isError, undefined, JSON.stringify(res));
  const v = res.structuredContent;
  assert.equal(v.ok, true, JSON.stringify(v.checks));
  assert.equal(v.verdict, "VERIFIED");
  assert.equal(v.digest, DIGEST);
  const k = byName(v.checks);
  for (const n of ["requested_digest", "has_ed25519", "digest", "key_matches_published", "signature", "tx_success", "payer", "pay_to", "amount"]) {
    assert.equal(k[n]?.result, "PASS", `${n}: ${JSON.stringify(k[n])}`);
  }
  assert.ok(calls.some((x) => x.url === GATE + "/attestations/" + DIGEST));
});

test("verify: an uppercase digest is accepted and normalised", async () => {
  const c = await connect();
  const res = await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST.toUpperCase() });
  assert.equal(res.structuredContent.ok, true, JSON.stringify(res.structuredContent?.checks));
});

test("verify: a digest that is not 64 hex characters is refused before any fetch", async () => {
  const c = await connect();
  for (const bad of ["nothex", DIGEST.slice(0, 63), DIGEST + "0", "../attestations/key", "z".repeat(64)]) {
    const before = calls.length;
    const res = await call(c, "genesis402_btc402_verify_attestation", { digest: bad });
    assert.equal(res.isError, true, `accepted ${bad}`);
    assert.match(res.content[0].text, /invalid/i, `not an input-validation refusal for ${bad}`);
    assert.equal(calls.slice(before).some((x) => isGate(x.url)), false, `fetched for ${bad}`);
  }
});

test("verify: unknown digest (404) is NOT VERIFIED with attestation UNAVAILABLE, and no PASS at all", async () => {
  const other = "a".repeat(64);
  install({ [GATE + "/attestations/" + other]: () => json(404, { error: "attestation not found" }) });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: other })).structuredContent;
  assert.equal(v.ok, false);
  assert.equal(v.verdict, "NOT VERIFIED");
  assert.equal(byName(v.checks).attestation.result, "UNAVAILABLE");
  assert.equal(v.checks.some((x) => x.result === "PASS"), false);
});

test("verify: gate down gives UNAVAILABLE, never PASS", async () => {
  install({ [GATE + "/attestations/" + DIGEST]: () => { throw new TypeError("fetch failed"); } });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST })).structuredContent;
  assert.equal(v.ok, false);
  assert.equal(v.verdict, "NOT VERIFIED");
  assert.equal(byName(v.checks).attestation.result, "UNAVAILABLE");
  assert.equal(v.checks.some((x) => x.result === "PASS"), false);
});

test("verify: a tampered record fails the digest check", async () => {
  const t = structuredClone(REC); t.execution.bitcoin_block_tip = 1;
  install({ [GATE + "/attestations/" + DIGEST]: () => json(200, t) });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST })).structuredContent;
  assert.equal(v.ok, false);
  assert.equal(byName(v.checks).digest.result, "FAIL");
});

test("verify: the server returning a different record than requested fails requested_digest", async () => {
  const other = "b".repeat(64);
  install({ [GATE + "/attestations/" + other]: () => json(200, REC) });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: other })).structuredContent;
  assert.equal(v.ok, false);
  assert.equal(byName(v.checks).requested_digest.result, "FAIL");
});

test("verify: published key unreachable makes the key and signature checks UNAVAILABLE, verdict NOT VERIFIED", async () => {
  install({ [GATE + "/attestations/key"]: () => { throw new TypeError("fetch failed"); } });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST })).structuredContent;
  const k = byName(v.checks);
  assert.equal(v.ok, false);
  assert.equal(k.key_matches_published.result, "UNAVAILABLE");
  assert.equal(k.signature.result, "UNAVAILABLE");
});

test("verify: Base RPC down makes the settlement checks UNAVAILABLE, verdict NOT VERIFIED", async () => {
  install({ "https://mainnet.base.org": () => { throw new TypeError("fetch failed"); } });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: DIGEST })).structuredContent;
  const k = byName(v.checks);
  assert.equal(v.ok, false);
  for (const n of ["tx_success", "payer", "pay_to", "amount"]) assert.equal(k[n].result, "UNAVAILABLE", n);
  assert.equal(k.signature.result, "PASS", "the offline signature check still runs");
});

test("verify: a pre-v2 (HMAC-only) record is NOT VERIFIED", async () => {
  const v1 = fx("att-v1.json");
  const d = String(v1.digest).toLowerCase();
  install({ [GATE + "/attestations/" + d]: () => json(200, v1) });
  const c = await connect();
  const v = (await call(c, "genesis402_btc402_verify_attestation", { digest: d })).structuredContent;
  assert.equal(v.ok, false);
  assert.equal(byName(v.checks).has_ed25519.result, "FAIL");
});

// ---------------------------------------------------------------------------
// Paid tools: the same model as every other paid genesis402_* tool.
test("paid: without payment it returns QUOTE_ONLY and sends no payment", async () => {
  const c = await connect({ hosted: true });
  const res = await call(c, "genesis402_btc402_blockcount", {});
  assert.equal(res.isError, undefined, JSON.stringify(res));
  const r = res.structuredContent;
  assert.equal(r.mode, "QUOTE_ONLY");
  assert.equal(r.paid, false);
  assert.deepEqual(r.price, { atomic: "3000", usd: 0.003, payTo: PAY_TO, network: "eip155:8453" });
  assert.equal(r.resource, GATE + "/blockcount");
  assert.equal(typeof r.payment_required_b64, "string");
  assert.equal(sentPayment(), false);
});

test("paid: a caller-signed payment_signature is forwarded as PAYMENT-SIGNATURE on a GET and the result is returned", async () => {
  const settlement = { success: true, transaction: "0xabc", network: "eip155:8453", payer: "0x710CbD5b3Ee298Bb3e1FA9a231239EDe615A7ab9" };
  let seen = null;
  install({ [GATE + "/feeestimate"]: (req) => {
    if (!req.headers.get("payment-signature")) return json(402, V1_BODY, { "payment-required": b64(challenge("/feeestimate")) });
    seen = req;
    return json(200, { result: { feerate: 0.0001, blocks: 2 }, attestation: { digest: DIGEST } }, { "payment-response": b64(settlement) });
  } });
  const c = await connect({ hosted: true });
  const res = await call(c, "genesis402_btc402_feeestimate", { payment_signature: "SIGNED-BY-CALLER" });
  assert.equal(res.isError, undefined, JSON.stringify(res));
  assert.equal(seen.method, "GET");
  assert.equal(seen.headers.get("payment-signature"), "SIGNED-BY-CALLER");
  assert.equal(seen.headers.get("x-payment"), null, "a v2 payment is not also sent as a v1 X-PAYMENT");
  const r = res.structuredContent;
  assert.equal(r.mode, "PAID");
  assert.equal(r.paid, true);
  assert.equal(r.result.attestation.digest, DIGEST);
  assert.equal(r.settlement.transaction, "0xabc");
});

test("paid: a payment the gate rejects comes back as an error with paid=false", async () => {
  install({ [GATE + "/blockcount"]: () => json(402, { error: "invalid payment" }, { "payment-required": b64(challenge("/blockcount")) }) });
  const c = await connect({ hosted: true });
  const res = await call(c, "genesis402_btc402_blockcount", { payment_signature: "BAD" });
  assert.equal(res.isError, true);
  assert.equal(body(res).paid, false);
  assert.equal(body(res).status, 402);
});

test("paid: gate down is an error, nothing paid", async () => {
  install({ [GATE + "/blockcount"]: () => { throw new TypeError("fetch failed"); } });
  const c = await connect({ hosted: true });
  const res = await call(c, "genesis402_btc402_blockcount", {});
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "gate_unreachable");
});

test("paid: a local payer refuses a quote above its cap and never calls its paid fetch", async () => {
  let used = false;
  const payer = { maxUsd: 0.001, paidFetch: async () => { used = true; throw new Error("must not pay"); } };
  const c = await connect({ payer });
  const res = await call(c, "genesis402_btc402_blockcount", {});
  assert.equal(res.isError, true);
  assert.equal(body(res).error, "price_above_cap");
  assert.equal(used, false);
});

test("paid: a local payer within its cap pays through its own paid fetch with GET", async () => {
  let got = null;
  const payer = { maxUsd: 0.25, paidFetch: async (url, init) => { got = { url, init }; return json(200, { result: 975094 }); } };
  const c = await connect({ payer });
  const res = await call(c, "genesis402_btc402_blockcount", {});
  assert.equal(res.isError, undefined, JSON.stringify(res));
  assert.equal(got.url, GATE + "/blockcount");
  assert.equal((got.init?.method || "GET").toUpperCase(), "GET");
  assert.equal(res.structuredContent.mode, "PAID");
  assert.equal(res.structuredContent.price.atomic, "3000");
});

// ---------------------------------------------------------------------------
// The verifier is the public MIT btc402-verify code, vendored unchanged.
test("vendored btc402-verify files are byte-identical to the pinned upstream commit", () => {
  const manifest = JSON.parse(readFileSync(here("../vendor/btc402-verify/SOURCE.json"), "utf8"));
  assert.equal(manifest.repository, "https://github.com/FTHTrading/btc402-verify");
  assert.match(manifest.commit, /^[0-9a-f]{40}$/);
  const files = Object.entries(manifest.sha256);
  assert.ok(files.length >= 7);
  for (const [f, want] of files) {
    const got = createHash("sha256").update(readFileSync(here(`../vendor/btc402-verify/${f}`), "utf8").replace(/\r\n/g, "\n")).digest("hex");
    assert.equal(got, want, `${f} differs from upstream ${manifest.commit}`);
  }
});
