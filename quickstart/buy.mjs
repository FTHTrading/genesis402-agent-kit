// Buy one Genesis402 call. Usage:
//   node buy.mjs <task> '<params json>'            -> quote only (nothing signed)
//   PAYER_KEY=0x... LIVE=1 node buy.mjs genesis-sim '{"n":20,"epochs":20}'   -> pay $0.25 USDC on Base
//   XRPL_SEED=s... LIVE=1 RAIL=xrpl node buy.mjs genesis-sim '{"n":20,"epochs":20}' -> pay 0.05 XRP on XRPL
// Tasks: risk, prove, genesis-sim, wallet-ops, rwa-screen, llm
import { wrapFetchWithPaymentFromConfig, decodePaymentResponseHeader } from "@x402/fetch";
import { ExactEvmScheme } from "@x402/evm";
import { privateKeyToAccount } from "viem/accounts";

const ORIGIN = process.env.ORIGIN || "https://twin.unykorn.org";
const [task = "genesis-sim", raw = "{}"] = process.argv.slice(2);
const url = `${ORIGIN}/${task}`;
const init = { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ params: JSON.parse(raw) }) };
const MAX_ATOMIC = BigInt(process.env.MAX_ATOMIC || "250000"); // $0.25

// 1. Unpaid request -> 402 challenge (base64 JSON in PAYMENT-REQUIRED).
const probe = await fetch(url, init);
if (probe.status !== 402) { console.log(probe.status, await probe.text()); process.exit(probe.ok ? 0 : 1); }
const challenge = JSON.parse(Buffer.from(probe.headers.get("payment-required"), "base64").toString());
const base = challenge.accepts.find((a) => a.network === "eip155:8453");
const xrp = challenge.accepts.find((a) => a.network === "xrpl:mainnet");
console.log("402 challenge:", { base: base && `${Number(base.amount) / 1e6} USDC -> ${base.payTo}`, xrpl: xrp && `${xrp.price} XRP -> ${xrp.payTo}` });
if (process.env.LIVE !== "1") { console.log("Quote only. Set LIVE=1 plus PAYER_KEY (Base) or XRPL_SEED with RAIL=xrpl to pay."); process.exit(0); }

let res;
if (process.env.RAIL === "xrpl") {
  // 2a. XRPL is pay-first: send the payment on-chain, then present the validated tx hash.
  const xrpl = await import("xrpl");
  const client = new xrpl.Client("wss://xrplcluster.com");
  await client.connect();
  const wallet = xrpl.Wallet.fromSeed(process.env.XRPL_SEED);
  // Bind the payment to THIS challenge: memo = sha256(nonce). Only we know the nonce, so nobody watching the ledger can redeem it.
  const tx = await client.submitAndWait({ TransactionType: "Payment", Account: wallet.address, Destination: xrp.payTo, Amount: xrp.amount, Memos: [{ Memo: { MemoData: xrp.extra.memo_sha256.toUpperCase() } }] }, { wallet });
  await client.disconnect();
  const txHash = tx.result.hash;
  console.log("XRPL payment validated:", txHash);
  res = await fetch(url, { ...init, headers: { ...init.headers, "X-PAYMENT": JSON.stringify({ network: "xrpl:mainnet", txHash, nonce: xrp.extra.nonce }) } });
} else {
  // 2b. Base: sign an EIP-3009 transferWithAuthorization; the standard x402 client builds PAYMENT-SIGNATURE and retries.
  if (BigInt(base.amount) > MAX_ATOMIC) throw new Error(`price ${base.amount} above cap ${MAX_ATOMIC}`);
  const account = privateKeyToAccount(process.env.PAYER_KEY);
  const paidFetch = wrapFetchWithPaymentFromConfig(fetch, { schemes: [{ network: "eip155:8453", client: new ExactEvmScheme(account) }] });
  res = await paidFetch(url, init);
  const pr = res.headers.get("payment-response");
  if (pr) console.log("settlement:", decodePaymentResponseHeader(pr));
}
// 3. Verified result (includes a receipt id you can look up at /receipts/{id}).
console.log(res.status, JSON.stringify(await res.json(), null, 2));
