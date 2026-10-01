// Offline test of the make-good path against a mock rail. No network, no keys, nothing signed.
// A paid call that settles but is not delivered must be redeemed with its X-Make-Good token,
// never by paying again.
import http from "node:http";

const TOKEN_1 = "mg1_" + "a".repeat(32);
const TOKEN_2 = "mg2_" + "b".repeat(32);
const state = { mode: "fail_then_ok", hits: [] };

const rail = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => { body += c; });
  req.on("end", () => {
    const send = (status, obj, headers = {}) => { res.writeHead(status, { "content-type": "application/json", ...headers }); res.end(JSON.stringify(obj)); };
    if (req.url === "/.well-known/x402") return send(200, { services: [{ name: "counterparty-report", endpoint: "http://mock/report/counterparty", price: { usd: 0.25 } }] });
    if (req.url === "/__validate") return send(200, { checked: true, valid: true });
    if (req.url !== "/report/counterparty") return send(404, { error: "not_found" });
    const pay = req.headers["payment-signature"] || req.headers["x-payment"];
    const mg = req.headers["x-make-good"];
    state.hits.push({ pay: Boolean(pay), mg: mg || null });
    if (mg) {
      if (state.mode === "fail_twice" && mg === TOKEN_1) return send(500, { error: "task_execution_failed", make_good: { token: TOKEN_2 } }, { "X-Make-Good": TOKEN_2 });
      if (mg === TOKEN_1 || mg === TOKEN_2) return send(200, { ok: true, receipt: { receipt_id: "g402-mock", make_good: { attempts: 1 } }, verdict: "clear" });
      return send(404, { error: "make_good_token_unknown" });
    }
    if (pay) return send(500, { error: "task_execution_failed", payment: { status: "settled — delivery owed" }, make_good: { token: TOKEN_1 } }, { "X-Make-Good": TOKEN_1 });
    return send(402, { x402Version: 2, accepts: [] });
  });
});
await new Promise((r) => rail.listen(0, "127.0.0.1", r));
process.env.GENESIS402_ORIGIN = "http://127.0.0.1:" + rail.address().port;

const { createServer } = await import("./core.mjs");
const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
const { InMemoryTransport } = await import("@modelcontextprotocol/sdk/inMemory.js");

const server = await createServer({ hosted: true });
const [a, b] = InMemoryTransport.createLinkedPair();
const client = new Client({ name: "makegood-test", version: "1" });
await Promise.all([server.connect(a), client.connect(b)]);

let pass = 0, fail = 0;
const check = (name, cond, detail) => { if (cond) { pass++; console.log("  PASS  " + name); } else { fail++; console.log("  FAIL  " + name + (detail ? "  <- " + JSON.stringify(detail).slice(0, 300) : "")); } };
const call = (args) => client.callTool({ name: "genesis402_counterparty_report", arguments: { query: "Example Holdings Ltd", ...args } });
const parse = (r) => JSON.parse(r.content[0].text);

console.log("\n-- settled but undelivered: one automatic free redemption --");
state.mode = "fail_then_ok"; state.hits = [];
let r = await call({ payment_signature: "signed-by-caller" });
let j = parse(r);
check("call succeeds after the automatic make-good", !r.isError && j.mode === "PAID" && j.made_good === true, j);
check("structuredContent carries made_good", r.structuredContent && r.structuredContent.made_good === true);
check("exactly one payment was presented", state.hits.filter((h) => h.pay).length === 1, state.hits);
check("the redemption carried the token and no payment", state.hits.length === 2 && state.hits[1].mg === TOKEN_1 && !state.hits[1].pay, state.hits);

console.log("\n-- redemption also fails: hand back the newest token, never pay again --");
state.mode = "fail_twice"; state.hits = [];
r = await call({ payment_signature: "signed-by-caller" });
j = parse(r);
check("result is an error", r.isError === true);
check("it says the payment settled but was not delivered", j.paid === true && j.delivered === false, j);
check("it returns the rotated token", j.make_good_token === TOKEN_2, j);
check("only one payment was presented", state.hits.filter((h) => h.pay).length === 1, state.hits);

console.log("\n-- caller redeems later with make_good_token --");
state.hits = [];
r = await call({ make_good_token: TOKEN_2 });
j = parse(r);
check("explicit token redeems free", !r.isError && j.mode === "PAID" && j.made_good === true, j);
check("no payment presented on explicit redemption", state.hits.length === 1 && !state.hits[0].pay && state.hits[0].mg === TOKEN_2, state.hits);

state.hits = [];
r = await call({ make_good_token: "mgX_" + "c".repeat(32) });
j = parse(r);
check("unknown token is an error that charged nothing", r.isError === true && /nothing/.test(j.charged || ""), j);

await client.close(); rail.close();
console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail ? 1 : 0);
