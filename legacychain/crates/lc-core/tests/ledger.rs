use lc_core::block::{build_block, execute_block, sign_vote, verify_quorum};
use lc_core::crypto::{eip191_digest, evm_address_of, evm_key_from_hex, evm_sign, MemberKeys, TransferAuthorization};
use lc_core::envelope::{Envelope, PayloadKind, Recipient, SealedPayload};
use lc_core::genesis::*;
use lc_core::payments::{self, DepositOrder};
use lc_core::state::{DispatchStatus, State};
use lc_core::tx::{DepositProof, Tx, TxBody};
use lc_core::types::{Bytes, EvmAddress, H256};
use std::collections::BTreeSet;

struct Net {
    g: Genesis,
    gh: H256,
    hq: MemberKeys,
    london: MemberKeys,
    alaska: MemberKeys,
    v2: MemberKeys,
    v3: MemberKeys,
}

fn member(handle: &str, k: &MemberKeys, roles: &[Role]) -> Member {
    Member {
        handle: handle.into(),
        address: k.address(),
        sign_pk: k.sign_pk(),
        seal_pk: k.seal_pk(),
        roles: roles.iter().copied().collect::<BTreeSet<_>>(),
        erc8004: None,
    }
}

fn net(mode: NetworkMode, alloc: u64) -> Net {
    let hq = MemberKeys::generate();
    let london = MemberKeys::generate();
    let alaska = MemberKeys::generate();
    let v2 = MemberKeys::generate();
    let v3 = MemberKeys::generate();
    let g = Genesis {
        chain_id: "legacychain-test".into(),
        mode,
        created_at_ms: 1,
        asset: AssetSpec { symbol: "LUSD".into(), decimals: 6, description: "test".into() },
        bridge: BridgeSpec {
            network: "eip155:8453".into(),
            evm_chain_id: BASE_CHAIN_ID,
            usdc: BASE_USDC.parse().unwrap(),
            usdc_eip712_name: "USD Coin".into(),
            usdc_eip712_version: "2".into(),
            pay_to: "0x1111111111111111111111111111111111111111".parse().unwrap(),
            min_deposit: 1_000_000,
            max_deposit: 1_000_000_000_000,
        },
        erc8004: Erc8004Spec { chain_id: BASE_CHAIN_ID, registry: ERC8004_IDENTITY_REGISTRY.parse().unwrap() },
        consensus: ConsensusParams { block_interval_ms: 500, round_timeout_ms: 4000, max_block_txs: 100, quorum: 2 },
        validators: vec![hq.address(), v2.address(), v3.address()],
        members: vec![
            member("hq", &hq, &[Role::Validator, Role::Admin, Role::Bridge, Role::Issuer]),
            member("v2", &v2, &[Role::Validator]),
            member("v3", &v3, &[Role::Validator]),
            member("london", &london, &[]),
            member("alaska", &alaska, &[]),
        ],
        allocations: if alloc > 0 { vec![(hq.address(), alloc)] } else { vec![] },
        charter_hash: None,
    };
    let gh = g.hash();
    Net { g, gh, hq, london, alaska, v2, v3 }
}

fn payload(kind: PayloadKind, body: &str) -> SealedPayload {
    SealedPayload {
        kind,
        title: "t".into(),
        content_type: "text/plain".into(),
        body: Bytes(body.as_bytes().to_vec()),
        created_at_ms: 0,
        refs: vec![],
        meta: vec![],
    }
}

fn conserved(s: &State) -> bool {
    let balances: u64 = s.accounts.values().map(|a| a.balance).sum();
    let escrow: u64 = s.dispatches.values().filter(|d| d.status == DispatchStatus::Open).map(|d| d.escrow).sum();
    balances + escrow == s.supply
}

#[test]
fn production_genesis_rejects_allocations() {
    let mut n = net(NetworkMode::Production, 0);
    n.g.validate().unwrap();
    n.g.allocations = vec![(n.hq.address(), 1)];
    assert!(n.g.validate().is_err());
}

