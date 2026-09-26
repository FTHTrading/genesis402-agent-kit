# Genesis402 agent kit

[![npm](https://img.shields.io/npm/v/genesis402-mcp)](https://www.npmjs.com/package/genesis402-mcp)
[![Listed on mcpservers.org](https://mcpservers.org/badge.svg)](https://mcpservers.org/servers/fthtrading/genesis402-agent-kit)
[![smithery badge](https://smithery.ai/badge/kevanbtc/genesis402-mcp)](https://smithery.ai/servers/kevanbtc/genesis402-mcp)
[![MCP Registry](https://img.shields.io/badge/MCP%20Registry-io.github.FTHTrading%2Fgenesis402--mcp-blue)](https://registry.modelcontextprotocol.io/v0/servers?search=genesis402)
[![ERC-8004 agent 95721](https://img.shields.io/badge/ERC--8004-agent%2095721%20on%20Base-purple)](https://8004scan.io/agents/base/95721)
[![Live status](https://img.shields.io/badge/status-twin.unykorn.org%2Fstatus-green)](https://twin.unykorn.org/status)

**360 pay-per-call APIs for AI agents over x402 and MPP.** USDC on Base, $0.001 to $0.25 a call, no API key, no account. Every response names its sources and carries an evidence hash; a paid call that is not delivered is credited automatically.

Rail: https://twin.unykorn.org · Catalog: https://twin.unykorn.org/catalog · Live status: https://twin.unykorn.org/status · Receipts: https://twin.unykorn.org/receipts

## Use it in 30 seconds

**Hosted MCP (nothing to install).** Add this remote to Claude, Cursor, or any MCP client that speaks streamable HTTP:

```
https://twin.unykorn.org/mcp
```

The hosted server holds no keys. Free tools (catalog, receipts, examples) just work. Paid tools take a `payment_signature` you produce with your own wallet, or run the local server below and let it pay for you within a cap.

**Local MCP (pays for you, within a cap you set):**

```json
{
  "mcpServers": {
    "genesis402": {
      "command": "npx",
      "args": ["-y", "genesis402-mcp"],
      "env": {
        "GENESIS402_LIVE": "1",
        "GENESIS402_PAYER_KEY": "0x<private key of a wallet holding a little USDC on Base>",
        "GENESIS402_MAX_USD": "0.25"
      }
    }
  }
}
```

Without `GENESIS402_LIVE=1` the server is quote-only: it shows the price and signs nothing. The server never raises a price on its own; the rail sets it, the client decides.

**Plain HTTP (any language), x402:**

```
GET https://twin.unykorn.org/price/bitcoin
→ 402, header payment-required: <base64 x402 v2 challenge>
sign the USDC authorization (EIP-3009) for the quoted amount, retry with X-PAYMENT
→ 200 + result + receipt id
```

**Plain HTTP, MPP (the "Payment" HTTP auth scheme):**

```
GET https://twin.unykorn.org/price/bitcoin
→ 402, header WWW-Authenticate: Payment id="…", method="usdc", intent="charge", request="…"
sign the same EIP-3009 authorization for the challenged amount, retry with
   Authorization: Payment <base64url credential>
→ 200 + result + Payment-Receipt header
```

Both dialects quote the same price to the same address in the same token. Discovery for MPP clients is the standard `/openapi.json` with `x-payment-info` on every operation.

Free before you pay: `POST /__validate` checks your parameters with the same validator the paid path uses, so a bad request is never charged.

## What is on the rail (360 endpoints)

| Family | Count | Examples | Price |
|---|---:|---|---|
| Crypto prices | 72 | `/price/bitcoin`, `/price/ethereum` … | $0.001 |
| DeFi and markets | 47 | `/defi/yields`, `/defi/stablecoins`, `/defi/chain/base`, `/defi/fees` | $0.002–$0.01 |
| Chain reads, 10 EVM chains + BTC, SOL, XLM, XRP | 83 | `erc20-balance`, `evm-multi-chain-scan`, `evm-proof`, `xrpl-account`, `stellar-account`, `solana-account`, `btc-utxos` | $0.001–$0.008 |
| Deterministic compute | 64 | hashes, encodings, ABI encode/decode, EIP-712, merkle, ENS | $0.001–$0.003 |
| Due diligence and risk | 10 | `/wallet-brief`, `/token-brief`, `screen-sanctions`, `holder-concentration`, `/risk` | $0.008–$0.25 |
| Public records | 12 | `/records/sec-financials`, `/records/sec-filings`, `/records/fx-rates`, `/records/treasury-yields`, `/records/vin-decode` | $0.001–$0.01 |
| Research | 15 | `/research/arxiv-search`, `/research/doi`, `/research/npm-package`, `/research/wiki-summary` | $0.001–$0.003 |
| Web and domain intelligence | 15 | `/web/whois`, `/web/dns`, `/web/tls-cert`, `/web/email-check`, `/web-extract`, `/summarize-url` | $0.001–$0.004 |
| Places, time, weather | 10 | `/geo/geocode`, `/geo/weather-forecast`, `/geo/public-holidays` | $0.001–$0.002 |
| AI on our own GPU | 16 | `/v1/chat/completions` (OpenAI-compatible), `/v1/embeddings`, `/ai/extract`, `/ai/text-to-sql`, `/ai/summarize` | $0.002–$0.01 |
| Proofs | 2 | `/prove` (Ed25519-signed receipt binding your digest to a settled payment), `/json-canonical` | $0.001–$0.25 |

The full list with schemas and live examples: https://twin.unykorn.org/catalog, `/.well-known/x402`, and `/openapi.json`.

## MCP tools (v0.3.1)

Free: `catalog`, `receipt`, `examples`. Paid, generic: `call(name, params)`. Paid, named: `wallet_brief`, `token_brief`, `screen_sanctions`, `multi_chain_scan`, `defi_yields`, `sec_financials`, `email_check`, `whois`, `extract_json`, `web_extract`, `prove`. Every paid tool returns the receipt id and evidence hash with the result.

## Guarantees

- **Validate before pay.** Bad input returns 400 with "nothing was charged".
- **Sources on every answer.** Each response lists the upstream it read and an `evidence_hash`. A failed upstream is an error, never a silent zero.
- **Make-good.** A paid call that is not delivered is credited to the payer automatically; the guardian reconciles every settlement on-chain within 48 hours.
- **Public status.** https://twin.unykorn.org/status is the guardian's own report, refreshed every 5 minutes.
- **Identity.** ERC-8004 agent 95721 on Base; A2A card at the rail root.

## Payment rails

x402 v2 `exact` scheme and MPP `usdc`/`charge` (EIP-3009 authorization), both on USDC on Base (primary) and Polygon; Solana USDC via facilitator settlement; XRP and USDC on XRPL and Stellar are pay-first with the payment bound to the challenge nonce. The 402 lists exactly which lanes are payable right now.

## Repo layout

- `mcp/` the MCP server (`npx genesis402-mcp`, `genesis402-mcp-http` for the hosted mode), `server.json` for the MCP Registry, `smoke.mjs` (13 read-only checks against the live rail).
- `quickstart/` Node and Python payers for Base, XRPL and Stellar.

## License

MIT. UnyKorn LLC, Wyoming. Not financial, legal or tax advice.
