# Screen a wallet before your agent moves money

**$0.008 a check. No API key, no account, no subscription.** Your agent pays per call in USDC on Base over [x402](https://x402.org).

Before an agent sends funds to an address, or accepts funds from one, it should know whether that address appears on a published sanctions or scam list. `screen-sanctions` checks one address against the OFAC SDN digital-currency entries, community scam-address blocklists and the Blockchain Fraud case registry, and returns every list it consulted with an evidence hash.

It is a signal from public data, not a compliance determination. A hit means the address is on a published list; a clean result is not a clearance.

## Option 1: from an MCP client (Claude, Cursor, VS Code, Cline)

Add the hosted server. Nothing to install, no keys held by the server:

```
https://twin.unykorn.org/mcp
```

Then ask your agent: *"Screen 0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed for sanctions."* It calls `genesis402_screen_sanctions`, shows you the $0.008 quote, and runs once you pay with your own wallet.

To let a local agent pay by itself within a cap you set:

```json
{
  "mcpServers": {
    "genesis402": {
      "command": "npx",
      "args": ["-y", "genesis402-mcp"],
      "env": {
        "GENESIS402_LIVE": "1",
        "GENESIS402_PAYER_KEY": "0x<key of a dedicated wallet holding a little USDC on Base>",
        "GENESIS402_MAX_USD": "0.01"
      }
    }
  }
}
```

Without `GENESIS402_LIVE=1` the server only quotes and signs nothing.

## Option 2: from code (Node 20+)

```bash
git clone https://github.com/FTHTrading/genesis402-agent-kit
cd genesis402-agent-kit/quickstart && npm install

# Free: validates the address and prints the exact price. Nothing is signed.
node screen.mjs 0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed
# quote: $0.008 USDC on Base -> 0x7d9a65d06dcc435a52D5880C6310Bd6E96c156DB

# Paid: signs one $0.008 USDC authorization and prints the result.
PAYER_KEY=0x... LIVE=1 node screen.mjs 0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed
```

The payer wallet needs USDC on Base only; no ETH for gas. `screen.mjs` refuses any quote above `MAX_USD` (default $0.01).

What [`screen.mjs`](../../quickstart/screen.mjs) does, in three steps:

1. `POST /__validate` checks the parameters for free, so a bad address is never charged.
2. `POST /screen-sanctions` without payment returns `402` with the price in the `payment-required` header (x402 v2).
3. The standard `@x402/fetch` client signs an EIP-3009 USDC authorization for that quote and retries; the response carries the result, the settlement and a receipt id.

## Option 3: any language, raw HTTP

```bash
curl -i -X POST https://twin.unykorn.org/screen-sanctions \
  -H 'content-type: application/json' \
  -d '{"params":{"address":"0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed"}}'
# HTTP/2 402
# payment-required: <base64 x402 v2 challenge>
# www-authenticate: Payment ... method="usdc", intent="charge"   (MPP dialect)
```

Sign the challenge with any x402 v2 client (Coinbase AgentKit/CDP wallets, `@x402/fetch`, the Python `x402` package) and resend with `PAYMENT-SIGNATURE`.

## Going further

| Need | Endpoint | Price |
|---|---|---|
| Sanctions lists plus balances and activity across 10 EVM chains, with a summary | `wallet-brief` | $0.03 |
| Is this token safe to buy or accept? | `token-brief` | $0.03 |
| Where is this address active? | `evm-multi-chain-scan` | $0.008 |

All 360 endpoints, with schemas and live prices: [twin.unykorn.org/catalog](https://twin.unykorn.org/catalog). Every paid call is on the public [receipts feed](https://twin.unykorn.org/receipts); a paid call that is not delivered is re-delivered free on retry. Live health: [twin.unykorn.org/status](https://twin.unykorn.org/status).

Stuck, or want to see it run first? Try `/screen` and `/check` free in the [UnyKorn Discord](https://discord.gg/qqH42PswDd).
