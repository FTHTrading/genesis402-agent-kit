# USDT on Tron vs. LegacyChain

**The short version:** USDT on Tron moves dollars on a public ledger that anyone can read. LegacyChain moves dollars, and the documents, contracts and instructions that go with them, inside a private network. Only the people a payload is addressed to can open it. Money enters and leaves as USDC on Base over the same x402/MPP rails UnyKorn's agents already pay on.

| | USDT on Tron | LegacyChain |
|---|---|---|
| Who can see a payment | Everyone, permanently: every address, amount and timestamp is on Tronscan | Only members, using signed requests. Outsiders see nothing; validators see amounts but never envelope contents or recipients |
| What you can send | A token balance, plus a public memo | Money, contracts, files, RWA terms and agent tasks, each sealed to its recipients |
| Information with the money | Has to travel separately (email, WhatsApp, PDF) and is unlinked and unauthenticated | Sealed into the same transaction, signed by the sender, and bound to this chain's genesis |
| Fees | Paid in TRX energy on every transfer (a USDT transfer burns tens of thousands of energy units; it costs real money unless you stake TRX) | Zero inside the network. Base gas applies only when money enters or leaves |
| Finality | 3-second blocks, irreversible after solidification (~19 blocks, about a minute) | Final as soon as a validator quorum signs. No reorgs |
| Identity | None. An address is just a key | Every member is admitted, has a known handle, and can be bound to an ERC-8004 agent on Base |
| Conditional payments | Needs a separate smart contract | Built in: send an agent with escrow, which releases on acceptance and refunds on decline or expiry |
| Control | The issuer can freeze addresses, and the network is run by 27 elected super representatives | You and your counterparties run the validators (HQ, London, Alaska), so the network can't be paused by a third party. USDC freeze risk still applies to the treasury on Base |
| Counterparty optics | UNODC's January 2024 report on Southeast Asian money laundering named USDT on Tron as a preferred rail for scam and laundering networks | Every member is admitted, every deposit traces to a settled Base USDC transfer, and every block is signed by named validators |

## What "more than money" looks like (all in the demo)

1. **Trade finance.** HQ sends an agent to Alaska with 5,000 in escrow: "inspect warehouse receipts for lot 7, accept if they match." Alaska accepts with a sealed result, and the escrow releases in the same block.
2. **Deal flow.** A term sheet is sealed to London and Alaska. Stravin is also a member, but they weren't addressed, so their inbox opens nothing.
3. **RWA.** A note is issued with its offering memorandum sealed to the holders. Only the document's hash is visible to the network, and units then transfer between members.
4. **Invoices with the payment.** The transfer to London carries the invoice text, which only London can read.
5. **Legacy instructions.** The chain's own namesake: letters, wills and access instructions sealed to beneficiaries, anchored at a block height, and openable only with their keys.

## Running it in front of Stravin

```bash
cd legacychain && demo/devnet.sh
```

The script runs three validators (HQ, London, Alaska) and prints every step above. It ends by showing three things: an anonymous request is refused, all three validators report the same height and genesis hash, and the x402 and MPP deposits produced real EIP-3009 signatures (settlement is simulated on devnet; on the production genesis it settles on Base through the facilitator).
