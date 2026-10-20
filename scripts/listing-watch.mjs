#!/usr/bin/env node
// Daily listing watchdog for the Genesis402 rail. Read-only: it only issues GET requests (plus one free MCP
// initialize/tools-list against the rail) and writes two files. It signs nothing and pays nothing.
//
//   node scripts/listing-watch.mjs            fetch every surface, write REGISTRY-WATCH.md, append one RECORDS.md line
//   node scripts/listing-watch.mjs --dry      fetch and print the table only; write nothing
//
// Environment (all optional):
//   WARROOM_DIR   default C:\UnyKorn-Sync\war-room   (REGISTRY-WATCH.md goes to orchestrator/, the line to RECORDS.md)
//   EXIT_ON_BAD=1 exit 2 when any surface is DRIFT or DOWN (for the scheduler); default exit 0
//
// Surfaces are the ones in orchestrator/dispatch/X402-SYSTEMS-MAP-2026-10-20.md section 1. Two have no free public
// lookup and are reported UNREADABLE with the reason: agent402.tools (its seller lookup is a paid read, $0.005) and
// mcpservers.org (no listing URL on record). Never claim LIVE for those from this script.
import { writeFileSync, appendFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  EXPECT, classify, parseServed, parseOfficialRegistry, parseNpm, parseSmithery, parseGlamaPage, parseWellknownPage,
  parseNamedPage, parsePayapi, countBazaar, renderTable, recordsLine
} from "./lib/listing-parse.mjs";

const UA = "genesis402-listing-watch/1 (+https://github.com/FTHTrading/genesis402-agent-kit)";
const dry = process.argv.includes("--dry");
const WAR = process.env.WARROOM_DIR || "C:\\UnyKorn-Sync\\war-room";

async function get(url, init = {}) {
  const r = await getOnce(url, init);
  if (r.status !== 429 && r.status < 500) return r;
  await new Promise((ok) => setTimeout(ok, 4000));   // one polite retry for rate limits and transient 5xx
  return getOnce(url, init);
}
async function getOnce(url, init) {
  try {
    const res = await fetch(url, { ...init, headers: { "user-agent": UA, accept: "application/json, text/html;q=0.9", ...(init.headers || {}) }, signal: AbortSignal.timeout(45000), redirect: "follow" });
    return { status: res.status, text: await res.text() };
  } catch (e) {
    return { status: 599, text: "", error: String(e?.message || e) };
  }
}
const asJson = (r) => { try { return JSON.parse(r.text); } catch { return null; } };
const mcp = (id, method) => get(EXPECT.mcpUrl, { method: "POST", headers: { "content-type": "application/json", accept: "application/json, text/event-stream" }, body: JSON.stringify({ jsonrpc: "2.0", id, method, ...(method === "initialize" ? { params: { protocolVersion: "2025-03-26", capabilities: {}, clientInfo: { name: "listing-watch", version: "1" } } } : {}) }) });

/** A transport failure (status 599) must not read as "not listed": report it as UNREADABLE. */
const fromFetch = (r, parsed) => (r.status === 599 ? { ...parsed, status: 429, note: `fetch failed: ${r.error}` } : parsed);

async function bazaarPages() {
  const pages = [];
  const limit = 1000;
  for (let offset = 0; offset < 60000; offset += limit) {
    const r = await get(`https://api.cdp.coinbase.com/platform/v2/x402/discovery/resources?limit=${limit}&offset=${offset}`);
    const j = asJson(r);
    if (!j || !Array.isArray(j.items)) return { pages, ok: pages.length > 0 && false, status: r.status };
    pages.push(j);
    if (j.items.length < limit || offset + limit >= (j.pagination?.total ?? 0)) break;
  }
  return { pages, ok: true, status: 200 };
}

async function main() {
  const iso = new Date().toISOString().replace(/\.\d+Z$/, "Z");
  const [init, list] = await Promise.all([mcp(1, "initialize"), mcp(2, "tools/list")]);
  const served = parseServed(asJson(init) ?? parseSse(init.text), asJson(list) ?? parseSse(list.text));

  const [reg, npm, smi, glama, wk, rush, payapi, bz] = await Promise.all([
    get(`https://registry.modelcontextprotocol.io/v0/servers?search=genesis402`),
    get("https://registry.npmjs.org/genesis402-mcp"),
    get("https://registry.smithery.ai/servers/kevanbtc/genesis402-mcp"),
    get("https://glama.ai/mcp/servers/FTHTrading/genesis402-agent-kit"),
    get("https://wellknown.network/agents/genesis402-x402-pay-per-call"),
    get("https://mcprush.com/fthtrading/genesis402-x402-pay-per-call-mcp"),
    get("https://payapi.market/agent/search?q=genesis402"),
    bazaarPages()
  ]);

  const rows = [
    classify(fromFetch(reg, parseOfficialRegistry(asJson(reg))), served),
    classify(fromFetch(npm, parseNpm(asJson(npm))), served),
    classify(fromFetch(smi, parseSmithery(asJson(smi))), served),
    classify(fromFetch(glama, parseGlamaPage(glama.text, glama.status)), served),
    classify(fromFetch(wk, parseWellknownPage(wk.text, wk.status)), served),
    classify(fromFetch(rush, parseNamedPage("mcprush.com", rush.text, rush.status)), served),
    classify(fromFetch(payapi, parsePayapi(asJson(payapi))), served),
    bz.ok ? classify(countBazaar(bz.pages), served) : classify({ surface: "Coinbase x402 Bazaar", status: bz.status === 200 ? 429 : bz.status, note: "discovery API did not return a page" }, served),
    { surface: "agent402.tools", state: "UNREADABLE", note: "seller lookup is a paid read ($0.005); not probed" },
    { surface: "mcpservers.org", state: "UNREADABLE", note: "no listing URL on record; not probed" }
  ];
  if (served.version === null) rows.unshift({ surface: "Rail MCP (twin.unykorn.org/mcp)", state: "DOWN", note: "initialize failed; other rows have no reference version" });
  else rows.unshift({ surface: "Rail MCP (twin.unykorn.org/mcp)", state: "LIVE", version: served.version, tools: served.tools, url: EXPECT.mcpUrl, note: "" });

  const table = renderTable(rows, iso, served);
  process.stdout.write(table);
  if (!dry) {
    const out = path.join(WAR, "orchestrator", "REGISTRY-WATCH.md");
    mkdirSync(path.dirname(out), { recursive: true });
    writeFileSync(out, table);
    appendFileSync(path.join(WAR, "RECORDS.md"), recordsLine(rows, iso, table) + "\n");
  }
  if (process.env.EXIT_ON_BAD === "1" && rows.some((r) => r.state === "DRIFT" || r.state === "DOWN")) process.exitCode = 2;
}

/** Streamable HTTP may answer as an SSE frame: "data: {json}". */
function parseSse(text) {
  const line = (text || "").split(/\r?\n/).find((l) => l.startsWith("data:"));
  try { return line ? JSON.parse(line.slice(5)) : null; } catch { return null; }
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  main().catch((e) => { process.stderr.write(`listing-watch failed: ${e?.message || e}\n`); process.exitCode = 1; });
}
