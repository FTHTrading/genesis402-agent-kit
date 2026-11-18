// Offline test (0.3.13): the specialist seller in tool results, the catalog grouped by specialist, the README section.
// A fake rail runs on loopback (GENESIS402_ORIGIN); nothing touches the network, nothing is signed or paid.
// Run: node specialists-test.mjs
import http from "node:http";
import fs from "node:fs";

let fails = 0;
const ok = (c, m, extra) => { console.log((c ? "PASS " : "FAIL ") + m + (!c && extra !== undefined ? "  " + JSON.stringify(extra).slice(0, 400) : "")); if (!c) fails++; };

const TREASURY = "0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB";
const REGISTRY = "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432";
let minted = false;   // the prices specialist has an id once minted
const sellerFor = (origin) => ({ agent: "prices", name: "Genesis402 Prices", agent_id: minted ? 1300001 : 95721, specialist: "prices", collector: 95721, registry: "eip155:8453:" + REGISTRY, registry_address: REGISTRY, chain: "base", network: "eip155:8453", wallet: TREASURY, pay_to: TREASURY, card: "https://twin.unykorn.org/agents/prices/registration.json", receipts: "https://twin.unykorn.org/receipts?agent=" + (minted ? 1300001 : "prices"), storefront: "https://genesis402.com" });
const SERVICES = (origin) => [
  { name: "price-bitcoin", endpoint: origin + "/price/bitcoin", price: { usd: 0.003 }, title: "Bitcoin price", tags: ["price"], specialist: "prices" },
  { name: "price-ethereum", endpoint: origin + "/price/ethereum", price: { usd: 0.003 }, title: "Ethereum price", tags: ["price"], specialist: "prices" },
  { name: "evm-block-number", endpoint: origin + "/evm-block-number", price: { usd: 0.003 }, title: "Latest block", tags: ["evm"], specialist: "chain-reader" },
  { name: "hash", endpoint: origin + "/hash", price: { usd: 0.003 }, title: "Hashes", tags: [], specialist: "utilities" },
  { name: "examiner-dossier", endpoint: origin + "/agents/evidence-01/dossier", price: { usd: 0.75 }, title: "Dossier", tags: [], seller: { agent: "evidence-01", agent_id: 96494, registry: "eip155:8453:" + REGISTRY, pay_to: "0x117459d013492eC5fF90387B16764e753C506ec9" } },
  { name: "legacy-route", endpoint: origin + "/legacy", price: { usd: 0.01 }, title: "A route with no specialist", tags: [] }
];
const srv = http.createServer((q, r) => {
  let b = ""; q.on("data", (c) => { b += c; });
  q.on("end", () => {
    const send = (s, o, h) => { r.writeHead(s, { "content-type": "application/json", ...(h || {}) }); r.end(JSON.stringify(o)); };
    const origin = "http://127.0.0.1:" + srv.address().port;
    if (q.url === "/.well-known/x402") return send(200, { x402Version: 2, services: SERVICES(origin), seller: { agent: "genesis402", agent_id: 95721, registry: "eip155:8453:" + REGISTRY, registry_address: REGISTRY, chain: "base", pay_to: TREASURY } });
    if (q.url === "/__validate") return send(200, { checked: false });
    if (q.url === "/price/bitcoin") {
      if (q.headers["payment-signature"]) return send(200, { usd: 1 }, { "payment-response": Buffer.from(JSON.stringify({ success: true, transaction: "0xabc", network: "eip155:8453" })).toString("base64") });
      const body = { x402Version: 2, error: "Payment required", resource: { url: origin + "/price/bitcoin", description: "x" }, accepts: [{ scheme: "exact", network: "eip155:8453", asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", amount: "3000", payTo: TREASURY }], seller: sellerFor(origin) };
      return send(402, body, { "payment-required": Buffer.from(JSON.stringify(body)).toString("base64") });
    }
    if (q.url === "/hash") return send(200, { sha256: "x" });   // answers without payment
    if (q.url === "/legacy") return send(200, { ok: true });
    if (q.url === "/receipts/rcptS") return send(200, { receiptId: "rcptS", amount_usd: 0.003, seller_specialist: "prices", collector_agent_id: 95721, ...(minted ? { seller_agent_id: 1300001 } : {}) });
    if (q.url === "/receipts/rcptD") return send(200, { receiptId: "rcptD", amount_usd: 0.75, seller_agent_id: 96494, seller_wallet: "0x117459d013492eC5fF90387B16764e753C506ec9", seller_registry: "eip155:8453:" + REGISTRY });
    send(404, { error: "not_found" });
  });
});
await new Promise((r) => srv.listen(0, "127.0.0.1", r));
process.env.GENESIS402_ORIGIN = "http://127.0.0.1:" + srv.address().port;
const { createServer, VERSION, loadCatalog } = await import("./core.mjs");
const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = await import("@modelcontextprotocol/sdk/inMemory.js");

async function client() {
  await loadCatalog(true);
  const server = await createServer({ payer: null, hosted: false });
  const [a, b] = InMemoryTransport.createLinkedPair();
  const c = new Client({ name: "specialists-test", version: "1" });
  await Promise.all([server.connect(a), c.connect(b)]);
  return c;
}
const call = async (c, name, args) => { const r = await c.callTool({ name, arguments: args }); return { r, j: r.structuredContent || null }; };

try {
  ok(VERSION === "0.3.13", "version is 0.3.13", VERSION);
  let c = await client();
  // quote: the 402's specialist block passes through unchanged
  let { r, j } = await call(c, "genesis402_call", { endpoint: "price-bitcoin", params: {} });
  ok(!r.isError && j.mode === "QUOTE_ONLY" && j.seller.specialist === "prices" && j.seller.collector === 95721 && j.seller.agent_id === 95721 && j.seller.source === "rail" && j.seller.pay_to === TREASURY && j.price.payTo === TREASURY, "quote: the specialist seller from the 402 is kept (agent_id 95721 until minted), payTo untouched", j && j.seller);
  ok(j.receipts_url === "https://twin.unykorn.org/receipts?agent=prices", "quote: the receipts link is the specialist's feed", j.receipts_url);
  // paid with a caller-signed payment: no 402 in hand, the manifest names the specialist
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "price-bitcoin", params: {}, payment_signature: "sig" }));
  ok(j.mode === "PAID" && j.seller.specialist === "prices" && j.seller.collector === 95721 && j.seller.source === "rail" && /agents\/prices\/registration\.json$/.test(j.seller.card) && /receipts\?agent=prices$/.test(j.receipts_url), "paid (caller-signed): the seller is the specialist the manifest names", j.seller);
  // free answer
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "hash", params: {} }));
  ok(j.mode === "FREE" && j.seller.specialist === "utilities" && j.seller.collector === 95721, "free answer: names the utilities specialist", j.seller);
  // a route with no specialist and a desk route keep what they had
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "legacy-route", params: {} }));
  ok(j.seller && j.seller.specialist === undefined && j.seller.agent_id === 95721, "a route with no specialist: the rail seller, no specialist field", j && j.seller);
  // receipts
  ({ r, j } = await call(c, "genesis402_receipt", { receipt_id: "rcptS" }));
  ok(j.seller.specialist === "prices" && j.seller.collector === 95721 && j.seller.agent_id === 95721 && /agent=prices$/.test(j.receipts_url), "receipt of a specialist sale before minting: names the specialist and the collector", j.seller);
  ({ r, j } = await call(c, "genesis402_receipt", { receipt_id: "rcptD" }));
  ok(j.seller.agent_id === 96494 && j.seller.specialist === undefined, "receipt of a desk sale: the desk seller, unchanged", j.seller);
  // catalog grouped by specialist
  ({ r, j } = await call(c, "genesis402_catalog", {}));
  ok(j.total_endpoints === 6 && j.by_specialist && j.by_specialist.prices === 2 && j.by_specialist["chain-reader"] === 1 && j.by_specialist.utilities === 1 && j.by_specialist.none === 2, "catalog: counts per specialist (prices 2, chain-reader 1, utilities 1, none 2: a desk route and a route with no specialist)", j.by_specialist);
  ok(j.endpoints.find((e) => e.name === "price-bitcoin").specialist === "prices" && j.endpoints.find((e) => e.name === "legacy-route").specialist === undefined, "catalog: each row carries its specialist");
  ok(j.endpoints.map((e) => e.specialist || "~").join() === [...j.endpoints.map((e) => e.specialist || "~")].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0)).join(), "catalog: rows are grouped by specialist (sorted by it, routes without one last-sorted by name)");
  ({ r, j } = await call(c, "genesis402_catalog", { specialist: "prices" }));
  ok(j.matched === 2 && j.endpoints.every((e) => e.specialist === "prices"), "catalog: the specialist filter returns only its routes", j.matched);
  ({ r, j } = await call(c, "genesis402_catalog", { specialist: "nosuch" }));
  ok(j.matched === 0 && j.endpoints.length === 0, "catalog: an unknown specialist matches nothing");
  const instr = c.getInstructions() || "";
  ok(/specialist/i.test(instr) && /collector/i.test(instr) && /95721/.test(instr), "instructions explain specialists and the collector (95721)");

  // after minting: the specialist's own id is the agent_id, the collector stays 95721
  minted = true; c = await client();
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "price-bitcoin", params: {} }));
  ok(j.seller.agent_id === 1300001 && j.seller.specialist === "prices" && j.seller.collector === 95721, "after minting: seller agent_id is the specialist's id, collector 95721", j.seller);
  ({ r, j } = await call(c, "genesis402_receipt", { receipt_id: "rcptS" }));
  ok(j.seller.agent_id === 1300001 && j.seller.collector === 95721 && j.seller.specialist === "prices", "after minting: the receipt names the specialist id and the collector", j.seller);

  // docs
  const readme = fs.readFileSync("./README.md", "utf8");
  ok(/## How this rail uses ERC-8004/.test(readme) && /collector/i.test(readme) && /specialist/i.test(readme) && /validation/i.test(readme) && /receipt/i.test(readme), "README has the section 'How this rail uses ERC-8004' (collector, specialists, validation, receipts)");
  ok(!/zero-know|insur|guarante|licen[cs]ed|custod|bitgo|fireblocks|falconx/i.test(readme.split("## How this rail uses ERC-8004")[1] || "x-forbidden"), "that section carries no forbidden wording");
  const pkg = JSON.parse(fs.readFileSync("./package.json", "utf8")); const sj = JSON.parse(fs.readFileSync("./server.json", "utf8"));
  ok(pkg.version === "0.3.13" && sj.version === "0.3.13" && sj.packages.every((p) => p.version === "0.3.13") && /0\.3\.13/.test(fs.readFileSync("./CHANGELOG.md", "utf8").split("\n").slice(0, 8).join("\n")), "package.json, server.json and CHANGELOG carry 0.3.13");
  ok(/specialists-test\.mjs/.test(pkg.scripts.test), "npm test runs this test");
} catch (e) { console.error("SPECIALISTS TEST ERROR", e); fails++; }
srv.close();
console.log(fails ? "FAILED " + fails : "ALL PASSED");
process.exit(fails ? 1 : 0);
