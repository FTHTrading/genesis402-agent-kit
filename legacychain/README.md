# LegacyChain

A private, zero-fee Rust chain for **legacychain.app**. Members move money, contracts, documents, RWA instruments, and agent tasks to each other. Every payload is sealed so only the people it's addressed to can open it. Real money comes in and goes out as USDC on Base over **x402** and **MPP**. Members can bind their identity to an **ERC-8004** agent, the same registry the sixteen UnyKorn agents use (`0x8004A169…a432` on Base).

```
 HQ (validator, bridge)        London (validator)          Alaska (validator)
 hq.chain.legacychain.app  ⇄  london.chain.legacychain.app ⇄ alaska.chain.legacychain.app
        │   Cloudflare Tunnel; no open ports. Signed peer requests. Each block needs a quorum.
        │
        ├─ x402 / MPP deposit ──► USDC on Base ──► treasury (BitGo / Safe multisig)
        └─ ERC-8004 ownerOf ────► Identity Registry on Base
```

## What it does

| You want to… | Command | What goes on chain |
|---|---|---|
| Send money to London | `lc send --to london --amount 250000 --memo "Invoice 42"` | Transfer, plus a memo envelope only London can open |
| Send a contract or file | `lc seal --to london,alaska --kind contract --file spa.pdf` | Ciphertext only. Validators can't see who it's for |
| Send an agent with money attached | `lc dispatch --to alaska --escrow 5000 --task "verify lot 7"` | Sealed task. Escrow is released when Alaska accepts and refunded if they decline or the dispatch expires |
| Unwrap what arrived | `lc inbox` | Nothing new: the CLI scans and decrypts locally |
| Issue an RWA | `lc rwa-issue --asset-id LDN-NOTE-2031 --document om.pdf --holders london` | Public: the document hash. Sealed to the holders: the document itself |
| Bring in real money | `lc deposit --amount 25` (x402) or `--mpp` | Mint, after the USDC has settled on Base |
| Take money out | `lc withdraw --amount 5 --payout-to 0x…` | Burn, then the bridge records the Base payout |

Run all of it locally across three validators:

```bash
cd legacychain && demo/devnet.sh
```

## How "locked into the genesis, unwrapped on arrival" works

1. **Genesis.** The genesis fixes the chain id, the validators, the founding members and their keys, the USDC-on-Base treasury, the ERC-8004 registry, and the sha256 of your founding charter. Its hash goes into every transaction signature, every block vote, and every envelope key derivation. Anything produced on this chain is cryptographically invalid on any other chain.
2. **Sealing.** Each envelope gets a fresh content key, and the payload is encrypted with XChaCha20-Poly1305. For each recipient, the content key is wrapped using X25519 + HKDF-SHA256, with the genesis hash as the salt. Recipient slots are opaque 16-byte tags that are shuffled and padded with decoys, and payload sizes are padded to buckets. The chain therefore learns neither who an envelope is for nor exactly how large it is. The sender signs the whole envelope with Ed25519.
3. **Unwrapping.** A recipient's CLI pulls envelope ciphertexts (members only), finds its tag, unwraps the content key, checks the sender's signature against the directory, and only then decrypts. If anything is tampered with, or the envelope was sealed under a different genesis, it fails closed.

## Privacy model

| Party | Sees | Does not see |
|---|---|---|
| The internet | Each node's health line (chain id, height) and the 402 deposit quote | Accounts, balances, transfers, envelopes, members: every other read needs a member signature |
| A stolen disk | Ciphertext (`LC_STORAGE_KEY` encrypts the block log at rest) | Anything |
| A member | The directory, their own account, and envelope ciphertexts (so they can scan) | Other members' balances, and envelopes not addressed to them |
| A validator operator (you, London, Alaska) | Balances and transfer amounts/parties, because they need them to validate | Envelope contents and envelope recipients |
| Base | USDC entering and leaving the treasury | Anything that happens inside LegacyChain |

**Next step if amounts must be hidden from validators too:** add confidential amounts (Pedersen commitments plus Bulletproofs range proofs). Validators would then check that value is conserved without seeing amounts. The state machine is set up so this slots in as a new transfer type.

## Real money: x402 and MPP

`POST /v1/bridge/deposit {"to":"london","amount":"25"}` with no payment returns **402** carrying two challenges:

* `PAYMENT-REQUIRED`: an x402 v2 `exact` requirement for USDC on `eip155:8453`, paid to the treasury.
* `WWW-Authenticate: Payment id=… method="usdc" intent="charge" request=…` (MPP).