#[test]
fn transfer_with_sealed_memo() {
    let n = net(NetworkMode::Devnet, 1_000_000_000);
    let mut s = State::from_genesis(&n.g).unwrap();
    let memo = Envelope::seal(&n.gh, &n.hq, &[Recipient { seal_pk: n.london.seal_pk() }], &payload(PayloadKind::PaymentAdvice, "invoice 42")).unwrap();
    let tx = Tx::sign(&n.gh, &n.hq, 0, TxBody::Transfer { to: n.london.address(), amount: 250_000_000, memo: Some(memo.clone()) });
    s.apply_tx(&n.g, &n.gh, &tx).unwrap();
    assert_eq!(s.balance(&n.london.address()), 250_000_000);
    assert_eq!(s.balance(&n.hq.address()), 750_000_000);
    assert!(conserved(&s));

    // Replay is rejected (nonce), and so is re-anchoring the same envelope.
    assert!(s.apply_tx(&n.g, &n.gh, &tx).is_err());
    let again = Tx::sign(&n.gh, &n.hq, 1, TxBody::Seal { envelope: memo.clone() });
    assert!(s.apply_tx(&n.g, &n.gh, &again).is_err());

    // Only London opens the memo.
    assert_eq!(memo.open(&n.gh, &n.london, &n.hq.sign_pk()).unwrap().payload.body.0, b"invoice 42");
    assert!(memo.open(&n.gh, &n.alaska, &n.hq.sign_pk()).is_err());
}

#[test]
fn signature_bound_to_genesis() {
    let n = net(NetworkMode::Devnet, 10_000_000);
    let other = net(NetworkMode::Devnet, 10_000_000);
    let mut s = State::from_genesis(&n.g).unwrap();
    let tx = Tx::sign(&other.gh, &n.hq, 0, TxBody::Transfer { to: n.london.address(), amount: 1, memo: None });
    assert!(s.apply_tx(&n.g, &n.gh, &tx).is_err());
}

#[test]
fn bridge_mint_burn_and_replay() {
    let n = net(NetworkMode::Production, 0);
    let mut s = State::from_genesis(&n.g).unwrap();
    let dep = DepositProof {
        dialect: "x402".into(),
        network: "eip155:8453".into(),
        payer: "0x2222222222222222222222222222222222222222".parse().unwrap(),
        auth_nonce: H256([7u8; 32]),
        settlement_tx: "0xabc".into(),
        amount: 5_000_000,
    };
    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 0, TxBody::BridgeMint { to: n.london.address(), deposit: dep.clone() })).unwrap();
    assert_eq!(s.supply, 5_000_000);
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::BridgeMint { to: n.london.address(), deposit: dep.clone() })).is_err(), "double mint");

    // Non-bridge members cannot mint; simulated deposits are refused in production.
    let mut fake = dep.clone();
    fake.auth_nonce = H256([8u8; 32]);
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.london, 0, TxBody::BridgeMint { to: n.london.address(), deposit: fake.clone() })).is_err());
    fake.dialect = "devnet".into();
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::BridgeMint { to: n.london.address(), deposit: fake })).is_err());

    let payout: EvmAddress = "0x3333333333333333333333333333333333333333".parse().unwrap();
    let burn = Tx::sign(&n.gh, &n.london, 0, TxBody::BridgeBurn { amount: 2_000_000, payout_to: payout });
    let burn_id = burn.hash();
    s.apply_tx(&n.g, &n.gh, &burn).unwrap();
    assert_eq!(s.supply, 3_000_000);
    assert!(conserved(&s));
    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::BridgePayoutSettled { burn_id, settlement_tx: "0xdef".into() })).unwrap();
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 2, TxBody::BridgePayoutSettled { burn_id, settlement_tx: "0xdef".into() })).is_err());
}

#[test]
fn agent_dispatch_escrow_lifecycle() {
    let n = net(NetworkMode::Devnet, 1_000_000_000);
    let mut s = State::from_genesis(&n.g).unwrap();
    let task = Envelope::seal(&n.gh, &n.hq, &[Recipient { seal_pk: n.london.seal_pk() }], &payload(PayloadKind::AgentTask, "verify title deed #88 and accept"))
        .unwrap();
    let d = Tx::sign(&n.gh, &n.hq, 0, TxBody::AgentDispatch { to: n.london.address(), agent_id: None, task, escrow: 100_000_000, expires_in_blocks: 10 });
    let id = d.hash();
    s.apply_tx(&n.g, &n.gh, &d).unwrap();
    assert_eq!(s.balance(&n.hq.address()), 900_000_000);
    assert!(conserved(&s));

    // Wrong party cannot respond; early reclaim refused.
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.alaska, 0, TxBody::AgentRespond { dispatch_id: id, accept: true, result: None })).is_err());
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::AgentReclaim { dispatch_id: id })).is_err());

    let result = Envelope::seal(&n.gh, &n.london, &[Recipient { seal_pk: n.hq.seal_pk() }], &payload(PayloadKind::AgentResult, "deed verified")).unwrap();
    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.london, 0, TxBody::AgentRespond { dispatch_id: id, accept: true, result: Some(result) })).unwrap();
    assert_eq!(s.balance(&n.london.address()), 100_000_000);
    assert_eq!(s.dispatches[&id].status, DispatchStatus::Accepted);
    assert!(conserved(&s));

    // Expiry path.
    let task2 = Envelope::seal(&n.gh, &n.hq, &[Recipient { seal_pk: n.alaska.seal_pk() }], &payload(PayloadKind::AgentTask, "x")).unwrap();
    let d2 = Tx::sign(&n.gh, &n.hq, 1, TxBody::AgentDispatch { to: n.alaska.address(), agent_id: None, task: task2, escrow: 50_000_000, expires_in_blocks: 1 });
    let id2 = d2.hash();
    s.apply_tx(&n.g, &n.gh, &d2).unwrap();
    s.height += 2;
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.alaska, 0, TxBody::AgentRespond { dispatch_id: id2, accept: true, result: None })).is_err());
    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 2, TxBody::AgentReclaim { dispatch_id: id2 })).unwrap();
    assert_eq!(s.balance(&n.hq.address()), 900_000_000);
    assert!(conserved(&s));
}

