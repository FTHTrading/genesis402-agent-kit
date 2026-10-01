# Changelog

All notable changes to `genesis402-mcp`. Dates are UTC. Releases are built and signed on GitHub Actions (SLSA provenance) and published to npm and the MCP Registry by `.github/workflows/release-mcp.yml`.

## 0.3.9 - 2026-10-01
- Make-good: when a paid call settles but the rail fails to deliver, the rail returns a one-time make-good token. The client now redeems it once automatically (no new payment) and returns the result with `made_good: true`. If that retry also fails, the tool error carries `make_good_token`; pass it back on any paid tool as `make_good_token` (without `payment_signature`) to receive the result free later. A paid call is never paid twice.
- A failed paid call that did settle now reports `paid: true, delivered: false` instead of `paid: false`.
- `makegood-test.mjs`: offline test against a mock rail (11 checks), part of `npm test`.

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
