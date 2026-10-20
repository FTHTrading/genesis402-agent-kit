// Parsing tests for scripts/lib/listing-parse.mjs. Fixtures are trimmed copies of real responses fetched 2026-10-20.
// Run: node --test scripts/test/
import test from "node:test";
import assert from "node:assert/strict";
import {
  EXPECT, classify, parseServed, parseOfficialRegistry, parseNpm, parseSmithery, parseGlamaPage,
  parseWellknownPage, parsePayapi, parseSitemapListing, countBazaar, renderTable, recordsLine
} from "../lib/listing-parse.mjs";

const SERVED = { version: "0.3.10", tools: 21 };

test("parseServed reads serverInfo.version and counts tools/list", () => {
  const init = { result: { serverInfo: { name: "genesis402", version: "0.3.10" } } };
  const list = { result: { tools: new Array(21).fill({ name: "t" }) } };
  assert.deepEqual(parseServed(init, list), { version: "0.3.10", tools: 21 });
  assert.deepEqual(parseServed({}, {}), { version: null, tools: null });
});

test("official registry: latest active entry is LIVE and its version/url are read", () => {
  const body = { servers: [
    { server: { name: "io.github.FTHTrading/genesis402-mcp", version: "0.2.0", remotes: [] }, _meta: { "io.modelcontextprotocol.registry/official": { status: "deprecated", isLatest: false } } },
    { server: { name: "io.github.FTHTrading/genesis402-mcp", version: "0.3.10", remotes: [{ type: "streamable-http", url: "https://twin.unykorn.org/mcp" }] }, _meta: { "io.modelcontextprotocol.registry/official": { status: "active", isLatest: true } } }
  ] };
  const r = parseOfficialRegistry(body);
  assert.equal(r.version, "0.3.10");
  assert.equal(r.url, "https://twin.unykorn.org/mcp");
  assert.equal(classify(r, SERVED).state, "LIVE");
});

test("official registry: no matching entry is DOWN; stale version is DRIFT", () => {
  assert.equal(classify(parseOfficialRegistry({ servers: [] }), SERVED).state, "DOWN");
  const old = parseOfficialRegistry({ servers: [{ server: { name: "io.github.FTHTrading/genesis402-mcp", version: "0.3.9", remotes: [{ url: "https://twin.unykorn.org/mcp" }] }, _meta: { "io.modelcontextprotocol.registry/official": { status: "active", isLatest: true } } }] });
  const c = classify(old, SERVED);
  assert.equal(c.state, "DRIFT");
  assert.match(c.note, /0\.3\.9/);
});

test("npm: dist-tags.latest is the version", () => {
  const r = parseNpm({ "dist-tags": { latest: "0.3.10" } });
  assert.equal(r.version, "0.3.10");
  assert.equal(classify(r, SERVED).state, "LIVE");
  assert.equal(classify(parseNpm({}), SERVED).state, "DOWN");
});

test("smithery: run.tools-only listing is DRIFT (dies 2026-10-31) and a stale tool count is noted", () => {
  const body = { qualifiedName: "kevanbtc/genesis402-mcp", deploymentUrl: "https://genesis402-mcp--kevanbtc.run.tools", connections: [{ type: "http", deploymentUrl: "https://genesis402-mcp--kevanbtc.run.tools" }], tools: new Array(22).fill({}) };
  const r = parseSmithery(body);
  assert.equal(r.url, "https://genesis402-mcp--kevanbtc.run.tools");
  assert.equal(r.tools, 22);
  const c = classify(r, SERVED);
  assert.equal(c.state, "DRIFT");
  assert.match(c.note, /run\.tools/);
});

test("smithery: direct URL with matching tool count is LIVE", () => {
  const r = parseSmithery({ connections: [{ type: "http", deploymentUrl: EXPECT.mcpUrl }], tools: new Array(21).fill({}) });
  assert.equal(classify(r, SERVED).state, "LIVE");
});

