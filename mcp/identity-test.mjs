// Offline test: seller identity (ERC-8004 agent, registry, receipts link) in the MCP manifest and in every tool result.
// A fake rail runs on loopback (GENESIS402_ORIGIN); nothing touches the network, nothing is signed or paid.
// Run: node identity-test.mjs
import http from "node:http";
import fs from "node:fs";

let fails = 0;
const ok = (c, m, extra) => { console.log((c ? "PASS " : "FAIL ") + m + (!c && extra !== undefined ? "  " + JSON.stringify(extra).slice(0, 300) : "")); if (!c) fails++; };

const TREASURY = "0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB";
const REGISTRY = "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432";
let mode = "rail";       // rail: the 402 carries seller | manifest: only the manifest does | none: neither | desk: a desk seller
const RAIL_SELLER = { agent: "genesis402", name: "Genesis402", agent_id: 95721, registry: "eip155:8453:" + REGISTRY, registry_address: REGISTRY, chain: "base", network: "eip155:8453", pay_to: TREASURY, card: "https://twin.unykorn.org/.well-known/agent-registration.json", receipts: "https://twin.unykorn.org/receipts", storefront: "https://genesis402.com" };
const DESK_SELLER = { agent: "evidence-01", name: "The Examiner", agent_id: 96494, registry: "eip155:8453:" + REGISTRY, wallet: "0x117459d013492eC5fF90387B16764e753C506ec9", pay_to: "0x117459d013492eC5fF90387B16764e753C506ec9", network: "eip155:8453" };

