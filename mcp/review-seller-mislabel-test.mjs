// Review test (FORGE phase 4): a PAID call with payment_signature on a desk-sold route must not be labelled with the rail seller.
// Fake rail: manifest has top-level seller 95721 AND a per-service seller 96494 on "dossier" (as the live /.well-known/x402 does).
import http from "node:http";
let fails = 0; const ok = (c, m, x) => { console.log((c ? "PASS " : "FAIL ") + m + (!c && x !== undefined ? "  " + JSON.stringify(x).slice(0, 300) : "")); if (!c) fails++; };
const REG = "0x8004A169FB4a3325136EB29fA0ceB6D2e539a432", DESK = "0x117459d013492eC5fF90387B16764e753C506ec9", TRE = "0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB";
const RAIL = { agent: "genesis402", agent_id: 95721, registry: "eip155:8453:" + REG, registry_address: REG, chain: "base", pay_to: TRE };
const DS = { agent: "evidence-01", agent_id: 96494, registry: "eip155:8453:" + REG, pay_to: DESK, wallet: DESK };
const srv = http.createServer((q, r) => { q.resume(); q.on("end", () => {
  const send = (s, o, h) => { r.writeHead(s, { "content-type": "application/json", ...(h || {}) }); r.end(JSON.stringify(o)); };
  const origin = "http://127.0.0.1:" + srv.address().port;
  if (q.url === "/.well-known/x402") return send(200, { seller: RAIL, services: [{ name: "dossier", endpoint: origin + "/agents/evidence-01/dossier", price: { usd: 0.01 }, title: "d", tags: [], seller: DS }] });
  if (q.url === "/__validate") return send(200, { checked: false });
  if (q.url.startsWith("/agents/evidence-01/dossier")) {
    if (q.headers["payment-signature"]) return send(200, { ok: 1 }, { "payment-response": Buffer.from(JSON.stringify({ success: true, transaction: "0xabc", network: "eip155:8453" })).toString("base64") });
    const body = { x402Version: 2, accepts: [{ scheme: "exact", network: "eip155:8453", asset: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", amount: "10000", payTo: DESK }], seller: DS };
    return send(402, body, { "payment-required": Buffer.from(JSON.stringify(body)).toString("base64") });
  }
  send(404, {}); }); });
await new Promise((r) => srv.listen(0, "127.0.0.1", r));
process.env.GENESIS402_ORIGIN = "http://127.0.0.1:" + srv.address().port;
const { createServer, loadCatalog } = await import("./core.mjs");
const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = await import("@modelcontextprotocol/sdk/inMemory.js");
await loadCatalog(true);
const s = await createServer({ payer: null, hosted: false }); const [a, b] = InMemoryTransport.createLinkedPair(); const c = new Client({ name: "t", version: "1" }); await Promise.all([s.connect(a), c.connect(b)]);
let r = await c.callTool({ name: "genesis402_call", arguments: { endpoint: "dossier", params: {} } });
ok(r.structuredContent.seller.agent_id === 96494, "quote on desk route names 96494", r.structuredContent.seller);
r = await c.callTool({ name: "genesis402_call", arguments: { endpoint: "dossier", params: {}, payment_signature: "sig" } });
ok(r.structuredContent.mode === "PAID" && r.structuredContent.seller.agent_id === 96494, "PAID (payment_signature) on desk route names 96494, not the rail 95721", r.structuredContent.seller);
srv.close(); console.log(fails ? "FAILED " + fails : "ALL PASSED"); process.exit(fails ? 1 : 0);
