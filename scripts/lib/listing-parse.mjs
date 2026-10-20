// Pure parsers for scripts/listing-watch.mjs: no network, no filesystem, so they can be tested with fixtures.
// Each parseX returns a row fragment { surface, status?, version?, tools?, url?, listings?, ... }; classify() turns a
// fragment plus the version/tools the rail actually serves into one of LIVE / DRIFT / DOWN / UNREADABLE.
//   LIVE        the surface shows us and matches what the rail serves
//   DRIFT       the surface shows us but something differs (old version, stale tool count, URL that is going away)
//   DOWN        the surface does not show us, or answers 404/5xx
//   UNREADABLE  the surface refused a script (403/429 or similar): we cannot say, so we do not claim LIVE or DOWN
import { createHash } from "node:crypto";

export const EXPECT = Object.freeze({
  host: "twin.unykorn.org",
  mcpUrl: "https://twin.unykorn.org/mcp",
  registryName: "io.github.FTHTrading/genesis402-mcp",
  // Hosts whose URLs stop working. Smithery run.tools / server.smithery.ai are retired on 2026-10-31 (Smithery email 2026-10-20).
  retiringHosts: ["run.tools", "server.smithery.ai"]
});

const hostOf = (u) => { try { return new URL(u).host; } catch { return ""; } };

/** The version and tool count the hosted rail serves right now (the reference every surface is compared with). */
export function parseServed(initialize, toolsList) {
  const version = initialize?.result?.serverInfo?.version ?? null;
  const tools = Array.isArray(toolsList?.result?.tools) ? toolsList.result.tools.length : null;
  return { version, tools };
}

export function parseOfficialRegistry(body) {
  const rows = (body?.servers || []).map((s) => ({ server: s.server || s, meta: s._meta?.["io.modelcontextprotocol.registry/official"] || {} }))
    .filter((s) => s.server?.name === EXPECT.registryName);
  const latest = rows.find((r) => r.meta.isLatest && r.meta.status === "active");
  if (!latest) return { surface: "Official MCP Registry", found: false };
  return { surface: "Official MCP Registry", found: true, version: latest.server.version, url: latest.server.remotes?.[0]?.url ?? null };
}

export function parseNpm(body) {
  const v = body?.["dist-tags"]?.latest;
  return v ? { surface: "npm genesis402-mcp", found: true, version: v } : { surface: "npm genesis402-mcp", found: false };
}

export function parseSmithery(body) {
  const url = body?.connections?.find((c) => c.deploymentUrl)?.deploymentUrl ?? body?.deploymentUrl ?? null;
  if (!url && !Array.isArray(body?.tools)) return { surface: "Smithery", found: false };
  return { surface: "Smithery", found: true, url, tools: Array.isArray(body?.tools) ? body.tools.length : null };
}

export function parseGlamaPage(html, status) {
  const names = new Set((html || "").match(/genesis402_[a-z_]+/g) || []);
  const m = /call\s+(\d{3})\s+endpoints/i.exec(html || "");
  return { surface: "Glama server", status, found: status === 200 && names.size > 0, tools: names.size || null, endpoints: m ? Number(m[1]) : null };
}

export function parseWellknownPage(html, status) {
  return { surface: "wellknown.network", status, found: status === 200 && /genesis402/i.test(html || "") };
}

/** Generic "does this page name us" parse for mcpservers.org, mcprush.com, payforapi.com and the like. */
export function parseNamedPage(surface, html, status) {
  return { surface, status, found: status === 200 && /genesis402/i.test(html || "") };
}

export function parsePayapi(body) {
  const mine = (body?.results || []).filter((r) => hostOf(r.base_url) === EXPECT.host);
  return { surface: "PayAPI Market", found: mine.length > 0, listings: mine.length };
}

/** `pages` are CDP discovery pages ({items:[{resource|url}]}). Host must equal ours exactly (not a lookalike suffix). */
export function countBazaar(pages) {
  let n = 0;
  for (const p of pages || []) for (const it of p?.items || []) if (hostOf(it.resource || it.url) === EXPECT.host) n += 1;
  return { surface: "Coinbase x402 Bazaar", found: n > 0, listings: n };
}

export function classify(row, served) {
  if (row.status === 403 || row.status === 429) return { ...row, state: "UNREADABLE", note: row.note || `HTTP ${row.status}: the site blocked the script` };
  if (row.status && (row.status >= 500 || row.status === 404 || row.status === 410)) return { ...row, state: "DOWN", note: `HTTP ${row.status}` };
  if (row.found === false) return { ...row, state: "DOWN", note: "not listed" };
  const notes = [];
  if (row.version && served?.version && row.version !== served.version) notes.push(`shows ${row.version}, rail serves ${served.version}`);
  if (row.tools && served?.tools && row.tools !== served.tools) notes.push(`${row.tools} tools shown, rail serves ${served.tools}`);
  if (row.url) {
    const h = hostOf(row.url);
    if (EXPECT.retiringHosts.some((r) => h === r || h.endsWith("." + r))) notes.push(`URL ${h} is being retired (2026-10-31); direct URL ${EXPECT.mcpUrl} is not shown`);
    else if (row.surface !== "Smithery" && row.url !== EXPECT.mcpUrl) notes.push(`URL shown is ${row.url}`);
  }
  return { ...row, state: notes.length ? "DRIFT" : "LIVE", note: notes.join("; ") };
}

const cell = (v) => (v === null || v === undefined || v === "" ? "-" : String(v).replace(/\|/g, "/"));

export function renderTable(rows, isoTime, served) {
  const head = [
    "# Registry watch",
    "",
    `Generated ${isoTime} by genesis402-agent-kit/scripts/listing-watch.mjs. Rail serves version ${cell(served?.version)} with ${cell(served?.tools)} tools (reference for DRIFT).`,
    "States: LIVE = shown and matches; DRIFT = shown but differs; DOWN = not shown or 404/5xx; UNREADABLE = site blocked the script, unknown.",
    "",
    "| Surface | State | Version | Tools | URL / count | Note |",
    "|---|---|---|---|---|---|"
  ];
  const body = rows.map((r) => `| ${cell(r.surface)} | ${r.state} | ${cell(r.version)} | ${cell(r.tools)} | ${cell(r.url ?? (r.listings != null ? `${r.listings} listings` : null))} | ${cell(r.note)} |`);
  return [...head, ...body, ""].join("\n");
}

/** One RECORDS.md line in the protocol format (rules/10 section 4). Hash is of the rendered table. */
export function recordsLine(rows, isoTime, table) {
  const count = (s) => rows.filter((r) => r.state === s).length;
  const hash = createHash("sha256").update(table).digest("hex");
  const bad = rows.filter((r) => r.state === "DRIFT" || r.state === "DOWN").map((r) => `${r.surface}=${r.state}`).join("; ");
  return `${isoTime} | listings | agent | ${hash} | - | - | - | - | listing-watch.mjs | ${count("LIVE")} LIVE, ${count("DRIFT")} DRIFT, ${count("DOWN")} DOWN, ${count("UNREADABLE")} UNREADABLE${bad ? ` (${bad})` : ""}`;
}