Both challenges contain a node-issued HMAC order that binds the LegacyChain recipient and the amount, so a captured payment can't be redirected to anyone else. The payer signs an EIP-3009 `transferWithAuthorization`. The digest and signature are byte-identical to viem's, and a test vector pins this. The node verifies the payment locally, settles it through your x402 facilitator (`LC_FACILITATOR_URL`, `POST /settle`), journals the settlement to disk, and then mints. If the node crashes after settlement but before the mint, the mint is replayed at boot. Each EIP-3009 nonce mints exactly once. A production genesis refuses pre-allocated balances, so supply only enters through settled deposits.

Withdrawals burn immediately and appear in `GET /v1/bridge/burns`. The bridge operator pays out from the multisig (a human approval gate), then runs `lc payout-settled --burn <id> --settlement-tx 0x…`.

## ERC-8004

`lc bind-8004 --member envoy.member.json --agent-id 96497 --chain-id <id>` prints a statement for the agent's owner to `personal_sign` (or signs it with `LC_EVM_KEY`). The statement names this chain, the registry, the agent id, and the member's two public keys. Validators verify the signature. When `LC_BASE_RPC` is set, the node also checks `ownerOf(agentId)` on Base before admitting the member. `lc dispatch --agent-id 96497` then marks a dispatch as sent by that on-chain agent, and `lc agent 96497` reads the live owner and `tokenURI`.

## Consensus

The validator set is fixed at genesis. The proposer for `(height, round)` is `validators[(height+round) % n]`. Every validator re-executes the proposed block and signs its hash, at most once per height. That vote is fsync'd before it is sent, so a restart can't cause a double vote. A block is final once it has `quorum` signatures, so there are no reorgs and no confirmations to wait for. The default quorum is BFT `⌊2n/3⌋+1` for four or more validators, or a strict majority for smaller sets: with three sites, any two finalize, so one site can be offline. If a proposer is down, the round rotates after `round_timeout_ms`. A node that falls behind syncs from its peers and verifies every quorum certificate.

Tested in this repo: unit and integration tests for the ledger, envelopes, payments, and storage. `demo/devnet.sh` runs end to end. A crash test was also run: one validator was killed, the other two kept finalizing with a rotated proposer, and the killed validator caught up when it restarted.

Known limit: if a proposer crashes after collecting some votes but short of a quorum, that height can stall until an operator restarts the stuck validators. Full lock-and-unlock view change in the Tendermint style is the next consensus step.

## Deploy on legacychain.app

The Rust node runs on a server, not inside Workers. Each site gets its own subdomain through a Cloudflare Tunnel, which leaves your existing routes (`www.legacychain.app/*`, `/approvals/`, `/unykorn`) untouched.

1. **Keys, at each site:** `LC_PASSPHRASE=… lc keygen --handle london --encrypt`. Send only `london.member.json` to HQ.
2. **Genesis, at HQ:**
   ```bash
   lc genesis --chain-id legacychain-1 --mode production \
     --member hq.member.json --member london.member.json --member alaska.member.json \
     --role hq=validator,admin,bridge,issuer --role london=validator --role alaska=validator \
     --pay-to 0x<BitGo or Safe treasury on Base> --charter charter.pdf --out genesis.json
   ```
   Send `genesis.json` to every site and pin its hash (`LC_GENESIS_HASH`) in every client.
3. **Tunnel, at each site:** Cloudflare Zero Trust → Networks → Tunnels → Create. Public hostname `<site>.chain.legacychain.app` → `http://node:7402`. Paste the token into `deploy/site.env`.
4. **Run:** `cd deploy && docker compose --env-file site.env up -d`.
5. **Members:** admins admit new members with `lc admit --member x.member.json`. That's the KYC gate. Run members through your compliance check before you admit them.

Operating the bridge for third parties is money transmission. Keep the treasury with your licensed partner and custody stack (BitGo), and let `AdmitMember` stand in for the KYC/KYB result.

## Layout

```
crates/lc-core   ledger state machine, genesis, sealed envelopes, x402/MPP + EIP-3009/712, ERC-8004 binding, auth
crates/lc-node   legacychain-node: axum API, quorum consensus, sync, encrypted append-only store, bridge
crates/lc-cli    lc: keys, genesis, send, seal, inbox, dispatch, RWA, deposit, withdraw
demo/devnet.sh   three validators, every flow, one command
deploy/          docker compose + Cloudflare Tunnel per site
```
