// Check one company before your agent deals with it, pays it, or signs with it.
// Genesis402 /report/counterparty: $0.25 USDC on Base per report, paid over x402. No API key, no account.
//
//   node counterparty.mjs "Deutsche Bank AG"
//       -> free parameter check + exact price quote. Nothing is signed.
//   PAYER_KEY=0x... LIVE=1 node counterparty.mjs "Deutsche Bank AG"
//       -> pays $0.25 from that Base wallet (USDC only, no ETH needed) and prints the verdict.
//
// A name or a 20-character LEI. Screening and registry signals only: not KYC, not a compliance
// determination, not legal advice.
import { wrapFetchWithPaymentFromConfig, decodePaymentResponseHeader } from "@x402/fetch";
import { ExactEvmScheme } from "@x402/evm";
import { privateKeyToAccount } from "viem/accounts";

const ORIGIN = process.env.ORIGIN || "https://twin.unykorn.org";
const BASE = "eip155:8453";
const MAX_USD_RAW = Number(process.env.MAX_USD ?? "0.25"); // refuse any quote above this
const MAX_USD = Number.isFinite(MAX_USD_RAW) && MAX_USD_RAW > 0 ? MAX_USD_RAW : 0.25;

const query = (process.argv[2] || "").trim();
if (query.length < 3 || query.length > 200) {
  console.error('usage: node counterparty.mjs "<company legal name or 20-character LEI>"');
  process.exit(2);
}
const params = { query };
const url = `${ORIGIN}/report/counterparty`;
const init = { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ params }) };

// 1. Free: the rail validates parameters with the same validator the paid path uses.
const v = await (await fetch(`${ORIGIN}/__validate`, { method: "POST", headers: init.headers, body: JSON.stringify({ name: "counterparty-report", params }) })).json();
if (v.checked && !v.valid) { console.error("invalid input, nothing charged:", v.message); process.exit(1); }

// 2. Free: the unpaid request returns the exact price as an x402 v2 challenge.
const probe = await fetch(url, init);
if (probe.status !== 402) { console.log(probe.status, await probe.text()); process.exit(probe.ok ? 0 : 1); }
const challenge = JSON.parse(Buffer.from(probe.headers.get("payment-required"), "base64").toString());
const lane = challenge.accepts.find((a) => a.network === BASE);
const usd = Number(lane.amount ?? lane.maxAmountRequired) / 1e6;
console.log(`quote: $${usd} USDC on Base -> ${lane.payTo}`);
if (process.env.LIVE !== "1") { console.log("quote only; set LIVE=1 and PAYER_KEY to pay"); process.exit(0); }
if (usd > MAX_USD) { console.error(`quote $${usd} is above MAX_USD $${MAX_USD}; not paying`); process.exit(1); }

// 3. Paid: sign an EIP-3009 USDC authorization; the x402 client retries with PAYMENT-SIGNATURE.
const account = privateKeyToAccount(process.env.PAYER_KEY);
// spendControls re-checks the cap when signing, so a price that changed after the quote is refused.
const paidFetch = wrapFetchWithPaymentFromConfig(fetch, { schemes: [{ network: BASE, client: new ExactEvmScheme(account) }], spendControls: { maxAmountPerPayment: `$${MAX_USD}` } });
let res = await paidFetch(url, init);
const pr = res.headers.get("payment-response");
if (pr) console.log("settlement:", decodePaymentResponseHeader(pr));

// 4. Settled but not delivered: redeem the make-good token once. No new payment is signed.
const token = res.headers.get("x-make-good");
if (!res.ok && token) {
  console.error("payment settled but the report was not delivered; redeeming the make-good token (free)");
  res = await fetch(url, { ...init, headers: { ...init.headers, "X-Make-Good": token } });
  if (!res.ok) console.error(`still not delivered. Keep this token and retry later with header X-Make-Good: ${res.headers.get("x-make-good") || token}`);
}

const body = await res.json();
console.log(res.status, JSON.stringify(body, null, 2));
if (res.ok && body.verdict) console.log(`\nverdict: ${String(body.verdict).toUpperCase()}`);
