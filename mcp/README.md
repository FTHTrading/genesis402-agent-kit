# genesis402-mcp

MCP server for the [Genesis402](https://twin.unykorn.org) x402 rail â€” **179 pay-per-call
endpoints** across Base, Ethereum, Solana, Stellar, XRPL and Bitcoin, paid in USDC on Base
via the x402 v2 protocol.

No account. No API key. No signup. Your agent reads a 402, pays a fraction of a cent, and
gets JSON back.

## Quote-only by default

**Nothing is ever signed until you explicitly opt in.** Out of the box every paid tool
returns the price quote and stops. To enable payment you must set *both* `GENESIS402_LIVE=1`
and `GENESIS402_PAYER_KEY`. Even then, any quote above `GENESIS402_MAX_USD` (default `$0.25`)
is refused rather than paid.

The server sets the price. This client never does â€” it only decides whether to accept it.

## Install

```jsonc
// Claude Desktop: claude_desktop_config.json
// Cursor:         .cursor/mcp.json
{
  "mcpServers": {
    "genesis402": {
      "command": "npx",
      "args": ["-y", "genesis402-mcp"]
    }
  }
}
```

That is the safe configuration: your agent can browse the catalog and see prices, and cannot
spend anything.

To let it actually pay:

```jsonc
{
  "mcpServers": {
    "genesis402": {
      "command": "npx",
      "args": ["-y", "genesis402-mcp"],
      "env": {
        "GENESIS402_LIVE": "1",
        "GENESIS402_PAYER_KEY": "0x<a funding key you are willing to spend from>",
        "GENESIS402_MAX_USD": "0.05"
      }
    }
  }
}
```

Use a dedicated key holding a few dollars of USDC on Base. Never your main wallet.

## Tools

| Tool | Cost | What it does |
|---|---|---|
| `genesis402_catalog` | free | Every endpoint, price and parameter, from the live manifest. Takes an optional `filter`. |
| `genesis402_receipt` | free | Look up a paid-call receipt by id. |
| `genesis402_call` | varies | **Call any of the 179 endpoints by name.** Start here. |
| `genesis402_multi_chain_scan` | $0.008 | One call across 10 EVM chains. The alternative is ten RPC providers and the fan-out code. |
| `genesis402_screen_sanctions` | $0.008 | OFAC SDN + community blocklist screen. |
| `genesis402_token_concentration` | $0.008 | Holder count and top-holder concentration â€” the rug/whale pre-trade check. |
| `genesis402_contract_verified` | $0.006 | Source-verification and proxy status. |
| `genesis402_address_activity` | $0.008 | Age, cadence, counterparty spread. |
| `genesis402_risk` | $0.25 | Full risk snapshot with evidence hash. |
| `genesis402_prove` | $0.25 | Ed25519-signed receipt that a digest existed at time T. Verifies offline. |
| `genesis402_genesis_sim` | $0.25 | Deterministic agent-economy simulation. |
| `genesis402_wallet_ops` | $0.25 | Live operator treasury balances. |
| `genesis402_rwa_screen` | $0.25 | Energy RWA market readiness table. |
| `genesis402_llm` | $0.002 | One chat completion; the response names the model that answered. |

Prices are read from the live `/.well-known/x402` manifest at startup, never hardcoded â€” so
a price change on the rail can never leave this client advertising a stale number.

## How a paid call works

1. **Free validation.** Parameters go to the rail's free `/__validate` first. Bad input is
   refused here, before a quote is even requested.
2. **Free quote.** An unpaid request returns the 402 challenge with the price.
3. **Payment**, only in live mode, only under your cap, through the standard x402 v2 client.
4. **Result + receipt.** Every settled call appears on the rail's public
   [receipts feed](https://twin.unykorn.org/receipts).

One payment proof buys exactly one execution; re-presenting it returns 409. If delivery
fails after payment, the proof is released and can be re-presented at no extra cost.

## What's genuinely hard to get elsewhere

Much of the catalog is commodity â€” hashing, encoding, checksums. Those are there for
completeness and cost a tenth of a cent. The endpoints worth an agent's attention:

- **`evm-multi-chain-scan`** â€” one call, ten EVM chains.
- **The non-EVM spread** â€” XRPL, Stellar, Solana and Bitcoin behind one auth and one
  response shape. Very few providers cover all four.
- **Judgement, not reads** â€” `screen-sanctions`, `token-concentration`, `contract-verified`.
- **`prove`** â€” a signed, hash-chained receipt that a digest existed at a point in time.

## Limits, stated plainly

- Risk and sanctions output is a **heuristic summary of public evidence**. It is not a KYC
  decision and not advice. Absence from every list is not a clearance.
- `prove` records a claim; it does not evaluate it. The hash chain is **not externally
  anchored**, so a valid verdict proves the rail issued the receipt and has not altered it â€”
  not that the rail could not have back-dated its own chain.
- `rwa-screen` is a curated static table, not a live feed.

## Environment

| Variable | Default | Meaning |
|---|---|---|
| `GENESIS402_ORIGIN` | `https://twin.unykorn.org` | Rail origin. |
| `GENESIS402_LIVE` | unset | `1` enables payment. Anything else is quote-only. |
| `GENESIS402_PAYER_KEY` | unset | Private key that funds calls. Required for live mode. |
| `GENESIS402_MAX_USD` | `0.25` | Hard cap per call. Quotes above this are refused. |

## Testing

```bash
node smoke.mjs      # 13 read-only checks against the live rail; never pays
node boot-test.mjs  # boots over stdio and lists the tools a client would see
```

Operated by UnyKorn LLC (Wyoming). MIT licensed.
Discovery: [`/.well-known/x402`](https://twin.unykorn.org/.well-known/x402) Â·
Receipts: [`/receipts`](https://twin.unykorn.org/receipts)

## Hosted endpoint (no install)

Connect any MCP client that supports streamable HTTP to **https://twin.unykorn.org/mcp**. It exposes the same 14 tools over all 360 endpoints.

- The hosted server holds **no keys** and cannot spend anything. A paid tool returns the exact x402 quote.
- To pay, sign an x402 v2 payment for that quote with your own wallet (for example `@x402/fetch` or a CDP/AgentKit wallet) and call the tool again with `payment_signature`.
- Health check: https://twin.unykorn.org/mcp/health

