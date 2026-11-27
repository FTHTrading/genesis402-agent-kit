#!/usr/bin/env node
// Genesis402 quickstart: connect, make one call, verify a signed receipt.
// Read-only. It signs nothing, pays nothing and needs no wallet, account or API key.
// Node 20 or newer, no dependencies:  node examples/quickstart.mjs
import { createPublicKey, verify } from "node:crypto";

const MCP = process.env.GENESIS402_MCP_URL || "https://twin.unykorn.org/mcp";
const ORIGIN = new URL(MCP).origin;
let rpcId = 0;

async function rpc(method, params) {
  const res = await fetch(MCP, {
    method: "POST",
    headers: { "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: ++rpcId, method, ...(params ? { params } : {}) }),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method}: HTTP ${res.status} ${text.slice(0, 200)}`);
  // The server may answer as plain JSON or as one server-sent event.
  const body = text.trimStart().startsWith("{")
    ? text
    : text.split(/\r?\n/).filter((l) => l.startsWith("data:")).map((l) => l.slice(5)).join("");
  const msg = JSON.parse(body);
  if (msg.error) throw new Error(`${method}: ${JSON.stringify(msg.error)}`);
  return msg.result;
}

async function getJson(path) {
  const res = await fetch(ORIGIN + path, { headers: { accept: "application/json" } });
  if (!res.ok) throw new Error(`GET ${path}: HTTP ${res.status}`);
  return res.json();
}

// RFC 8785 (JCS) canonical JSON for the value types a receipt uses: sorted keys, no whitespace.
function canonical(v) {
  if (v === null || typeof v !== "object") return JSON.stringify(v);
  if (Array.isArray(v)) return "[" + v.map(canonical).join(",") + "]";
  return "{" + Object.keys(v).sort().map((k) => JSON.stringify(k) + ":" + canonical(v[k])).join(",") + "}";
}

function receiptSignatureIsValid(receipt, keys) {
  const { signature, ...unsigned } = receipt;
  const key = keys.find((k) => k.keyId === signature.key_id);
  if (!key || signature.alg !== "ed25519") return false;
  const message = Buffer.from(signature.domain + "\n" + canonical(unsigned));
  return verify(null, message, createPublicKey(key.publicKeyPem), Buffer.from(signature.value, "hex"));
}

// Step 1: connect to the hosted MCP endpoint.
const init = await rpc("initialize", {
  protocolVersion: "2025-03-26",
  capabilities: {},
  clientInfo: { name: "genesis402-quickstart", version: "1" },
});
const { tools } = await rpc("tools/list");
console.log(`1. Connected to ${MCP}`);
console.log(`   server ${init.serverInfo.name} ${init.serverInfo.version}, ${tools.length} tools`);

// Step 2: one free call, then one price quote. A paid tool called without a payment charges nothing.
const catalogResult = await rpc("tools/call", { name: "genesis402_catalog", arguments: { filter: "gas" } });
const catalog = catalogResult.structuredContent ?? JSON.parse(catalogResult.content[0].text);
const status = await getJson("/status");
const first = catalog.endpoints[0];
console.log(`2. genesis402_catalog (free): ${catalog.matched} endpoints match "gas"; first: ${first.name} at $${first.price_usd}`);
console.log(`   paid routes right now, from ${ORIGIN}/status: ${status.endpoints.payable} payable of ${status.endpoints.advertised} advertised`);
const quoteResult = await rpc("tools/call", { name: "genesis402_whois", arguments: { domain: "example.com" } });
const quote = quoteResult.structuredContent ?? JSON.parse(quoteResult.content[0].text);
if (quote.mode !== "QUOTE_ONLY" || quote.paid !== false) {
  throw new Error("expected a price quote, got: " + JSON.stringify(quote).slice(0, 200));
}
console.log(`   genesis402_whois without a payment: mode ${quote.mode}, paid ${quote.paid}, price $${quote.price.usd} USDC on ${quote.price.network}`);
console.log("   Nothing was charged. Paying means signing that quote with your own wallet.");

// Step 3: verify a signed receipt yourself, with the rail's published public key.
const { receipt } = await getJson("/receipts/sample");
const keys = await getJson("/prove/keys");
const valid = receiptSignatureIsValid(receipt, keys);
const tamperedRejected = !receiptSignatureIsValid({ ...receipt, amount_usd: receipt.amount_usd + 1 }, keys);
console.log(`3. Sample receipt ${receipt.receipt_id} (${receipt.task}, ${receipt.paid}, tx ${receipt.tx_hash.slice(0, 12)}...)`);
console.log(`   Ed25519 signature, key ${receipt.signature.key_id}: ${valid ? "VALID" : "INVALID"} (checked on this machine)`);
console.log(`   same receipt with the amount changed: ${tamperedRejected ? "REJECTED, as it should be" : "ACCEPTED, which is wrong"}`);
if (!valid || !tamperedRejected) {
  console.error("Receipt verification failed.");
  process.exit(1);
}

console.log("\nDone. Tell us how it went: https://github.com/FTHTrading/genesis402-agent-kit/issues/new?template=feedback.yml");
