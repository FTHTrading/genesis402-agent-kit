# Genesis402 agent kit

Everything an outside developer needs to call and pay for the Genesis402 rail (https://twin.unykorn.org) from an agent.

| Service | Price | What it returns |
|---|---|---|
| risk | $0.25 | Risk snapshot of one EVM address on Base/Polygon (OFAC SDN, scam lists, explorer flags, activity, token/contract facts, evidence hash) |
| prove | $0.25 | Ed25519-signed receipt binding your SHA-256 digest + claim to a settled payment; verify offline |
| genesis-sim | $0.25 | Deterministic agent-economy simulation (Gini series, energy, SHA-256 state commitment) |
| wallet-ops | $0.25 | Live operator treasury balances on Base, XRPL, Stellar |
| rwa-screen | $0.25 | Curated static readiness table for 4 energy RWA markets (not a live feed) |
| llm | $0.02 | One chat completion from an allowlisted model |

Payment: x402 v2. USDC on Base, Polygon or Solana (facilitator-settled), USDC on Stellar or XRP on XRPL (pay-first, present the tx hash).

## 1. MCP server (Claude, Cursor, any MCP client)

```json
{
  "mcpServers": {
    "genesis402": {
      "command": "node",
      "args": ["/path/to/genesis402-agent-kit/mcp/index.mjs"],
      "env": {
        "GENESIS402_LIVE": "0",
        "GENESIS402_PAYER_KEY": "",
        "GENESIS402_MAX_USD": "0.25"
      }
    }
  }
}
```

Once published to npm, the args become `"command": "npx", "args": ["-y", "genesis402-mcp"]`.

- Default is **quote only**: paid tools return the price and what you get; nothing is signed.
- Set `GENESIS402_LIVE=1` and `GENESIS402_PAYER_KEY` (a Base wallet holding a little USDC, no ETH needed) to pay.
- Any quote above `GENESIS402_MAX_USD` is refused.

Tools: `genesis402_catalog` (free), `genesis402_receipt` (free), `genesis402_risk`, `genesis402_prove`, `genesis402_genesis_sim`, `genesis402_wallet_ops`, `genesis402_rwa_screen`, `genesis402_llm`.

## 2. Quickstart scripts

```bash
cd quickstart && npm install
node buy.mjs risk '{"address":"0x4ed4E862860beD51a9570b96d89aF5E1B0Efefed"}'          # quote
PAYER_KEY=0x... LIVE=1 node buy.mjs genesis-sim '{"n":20,"epochs":20}'                # $0.25 USDC on Base
XRPL_SEED=s... LIVE=1 RAIL=xrpl node buy.mjs genesis-sim '{"n":20,"epochs":20}'       # 0.05 XRP on XRPL

pip install "x402[httpx,evm]" eth-account xrpl-py
python buy.py genesis-sim '{"n":20,"epochs":20}'
PAYER_KEY=0x... LIVE=1 python buy.py genesis-sim '{"n":20,"epochs":20}'
```

Flow: POST without payment -> 402 with `PAYMENT-REQUIRED` (base64 JSON) -> sign EIP-3009 `transferWithAuthorization` (Base) or send the XRPL payment -> retry with `PAYMENT-SIGNATURE` / `X-PAYMENT` -> 200 with result and a receipt id (`/receipts/{id}`).

Failed settlement never charges: the rail answers 402 `settlement_failed` and states that no funds moved.
