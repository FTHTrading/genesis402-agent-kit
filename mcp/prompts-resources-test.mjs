// Offline test: verifies MCP prompts/list (3 prompts) and resources/list + resources/read (3 resources).
import http from "node:http";
import assert from "node:assert/strict";

const srv = http.createServer((q, r) => {
  const origin = "http://127.0.0.1:" + srv.address().port;
  const send = (s, o) => { r.writeHead(s, { "content-type": "application/json" }); r.end(JSON.stringify(o)); };
  if (q.url === "/.well-known/x402") {
    return send(200, {
      x402Version: 2,
      seller: { agent: "genesis402", agent_id: 95721, registry: "eip155:8453:0x8004A169FB4a3325136EB29fA0ceB6D2e539a432" },
      services: [{ name: "check-address", endpoint: origin + "/check/address", price: { usd: 0.01 }, specialist: "chain-reader", title: "Check address" }]
    });
  }
  if (q.url === "/status") return send(200, { healthy: true, endpoints: { advertised: 1, payable: 1 } });
  send(404, { error: "not_found" });
});

await new Promise((r) => srv.listen(0, "127.0.0.1", r));
process.env.GENESIS402_ORIGIN = "http://127.0.0.1:" + srv.address().port;

try {
  const { createServer, loadCatalog } = await import("./core.mjs");
  const { Client } = await import("@modelcontextprotocol/sdk/client/index.js");
  const { InMemoryTransport } = await import("@modelcontextprotocol/sdk/inMemory.js");

  await loadCatalog(true);
  const server = await createServer({ payer: null, hosted: true });
  const [a, b] = InMemoryTransport.createLinkedPair();
  const c = new Client({ name: "prompts-resources-test", version: "1" });
  await Promise.all([server.connect(a), c.connect(b)]);

  const prompts = (await c.listPrompts()).prompts;
  assert.equal(prompts.length, 3, "exactly 3 prompts registered");
  assert.deepEqual(
    prompts.map((p) => p.name).sort(),
    ["counterparty-due-diligence", "screen-wallet-sanctions", "verify-x402-receipt"]
  );
  const promptOut = await c.getPrompt({ name: "verify-x402-receipt", arguments: { receipt_id: "g402-sample" } });
  assert.ok(promptOut.messages?.[0]?.content?.text?.includes("g402-sample"));

  const resources = (await c.listResources()).resources;
  assert.equal(resources.length, 3, "exactly 3 resources registered");
  assert.deepEqual(
    resources.map((r) => r.uri).sort(),
    ["genesis402://catalog", "genesis402://seller", "genesis402://status"]
  );

  const catRes = await c.readResource({ uri: "genesis402://catalog" });
  const catJson = JSON.parse(catRes.contents[0].text);
  assert.equal(catJson.total_endpoints, 1);

  const statusRes = await c.readResource({ uri: "genesis402://status" });
  assert.equal(JSON.parse(statusRes.contents[0].text).healthy, true);

  const sellerRes = await c.readResource({ uri: "genesis402://seller" });
  assert.equal(JSON.parse(sellerRes.contents[0].text).agent_id, 95721);

  console.log("ALL PASS: 3 prompts and 3 resources verified");
} finally {
  srv.close();
}
