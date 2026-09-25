// Genesis402 MCP core: the x402 rail at https://twin.unykorn.org as MCP tools.
// Used by BOTH transports:
//   - index.mjs  : stdio (npx genesis402-mcp). Can pay itself with GENESIS402_PAYER_KEY + GENESIS402_LIVE=1.
//   - http.mjs   : hosted streamable-HTTP endpoint (https://twin.unykorn.org/mcp). Holds NO key, ever.
//                  It returns the price quote, or forwards a payment the CALLER signed
//                  (x402 v2 `payment_signature`), so the caller's wallet always pays for itself.
//
// Safe by default:
//   - Without a payer, a paid tool only returns the price quote (the 402 challenge). Nothing is signed.
//   - The server sets the price. This client never does; it only decides whether to accept it.
//   - Prices and paths come from the live /.well-known/x402 manifest, never hardcoded.
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";
import { wrapFetchWithPaymentFromConfig, decodePaymentResponseHeader } from "@x402/fetch";
import { ExactEvmScheme } from "@x402/evm";
import { privateKeyToAccount } from "viem/accounts";

export const VERSION = "0.3.0";
const ORIGIN = (process.env.GENESIS402_ORIGIN || "https://twin.unykorn.org").replace(/\/$/, "");
const BASE = "eip155:8453";
const BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

const text = (obj) => ({ content: [{ type: "text", text: typeof obj === "string" ? obj : JSON.stringify(obj, null, 2) }] });
const fail = (obj) => ({ ...text(obj), isError: true });

// ---------------------------------------------------------------------------
// Live manifest (shared, refreshed every 10 minutes).
// ---------------------------------------------------------------------------
const CATALOG = new Map(); // name -> service entry from /.well-known/x402
let CATALOG_OK = false;
let loadedAt = 0;
export async function loadCatalog(force) {
  if (!force && CATALOG_OK && Date.now() - loadedAt < 10 * 60 * 1000) return;
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), 8000);
  try {
    const r = await fetch(ORIGIN + "/.well-known/x402", { signal: ctrl.signal });
    const j = await r.json();
    const next = new Map();
    for (const s of j.services || []) { const name = s.name || String(s.endpoint || "").split("/").pop(); if (name) next.set(name, s); }
    if (next.size) { CATALOG.clear(); for (const [k, v] of next) CATALOG.set(k, v); CATALOG_OK = true; loadedAt = Date.now(); }
  } catch { /* keep the previous catalog */ } finally { clearTimeout(t); }
}
export const catalogSize = () => CATALOG.size;

const priceOf = (name) => {
  const s = CATALOG.get(name); const p = s && s.price;
  if (p == null) return null;
  if (typeof p === "string" || typeof p === "number") return Number(p);
  const v = p.usd ?? p.amount ?? p.value; return v == null ? null : Number(v);
};
const priceTag = (name) => { const usd = priceOf(name); return usd == null ? "Paid (price from the live 402 quote)" : `Paid ($${usd} USDC)`; };
// The real path from the manifest. Many endpoints do not live at "/" + name
// (/v1/chat/completions, /defi/yields, /price/bitcoin ...). 0.2.0 assumed they did.
const pathOf = (name) => { const s = CATALOG.get(name); try { return s && s.endpoint ? new URL(s.endpoint).pathname : "/" + name; } catch { return "/" + name; } };

function decodeChallenge(res, body) {
  const h = res.headers.get("payment-required");
  if (h) { try { return JSON.parse(Buffer.from(h, "base64").toString("utf8")); } catch {} }
  return body && body.accepts ? body : null;
}
function baseQuote(challenge) {
  const a = (challenge?.accepts || []).find((x) => x.network === BASE && String(x.asset).toLowerCase() === BASE_USDC);
  if (!a) return null;
  const atomic = BigInt(a.amount ?? a.maxAmountRequired);
  return { atomic: atomic.toString(), usd: Number(atomic) / 1e6, payTo: a.payTo, network: a.network };
}
async function preValidate(name, params) {
  try {
    const r = await fetch(ORIGIN + "/__validate", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name, params: params || {} }) });
    if (!r.ok) return null;
    const j = await r.json();
    return j.checked === true && j.valid === false ? j : null;
  } catch { return null; }
}

