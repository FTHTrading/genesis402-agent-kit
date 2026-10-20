#!/usr/bin/env node
// Bazaar re-seed: make ONE small settled payment per new endpoint so Coinbase's CDP discovery (Bazaar) indexes it.
// Coinbase indexes a resource after a CDP-facilitated payment settles on a route that declares the bazaar extension
// (war-room/outbox/army/scale-up/manifests/BAZAAR-PATCH-STAGED.md). How the current 369 listings were first seeded is
// NOT recorded anywhere I could find (the 2026-10-06 keep-warm note says no paid script existed); this script is new.
//
//   node mcp/scripts/bazaar-seed.mjs seeds.json            DRY RUN (default): resolves each seed against the live
//                                                            manifest, prints method, URL, price and the total. Signs nothing.
//   GENESIS402_LIVE=1 GENESIS402_PAYER_KEY=0x... node mcp/scripts/bazaar-seed.mjs seeds.json --pay
//                                                          REAL: pays. Needs both env vars. Never run by CI.
//
// seeds.json: [{ "path": "/price/bitcoin", "method": "GET", "query": {"x":"y"} }, { "path": "/hash/sha256", "body": {"text":"a"} }]
// method defaults to POST; the body/query must be a valid input for that endpoint (a rejected input is not charged,
// but then nothing gets indexed). The script never guesses inputs.
//
// Hard limits (each checked before any signature, each covered by mcp/test/bazaar-seed-plan.test.mjs):
//   - per call at most $0.005 (an endpoint whose manifest price is higher is refused, not paid)
//   - per run at most $0.10 in total (SEED_MAX_USD may lower it, never raise it)
//   - only paths on https://twin.unykorn.org; only Base USDC; the payer signs at most the quoted amount (x402 spendControls)
// The key comes from the environment only, is never printed, never written, never put in a report. Report lines carry
// path, price, HTTP status and the settlement transaction hash (public data).
import { pathToFileURL } from "node:url";
import { readFileSync } from "node:fs";

export const ORIGIN = "https://twin.unykorn.org";
export const PER_CALL_MAX_USD = 0.005;
export const RUN_MAX_USD = 0.10;

export function budgetFromEnv(env = process.env) {
  const raw = env.SEED_MAX_USD;
  if (raw === undefined || raw === "") return RUN_MAX_USD;
  const n = Number(raw);
  if (!Number.isFinite(n) || n <= 0) throw new Error(`SEED_MAX_USD="${raw}" is not a positive number`);
  if (n > RUN_MAX_USD) throw new Error(`SEED_MAX_USD=${raw} is above the $${RUN_MAX_USD} per-run ceiling`);
  return n;
}

const micro = (usd) => Math.round(usd * 1e6);

/**
 * Turn seeds + the live manifest into a plan. Pure: no network. Each item is { ok, path, url, method, usd, reason }.
 * Items are accepted in order until the run budget would be exceeded; the rest are marked over_budget.
 */
export function planSeeds(manifest, seeds, budgetUsd) {
  const byPath = new Map();
  for (const s of manifest?.services || []) {
    try { byPath.set(new URL(s.endpoint).pathname, s); } catch { /* skip malformed */ }
  }
  let spent = 0;
  const items = [];
  for (const seed of seeds) {
    const path = String(seed?.path || "");
    const method = String(seed?.method || "POST").toUpperCase();
    const base = { path, method, url: `${ORIGIN}${path}`, usd: null };
    if (!/^\/[A-Za-z0-9._~\-\/]*$/.test(path) || path.includes("..")) { items.push({ ...base, ok: false, reason: "path_not_allowed" }); continue; }
    if (!["GET", "POST"].includes(method)) { items.push({ ...base, ok: false, reason: "method_not_allowed" }); continue; }
    const svc = byPath.get(path);
    if (!svc) { items.push({ ...base, ok: false, reason: "not_in_manifest" }); continue; }
    const usd = Number(svc.price?.usd);
    if (!Number.isFinite(usd) || usd <= 0) { items.push({ ...base, ok: false, reason: "no_price" }); continue; }
    if (micro(usd) > micro(PER_CALL_MAX_USD)) { items.push({ ...base, usd, ok: false, reason: "price_above_per_call_cap" }); continue; }
    if (micro(spent + usd) > micro(budgetUsd)) { items.push({ ...base, usd, ok: false, reason: "over_budget" }); continue; }
    spent += usd;
    items.push({ ...base, usd, ok: true, reason: "", seed, bazaarListing: svc.bazaar_listing === true });
  }
  return { items, totalUsd: Math.round(spent * 1e6) / 1e6, budgetUsd };
}

async function main() {
  const args = process.argv.slice(2);
  const pay = args.includes("--pay");
  const file = args.find((a) => !a.startsWith("--"));
  if (!file) { process.stderr.write("usage: bazaar-seed.mjs seeds.json [--pay]\n"); process.exitCode = 64; return; }
  const seeds = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(seeds) || seeds.length === 0) throw new Error("seeds.json must be a non-empty array");
  const manifest = await (await fetch(`${ORIGIN}/.well-known/x402`, { signal: AbortSignal.timeout(30000) })).json();
  const plan = planSeeds(manifest, seeds, budgetFromEnv());
  for (const it of plan.items) process.stdout.write(`${it.ok ? "PLAN " : "SKIP "} ${it.method} ${it.path} ${it.usd ?? "-"} ${it.reason}${it.ok && !it.bazaarListing ? " (manifest does not mark bazaar_listing; payment may not index it)" : ""}\n`);
  process.stdout.write(`total $${plan.totalUsd} of $${plan.budgetUsd} budget; ${pay ? "PAYING" : "DRY RUN, nothing signed"}\n`);
  if (!pay) return;

  if (process.env.GENESIS402_LIVE !== "1" || !process.env.GENESIS402_PAYER_KEY) throw new Error("--pay needs GENESIS402_LIVE=1 and GENESIS402_PAYER_KEY in the environment");
  const { createPayer } = await import("../core.mjs");
  const BASE = "eip155:8453";
  const { privateKeyToAccount } = await import("viem/accounts");
  const { ExactEvmScheme } = await import("@x402/evm");
  const key = process.env.GENESIS402_PAYER_KEY;
  const account = privateKeyToAccount(key.startsWith("0x") ? key : `0x${key}`);
  const payer = createPayer({ network: BASE, client: new ExactEvmScheme(account), maxUsd: PER_CALL_MAX_USD });
  for (const it of plan.items.filter((i) => i.ok)) {
    const init = it.method === "GET"
      ? {}
      : { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(it.seed.body ?? {}) };
    const qs = it.method === "GET" && it.seed.query ? `?${new URLSearchParams(it.seed.query)}` : "";
    const res = await payer.paidFetch(`${it.url}${qs}`, init);
    let tx = "-";
    const hdr = res.headers.get("payment-response") || res.headers.get("x-payment-response");
    if (hdr) { try { tx = JSON.parse(Buffer.from(hdr, "base64").toString("utf8")).transaction || "-"; } catch { /* ignore */ } }
    process.stdout.write(`PAID? ${it.method} ${it.path} http=${res.status} tx=${tx}\n`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((e) => { process.stderr.write(`bazaar-seed: ${e?.message || e}\n`); process.exitCode = 1; });
}
