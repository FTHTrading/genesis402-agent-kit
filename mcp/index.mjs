#!/usr/bin/env node
// Genesis402 MCP server: the six paid services on https://twin.unykorn.org as MCP tools.
//
// Safe by default:
//   - Without GENESIS402_LIVE=1 a paid tool only returns the price quote (the 402 challenge). Nothing is signed.
//   - With GENESIS402_LIVE=1 and GENESIS402_PAYER_KEY set, it pays in USDC on Base (eip155:8453) through the
//     standard x402 v2 flow, refusing any quote above GENESIS402_MAX_USD (default 0.25).
//   - The server sets the price. This client never does; it only decides whether to accept it.
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { wrapFetchWithPaymentFromConfig, decodePaymentResponseHeader } from "@x402/fetch";
import { ExactEvmScheme } from "@x402/evm";
import { privateKeyToAccount } from "viem/accounts";

const ORIGIN = (process.env.GENESIS402_ORIGIN || "https://twin.unykorn.org").replace(/\/$/, "");
const LIVE = process.env.GENESIS402_LIVE === "1";
const KEY = process.env.GENESIS402_PAYER_KEY || "";
const MAX_USD = Number(process.env.GENESIS402_MAX_USD || "0.25");
const BASE = "eip155:8453";
const BASE_USDC = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";

const account = KEY ? privateKeyToAccount(KEY.startsWith("0x") ? KEY : `0x${KEY}`) : null;
const paidFetch = account
  ? wrapFetchWithPaymentFromConfig(fetch, { schemes: [{ network: BASE, client: new ExactEvmScheme(account) }] })
  : null;

const text = (obj) => ({ content: [{ type: "text", text: typeof obj === "string" ? obj : JSON.stringify(obj, null, 2) }] });
const fail = (obj) => ({ ...text(obj), isError: true });

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

async function callPaid(path, params) {
  const url = ORIGIN + path;
  const init = { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ params }) };

  // 1. Ask without paying. A free 4xx (bad params) is returned before any money is involved.
  const probe = await fetch(url, init);
  const probeBody = await probe.json().catch(() => null);
  if (probe.status !== 402) {
    return probe.ok ? text(probeBody) : fail({ status: probe.status, ...probeBody });
  }
  const challenge = decodeChallenge(probe, probeBody);
  const quote = baseQuote(challenge);
  const other = (challenge?.accepts || []).map((a) => a.network);

  if (!LIVE || !paidFetch) {
    return text({
      mode: "QUOTE_ONLY",
      why: !paidFetch ? "GENESIS402_PAYER_KEY is not set" : "GENESIS402_LIVE is not 1",
      resource: url,
      price: quote,
      also_accepted_networks: other,
      what_you_get: challenge?.resource?.what_you_get || challenge?.resource?.description,
    });
  }
  if (!quote) return fail({ error: "no_base_usdc_lane", accepted_networks: other });
  if (quote.usd > MAX_USD) return fail({ error: "price_above_cap", quote, cap_usd: MAX_USD });

  // 2. Pay through the standard x402 v2 client and retry.
  const res = await paidFetch(url, init);
  const body = await res.json().catch(() => null);
  let settlement = null;
  const pr = res.headers.get("payment-response") || res.headers.get("x-payment-response");
  if (pr) { try { settlement = decodePaymentResponseHeader(pr); } catch {} }
  return res.ok ? text({ paid: quote, settlement, result: body }) : fail({ status: res.status, paid: false, ...body });
}

const server = new McpServer({ name: "genesis402", version: "0.1.0" });

server.tool("genesis402_catalog", "Free. Lists the paid Genesis402 services, prices and accepted payment networks from the live /.well-known/x402 manifest.", {},
  async () => {
    const j = await (await fetch(ORIGIN + "/.well-known/x402")).json();
    return text({ provider: j.provider, services: (j.services || []).map((s) => ({ name: s.name, endpoint: s.endpoint, price: s.price, parameters: s.parameters })), payer_mode: LIVE && paidFetch ? `LIVE (cap $${MAX_USD})` : "QUOTE_ONLY" });
  });

server.tool("genesis402_receipt", "Free. Looks up a paid-call receipt by id on the rail's public receipts feed.", { receipt_id: z.string().min(4).max(80) },
  async ({ receipt_id }) => {
    const r = await fetch(`${ORIGIN}/receipts/${encodeURIComponent(receipt_id)}`);
    return r.ok ? text(await r.json()) : fail({ status: r.status });
  });

server.tool("genesis402_risk", "Paid ($0.25 USDC). Risk snapshot of one EVM address on Base or Polygon: OFAC SDN screen, scam blocklists, explorer flags, activity sample, contract/token facts, evidence hash. Heuristic summary of public evidence; not KYC, not advice.",
  { address: z.string().regex(/^0x[0-9a-fA-F]{40}$/), chain: z.enum(["base", "polygon"]).optional() },
  async (p) => callPaid("/risk", p));

server.tool("genesis402_prove", "Paid ($0.25 USDC). Signed Ed25519 receipt binding your SHA-256 digest (or text, hashed by the rail) and optional claim to a settled payment. Verifiable offline with @genesis402/verify. The claim is recorded, not evaluated.",
  { sha256: z.string().regex(/^[0-9a-f]{64}$/).optional(), text: z.string().max(16384).optional(), claim: z.string().max(512).optional(), subject: z.string().max(200).optional() },
  async (p) => callPaid("/prove", p));

server.tool("genesis402_genesis_sim", "Paid ($0.25 USDC). Deterministic agent-economy simulation: Gini series, total energy, SHA-256 state commitment. Same inputs always give the same result.",
  { n: z.number().int().min(5).max(500).optional(), epochs: z.number().int().min(10).max(200).optional() },
  async (p) => callPaid("/genesis-sim", p));

server.tool("genesis402_wallet_ops", "Paid ($0.25 USDC). Live balances of the operator treasury on Base, XRPL and Stellar, queried from public nodes at request time.",
  { chains: z.array(z.enum(["base", "xrpl", "stellar"])).optional() },
  async (p) => callPaid("/wallet-ops", p));

server.tool("genesis402_rwa_screen", "Paid ($0.25 USDC). Curated static readiness table for one energy RWA market (texas, florida, newyork, oregon). Not a live feed; not advice.",
  { market: z.enum(["texas", "florida", "newyork", "oregon"]) },
  async (p) => callPaid("/rwa-screen", p));

server.tool("genesis402_llm", "Paid ($0.02 USDC). One chat completion from an allowlisted hosted model or the operator's local model; the response names the model that answered.",
  { prompt: z.string().max(24000).optional(), messages: z.array(z.object({ role: z.enum(["system", "user", "assistant"]), content: z.string() })).optional(), model: z.string().optional(), max_tokens: z.number().int().min(1).max(1024).optional(), temperature: z.number().min(0).max(2).optional() },
  async (p) => callPaid("/llm", p));

await server.connect(new StdioServerTransport());
