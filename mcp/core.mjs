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

export const VERSION = "0.3.7";
const ORIGIN = (process.env.GENESIS402_ORIGIN || "https://twin.unykorn.org").replace(/\/$/, "");
const BASE = "eip155:8453";
const BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

const text = (obj) => ({ content: [{ type: "text", text: typeof obj === "string" ? obj : JSON.stringify(obj, null, 2) }] });
const fail = (obj) => ({ ...text(obj), isError: true });
// Success results carry the same object twice: as structuredContent (validated
// against the tool's outputSchema) and as its JSON text for clients that only read text.
const ok = (obj) => ({ ...text(obj), structuredContent: obj });

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
    return res.ok ? ok({ mode: "PAID", paid: true, price: null, settlement, result: body }) : fail({ status: res.status, paid: false, ...body });
  }

  const probe = await fetch(url, init);
  const probeBody = await probe.json().catch(() => null);
  if (probe.status !== 402) return probe.ok ? ok({ mode: "FREE", paid: false, result: probeBody }) : fail({ status: probe.status, ...probeBody });
  const challenge = decodeChallenge(probe, probeBody);
  const quote = baseQuote(challenge);

  if (!payer) {
    return ok({
      mode: "QUOTE_ONLY", paid: false, resource: url, price: quote,
      also_accepted_networks: (challenge?.accepts || []).map((a) => a.network),
      what_you_get: challenge?.resource?.description || challenge?.resource?.what_you_get,
      payment_required_b64: probe.headers.get("payment-required") || null,
      how_to_pay: "Sign an x402 v2 payment for this quote with your own wallet (e.g. @x402/fetch, Coinbase AgentKit/CDP wallet) and call this tool again with payment_signature=<the PAYMENT-SIGNATURE header value>. Or call " + url + " directly over HTTP with any x402 client. This server never holds your keys."
    });
  }
  if (!quote) return fail({ error: "no_base_usdc_lane" });
  if (quote.usd > payer.maxUsd) return fail({ error: "price_above_cap", quote, cap_usd: payer.maxUsd });
  let res;
  try { res = await payer.paidFetch(url, init); }
  catch (e) { return fail({ error: "payment_not_made", message: String(e?.message || e).slice(0, 300), quote, cap_usd: payer.maxUsd }); }
  const body = await res.json().catch(() => null);
  let settlement = null; const pr = res.headers.get("payment-response") || res.headers.get("x-payment-response");
  if (pr) { try { settlement = decodePaymentResponseHeader(pr); } catch {} }
  return res.ok ? ok({ mode: "PAID", paid: true, price: quote, settlement, result: body }) : fail({ status: res.status, paid: false, ...body });
}

export function localPayerFromEnv() {
  const key = process.env.GENESIS402_PAYER_KEY || "";
  if (process.env.GENESIS402_LIVE !== "1" || !key) return null;
  const account = privateKeyToAccount(key.startsWith("0x") ? key : `0x${key}`);
  // The cap is enforced twice: against the quote before paying, and by the x402 client's
  // spendControls when it signs, so a price that changes between quote and payment is refused.
  // GENESIS402_MAX_USD defaults to $0.25. A non-numeric, zero or negative value never disables the cap:
  // the server stays quote-only and signs nothing.
  const maxUsd = Number(process.env.GENESIS402_MAX_USD ?? "0.25");
  if (!Number.isFinite(maxUsd) || maxUsd <= 0) {
    process.stderr.write(`[genesis402] GENESIS402_MAX_USD="${process.env.GENESIS402_MAX_USD}" is not a positive number; staying quote-only\n`);
    return null;
  }
  return { paidFetch: wrapFetchWithPaymentFromConfig(fetch, { schemes: [{ network: BASE, client: new ExactEvmScheme(account) }], spendControls: { maxAmountPerPayment: `$${maxUsd}` } }), maxUsd };
}

