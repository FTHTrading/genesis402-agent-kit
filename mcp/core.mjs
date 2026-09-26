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

export const VERSION = "0.3.2";
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

const EVM_ADDR = z.string().regex(/^0x[0-9a-fA-F]{40}$/);
const CHAIN = z.string().max(24).optional().describe("Optional EVM chain name to focus on, e.g. ethereum or base. Omit to cover all supported chains.");
const NAMED = [
  ["genesis402_wallet_brief", "wallet-brief",
    "Wallet risk signals in ONE call: public sanctions-list check, 10-chain EVM balance/activity scan and a plain-language summary, with an evidence hash. Use before sending funds to or accepting funds from an unknown address. The result lists its sources and an evidence hash. Heuristic signals from public data: not KYC, not a compliance determination, not legal advice. For a sanctions check alone use genesis402_screen_sanctions (cheaper); for raw balances alone use genesis402_multi_chain_scan.",
    { address: EVM_ADDR.describe("The EVM wallet address to check, 0x followed by 40 hex characters."), chain: CHAIN }],
  ["genesis402_token_brief", "token-brief",
    "Token pre-trade check in ONE call for an ERC-20 contract: metadata, price, holder concentration, contract verification and a sanctions-list check. Use before buying, listing or accepting an unfamiliar token. The result lists its sources and an evidence hash. Signals only, not investment advice.",
    { contract: EVM_ADDR.describe("The ERC-20 token contract address, 0x followed by 40 hex characters (not a wallet address)."), chain: CHAIN }],
  ["genesis402_screen_sanctions", "screen-sanctions",
    "Public-data sanctions-list signal for one address on any chain: OFAC SDN digital-currency entries plus community blocklists. Use as a fast first gate on a counterparty address. The result lists its sources and an evidence hash. Automated heuristic signal from public data: not KYC, not a compliance determination, not legal advice. For a fuller risk picture use genesis402_wallet_brief.",
    { address: z.string().min(4).max(120).describe("The crypto address to screen, as a string (4 to 120 characters).") }],
  ["genesis402_multi_chain_scan", "evm-multi-chain-scan",
    "Reads ONE address across 10 EVM chains in a single call, showing where it holds funds and is active. Use to find where an address is active before drilling into one chain. Public on-chain data with sources and an evidence hash; no risk scoring (use genesis402_wallet_brief for that).",
    { address: EVM_ADDR.describe("The EVM address to scan, 0x followed by 40 hex characters.") }],
  ["genesis402_defi_yields", "defi-yields",
    "Ranked DeFi yield opportunities from 15,000+ pools, filterable by chain, protocol, token, stablecoin-only and minimum TVL. Use to answer 'where is the best yield for X'. Returns up to `limit` pools with protocol, chain, token, APY and TVL, plus sources and an evidence hash. APYs are variable and backward-looking; not investment advice.",
    { chain: z.string().max(40).optional().describe("Filter to one chain by name, e.g. Ethereum, Base, Arbitrum. Omit for all chains."),
      token: z.string().max(20).optional().describe("Filter to pools containing this token symbol, e.g. USDC, ETH, WBTC. Case-insensitive."),
      project: z.string().max(60).optional().describe("Filter to one protocol by its slug, e.g. aave-v3. Omit for all protocols."),
      stablecoin_only: z.boolean().optional().describe("true = only stablecoin pools."),
      min_tvl_usd: z.number().optional().describe("Minimum pool TVL in USD, e.g. 1000000 to skip small pools."),
      sort: z.enum(["apy", "tvl"]).optional().describe("Sort order: 'apy' (highest yield first) or 'tvl' (largest pool first)."),
      limit: z.number().int().min(1).max(100).optional().describe("How many pools to return, 1 to 100.") }],
  ["genesis402_sec_financials", "sec-financials",
    "As-reported fundamentals for a US public company from SEC XBRL filings: revenue, net income, assets, cash and EPS. Use for quick fundamentals without a data vendor. Give either ticker or cik (one is required). Values are as filed, with sources and an evidence hash; not investment advice.",
    { ticker: z.string().max(60).optional().describe("Stock ticker, e.g. AAPL or MSFT. Provide this or cik."),
      cik: z.string().max(10).optional().describe("SEC Central Index Key (CIK), up to 10 digits. Provide this or ticker.") }],
  ["genesis402_email_check", "email-domain-check",
    "Email/domain deliverability and trust check: MX, mail provider, SPF, DMARC policy, disposable-domain flag and a trust score. Use to vet a signup email or an inbound sender's domain. Does not send any email.",
    { email_or_domain: z.string().max(320).describe("A full email address (name@example.com) or a bare domain (example.com).") }],
  ["genesis402_whois", "whois-domain",
    "Domain registration facts via RDAP: registrar, age, expiry, status and nameservers. Young domains are a common fraud signal. Use to vet a website or sender domain.",
    { domain: z.string().max(260).describe("The domain to look up, e.g. example.com (no scheme or path).") }],
  ["genesis402_extract_json", "text-extract-json",
    "Extract the fields YOU name from any text as JSON. Fields not present in the text come back null, never invented. Use to turn emails, invoices or pages into structured data.",
    { text: z.string().max(16000).describe("The source text to read, up to 16,000 characters."),
      fields: z.record(z.string()).describe("The fields to extract: an object mapping each field name to a short description, e.g. { \"invoice_total\": \"total amount due in USD\", \"due_date\": \"ISO date\" }.") }],
  ["genesis402_web_extract", "web-extract",
    "Fetch one public web page and return it as clean text with its title, headings and links. Use when you need a page's content rather than a summary. Pages behind a login or paywall are not accessible.",
    { url: z.string().url().describe("Full http(s) URL of the public page to fetch."),
      max_chars: z.number().int().min(500).max(60000).optional().describe("Maximum characters of text to return, 500 to 60,000.") }],
  ["genesis402_prove", "prove",
    "Issue a signed Ed25519 receipt that binds your SHA-256 digest (or text, which is hashed for you) and an optional claim to a settled payment and timestamp. Use as a cheap timestamped proof that you held a document or statement. Give sha256 or text. The receipt signature verifies offline; the receipt chain is not externally anchored on-chain.",
    { sha256: z.string().regex(/^[0-9a-f]{64}$/).optional().describe("Lowercase hex SHA-256 digest (64 characters) of the content to prove. Provide this or text."),
      text: z.string().max(16384).optional().describe("Raw text to hash and prove, up to 16,384 characters. Provide this or sha256."),
      claim: z.string().max(512).optional().describe("Optional short statement bound into the receipt, e.g. 'Draft v2 of the purchase agreement'.") }]
];
const PAY_ARG = { payment_signature: z.string().max(8000).optional().describe("Optional. An x402 v2 payment you signed for this call's quote (the PAYMENT-SIGNATURE header value). Omit to get the price quote first; nothing is charged without it.") };