const srv = http.createServer((q, r) => {
  let b = ""; q.on("data", (c) => { b += c; });
  q.on("end", () => {
    const send = (s, o, h) => { r.writeHead(s, { "content-type": "application/json", ...(h || {}) }); r.end(JSON.stringify(o)); };
    const origin = "http://127.0.0.1:" + srv.address().port;
    if (q.url === "/.well-known/x402") return send(200, { x402Version: 2, services: [{ name: "check-address", endpoint: origin + "/check/address", price: { usd: 0.01 }, title: "Check an address", tags: [] }], ...(mode === "manifest" || mode === "rail" ? { seller: RAIL_SELLER } : {}) });
    if (q.url === "/__validate") return send(200, { checked: false });
    if (q.url === "/check/address") {
      if (q.headers["payment-signature"]) return send(200, { verdict: "pass" }, { "payment-response": Buffer.from(JSON.stringify({ success: true, transaction: "0xabc", network: "eip155:8453" })).toString("base64") });
      const accepts = [{ scheme: "exact", network: "eip155:8453", asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", amount: "10000", payTo: mode === "desk" ? DESK_SELLER.pay_to : TREASURY }];
      const body = { x402Version: 2, error: "Payment required", resource: { url: origin + "/check/address", description: "x" }, accepts, ...(mode === "rail" ? { seller: RAIL_SELLER } : mode === "desk" ? { seller: DESK_SELLER } : {}) };
      return send(402, body, { "payment-required": Buffer.from(JSON.stringify(body)).toString("base64") });
    }
    if (q.url === "/receipts/rcpt1") return send(200, { receiptId: "rcpt1", amount_usd: 0.01, seller_agent_id: 95721 });
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
  const c = new Client({ name: "identity-test", version: "1" });
  await Promise.all([server.connect(a), c.connect(b)]);
  return c;
}
const call = async (c, name, args) => { const r = await c.callTool({ name, arguments: args }); return { r, j: r.structuredContent || null }; };
const hasSeller = (t) => t.outputSchema && t.outputSchema.properties && t.outputSchema.properties.seller;

try {
  let c = await client();
  const info = c.getServerVersion();
  ok(info.version === VERSION, "serverInfo version equals VERSION (" + VERSION + ")");
  const instr = c.getInstructions() || "";
  ok(/95721/.test(instr) && instr.includes(REGISTRY) && /receipts/i.test(instr), "server instructions name agent 95721, the registry and the receipts link");
  const pkg = JSON.parse(fs.readFileSync("./package.json", "utf8")); const sj = JSON.parse(fs.readFileSync("./server.json", "utf8"));
  ok(pkg.version === VERSION && sj.version === VERSION && sj.packages.every((p) => p.version === VERSION), "package.json, server.json (top and packages) and core.mjs carry one version", [pkg.version, sj.version, sj.packages.map((p) => p.version), VERSION]);
  const meta = sj._meta && sj._meta["io.modelcontextprotocol.registry/publisher-provided"];
  ok(meta && meta.seller && meta.seller.agent_id === 95721 && meta.seller.registry_address === REGISTRY && /receipts/.test(meta.seller.receipts), "server.json _meta carries the seller identity and the receipts link");
  ok(/95721/.test(sj.description) && /ERC-8004/.test(sj.description) && sj.description.length <= 100, "server.json description names ERC-8004 agent 95721 and fits the registry limit of 100 characters (" + sj.description.length + ")");
  const readme = fs.readFileSync("./README.md", "utf8");
  ok(/95721/.test(readme) && readme.includes(REGISTRY) && /twin\.unykorn\.org\/receipts/.test(readme), "README names agent 95721, the registry and the receipts feed");
  const tools = (await c.listTools()).tools;
  ok(tools.length >= 3 && tools.every(hasSeller), "every tool's outputSchema declares a seller (" + tools.length + " tools)", tools.filter((t) => !hasSeller(t)).map((t) => t.name));

  mode = "rail"; c = await client();
  let { r, j } = await call(c, "genesis402_call", { endpoint: "check-address", params: { address: "0x0000000000000000000000000000000000000000" } });
  ok(!r.isError && j && j.mode === "QUOTE_ONLY" && j.seller && j.seller.agent_id === 95721 && j.seller.registry_address === REGISTRY && j.seller.chain === "base" && j.seller.source === "rail", "quote: seller 95721 from the rail's 402", j && j.seller);
  ok(j.seller.receipts === "https://twin.unykorn.org/receipts" && j.receipts_url === "https://twin.unykorn.org/receipts", "quote: carries the receipts link");
  ok(j.price && j.price.payTo === TREASURY && j.seller.pay_to === TREASURY, "quote: payTo untouched and equal to the seller's pay_to");

  mode = "manifest"; c = await client();
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "check-address", params: {} }));
  ok(j && j.seller && j.seller.agent_id === 95721 && j.seller.source === "rail", "quote: seller read from the rail's manifest when the 402 has none", j && j.seller);

  mode = "none"; c = await client();
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "check-address", params: {} }));
  ok(j && j.seller && j.seller.agent_id === 95721 && j.seller.registry_address === REGISTRY && j.seller.source === "package", "quote: falls back to the package default, labelled source=package", j && j.seller);
  ok(j.price && j.price.payTo === TREASURY, "quote: the fallback never alters payTo");

  mode = "desk"; c = await client();
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "check-address", params: {} }));
  ok(j && j.seller && j.seller.agent_id === 96494 && j.price.payTo === DESK_SELLER.pay_to, "quote: a desk seller named by the 402 is kept (96494), not replaced by 95721", j && j.seller);

  mode = "rail"; c = await client();
  ({ r, j } = await call(c, "genesis402_call", { endpoint: "check-address", params: {}, payment_signature: "sig" }));
  ok(j && j.mode === "PAID" && j.seller && j.seller.agent_id === 95721 && j.receipts_url === "https://twin.unykorn.org/receipts", "paid result: seller and receipts link present", j);

  ({ r, j } = await call(c, "genesis402_receipt", { receipt_id: "rcpt1" }));
  ok(j && j.receipt_id === "rcpt1" && j.seller && j.seller.agent_id === 95721 && j.receipts_url, "receipt lookup: result names the seller and the receipts feed", j);

  ({ r, j } = await call(c, "genesis402_catalog", {}));
  ok(j && j.seller && j.seller.agent_id === 95721, "catalog: names the seller");

  const names = (await c.listTools()).tools.map((t) => t.name);
  ok(names.includes("genesis402_call") && names.includes("genesis402_receipt") && names.includes("genesis402_catalog"), "tool names unchanged");
} catch (e) { console.error("IDENTITY TEST ERROR", e); fails++; }
srv.close();
console.log(fails ? "FAILED " + fails : "ALL PASSED");
process.exit(fails ? 1 : 0);
