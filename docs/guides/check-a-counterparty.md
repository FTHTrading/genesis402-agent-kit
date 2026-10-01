# Check a counterparty before your agent deals with it

**$0.25 a report. No API key, no account, no subscription.** Your agent pays per report in USDC on Base over [x402](https://x402.org).

Before an agent pays an invoice, wires a deposit, onboards a supplier or signs with a bank, it should know who the counterparty really is and whether anyone in its group is sanctioned. `counterparty-report` does that in one call:

1. Resolves the company in the [GLEIF](https://www.gleif.org) legal-entity register, by name or 20-character LEI.
2. Follows its registered direct and ultimate parent.
3. Screens every name in that group against the UN, US OFAC SDN, EU and UK consolidated sanctions lists, fetched from the official publishers.
4. Returns one verdict, `clear`, `review` or `stop`, with every reason sourced. You also get the registry record, the group that was screened, the list versions used, an Ed25519-signed receipt, a private link to a branded PDF (valid 7 days) and a public verify page you can hand to a colleague or an auditor.

A vague name (for example a bare brand) returns `review` and names the entity it assumed. Give the exact legal name or the LEI for the most precise result.

These are screening and registry signals, not KYC and not a compliance determination.

## Option 1: from an MCP client (Claude, Cursor, VS Code, Cline)

Add the hosted server. There is nothing to install, and the server holds no keys:

```
https://twin.unykorn.org/mcp
```

Then ask your agent: *"Run a counterparty report on Deutsche Bank AG."* It calls `genesis402_counterparty_report` and shows you the $0.25 quote. The report runs once you pay with your own wallet.

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
        "GENESIS402_MAX_USD": "0.25"
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

# Free: validates the input and prints the exact price. Nothing is signed.
node counterparty.mjs "Deutsche Bank AG"

# Paid: signs one $0.25 USDC authorization and prints the report.
PAYER_KEY=0x... LIVE=1 node counterparty.mjs "Deutsche Bank AG"
```

The payer wallet needs USDC on Base only; no ETH for gas. `counterparty.mjs` refuses any quote above `MAX_USD` (default $0.25).

What [`counterparty.mjs`](../../quickstart/counterparty.mjs) does:

1. `POST /__validate` checks the input for free, so a malformed query is never charged.
2. `POST /report/counterparty` without payment returns `402` with the price in the `payment-required` header (x402 v2).
3. The standard `@x402/fetch` client signs an EIP-3009 USDC authorization for that quote and retries. The response carries the report, the settlement and a receipt id.
4. If the payment settled but the report was not delivered, the response carries an `X-Make-Good` token. The script redeems it once for free; you never pay twice for the same report.

## Option 3: any language, raw HTTP

```bash
curl -i -X POST https://twin.unykorn.org/report/counterparty \
  -H 'content-type: application/json' \
  -d '{"params":{"query":"Deutsche Bank AG"}}'
# HTTP/2 402
# payment-required: <base64 x402 v2 challenge>
# www-authenticate: Payment ... method="usdc", intent="charge"   (MPP dialect)
```

Sign the challenge with any x402 v2 client (Coinbase AgentKit/CDP wallets, `@x402/fetch`, the Python `x402` package) and resend with `PAYMENT-SIGNATURE`.

## Put it in front of every payment

The pattern that pays for itself: no outbound payment to a company your agent has not checked.

```js
const report = await checkCounterparty(payee.legalName); // $0.25, the script above
if (report.verdict === "stop") throw new Error("payee failed sanctions screening");
if (report.verdict === "review") return queueForHuman(payee, report); // human decides
await pay(payee);
```

Keep the receipt id with the payment record. Anyone can confirm later what was checked, when, and against which list versions.

## Cheaper checks on the same desk

| Need | Endpoint | MCP tool | Price |
|---|---|---|---|
| Registry record only: legal name, LEI, jurisdiction, status, parents (GLEIF, SEC EDGAR) | `screen/entity` | `genesis402_company_lookup` | $0.01 |
| One person or company name against UN, OFAC, EU and UK lists | `screen/name` | `genesis402_sanctions_name_screen` | $0.02 |
| Offer text checked against regulator warnings on prime-bank, SBLC/MT760 and advance-fee schemes | `screen/deal` | `genesis402_call` | $0.05 |
| A wallet address against sanctions and scam lists | `screen-sanctions` | `genesis402_screen_sanctions` | $0.008 |
| Which sanctions lists and registers apply in 12 jurisdictions | `jurisdictions` | — | free |

The agents behind the desk, with their ERC-8004 identities on Base: [agent directory](../agents.md). Every paid call is on the public [receipts feed](https://twin.unykorn.org/receipts). Live health: [twin.unykorn.org/status](https://twin.unykorn.org/status).

Want to see a report before paying? Run `/check` free in the [UnyKorn Discord](https://discord.gg/qqH42PswDd).
