// Boots the MCP server over stdio exactly as Claude Desktop / Cursor would,
// and lists the tools it advertises. Never sets GENESIS402_LIVE.
import { spawn } from "node:child_process";

const p = spawn(process.execPath, ["index.mjs"], { stdio: ["pipe", "pipe", "pipe"] });
let buf = "";
const seen = new Map();

p.stdout.on("data", (d) => {
  buf += d.toString();
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i).trim();
    buf = buf.slice(i + 1);
    if (!line) continue;
    try { const m = JSON.parse(line); if (m.id != null) seen.set(m.id, m); } catch {}
  }
});
p.stderr.on("data", (d) => process.stderr.write("[server] " + d));

const send = (o) => p.stdin.write(JSON.stringify(o) + "\n");
const waitFor = (id, ms = 20000) => new Promise((res, rej) => {
  const t0 = Date.now();
  const iv = setInterval(() => {
    if (seen.has(id)) { clearInterval(iv); res(seen.get(id)); }
    else if (Date.now() - t0 > ms) { clearInterval(iv); rej(new Error(`timeout waiting for id ${id}`)); }
  }, 100);
});

try {
  send({ jsonrpc: "2.0", id: 1, method: "initialize",
    params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "boot-test", version: "1" } } });
  const init = await waitFor(1);
  console.log("initialize ->", init.result?.serverInfo?.name, init.result?.serverInfo?.version);

  send({ jsonrpc: "2.0", method: "notifications/initialized" });
  send({ jsonrpc: "2.0", id: 2, method: "tools/list" });
  const list = await waitFor(2);
  const tools = list.result?.tools || [];
  console.log(`\ntools/list -> ${tools.length} tools\n`);
  for (const t of tools) console.log("  " + t.name.padEnd(32) + " :: " + t.description.slice(0, 78));

  // Exercise the free catalog tool and the invalid-input guard.
  send({ jsonrpc: "2.0", id: 3, method: "tools/call",
    params: { name: "genesis402_catalog", arguments: { filter: "xrpl" } } });
  const cat = JSON.parse((await waitFor(3)).result.content[0].text);
  console.log(`\ncatalog(filter=xrpl) -> ${cat.matched} of ${cat.total_endpoints}, mode: ${cat.payer_mode}`);

  send({ jsonrpc: "2.0", id: 4, method: "tools/call",
    params: { name: "genesis402_call", arguments: { endpoint: "evm-multi-chain-scan", arguments: {}, params: { address: "nope" } } } });
  const badCall = await waitFor(4);
  const badTxt = badCall.result.content[0].text;
  console.log(`\ninvalid params -> isError=${badCall.result.isError}`);
  console.log("  " + badTxt.replace(/\s+/g, " ").slice(0, 150));

  send({ jsonrpc: "2.0", id: 5, method: "tools/call",
    params: { name: "genesis402_call", arguments: { endpoint: "totally-not-real", params: {} } } });
  const unknown = await waitFor(5);
  console.log(`\nunknown endpoint -> isError=${unknown.result.isError}`);
  console.log("  " + unknown.result.content[0].text.replace(/\s+/g, " ").slice(0, 150));

  send({ jsonrpc: "2.0", id: 6, method: "tools/call",
    params: { name: "genesis402_call", arguments: { endpoint: "evm-multi-chain-scan", params: { address: "0x0000000000000000000000000000000000000000" } } } });
  const quote = await waitFor(6);
  const q = JSON.parse(quote.result.content[0].text);
  console.log(`\nvalid params -> mode=${q.mode} price=$${q.price?.usd} (nothing signed)`);
} catch (e) {
  console.error("BOOT TEST FAILED:", e.message);
  p.kill();
  process.exit(1);
}
p.kill();
process.exit(0);
