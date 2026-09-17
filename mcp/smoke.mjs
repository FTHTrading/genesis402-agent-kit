import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StdioClientTransport } from "@modelcontextprotocol/sdk/client/stdio.js";
import { generatePrivateKey } from "viem/accounts";
async function session(env, label) {
  const t = new StdioClientTransport({ command: "node", args: ["index.mjs"], env: { ...process.env, ...env } });
  const c = new Client({ name: "smoke", version: "0" }); await c.connect(t);
  console.log(`== ${label}`);
  return c;
}
const short = (r) => (r.isError ? "ERROR " : "") + r.content[0].text.replace(/\s+/g, " ").slice(0, 330);
let c = await session({}, "quote mode");
const tools = await c.listTools(); console.log("tools:", tools.tools.map(t => t.name).join(", "));
console.log("catalog:", short(await c.callTool({ name: "genesis402_catalog", arguments: {} })));
console.log("risk quote:", short(await c.callTool({ name: "genesis402_risk", arguments: { address: "0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed" } })));
console.log("rwa bad market (validation):", short(await c.callTool({ name: "genesis402_rwa_screen", arguments: { market: "ohio" } })));
await c.close();
c = await session({ GENESIS402_LIVE: "1", GENESIS402_PAYER_KEY: generatePrivateKey() }, "live mode, empty throwaway wallet");
console.log("genesis_sim live:", short(await c.callTool({ name: "genesis402_genesis_sim", arguments: { n: 20, epochs: 20 } })));
await c.close();
c = await session({ GENESIS402_LIVE: "1", GENESIS402_PAYER_KEY: generatePrivateKey(), GENESIS402_MAX_USD: "0.01" }, "live mode, cap below price");
console.log("prove capped:", short(await c.callTool({ name: "genesis402_prove", arguments: { text: "hello" } })));
await c.close();