// payer: { paidFetch, maxUsd } for the local stdio server with its own key, or null.
// paymentSignature: an x402 v2 payment the CALLER signed for this exact quote (hosted mode).
async function callPaid(name, params, payer, paymentSignature) {
  const url = ORIGIN + pathOf(name);
  const init = { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ params }) };
  const invalid = await preValidate(name, params);
  if (invalid) return fail({ error: "invalid_input", endpoint: name, message: invalid.message, checked_by: "free /__validate - nothing was charged and no quote was requested" });

  if (paymentSignature) {
    const res = await fetch(url, { ...init, headers: { ...init.headers, "PAYMENT-SIGNATURE": paymentSignature, "X-PAYMENT": paymentSignature } });
    const body = await res.json().catch(() => null);
    let settlement = null; const pr = res.headers.get("payment-response") || res.headers.get("x-payment-response");
    if (pr) { try { settlement = decodePaymentResponseHeader(pr); } catch {} }
    return res.ok ? text({ paid: true, settlement, result: body }) : fail({ status: res.status, paid: false, ...body });
  }

  const probe = await fetch(url, init);
  const probeBody = await probe.json().catch(() => null);
  if (probe.status !== 402) return probe.ok ? text(probeBody) : fail({ status: probe.status, ...probeBody });
  const challenge = decodeChallenge(probe, probeBody);
  const quote = baseQuote(challenge);

  if (!payer) {
    return text({
      mode: "QUOTE_ONLY", resource: url, price: quote,
      also_accepted_networks: (challenge?.accepts || []).map((a) => a.network),
      what_you_get: challenge?.resource?.description || challenge?.resource?.what_you_get,
      payment_required_b64: probe.headers.get("payment-required"),
      how_to_pay: "Sign an x402 v2 payment for this quote with your own wallet (e.g. @x402/fetch, Coinbase AgentKit/CDP wallet) and call this tool again with payment_signature=<the PAYMENT-SIGNATURE header value>. Or call " + url + " directly over HTTP with any x402 client. This server never holds your keys."
    });
  }
  if (!quote) return fail({ error: "no_base_usdc_lane" });
  if (quote.usd > payer.maxUsd) return fail({ error: "price_above_cap", quote, cap_usd: payer.maxUsd });
  const res = await payer.paidFetch(url, init);
  const body = await res.json().catch(() => null);
  let settlement = null; const pr = res.headers.get("payment-response") || res.headers.get("x-payment-response");
  if (pr) { try { settlement = decodePaymentResponseHeader(pr); } catch {} }
  return res.ok ? text({ paid: quote, settlement, result: body }) : fail({ status: res.status, paid: false, ...body });
}

export function localPayerFromEnv() {
  const key = process.env.GENESIS402_PAYER_KEY || "";
  if (process.env.GENESIS402_LIVE !== "1" || !key) return null;
  const account = privateKeyToAccount(key.startsWith("0x") ? key : `0x${key}`);
  return { paidFetch: wrapFetchWithPaymentFromConfig(fetch, { schemes: [{ network: BASE, client: new ExactEvmScheme(account) }] }), maxUsd: Number(process.env.GENESIS402_MAX_USD || "0.25") };
}

const NAMED = [
  ["genesis402_wallet_brief", "wallet-brief", "Wallet due diligence in ONE call: sanctions screen + 10-chain scan + activity + summary, with an evidence hash.", { address: z.string().regex(/^0x[0-9a-fA-F]{40}$/), chain: z.string().max(24).optional() }],
  ["genesis402_token_brief", "token-brief", "Token pre-trade check in ONE call: metadata, price, holder concentration, verification, sanctions.", { contract: z.string().regex(/^0x[0-9a-fA-F]{40}$/), chain: z.string().max(24).optional() }],
  ["genesis402_screen_sanctions", "screen-sanctions", "Screens one address against OFAC SDN digital-currency entries and community blocklists. Not a KYC decision; absence is not clearance.", { address: z.string().min(4).max(120) }],
  ["genesis402_multi_chain_scan", "evm-multi-chain-scan", "One call across 10 EVM chains for a single address.", { address: z.string().regex(/^0x[0-9a-fA-F]{40}$/) }],
  ["genesis402_defi_yields", "defi-yields", "Best DeFi yields from 15,000+ pools, filterable by chain, protocol, token, stablecoin-only and minimum TVL.", { chain: z.string().max(40).optional(), token: z.string().max(20).optional(), project: z.string().max(60).optional(), stablecoin_only: z.boolean().optional(), min_tvl_usd: z.number().optional(), sort: z.enum(["apy", "tvl"]).optional(), limit: z.number().int().min(1).max(100).optional() }],
  ["genesis402_sec_financials", "sec-financials", "As-reported fundamentals for a US public company from SEC XBRL (revenue, net income, assets, cash, EPS).", { ticker: z.string().max(60).optional(), cik: z.string().max(10).optional() }],
  ["genesis402_email_check", "email-domain-check", "Email/domain deliverability: MX, provider, SPF, DMARC policy, disposable flag, trust score.", { email_or_domain: z.string().max(320) }],
  ["genesis402_whois", "whois-domain", "Domain registrar, age, expiry, status and nameservers via RDAP (young domains are a fraud signal).", { domain: z.string().max(260) }],
  ["genesis402_extract_json", "text-extract-json", "Extract YOUR fields from any text as JSON; missing fields are null, never invented.", { text: z.string().max(16000), fields: z.record(z.string()) }],
  ["genesis402_web_extract", "web-extract", "Any public web page as clean text, title, headings and links.", { url: z.string().url(), max_chars: z.number().int().min(500).max(60000).optional() }],
  ["genesis402_prove", "prove", "Signed Ed25519 receipt binding your SHA-256 digest (or text) and optional claim to a settled payment. Verifiable offline.", { sha256: z.string().regex(/^[0-9a-f]{64}$/).optional(), text: z.string().max(16384).optional(), claim: z.string().max(512).optional() }]
];
const PAY_ARG = { payment_signature: z.string().max(8000).optional().describe("Optional. An x402 v2 payment you signed for this call's quote (the PAYMENT-SIGNATURE header value). Omit to get the price quote first.") };