// ---------------------------------------------------------------------------
// Output schemas. Every successful call returns structuredContent matching one of these.
// ---------------------------------------------------------------------------
const QUOTE = z.object({
  atomic: z.string().describe("Price in USDC atomic units (6 decimals), as a decimal string."),
  usd: z.number().describe("Price in US dollars."),
  payTo: z.string().describe("Address the payment settles to."),
  network: z.string().describe("CAIP-2 network id of the lane, eip155:8453 (Base).")
});
const PAID_OUTPUT = {
  mode: z.enum(["QUOTE_ONLY", "PAID", "FREE"]).describe("QUOTE_ONLY: price quote only, nothing signed or charged. PAID: the call was paid and result holds the data. FREE: the endpoint answered without payment."),
  paid: z.boolean().describe("true only when a payment was settled for this call."),
  price: QUOTE.nullable().optional().describe("The Base USDC quote. Present on QUOTE_ONLY and on PAID calls made by a local payer; null when the rail offered no Base USDC lane or the caller signed the payment."),
  resource: z.string().optional().describe("QUOTE_ONLY: the full URL of the paid resource."),
  also_accepted_networks: z.array(z.string()).optional().describe("QUOTE_ONLY: every network the 402 challenge accepts."),
  what_you_get: z.string().optional().describe("QUOTE_ONLY: the rail's description of what the payment buys."),
  payment_required_b64: z.string().nullable().optional().describe("QUOTE_ONLY: the raw base64 PAYMENT-REQUIRED header, to sign with any x402 v2 client."),
  how_to_pay: z.string().optional().describe("QUOTE_ONLY: how to sign and resend the call."),
  settlement: z.any().optional().describe("PAID: decoded PAYMENT-RESPONSE settlement (transaction hash, network, payer), or null if the rail sent none."),
  result: z.any().optional().describe("PAID or FREE: the endpoint's JSON result.")
};
const CATALOG_OUTPUT = {
  origin: z.string().describe("Rail origin the catalog was read from."),
  total_endpoints: z.number().int().describe("Number of endpoints in the live manifest."),
  matched: z.number().int().describe("Number of endpoints matching the filter."),
  payer_mode: z.string().describe("QUOTE_ONLY, HOSTED, or LIVE (cap $X) for a local self-paying server."),
  endpoints: z.array(z.object({
    name: z.string().describe("Endpoint name to pass to genesis402_call."),
    path: z.string().describe("HTTP path on the rail."),
    price_usd: z.number().nullable().describe("Price per call in USD, null if the manifest carries none."),
    title: z.string().optional().describe("Short human title."),
    parameters: z.any().optional().describe("Parameter schema from the manifest.")
  })).describe("Matching endpoints, at most 400.")
};
const RECEIPT_OUTPUT = {
  receipt_id: z.string().describe("The id that was looked up."),
  receipt: z.record(z.any()).describe("The receipt exactly as published on the rail's receipts feed.")
};