export async function createServer({ payer = null, hosted = false } = {}) {
  await loadCatalog();
  const server = new McpServer({ name: "genesis402", version: VERSION });
  const ro = { readOnlyHint: true, openWorldHint: true };
  server.registerTool("genesis402_catalog", { title: "List endpoints and prices (free)", description: "Free, no payment. Lists every Genesis402 endpoint with its name, HTTP path, USD price, title and parameter schema, read from the live /.well-known/x402 manifest. Call this first to find the right endpoint name and parameters for genesis402_call. Returns the total count, the matches and the current payer mode.", inputSchema: { filter: z.string().max(60).optional().describe("Optional keyword to narrow the list, matched against endpoint name, title and tags, e.g. \"defi\", \"sec\", \"price\". Omit for all endpoints.") }, annotations: ro }, async ({ filter }) => {
    await loadCatalog(); const f = (filter || "").toLowerCase();
    const rows = [...CATALOG.values()].filter((s) => !f || `${s.name} ${s.title || ""} ${(s.tags || []).join(" ")}`.toLowerCase().includes(f)).map((s) => ({ name: s.name, path: pathOf(s.name), price_usd: priceOf(s.name), title: s.title, parameters: s.parameters }));
    return text({ origin: ORIGIN, total_endpoints: CATALOG.size, matched: rows.length, payer_mode: payer ? `LIVE (cap $${payer.maxUsd})` : hosted ? "HOSTED: quote, or pay with your own signed payment_signature" : "QUOTE_ONLY", endpoints: rows.slice(0, 400) });
  });
  server.registerTool("genesis402_receipt", { title: "Look up a receipt (free)", description: "Free, no payment. Fetches one paid-call receipt by id from the rail's public receipts feed, to confirm a call was paid and delivered. Returns the receipt as JSON, or an error with the HTTP status if the id is not found.", inputSchema: { receipt_id: z.string().min(4).max(80).describe("The receipt id returned with a paid call result.") }, annotations: ro }, async ({ receipt_id }) => {
    const r = await fetch(`${ORIGIN}/receipts/${encodeURIComponent(receipt_id)}`); return r.ok ? text(await r.json()) : fail({ status: r.status });
  });
  server.registerTool("genesis402_call", { title: "Call any endpoint", description: `Calls any of the ${CATALOG.size || 360} Genesis402 endpoints by name (DeFi, SEC filings, research, web/domain intel, AI text tools, multi-chain reads). Prices $0.001-$0.25 USDC on Base. Use for any endpoint without a dedicated tool; look up the name and parameters with genesis402_catalog first. Parameters are validated for free before any quote. Without payment_signature (or a local payer) it returns the exact price quote and signs nothing; with payment it returns the result plus the settlement details.`, inputSchema: { endpoint: z.string().min(2).max(64).describe("Endpoint name exactly as listed by genesis402_catalog, e.g. \"defi-yields\" or \"wallet-brief\" (a leading slash is ignored)."), params: z.record(z.any()).optional().describe("Parameters for that endpoint as an object, matching the parameter schema shown in genesis402_catalog. Omit if the endpoint takes none."), ...PAY_ARG }, annotations: { readOnlyHint: false, openWorldHint: true } }, async ({ endpoint, params, payment_signature }) => {
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
