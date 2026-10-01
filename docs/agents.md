# UnyKorn agent directory

Every UnyKorn agent has its own ERC-8004 identity on Base. Each link below opens the agent's on-chain record on 8004scan, so you can check it yourself before you put it to work.

- **Identity Registry:** [`0x8004A169FB4a3325136EB29fA0ceB6D2e539a432`](https://basescan.org/address/0x8004A169FB4a3325136EB29fA0ceB6D2e539a432) on Base
- **Owner of every identity:** [`0xFCc1D28Ad797d65CDDb2A7219c5BE6B062653cb3`](https://basescan.org/address/0xFCc1D28Ad797d65CDDb2A7219c5BE6B062653cb3) (UnyKorn LLC, Wyoming)
- **Live directory:** [twin.unykorn.org](https://twin.unykorn.org)

## The rail

| Agent | ERC-8004 | Role |
|---|---:|---|
| Genesis402 | [#95721](https://8004scan.io/agents/base/95721) | Pay-per-call APIs for AI agents over x402 |

## UnyKorn Control desk

| Agent | ERC-8004 | Role |
|---|---:|---|
| The Gatekeeper | [#96492](https://8004scan.io/agents/base/96492) | Front door of the UnyKorn Control desk |
| The Examiner | [#96494](https://8004scan.io/agents/base/96494) | Checks the public record before anyone trusts; runs the Global Capital Desk's checks |
| The Closer | [#96495](https://8004scan.io/agents/base/96495) | Turns loose material into clean records |
| The Treasurer | [#96496](https://8004scan.io/agents/base/96496) | Pays the desk's bills inside a published policy |
| The Envoy | [#96497](https://8004scan.io/agents/base/96497) | Carries the message out |

## Global Capital Desk

Compliance agents for cross-border deals: banks, companies and people checked against official registries and sanctions lists before any money moves. UnyKorn never holds deal funds.

| Agent | ERC-8004 | Scope |
|---|---:|---|
| Sanctions Screen · UN · OFAC · EU · UK | [#96609](https://8004scan.io/agents/base/96609) | Sanctions screening |
| Bank Verification | [#96610](https://8004scan.io/agents/base/96610) | Bank and counterparty-bank checks |
| GLEIF Registry | [#96611](https://8004scan.io/agents/base/96611) | Legal-entity (LEI) registry lookup |
| SEC EDGAR | [#96612](https://8004scan.io/agents/base/96612) | US public-company filings and financials |
| Counterparty Report | [#96613](https://8004scan.io/agents/base/96613) | Signed counterparty report |
| Wallet & Chain Risk | [#96614](https://8004scan.io/agents/base/96614) | Crypto wallet, token and contract risk |

### Desk endpoints

| Endpoint | Price | What it returns | MCP tool |
|---|---:|---|---|
| `/report/counterparty` | $0.25 | A bank or company resolved in the GLEIF registry, its reported parent group walked, every entity screened against the UN, US, EU and UK sanctions lists, delivered as a signed PDF with a public verify page | `genesis402_counterparty_report` |
| `/screen/entity` | $0.01 | Official registry record for a company: legal name, LEI, jurisdiction, status and reported parents (GLEIF, SEC EDGAR) | `genesis402_company_lookup` |
| `/screen/deal` | $0.05 | The text of an offer or funding package checked against published regulator warnings on prime-bank, SBLC/MT760 monetization and advance-fee schemes. Every flag cites its official source; the text itself is never stored | `genesis402_call` (find it with `genesis402_catalog`) |
| `/screen/name` | $0.02 | A person or company name checked against the UN, US OFAC, EU and UK lists, fetched from the official publishers | `genesis402_sanctions_name_screen` |
| `/jurisdictions` | free | Jurisdiction packs (see below) | — |

**Jurisdiction packs** cover the US, UK, EU, UAE, Singapore, Hong Kong, Switzerland, Canada, Cayman, Germany, France and Japan: which sanctions lists apply, the official company registry, the regulator register to check a bank or broker, and when a broker-dealer route is generally required. Every link is an official source, checked 30 September 2026. Reference information, not legal advice.

Every result is signed. A real signed receipt: [`/verify/g402-1e3079710d0501c7`](https://twin.unykorn.org/verify/g402-1e3079710d0501c7).

## Sales team

Ten agents for the UnyKorn sales team, each open and ready for its owner.

| Agent | ERC-8004 |
|---|---:|
| Aurora | [#96595](https://8004scan.io/agents/base/96595) |
| Blaze | [#96596](https://8004scan.io/agents/base/96596) |
| Cobalt | [#96597](https://8004scan.io/agents/base/96597) |
| Drift | [#96598](https://8004scan.io/agents/base/96598) |
| Ember | [#96599](https://8004scan.io/agents/base/96599) |
| Frost | [#96600](https://8004scan.io/agents/base/96600) |
| Glint | [#96601](https://8004scan.io/agents/base/96601) |
| Halo | [#96602](https://8004scan.io/agents/base/96602) |
| Ion | [#96603](https://8004scan.io/agents/base/96603) |
| Jade | [#96604](https://8004scan.io/agents/base/96604) |

---

<sub>Screening and registry signals only: not KYC, not a compliance determination, not legal advice.</sub>
