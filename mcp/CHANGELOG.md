# Changelog

All notable changes to `genesis402-mcp`. Dates are UTC. Releases are built and signed on GitHub Actions (SLSA provenance) and published to npm and the MCP Registry by `.github/workflows/release-mcp.yml`.

## 0.3.11 - 2026-10-26
- Owed deliveries are redeemed instead of stranded. When a paid call settles but the rail cannot deliver, the rail answers with `payment.status: "owed"` and a private `owed_ticket`. The client now re-sends the same signed payment with `X-OWED-TICKET` once, which re-delivers at no extra cost, and returns `made_good: true`. A local payer re-sends the authorization it already signed; nothing new is signed.
- If that re-delivery also fails, the tool error carries the newest `owed_ticket` (and, for a local payer, the `payment_signature` to re-send). Every paid tool accepts `owed_ticket` together with the original `payment_signature` to redeem later. An `owed_ticket` without its payment is refused locally and nothing is sent.
- A settled-but-undelivered call reports `paid: true, delivered: false` instead of `paid: false`.
- `owed-test.mjs`: 16 offline checks against a mock rail, including a real offline EIP-3009 signature from a throwaway key; part of `npm test`.

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