const EVM_ADDR = z.string().regex(/^0x[0-9a-fA-F]{40}$/);
const CHAIN = z.string().max(24).optional().describe("Optional EVM chain name to focus on, e.g. ethereum or base. Omit to cover all supported chains.");
const NAMED = [
  ["genesis402_wallet_brief", "wallet-brief",
    "Full risk brief for one EVM wallet in a single call: sanctions-list check (OFAC SDN digital-currency entries), native balance and activity scan across 10 EVM chains, recent activity on the chosen chain and a plain-language summary. Use before sending funds to, or accepting funds from, an unknown address. For a sanctions check alone use genesis402_screen_sanctions (cheaper, any chain); for balances without risk signals use genesis402_multi_chain_scan; for a token contract use genesis402_token_brief. Returns each part with its own sources (a part that cannot be read is marked unavailable with the reason, never zeroed) and an evidence hash over all parts. Heuristic signals from public data: not KYC, not a compliance determination, not legal advice.",
    { address: EVM_ADDR.describe("The EVM wallet address to check, 0x followed by 40 hex characters. Checksum case is optional."), chain: CHAIN }],
  ["genesis402_token_brief", "token-brief",
    "Pre-trade check for one ERC-20 token contract in a single call: name, symbol, decimals and supply, current USD price, top-holder concentration, source-verification status and a sanctions-list check on the contract address. Use before buying, listing or accepting an unfamiliar token. For a wallet rather than a token use genesis402_wallet_brief. Returns each part with its sources (unreadable parts are marked unavailable, never zeroed) and an evidence hash. Signals only, not investment advice.",
    { contract: EVM_ADDR.describe("The ERC-20 token contract address, 0x followed by 40 hex characters. Must be the token contract, not a holder's wallet."),
      chain: z.enum(["base", "ethereum", "polygon", "arbitrum", "optimism"]).optional().describe("Chain the token contract lives on. Defaults to base when omitted.") }],
  ["genesis402_screen_sanctions", "screen-sanctions",
    "List lookup for one address on any chain against OFAC SDN digital-currency entries, community scam-address blocklists and the Blockchain Fraud case registry. Use as a fast, cheap first gate on a counterparty address. For balances plus sanctions plus a summary on an EVM address use genesis402_wallet_brief. Returns whether any list matched, and for every list consulted its dataset name, fetch URL and entry count, plus an evidence hash. A hit means the address appears on a published list; absence from every list is not a clearance. Not a compliance decision, not a finding about any person, not legal advice.",
    { address: z.string().min(4).max(120).describe("The crypto address to screen, on any chain, exactly as written on that chain (4 to 120 characters, no spaces).") }],
  ["genesis402_multi_chain_scan", "evm-multi-chain-scan",
    "Sweeps one address across 10 EVM chains at once and reports where it is actually active. Use to find which networks an address uses before drilling into one chain. No risk scoring or sanctions check: for that use genesis402_wallet_brief. Returns, per chain, the native balance, the number of transactions sent and whether contract bytecode is deployed; chains whose RPC did not answer are listed separately, never counted as inactive. Activity is inferred from nonce, balance and code, so an address that only ever received tokens can show a zero nonce; check its balance field. Public on-chain data with sources and an evidence hash.",
    { address: EVM_ADDR.describe("The EVM address to scan, 0x followed by 40 hex characters. Checksum case is optional.") }],
  ["genesis402_defi_yields", "defi-yields",
    "Ranked DeFi yield pools from DefiLlama (15,000+ pools), filterable by chain, protocol, token, stablecoin-only and minimum TVL. Use to answer 'where is the best yield for X'. For other DeFi or market data search genesis402_catalog and call the endpoint through genesis402_call. All filters are optional and combine with AND; with none set it returns the top pools above $1M TVL. Returns up to limit pools, each with protocol, chain, symbol, TVL, base APY vs reward APY, 30-day mean APY, impermanent-loss risk, exposure and DefiLlama's outlook class. APYs are variable and backward-looking; not investment advice.",
    { chain: z.string().max(40).optional().describe("Filter to one chain by name, e.g. Ethereum, Base, Arbitrum. Omit for all chains."),
      token: z.string().max(20).optional().describe("Token symbol that must be in the pool, e.g. USDC, ETH, WBTC."),
      project: z.string().max(60).optional().describe("Filter to one protocol by its DefiLlama slug, e.g. aave-v3. Omit for all protocols."),
      stablecoin_only: z.boolean().optional().describe("true = only stablecoin pools. Omit or false for all pools."),
      min_tvl_usd: z.number().optional().describe("Minimum pool TVL in US dollars as a plain number. Defaults to 1000000 ($1M) when omitted."),
      sort: z.enum(["apy", "tvl"]).optional().describe("Sort order: 'apy' (highest yield first) or 'tvl' (largest pool first)."),
      limit: z.number().int().min(1).max(100).optional().describe("How many pools to return, 1 to 100.") }],
  ["genesis402_sec_financials", "sec-financials",
    "As-reported fundamentals for one SEC filer from XBRL company facts. Use for quick fundamentals without a data vendor. For other SEC data search genesis402_catalog for \"sec\" and use genesis402_call. Give ticker or cik; one is required. Returns the latest annual and latest quarterly values for revenue, net income, operating income, total assets, liabilities, equity, cash, operating cash flow and diluted EPS, each with period end and filing date. Missing tags are marked unavailable, never estimated. Not investment advice.",
    { ticker: z.string().max(60).optional().describe("Stock ticker, e.g. COIN or AAPL. Provide this or cik."),
      cik: z.string().max(10).optional().describe("SEC Central Index Key, digits only, up to 10, e.g. 320193. Provide this or ticker.") }],
  ["genesis402_email_check", "email-domain-check",
    "Deliverability and trust check for an email address or domain. Use to vet a signup email or an inbound sender. For the domain's registration age and registrar use genesis402_whois. Returns whether it can receive mail (MX), the hosting provider, SPF and DMARC records and policy, a disposable-domain flag and a 0-5 trust score. DNS-only: no SMTP probe, no email sent, the mailbox is never contacted.",
    { email_or_domain: z.string().max(320).describe("A full email address (user@domain.com) or a bare domain (domain.com), without scheme or path.") }],
  ["genesis402_whois", "whois-domain",
    "Registry data for one domain via RDAP. Young domains are a common phishing and fraud signal. Use to vet a website or counterparty domain. For mail setup (MX, SPF, DMARC) use genesis402_email_check; for page content use genesis402_web_extract. Returns registrar, registration and expiry dates, domain age in days, days to expiry, status codes, nameservers and DNSSEC state.",
    { domain: z.string().max(260).describe("The domain to look up, e.g. example.com. No scheme (https://), path or port.") }],
  ["genesis402_extract_json", "text-extract-json",
    "Pull the fields you define out of unstructured text as JSON. Anything the text does not state comes back null, never invented. Use to turn emails, invoices, contracts or listings into structured data. To get a web page's text first use genesis402_web_extract. The field descriptions steer the extraction, so include units and formats (e.g. \"ISO date\", \"number in USD\"). Returns one JSON object with exactly your keys, each holding the extracted value or null.",
    { text: z.string().max(16000).describe("The source text to read, up to 16,000 characters. Trim longer input first."),
      fields: z.record(z.string()).describe("Up to 30 fields: an object mapping each output key to a short description of what it should hold, including unit or format, e.g. { \"invoice_total\": \"total amount due in USD, number\", \"due_date\": \"ISO date\" }.") }],
  ["genesis402_web_extract", "web-extract",
    "Server-side fetch of one public web page, returned as clean readable text. Use when you need a page's actual content. To pull specific fields out of the result use genesis402_extract_json; for domain registration facts use genesis402_whois. Returns the title, meta description, readable text with markdown-style headings (capped at max_chars), up to 40 headings, up to 50 absolute links, the final URL after redirects and a SHA-256 of the fetched bytes. No JavaScript is executed, so client-rendered pages may come back thin. Private or internal addresses are refused, and pages behind a login or paywall are not accessible.",
    { url: z.string().url().describe("Full URL of one public page, starting with http:// or https://."),
      max_chars: z.number().int().min(500).max(60000).optional().describe("Maximum characters of page text to return, 500 to 60,000. Defaults to 20,000. Text past the cap is cut off."),
      include_links: z.boolean().optional().describe("Include the page's links in the result. Defaults to true; set false for text only.") }],
  ["genesis402_prove", "prove",
    "Issue one Ed25519-signed, hash-chained receipt recording that a SHA-256 digest existed at the time of payment. Use as a cheap timestamped proof that you held a document or statement. To look the receipt up later use genesis402_receipt (free). Send text (hashed by the rail, labelled OBSERVED) or a sha256 you computed yourself (labelled ATTESTED; keeps the content private), not both. Returns the receipt bound to your payment transaction, with its truth labels and limitations. It verifies offline with the open verifier at github.com/FTHTrading/402-truth. The rail stores the receipt, never your bytes. Not externally anchored on a public blockchain yet.",
    { sha256: z.string().regex(/^[0-9a-f]{64}$/).optional().describe("Lowercase hex SHA-256 digest (64 characters) of the content to prove. Provide this or text, not both."),
      text: z.string().max(16384).optional().describe("Raw UTF-8 text to hash and prove, up to 16 KiB (16,384 characters). Provide this or sha256, not both."),
      claim: z.string().max(512).optional().describe("Optional short statement bound into the signed receipt, up to 512 characters, e.g. 'Draft v2 of the purchase agreement'.") }],
  ["genesis402_summarize", "text-summarize",
    "Summarize long text you already have (a report, transcript, filing, thread) as a paragraph, bullet list or one-line TL;DR, within a word limit, on UnyKorn's own GPU. Use when the text is in hand; to summarize a live web page by URL instead, call the summarize-url endpoint through genesis402_call. To answer one specific question from the text use genesis402_answer_from_text. Returns the summary in the requested style as JSON. Works only from the supplied text.",
    { text: z.string().min(1).max(24000).describe("The text to summarize, up to 24,000 characters. Trim longer input first."),
      style: z.enum(["paragraph", "bullets", "tldr"]).optional().describe("Output shape: 'paragraph' (default), 'bullets' for a bullet list, or 'tldr' for one or two sentences."),
      max_words: z.number().int().min(20).max(400).optional().describe("Upper bound on summary length in words, 20 to 400. Defaults to 120.") }],
  ["genesis402_answer_from_text", "text-qa",
    "Answer one question strictly from a document you supply, and return the verbatim quote that supports the answer. Use for contract, policy, filing or email questions where the answer must be traceable to the source. For an overview rather than one answer use genesis402_summarize; to pull many named fields at once use genesis402_extract_json. Returns the answer and a supporting quote that the rail verifies appears in the text.",
    { text: z.string().min(1).max(24000).describe("The source document to answer from, up to 24,000 characters."),
      question: z.string().min(3).max(500).describe("One question about the text, up to 500 characters, e.g. 'What is the termination notice period?'.") }],
  ["genesis402_translate", "text-translate",
    "Translate text into any major language while preserving names, numbers and formatting. Use for customer messages, documents or UI strings. To only identify a text's language call the language-detect endpoint through genesis402_call. Returns the translation as JSON.",
    { text: z.string().min(1).max(8000).describe("The text to translate, up to 8,000 characters."),
      to: z.string().min(2).max(40).describe("Target language as a name or ISO 639-1 code, e.g. 'Spanish' or 'es'."),
      from: z.string().min(2).max(40).optional().describe("Source language as a name or ISO code. Omit to auto-detect.") }],
  ["genesis402_paper_search", "openalex-search",
    "Search scholarly literature in OpenAlex and get works with citation counts and open-access links. Use to find research, prior art or the most-cited work on a topic. For preprints by recency search arXiv (arxiv-search) and for one known DOI use crossref-doi, both through genesis402_call. Returns up to limit works, ranked by relevance or by citations.",
    { q: z.string().min(2).max(300).describe("Search terms, e.g. 'zero-knowledge proof rollups' or an exact paper title."),
      from_year: z.number().int().min(1900).max(2100).optional().describe("Only works published in or after this year, e.g. 2020. Omit for all years."),
      sort: z.enum(["relevance", "cited"]).optional().describe("'relevance' (default) or 'cited' for most-cited first."),
      limit: z.number().int().min(1).max(50).optional().describe("How many works to return, 1 to 50. Defaults to 10.") }]
];
const TITLES = {
  genesis402_wallet_brief: "Wallet risk brief", genesis402_token_brief: "Token pre-trade brief",
  genesis402_screen_sanctions: "Screen address against sanctions lists", genesis402_multi_chain_scan: "Scan address across 10 EVM chains",
  genesis402_defi_yields: "Find DeFi yields", genesis402_sec_financials: "Get SEC company financials",
  genesis402_email_check: "Check email or domain deliverability", genesis402_whois: "Look up domain registration (RDAP)",
  genesis402_extract_json: "Extract JSON fields from text", genesis402_web_extract: "Extract text from a web page",
  genesis402_prove: "Issue a signed proof receipt",
  genesis402_summarize: "Summarize text", genesis402_answer_from_text: "Answer a question from a document",
  genesis402_translate: "Translate text", genesis402_paper_search: "Search scholarly papers"
};
const PAY_FLOW = "Payment: call once without payment_signature to get the exact price quote (nothing is charged or signed), then call again with the signed payment to receive the result and settlement details.";
const PAY_ARG = { payment_signature: z.string().max(8000).optional().describe("Optional. An x402 v2 payment you signed for this call's quote (the PAYMENT-SIGNATURE header value). Omit to get the price quote first; nothing is charged without it.") };