export async function createServer({ payer = null, hosted = false } = {}) {
  await loadCatalog();
  const server = new McpServer({ name: "genesis402", version: VERSION });
  const ro = { readOnlyHint: true, openWorldHint: true };
  server.registerTool("genesis402_catalog", { title: "List endpoints and prices (free)", description: "Free. Every Genesis402 endpoint with price, title, path and parameters, from the live /.well-known/x402 manifest. Call first.", inputSchema: { filter: z.string().max(60).optional() }, annotations: ro }, async ({ filter }) => {
    await loadCatalog(); const f = (filter || "").toLowerCase();
    const rows = [...CATALOG.values()].filter((s) => !f || `${s.name} ${s.title || ""} ${(s.tags || []).join(" ")}`.toLowerCase().includes(f)).map((s) => ({ name: s.name, path: pathOf(s.name), price_usd: priceOf(s.name), title: s.title, parameters: s.parameters }));
    return text({ origin: ORIGIN, total_endpoints: CATALOG.size, matched: rows.length, payer_mode: payer ? `LIVE (cap $${payer.maxUsd})` : hosted ? "HOSTED: quote, or pay with your own signed payment_signature" : "QUOTE_ONLY", endpoints: rows.slice(0, 400) });
  });
  server.registerTool("genesis402_receipt", { title: "Look up a receipt (free)", description: "Free. A paid-call receipt by id from the rail's public receipts feed.", inputSchema: { receipt_id: z.string().min(4).max(80) }, annotations: ro }, async ({ receipt_id }) => {
    const r = await fetch(`${ORIGIN}/receipts/${encodeURIComponent(receipt_id)}`); return r.ok ? text(await r.json()) : fail({ status: r.status });
  });
  server.registerTool("genesis402_call", { title: "Call any endpoint", description: `Calls any of the ${CATALOG.size || 360} Genesis402 endpoints by name (DeFi, SEC filings, research, web/domain intel, AI text tools, multi-chain reads). Prices $0.001-$0.25 USDC on Base. Without payment it returns the exact quote.`, inputSchema: { endpoint: z.string().min(2).max(64), params: z.record(z.any()).optional(), ...PAY_ARG }, annotations: { readOnlyHint: false, openWorldHint: true } }, async ({ endpoint, params, payment_signature }) => {
    await loadCatalog(); const name = String(endpoint).replace(/^\//, "");
    if (CATALOG_OK && !CATALOG.has(name)) return fail({ error: "unknown_endpoint", endpoint: name, did_you_mean: [...CATALOG.keys()].filter((k) => k.includes(name) || name.includes(k)).slice(0, 8) });
    return callPaid(name, params || {}, payer, payment_signature);
  });
  for (const [tool, endpoint, blurb, schema] of NAMED) {
    if (CATALOG_OK && !CATALOG.has(endpoint)) continue;
    server.registerTool(tool, { title: blurb.split(':')[0].slice(0, 60), description: `${priceTag(endpoint)}. ${blurb}`, inputSchema: { ...schema, ...PAY_ARG }, annotations: { readOnlyHint: false, openWorldHint: true } }, async ({ payment_signature, ...p }) => callPaid(endpoint, p, payer, payment_signature));
  }
  return server;
}