#[test]
fn rwa_issue_and_transfer() {
    let n = net(NetworkMode::Devnet, 0);
    let mut s = State::from_genesis(&n.g).unwrap();
    let terms = Envelope::seal(&n.gh, &n.hq, &[Recipient { seal_pk: n.london.seal_pk() }], &payload(PayloadKind::RwaTerms, "8% senior note, 5y")).unwrap();
    s.apply_tx(
        &n.g,
        &n.gh,
        &Tx::sign(
            &n.gh,
            &n.hq,
            0,
            TxBody::RwaIssue {
                asset_id: "LDN-NOTE-1".into(),
                name: "London senior note".into(),
                supply: 1000,
                document_hash: H256([1u8; 32]),
                terms: Some(terms),
            },
        ),
    )
    .unwrap();
    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::RwaTransfer { asset_id: "LDN-NOTE-1".into(), to: n.london.address(), amount: 250, memo: None }))
        .unwrap();
    let a = &s.assets["LDN-NOTE-1"];
    assert_eq!(a.holders[&n.london.address()], 250);
    assert_eq!(a.holders[&n.hq.address()], 750);
    assert!(
        s.apply_tx(
            &n.g,
            &n.gh,
            &Tx::sign(
                &n.gh,
                &n.london,
                0,
                TxBody::RwaIssue { asset_id: "X1".into(), name: "x".into(), supply: 1, document_hash: H256([0u8; 32]), terms: None }
            )
        )
        .is_err(),
        "non-issuer"
    );
}

#[test]
fn erc8004_binding_admission() {
    let n = net(NetworkMode::Devnet, 0);
    let mut s = State::from_genesis(&n.g).unwrap();
    let newcomer = MemberKeys::generate();
    let owner_key = evm_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318").unwrap();
    let owner = evm_address_of(owner_key.verifying_key());
    let mut m = member("envoy", &newcomer, &[]);
    let stmt = Erc8004Binding::statement(&n.g.chain_id, &n.g.erc8004, 96497, &m.address, &m.sign_pk, &m.seal_pk);
    let sig = evm_sign(&owner_key, &eip191_digest(stmt.as_bytes())).unwrap();
    m.erc8004 = Some(Erc8004Binding { agent_id: 96497, owner, signature: Bytes(sig.to_vec()) });

    let mut forged = m.clone();
    forged.erc8004.as_mut().unwrap().agent_id = 96498;
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 0, TxBody::AdmitMember { member: forged })).is_err());

    s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 0, TxBody::AdmitMember { member: m.clone() })).unwrap();
    assert!(s.member_by_handle("envoy").is_some());
    let mut validator_grab = member("grab", &MemberKeys::generate(), &[Role::Validator]);
    validator_grab.erc8004 = None;
    assert!(s.apply_tx(&n.g, &n.gh, &Tx::sign(&n.gh, &n.hq, 1, TxBody::AdmitMember { member: validator_grab })).is_err());
}