const INSTRUCTIONS = [
  "Genesis402 by UnyKorn: 360 pay-per-call data and tool endpoints for agents, settled per call in USDC on Base over x402 ($0.001 to $0.25 each). No accounts or API keys.",
  "Pick a tool: use a dedicated genesis402_* tool when one fits (wallet or token risk, sanctions screening, multi-chain balances, DeFi yields, SEC financials, email and domain checks, web page text, JSON extraction, summaries, answers from a document, translation, scholarly paper search, signed proofs). For anything else, search genesis402_catalog (free) by keyword, then call the endpoint by name with genesis402_call.",
  "Payment: genesis402_catalog and genesis402_receipt are free. Every other tool, called without payment_signature, returns the exact price quote and charges nothing. Show the price to the user before paying. To pay, sign an x402 v2 payment for that quote with the user's own wallet and call the same tool again with payment_signature. A local server started with GENESIS402_LIVE=1 pays by itself, up to GENESIS402_MAX_USD per call.",
  "Parameters are validated for free before any quote, so fix invalid_input errors before asking for payment. Risk and sanctions outputs are heuristic signals from public data, not KYC, compliance determinations, legal or investment advice; say so when you relay them."
].join("\n\n");

export async function createServer({ payer = null, hosted = false } = {}) {
  await loadCatalog();
  const server = new McpServer({ name: "genesis402", title: "Genesis402 by UnyKorn", version: VERSION }, { instructions: INSTRUCTIONS });
  const ro = { readOnlyHint: true, openWorldHint: true };
  server.registerTool("genesis402_catalog", { title: "List endpoints and prices (free)", description: "Free, no payment, read-only. Lists Genesis402 endpoints with name, HTTP path, USD price, title and parameter schema, read from the rail's live /.well-known/x402 manifest (refreshed every 10 minutes). Use it to find an endpoint that has no dedicated genesis402_* tool, then pass its exact name and parameters to genesis402_call. Filter by keyword to keep the list short (e.g. \"defi\", \"sec\", \"wiki\"); with no filter it returns every endpoint, up to 400. Returns the rail origin, total endpoint count, number matched, the current payer mode (QUOTE_ONLY, HOSTED or LIVE with its cap) and the matching endpoints.", inputSchema: { filter: z.string().max(60).optional().describe("Optional keyword to narrow the list, matched against endpoint name, title and tags, e.g. \"defi\", \"sec\", \"price\". Omit for all endpoints.") }, outputSchema: CATALOG_OUTPUT, annotations: ro }, async ({ filter }) => {
    await loadCatalog(); const f = (filter || "").toLowerCase();
    const rows = [...CATALOG.values()].filter((s) => !f || `${s.name} ${s.title || ""} ${(s.tags || []).join(" ")}`.toLowerCase().includes(f)).map((s) => ({ name: s.name, path: pathOf(s.name), price_usd: priceOf(s.name), title: s.title, parameters: s.parameters }));
    return ok({ origin: ORIGIN, total_endpoints: CATALOG.size, matched: rows.length, payer_mode: payer ? `LIVE (cap $${payer.maxUsd})` : hosted ? "HOSTED: quote, or pay with your own signed payment_signature" : "QUOTE_ONLY", endpoints: rows.slice(0, 400) });
  });
  server.registerTool("genesis402_receipt", { title: "Look up a receipt (free)", description: "Free, no payment, read-only and idempotent. Fetches one receipt by id from the rail's public receipts feed (twin.unykorn.org/receipts) to confirm that a paid call was settled and delivered. Use after any paid call, or to check a genesis402_prove receipt, using the receipt id returned with that result. It does not list or search receipts; you need the exact id. Returns the receipt exactly as published: the task called, amount in USD, payment rail, settlement transaction hash, settler, payer and timestamp. An unknown id returns an error with HTTP status 404; nothing is charged either way.", inputSchema: { receipt_id: z.string().min(4).max(80).describe("The receipt id exactly as returned with a paid call result or by genesis402_prove, 4 to 80 characters.") }, outputSchema: RECEIPT_OUTPUT, annotations: ro }, async ({ receipt_id }) => {
    const r = await fetch(`${ORIGIN}/receipts/${encodeURIComponent(receipt_id)}`);
    if (!r.ok) return fail({ status: r.status, receipt_id });
    const receipt = await r.json().catch(() => null);
    return receipt && typeof receipt === "object" && !Array.isArray(receipt) ? ok({ receipt_id, receipt }) : fail({ error: "malformed_receipt", receipt_id });
  });
  server.registerTool("genesis402_call", { title: "Call any endpoint", description: `Calls any of the ${CATALOG.size || 360} Genesis402 endpoints by name (DeFi, SEC filings, research, web/domain intel, AI text tools, multi-chain reads). Prices $0.001-$0.25 USDC on Base. Use for any endpoint without a dedicated tool; look up the name and parameters with genesis402_catalog first. Parameters are validated for free before any quote. Without payment_signature (or a local payer) it returns the exact price quote and signs nothing; with payment it returns the result plus the settlement details.`, inputSchema: { endpoint: z.string().min(2).max(64).describe("Endpoint name exactly as listed by genesis402_catalog, e.g. \"defi-yields\" or \"wallet-brief\" (a leading slash is ignored)."), params: z.record(z.any()).optional().describe("Parameters for that endpoint as an object, matching the parameter schema shown in genesis402_catalog. Omit if the endpoint takes none."), ...PAY_ARG }, outputSchema: PAID_OUTPUT, annotations: { readOnlyHint: false, openWorldHint: true } }, async ({ endpoint, params, payment_signature }) => {
    await loadCatalog(); const name = String(endpoint).replace(/^\//, "");
    if (CATALOG_OK && !CATALOG.has(name)) return fail({ error: "unknown_endpoint", endpoint: name, did_you_mean: [...CATALOG.keys()].filter((k) => k.includes(name) || name.includes(k)).slice(0, 8) });
    return callPaid(name, params || {}, payer, payment_signature);
  });
  for (const [tool, endpoint, blurb, schema] of NAMED) {
    if (CATALOG_OK && !CATALOG.has(endpoint)) continue;
    server.registerTool(tool, { title: TITLES[tool] || tool, description: `${priceTag(endpoint)}. ${blurb} ${PAY_FLOW}`, inputSchema: { ...schema, ...PAY_ARG }, outputSchema: PAID_OUTPUT, annotations: { readOnlyHint: false, openWorldHint: true } }, async ({ payment_signature, ...p }) => callPaid(endpoint, p, payer, payment_signature));
  }
  return server;
}
