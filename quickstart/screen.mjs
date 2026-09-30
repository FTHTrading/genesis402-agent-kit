// Screen one wallet address against public sanctions and scam lists before your agent sends or accepts funds.
// Genesis402 /screen-sanctions: $0.008 USDC on Base per call, paid over x402. No API key, no account.
//
//   node screen.mjs 0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed
//       -> free parameter check + exact price quote. Nothing is signed.
//   PAYER_KEY=0x... LIVE=1 node screen.mjs 0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed
//       -> pays $0.008 from that Base wallet (USDC only, no ETH needed) and prints the result.
//
// Output is a heuristic signal from public lists, not a compliance determination.
import { wrapFetchWithPaymentFromConfig, decodePaymentResponseHeader } from "@x402/fetch";
import { ExactEvmScheme } from "@x402/evm";
import { privateKeyToAccount } from "viem/accounts";

const ORIGIN = process.env.ORIGIN || "https://twin.unykorn.org";
const BASE = "eip155:8453";
const MAX_USD_RAW = Number(process.env.MAX_USD ?? "0.01"); // refuse any quote above this
const MAX_USD = Number.isFinite(MAX_USD_RAW) && MAX_USD_RAW > 0 ? MAX_USD_RAW : 0.01;

const address = process.argv[2];
if (!/^0x[0-9a-fA-F]{40}$/.test(address || "")) {
  console.error("usage: node screen.mjs <0x EVM address>");
  process.exit(2);
}
const params = { address };
const url = `${ORIGIN}/screen-sanctions`;
const init = { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ params }) };

// 1. Free: the rail validates parameters with the same validator the paid path uses.
const v = await (await fetch(`${ORIGIN}/__validate`, { method: "POST", headers: init.headers, body: JSON.stringify({ name: "screen-sanctions", params }) })).json();
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
const res = await paidFetch(url, init);
const pr = res.headers.get("payment-response");
if (pr) console.log("settlement:", decodePaymentResponseHeader(pr));
console.log(res.status, JSON.stringify(await res.json(), null, 2));