#[test]
fn block_quorum_three_validators() {
    let n = net(NetworkMode::Devnet, 1_000_000_000);
    let s0 = State::from_genesis(&n.g).unwrap();
    let txs = vec![
        Tx::sign(&n.gh, &n.hq, 0, TxBody::Transfer { to: n.london.address(), amount: 10_000_000, memo: None }),
        Tx::sign(&n.gh, &n.london, 0, TxBody::Transfer { to: n.alaska.address(), amount: 999_000_000, memo: None }), // invalid: insufficient
        Tx::sign(&n.gh, &n.hq, 1, TxBody::Transfer { to: n.alaska.address(), amount: 5_000_000, memo: None }),
    ];
    let proposer = n.g.proposer(1, 0);
    assert_eq!(proposer, n.v2.address());
    let built = build_block(&n.g, &n.gh, &s0, &txs, 0, 1000, proposer);
    assert_eq!(built.block.txs.len(), 2);
    assert_eq!(built.rejected.len(), 1);

    // Followers re-execute and agree.
    let post = execute_block(&n.g, &n.gh, &s0, &built.block, 0).unwrap();
    assert_eq!(post, built.post_state);

    let mut block = built.block.clone();
    let bh = block.header.hash(&n.gh);
    block.votes.push(sign_vote(&n.gh, &bh, &n.v2));
    assert!(verify_quorum(&n.g, &n.gh, &s0, &block).is_err(), "1 of 3 is not quorum");
    block.votes.push(sign_vote(&n.gh, &bh, &n.v2));
    assert!(verify_quorum(&n.g, &n.gh, &s0, &block).is_err(), "a duplicate vote does not count twice");
    block.votes.push(sign_vote(&n.gh, &bh, &n.v3));
    verify_quorum(&n.g, &n.gh, &s0, &block).unwrap();
    let _ = &n.hq;

    // Wrong proposer and tampered state root are rejected.
    let wrong = build_block(&n.g, &n.gh, &s0, &txs, 0, 1000, n.hq.address());
    assert!(execute_block(&n.g, &n.gh, &s0, &wrong.block, 0).is_err());
    let mut tampered = built.block.clone();
    tampered.header.state_root = H256([0u8; 32]);
    assert!(execute_block(&n.g, &n.gh, &s0, &tampered, 0).is_err());
}

fn signed_auth(n: &Net, value: u128, now: u64, payer_hex: &str) -> (TransferAuthorization, [u8; 65]) {
    let k = evm_key_from_hex(payer_hex).unwrap();
    let auth = TransferAuthorization {
        from: evm_address_of(k.verifying_key()),
        to: n.g.bridge.pay_to,
        value,
        valid_after: 0,
        valid_before: now + 300,
        nonce: H256([0x42; 32]),
    };
    let sig = evm_sign(&k, &auth.digest(&n.g.eip712_domain())).unwrap();
    (auth, sig)
}

#[test]
fn x402_and_mpp_deposit_verification() {
    let n = net(NetworkMode::Production, 0);
    let secret = b"node-secret";
    let now = 1_800_000_000u64;
    let order = DepositOrder::new(secret, &n.gh, n.london.address(), 25_000_000, now);
    let req = payments::x402_requirements(&n.g, &order);
    let (auth, sig) = signed_auth(&n, 25_000_000, now, "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318");

    let header = payments::build_x402_payment(&req, None, &auth, &sig);
    let v = payments::verify_x402(&n.g, &n.gh, secret, &header, now).unwrap();
    assert_eq!(v.order.to, n.london.address());
    assert_eq!(v.authorization.from, auth.from);

    // Wrong amount, wrong secret, expired quote, redirected recipient all fail.
    let (bad_auth, bad_sig) = signed_auth(&n, 24_000_000, now, "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318");
    assert!(payments::verify_x402(&n.g, &n.gh, secret, &payments::build_x402_payment(&req, None, &bad_auth, &bad_sig), now).is_err());
    assert!(payments::verify_x402(&n.g, &n.gh, b"other", &header, now).is_err());
    assert!(payments::verify_x402(&n.g, &n.gh, secret, &header, now + 10_000).is_err());
    let mut redirected = order.clone();
    redirected.to = n.alaska.address();
    let req2 = payments::x402_requirements(&n.g, &redirected);
    assert!(payments::verify_x402(&n.g, &n.gh, secret, &payments::build_x402_payment(&req2, None, &auth, &sig), now).is_err());

    // MPP: echo the challenge, same EIP-3009 payload.
    let www = payments::mpp_challenge(&n.g, &order, "legacychain.app");
    let get = |k: &str| -> String {
        let start = www.find(&format!("{k}=\"")).unwrap() + k.len() + 2;
        www[start..start + www[start..].find('"').unwrap()].to_string()
    };
    let cred = serde_json::json!({
        "challenge": { "id": get("id"), "realm": get("realm"), "method": get("method"), "intent": get("intent"), "request": get("request"), "expires": get("expires") },
        "payload": {
            "signature": format!("0x{}", hex::encode(sig)),
            "authorization": { "from": auth.from.to_string(), "to": auth.to.to_string(), "value": auth.value.to_string(), "validAfter": "0", "validBefore": auth.valid_before.to_string(), "nonce": format!("0x{}", auth.nonce) }
        }
    });
    let v = payments::verify_mpp(&n.g, &n.gh, secret, &payments::b64url(&cred), now).unwrap();
    assert_eq!(v.dialect, "mpp");
    assert_eq!(v.payment_payload["accepted"]["payTo"], n.g.bridge.pay_to.to_string());
}
