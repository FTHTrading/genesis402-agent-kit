# Changelog

All notable changes to `genesis402-mcp`. Dates are UTC. Releases are built and signed on GitHub Actions (SLSA provenance) and published to npm and the MCP Registry by `.github/workflows/release-mcp.yml`.

## Unreleased
- Listings: no hardcoded endpoint count or price range in the READMEs, package description, `server.json`, server instructions or the `genesis402_call` description; they point to the live count at https://twin.unykorn.org/status (`genesis402_call` still names the count it reads from the live manifest). README: three-step quickstart with `examples/quickstart.mjs`, feedback form, a neutral note on ERC-8004 feedback; the Smithery badge is removed (that listing no longer exists).

## 0.3.13 - 2026-11-18
- Specialists: the 376 treasury-paid routes are each sold by one of eight ERC-8004 specialists (prices, chain-reader, defi, web, geo-records, research, ai-tools, utilities); the collector (agent 95721) is the wallet that receives the payment. Tool results name the specialist in `seller` (`specialist`, `collector`) whether the rail's 402, its manifest or a receipt names it; `agent_id` is the collector's until the specialist is registered, then the specialist's own.
- `genesis402_catalog`: endpoints are grouped by specialist, each row carries `specialist`, the result carries `by_specialist` counts, and a new `specialist` argument lists one specialist's routes.
- README: new section "How this rail uses ERC-8004" (collector, specialists, validation, receipts), written as a reference example. Prices, payTo and every endpoint are unchanged.

## 0.3.12 - 2026-11-18

Same content as 0.3.11 (never published: the release smoke check capped per-call prices at $0.25 while desk-agent routes are priced up to $0.75 on purpose). Smoke ceiling raised to $1.00.

## 0.3.11 - 2026-11-17 (not published)
- Seller identity: every tool result carries `seller` (ERC-8004 agent id, registry, chain, pay_to, source) and `receipts_url`; the `seller` block is the rail's own when it names one (402 first, then /.well-known/x402), else the package default (agent 95721 on Base, `source: "package"`). A desk seller named by the rail is never replaced. Prices and payTo are untouched.
- Server instructions, README, `server.json` description and `_meta` name agent 95721, the registry address and the receipts feed.
- `genesis402_receipt` reports the seller recorded in the receipt when it has one.
- Version drift fixed: `server.json` was left at 0.3.9 when npm moved to 0.3.10; package.json, server.json and `VERSION` now move together (checked by `identity-test.mjs`).
- New offline test `identity-test.mjs` (fake rail on loopback), added to `npm test`.

## 0.3.10 - 2026-10-06
- Server instructions, the `genesis402_call` description and the listing descriptions say "360+" endpoints instead of a fixed 360 (the live catalog lists 367).

## 0.3.9 - 2026-10-02
- New dedicated tool `genesis402_market_snapshot` (price/market-snapshot, $0.004): USD price with 24-hour and 7-day change for the twelve major coins, or up to 25 named coins, in one call. Accepts CoinGecko ids or major-coin tickers. Coins with no live price are marked unavailable.
- README no longer states a fixed endpoint count; the catalog is the source.

## 0.3.8 - 2026-09-30
- Three dedicated compliance tools: `genesis402_counterparty_report` (report/counterparty, $0.25: company + registered parents + UN/OFAC/EU/UK screening, verdict, signed receipt, branded PDF link, public verify page), `genesis402_sanctions_name_screen` (screen/name, $0.02) and `genesis402_company_lookup` (screen/entity, GLEIF + SEC EDGAR, $0.01). 21 tools total.
- Server instructions and listing descriptions lead with counterparty checks.

## 0.3.7 - 2026-09-30 (not published separately; ships in 0.3.8)
- Spend cap enforced twice for a local payer: against the quote before paying, and by the x402 client's `spendControls` when it signs, so a price that changes between quote and payment is refused.
- A zero, negative or non-numeric `GENESIS402_MAX_USD` now keeps the server quote-only instead of silently disabling the cap.
- A payment the client refuses to sign returns a clean `payment_not_made` tool error.

## 0.3.6 - 2026-09-30
- Four dedicated tools for the research and AI text endpoints: `genesis402_summarize` (text-summarize), `genesis402_answer_from_text` (text-qa), `genesis402_translate` (text-translate), `genesis402_paper_search` (openalex-search). 18 tools total.
- Sharper `genesis402_catalog` and `genesis402_receipt` descriptions: when to use, what they do not do, exact return shape.
- Smoke test: prepaid credit packs (`/credits/5`, `/credits/25`) are excluded from the per-call price band check, which had turned CI red once they went live on the rail.

## 0.3.5 - 2026-09-29
- Type declarations (`index.d.ts`) for `createServer`, `localPayerFromEnv`, `loadCatalog`, `catalogSize`, `VERSION`.
- `exports` map: `import { createServer } from "genesis402-mcp"` resolves to the library (`core.mjs`); the CLI entry is unchanged (`npx genesis402-mcp`).
- Package metadata: homepage, bugs, `sideEffects`; this changelog ships in the tarball. README carries the banner and badges on npm.

## 0.3.4 - 2026-09-28
- Every tool declares an `outputSchema` and returns `structuredContent` alongside text.

## 0.3.3 - 2026-09-28
- Tool descriptions are built from the rail's live `/.well-known/x402` manifest at startup, so prices never go stale in the client.
- Server `instructions` in the handshake; environment defaults documented.

## 0.3.2 - 2026-09-26
- Every tool and parameter described; UnyKorn title and icons in `server.json`. First release on the MCP Registry and Glama.

## 0.3.1 - 2026-09-25
- README rewrite with ADR-approved wording; MIT LICENSE; Dockerfile; `glama.json`.

## 0.3.0 - 2026-09-24
- Hosted streamable-HTTP endpoint (keyless) at `https://twin.unykorn.org/mcp`; real endpoint paths from the manifest; batch-3 named tools.

## 0.1.0 - 2026-09-17
- Initial MCP server: 8 tools, quote-only default, spend cap; Node and Python quickstart payers for Base USDC.
