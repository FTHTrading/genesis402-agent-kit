// Offline test of owed-delivery redemption against a mock rail. No network, nothing settles:
// the local-payer case signs a real EIP-3009 authorization with a throwaway key, offline.
// A paid call that settles but is not delivered must be redeemed with its owed_ticket and the
// SAME payment, never by signing a new one.
import http from "node:http";
import { generatePrivateKey } from "viem/accounts";

const BASE_USDC = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
const PAY_TO = "0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB";
const TICKET_1 = "owed_" + "a".repeat(40);
const TICKET_2 = "owed_" + "b".repeat(40);
const state = { mode: "owed_then_ok", hits: [], firstPayment: null };

const rail = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => { body += c; });
  req.on("end", () => {
    const send = (status, obj, headers = {}) => { res.writeHead(status, { "content-type": "application/json", ...headers }); res.end(JSON.stringify(obj)); };
    const origin = "http://" + req.headers.host;
    if (req.url === "/.well-known/x402") return send(200, { services: [{ name: "counterparty-report", endpoint: origin + "/report/counterparty", price: { usd: 0.25 } }] });
    if (req.url === "/__validate") return send(200, { checked: true, valid: true });
    if (req.url !== "/report/counterparty") return send(404, { error: "not_found" });
    const pay = req.headers["payment-signature"] || req.headers["x-payment"] || null;
    const ticket = req.headers["x-owed-ticket"] || null;
    state.hits.push({ pay, ticket });
    if (!pay) {
      const challenge = { x402Version: 2, resource: { url: origin + "/report/counterparty", description: "Counterparty report", mimeType: "application/json" },
        accepts: [{ scheme: "exact", network: "eip155:8453", amount: "250000", asset: BASE_USDC, payTo: PAY_TO, maxTimeoutSeconds: 300, extra: { name: "USD Coin", version: "2" } }] };
      return send(402, challenge, { "payment-required": Buffer.from(JSON.stringify(challenge)).toString("base64") });
    }
    if (ticket) {
      if (pay !== state.firstPayment) return send(400, { error: "owed_ticket_not_accepted", reason: "other_payment", charged: false });
      if (state.mode === "owed_twice" && ticket === TICKET_1) return send(500, { error: "task_execution_failed", payment: { status: "owed", owed_ticket: TICKET_2 } });
      if (ticket === TICKET_1 || ticket === TICKET_2) return send(200, { ok: true, receipt: { receipt_id: "g402-mock" }, verdict: "clear" });
      return send(400, { error: "owed_ticket_not_accepted", reason: "unknown_ticket", charged: false });
    }
    state.firstPayment = pay;
    return send(500, { error: "task_execution_failed", payment: { rail: "base:usdc", status: "owed", owed_ticket: TICKET_1 } });
  });
});
await new Promise((r) => rail.listen(0, "127.0.0.1", r));
process.env.GENESIS402_ORIGIN = "http://127.0.0.1:" + rail.address().port;

const { createServer, localPayerFromEnv } = await import("./core.mjs");
const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = await import("@modelcontextprotocol/sdk/inMemory.js");

async function connect(opts) {
  const server = await createServer(opts);
  const [a, b] = InMemoryTransport.createLinkedPair();
  const client = new Client({ name: "owed-test", version: "1" });
  await Promise.all([server.connect(a), client.connect(b)]);
  return client;
}

let pass = 0, fail = 0;
const check = (name, cond, detail) => { if (cond) { pass++; console.log("  PASS  " + name); } else { fail++; console.log("  FAIL  " + name + (detail ? "  <- " + JSON.stringify(detail).slice(0, 300) : "")); } };
const parse = (r) => JSON.parse(r.content[0].text);
const reset = (mode) => { state.mode = mode; state.hits = []; state.firstPayment = null; };
const paidHits = () => state.hits.filter((h) => h.pay);

const hosted = await connect({ hosted: true });
const call = (args) => hosted.callTool({ name: "genesis402_counterparty_report", arguments: { query: "Example Holdings Ltd", ...args } });

console.log("\n-- hosted: owed delivery redeemed once, same payment, no new signature --");
reset("owed_then_ok");
let r = await call({ payment_signature: "caller-signed-payment" });
let j = parse(r);
check("result delivered after the automatic redemption", !r.isError && j.mode === "PAID" && j.made_good === true, j);
check("structuredContent carries made_good", r.structuredContent?.made_good === true);
check("two paid requests, both with the same payment", paidHits().length === 2 && paidHits().every((h) => h.pay === "caller-signed-payment"), state.hits);
check("the second carried the owed ticket", state.hits[1]?.ticket === TICKET_1, state.hits);

console.log("\n-- hosted: redemption also fails, newest ticket handed back --");
reset("owed_twice");
r = await call({ payment_signature: "caller-signed-payment" });
j = parse(r);
check("result is an error", r.isError === true);
check("it says the payment settled but was not delivered", j.paid === true && j.delivered === false, j);
check("it returns the newest ticket", j.owed_ticket === TICKET_2, j);
check("it never asks for a new payment", /Do not sign a new payment/.test(j.message), j);

console.log("\n-- hosted: caller redeems later with owed_ticket + the same payment --");
state.hits = [];
r = await call({ payment_signature: "caller-signed-payment", owed_ticket: TICKET_2 });
j = parse(r);
check("explicit redemption delivers", !r.isError && j.made_good === true, j);
check("exactly one request, with the ticket and the same payment", state.hits.length === 1 && state.hits[0].ticket === TICKET_2 && state.hits[0].pay === "caller-signed-payment", state.hits);
r = await call({ owed_ticket: TICKET_2 });
j = parse(r);
check("owed_ticket without payment_signature is refused locally, nothing sent", r.isError === true && j.error === "owed_ticket_needs_payment_signature" && state.hits.length === 1, j);

console.log("\n-- local payer: signs once, redeems with the captured payment --");
process.env.GENESIS402_LIVE = "1";
process.env.GENESIS402_PAYER_KEY = generatePrivateKey();
process.env.GENESIS402_MAX_USD = "0.25";
const payer = localPayerFromEnv();
check("local payer is built from the env", Boolean(payer));
const local = await connect({ payer });
reset("owed_then_ok");
r = await local.callTool({ name: "genesis402_counterparty_report", arguments: { query: "Example Holdings Ltd" } });
j = parse(r);
check("local call delivered after redemption", !r.isError && j.mode === "PAID" && j.made_good === true, j);
const signed = paidHits();
check("one authorization signed, sent twice", signed.length === 2 && signed[0].pay === signed[1].pay && Boolean(signed[0].pay), state.hits.map((h) => ({ pay: Boolean(h.pay), ticket: h.ticket })));
check("the redemption carried the ticket", signed[1].ticket === TICKET_1);

reset("owed_twice");
r = await local.callTool({ name: "genesis402_counterparty_report", arguments: { query: "Example Holdings Ltd" } });
j = parse(r);
check("local failure returns the ticket and the payment to resend", r.isError === true && j.owed_ticket === TICKET_2 && j.payment_signature === state.firstPayment, { ticket: j.owed_ticket, hasPay: Boolean(j.payment_signature) });

await hosted.close(); await local.close(); rail.close();
console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