test("glama page: counts distinct genesis402_ tool names and endpoint figure", () => {
  const html = "<h1>Genesis402 by FTHTrading</h1> genesis402_catalog genesis402_receipt genesis402_call genesis402_catalog lets AI agents discover, quote, pay for, and call 366 endpoints";
  const r = parseGlamaPage(html, 200);
  assert.equal(r.tools, 3);
  assert.equal(r.endpoints, 366);
  assert.equal(parseGlamaPage("", 404).status, 404);
  assert.equal(classify(parseGlamaPage("", 404), SERVED).state, "DOWN");
});

test("wellknown page: 200 with our name is LIVE, 403 is UNREADABLE (not DOWN)", () => {
  assert.equal(classify(parseWellknownPage("<title>genesis402 pay-per-call</title>", 200), SERVED).state, "LIVE");
  assert.equal(classify({ surface: "x", status: 403 }, SERVED).state, "UNREADABLE");
  assert.equal(classify({ surface: "x", status: 503 }, SERVED).state, "DOWN");
});

test("payapi: counts verified listings on our origin", () => {
  const body = { results: [
    { slug: "genesis402-address-check", base_url: "https://twin.unykorn.org/check/address", provider_name: "UnyKorn LLC" },
    { slug: "other", base_url: "https://example.com/x" }
  ] };
  const r = parsePayapi(body);
  assert.equal(r.listings, 1);
  assert.equal(classify(r, SERVED).state, "LIVE");
  assert.equal(classify(parsePayapi({ results: [] }), SERVED).state, "DOWN");
});

test("bazaar: counts items whose resource host is twin.unykorn.org across pages", () => {
  const pages = [
    { items: [{ resource: "https://twin.unykorn.org/a" }, { resource: "https://www.ax1.vc/x" }] },
    { items: [{ resource: "https://twin.unykorn.org/b" }, { resource: "https://twin.unykorn.org.evil.example/c" }, { url: "https://twin.unykorn.org/d" }] }
  ];
  const r = countBazaar(pages);
  assert.equal(r.listings, 3);
  assert.equal(classify(r, SERVED).state, "LIVE");
  assert.equal(classify(countBazaar([{ items: [] }]), SERVED).state, "DOWN");
});

test("renderTable and recordsLine are stable and carry a hash, never raw secrets", () => {
  const rows = [{ surface: "npm", state: "LIVE", version: "0.3.10", tools: null, url: null, note: "" }];
  const md = renderTable(rows, "2026-10-20T09:00:00Z", SERVED);
  assert.match(md, /\| npm \| LIVE \| 0\.3\.10 \|/);
  const line = recordsLine(rows, "2026-10-20T09:00:00Z", md);
  assert.match(line, /^2026-10-20T09:00:00Z \| listings \| agent \| [0-9a-f]{64} \|/);
  assert.match(line, /1 LIVE, 0 DRIFT, 0 DOWN, 0 UNREADABLE/);
  assert.equal(line.includes("\n"), false);
});

test("sitemap listing: finds our page, ignores localized duplicates, reports LIVE with the honest note", () => {
  const xml = "<urlset><loc>https://mcpservers.org/servers/other/x</loc><loc>https://mcpservers.org/es/servers/fthtrading/genesis402-agent-kit</loc><loc>https://mcpservers.org/servers/fthtrading/genesis402-agent-kit</loc></urlset>";
  const r = parseSitemapListing("mcpservers.org", [xml], "fthtrading/genesis402");
  assert.equal(r.url, "https://mcpservers.org/servers/fthtrading/genesis402-agent-kit");
  const c = classify(r, SERVED);
  assert.equal(c.state, "LIVE");
  assert.match(c.note, /sitemap/);
  assert.equal(classify(parseSitemapListing("x", ["<urlset></urlset>"], "fthtrading/genesis402"), SERVED).state, "DOWN");
});
