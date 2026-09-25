// Tests the hosted MCP on :3105: initialize, tools/list, free catalog call, quote on a
// batch-3 endpoint (checks the path fix), and a validation refusal.
const U = process.env.MCP_URL || ("http://127.0.0.1:" + (process.env.MCP_HTTP_PORT || 3105) + "/mcp");
const H = { "content-type": "application/json", accept: "application/json, text/event-stream", "mcp-protocol-version": "2025-06-18" };
let id = 0;
async function rpc(method, params) {
  const r = await fetch(U, { method: "POST", headers: H, body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }) });
  const t = await r.text(); try { return JSON.parse(t); } catch { return { raw: t.slice(0, 300), status: r.status }; }
}
const init = await rpc("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "test", version: "1" } });
console.log("initialize:", init.result ? init.result.serverInfo : init);
const tools = await rpc("tools/list", {});
console.log("tools:", tools.result ? tools.result.tools.map((t) => t.name).join(", ") : JSON.stringify(tools).slice(0, 300));
const cat = await rpc("tools/call", { name: "genesis402_catalog", arguments: { filter: "yields" } });
console.log("catalog(yields):", cat.result ? cat.result.content[0].text.slice(0, 260).replace(/\s+/g, " ") : JSON.stringify(cat).slice(0, 300));
const q = await rpc("tools/call", { name: "genesis402_call", arguments: { endpoint: "defi-yields", params: { chain: "Base", limit: 3 } } });
console.log("quote defi-yields:", q.result ? q.result.content[0].text.slice(0, 300).replace(/\s+/g, " ") : JSON.stringify(q).slice(0, 300));
const bad = await rpc("tools/call", { name: "genesis402_call", arguments: { endpoint: "vin-decode", params: { vin: "nope" } } });
console.log("refusal:", bad.result ? (bad.result.isError + " " + bad.result.content[0].text.slice(0, 160).replace(/\s+/g, " ")) : JSON.stringify(bad).slice(0, 300));

