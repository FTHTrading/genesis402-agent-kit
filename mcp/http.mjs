#!/usr/bin/env node
// Hosted Genesis402 MCP over streamable HTTP (stateless). Served publicly at
// https://twin.unykorn.org/mcp via the rail's /mcp proxy.
//
// This process holds NO private key and cannot spend anything. Paid tools return the
// x402 quote, or forward a payment the caller signed with their own wallet.
import http from "node:http";
import { StreamableHTTPServerTransport } from "@modelcontextprotocol/sdk/server/streamableHttp.js";
import { createServer, loadCatalog, catalogSize, VERSION } from "./core.mjs";

const PORT = Number(process.env.MCP_HTTP_PORT || 3105);
const MAX_BODY = 256 * 1024;
let calls = 0;

function readBody(req) {
  return new Promise((resolve, reject) => {
    let n = 0; const chunks = [];
    req.on("data", (c) => { n += c.length; if (n > MAX_BODY) { reject(Object.assign(new Error("too_large"), { code: 413 })); req.destroy(); } else chunks.push(c); });
    req.on("end", () => { try { resolve(chunks.length ? JSON.parse(Buffer.concat(chunks).toString("utf8")) : undefined); } catch (e) { reject(Object.assign(new Error("bad_json"), { code: 400 })); } });
    req.on("error", reject);
  });
}

http.createServer(async (req, res) => {
  const url = new URL(req.url, "http://localhost");
  res.setHeader("Access-Control-Allow-Origin", "*");
  res.setHeader("Access-Control-Allow-Headers", "content-type, mcp-session-id, mcp-protocol-version, accept");
  res.setHeader("Access-Control-Expose-Headers", "mcp-session-id");
  if (req.method === "OPTIONS") { res.writeHead(204); return res.end(); }
  if (url.pathname === "/mcp/health" || (url.pathname === "/mcp" && req.method === "GET" && !String(req.headers.accept || "").includes("text/event-stream"))) {
    res.writeHead(200, { "content-type": "application/json" });
    return res.end(JSON.stringify({ ok: true, service: "genesis402-mcp (hosted)", version: VERSION, transport: "streamable-http", endpoint: "https://twin.unykorn.org/mcp", tools_backed_by_endpoints: catalogSize(), holds_keys: false, calls }));
  }
  if (url.pathname !== "/mcp") { res.writeHead(404); return res.end(); }
  if (req.method !== "POST") { res.writeHead(405, { allow: "POST, GET, OPTIONS" }); return res.end(); }
  let body;
  try { body = await readBody(req); } catch (e) { res.writeHead(e.code || 400); return res.end(); }
  calls++;
  try {
    const server = await createServer({ payer: null, hosted: true });
    const transport = new StreamableHTTPServerTransport({ sessionIdGenerator: undefined, enableJsonResponse: true });
    res.on("close", () => { transport.close(); server.close(); });
    await server.connect(transport);
    await transport.handleRequest(req, res, body);
  } catch (e) {
    if (!res.headersSent) { res.writeHead(500, { "content-type": "application/json" }); res.end(JSON.stringify({ jsonrpc: "2.0", error: { code: -32603, message: "internal error" }, id: null })); }
  }
}).listen(PORT, "127.0.0.1", async () => { await loadCatalog(true); console.log(`[genesis402-mcp-http] 127.0.0.1:${PORT} tools over ${catalogSize()} endpoints`); });
