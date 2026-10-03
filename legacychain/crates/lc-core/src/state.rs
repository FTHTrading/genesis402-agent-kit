//! Deterministic ledger state machine. Integer arithmetic only, no clocks, no
//! randomness: every validator applying the same block gets the same root.

use crate::crypto::verify;
use crate::envelope::Envelope;
use crate::error::{Error, Result};
use crate::genesis::{Genesis, Member, NetworkMode, Role};
use crate::tx::{Tx, TxBody, MAX_DISPATCH_BLOCKS, MAX_TX_BYTES};
use crate::types::{canonical, tagged_hash, Address, EvmAddress, H256};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub balance: u64,
    pub nonce: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberState {
    pub member: Member,
    pub suspended: bool,
    pub admitted_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RwaAsset {
    pub asset_id: String,
    pub name: String,
    pub issuer: Address,
    pub supply: u64,
    pub document_hash: H256,
    pub issued_at: u64,
    pub holders: BTreeMap<Address, u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchStatus {
    Open,
    Accepted,
    Declined,
    Reclaimed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dispatch {
    pub id: H256,
    pub from: Address,
    pub to: Address,
    pub agent_id: Option<u64>,
    pub task_envelope: H256,
    pub escrow: u64,
    pub created_at: u64,
    pub expires_at: u64,
    pub status: DispatchStatus,
    pub result_envelope: Option<H256>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BurnStatus {
    Pending,
    Paid { settlement_tx: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Burn {
    pub id: H256,
    pub owner: Address,
    pub amount: u64,
    pub payout_to: EvmAddress,
    pub height: u64,
    pub status: BurnStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct State {
    pub height: u64,
    pub tip: H256,
    pub members: BTreeMap<Address, MemberState>,
    pub accounts: BTreeMap<Address, Account>,
    pub supply: u64,
    pub deposited: u64,
    pub withdrawn: u64,
    pub assets: BTreeMap<String, RwaAsset>,
    pub dispatches: BTreeMap<H256, Dispatch>,
    pub burns: BTreeMap<H256, Burn>,
    pub used_deposits: BTreeSet<H256>,
    pub envelope_ids: BTreeSet<H256>,
}

fn bad(msg: impl Into<String>) -> Error {
    Error::InvalidTx(msg.into())
}

impl State {
    pub fn from_genesis(g: &Genesis) -> Result<Self> {
        g.validate()?;
        let mut s = State {
            height: 0,
            tip: g.hash(),
            members: BTreeMap::new(),
            accounts: BTreeMap::new(),
            supply: 0,
            deposited: 0,
            withdrawn: 0,
            assets: BTreeMap::new(),
            dispatches: BTreeMap::new(),
            burns: BTreeMap::new(),
            used_deposits: BTreeSet::new(),
            envelope_ids: BTreeSet::new(),
        };
        for m in &g.members {
            s.members.insert(m.address, MemberState { member: m.clone(), suspended: false, admitted_at: 0 });
            s.accounts.insert(m.address, Account::default());
        }
        for (a, amt) in &g.allocations {
            let acct = s.accounts.get_mut(a).expect("validated: allocation to member");
            acct.balance = acct.balance.checked_add(*amt).ok_or_else(|| Error::InvalidGenesis("overflow".into()))?;
            s.supply = s.supply.checked_add(*amt).ok_or_else(|| Error::InvalidGenesis("overflow".into()))?;
        }
        Ok(s)
    }

    pub fn root(&self) -> H256 {
        tagged_hash(b"lc/state/v1", &[&canonical(self)])
    }

    pub fn member(&self, a: &Address) -> Option<&Member> {
        self.members.get(a).map(|m| &m.member)
    }

    pub fn member_by_handle(&self, handle: &str) -> Option<&Member> {
        self.members.values().map(|m| &m.member).find(|m| m.handle.eq_ignore_ascii_case(handle))
    }

    pub fn balance(&self, a: &Address) -> u64 {
        self.accounts.get(a).map(|x| x.balance).unwrap_or(0)
    }

    pub fn next_nonce(&self, a: &Address) -> u64 {
        self.accounts.get(a).map(|x| x.nonce).unwrap_or(0)
    }

    fn active_member(&self, a: &Address) -> Result<&Member> {
        let ms = self.members.get(a).ok_or_else(|| bad(format!("{a} is not a member")))?;
        if ms.suspended {
            return Err(bad(format!("{} is suspended", ms.member.handle)));
        }
        Ok(&ms.member)
    }

    fn debit(&mut self, a: &Address, amt: u64) -> Result<()> {
        let acct = self.accounts.get_mut(a).ok_or_else(|| bad("no account"))?;
        acct.balance = acct.balance.checked_sub(amt).ok_or_else(|| bad("insufficient balance"))?;
        Ok(())
    }

    fn credit(&mut self, a: &Address, amt: u64) -> Result<()> {
        let acct = self.accounts.entry(*a).or_default();
        acct.balance = acct.balance.checked_add(amt).ok_or_else(|| bad("balance overflow"))?;
        Ok(())
    }

    fn check_envelope(&self, genesis_hash: &H256, sender: &Member, env: &Envelope) -> Result<()> {
        if env.sender != sender.address {
            return Err(bad("envelope sender must be the transaction signer"));
        }
        if self.envelope_ids.contains(&env.id) {
            return Err(bad("envelope already anchored"));
        }
        env.verify_structure(genesis_hash, &sender.sign_pk)
    }

    /// Validate and apply one transaction at the current height. On error the
    /// state is unchanged (callers apply on a scratch copy when batching).
    pub fn apply_tx(&mut self, g: &Genesis, genesis_hash: &H256, tx: &Tx) -> Result<()> {
        let mut next = self.clone();
        next.apply_tx_in_place(g, genesis_hash, tx)?;
        *self = next;
        Ok(())
    }

    fn apply_tx_in_place(&mut self, g: &Genesis, genesis_hash: &H256, tx: &Tx) -> Result<()> {
        if tx.encoded_len() > MAX_TX_BYTES {
            return Err(bad("transaction too large"));
        }
        let sender = self.active_member(&tx.from)?.clone();
        verify(&sender.sign_pk, &Tx::signing_digest(genesis_hash, &tx.from, tx.nonce, &tx.body), &tx.sig).map_err(|_| bad("bad signature"))?;
        let expected = self.next_nonce(&tx.from);
        if tx.nonce != expected {
            return Err(bad(format!("nonce {} != expected {expected}", tx.nonce)));
        }
        for env in tx.body.envelopes() {
            self.check_envelope(genesis_hash, &sender, env)?;
        }
        let tx_hash = tx.hash();
        let h = self.height;

        match &tx.body {
            TxBody::Transfer { to, amount, .. } => {
                self.active_member(to)?;
                if *amount == 0 {
                    return Err(bad("amount must be positive"));
                }
                if to == &tx.from {
                    return Err(bad("cannot transfer to self"));
                }
                self.debit(&tx.from, *amount)?;
                self.credit(to, *amount)?;
            }
            TxBody::Seal { .. } => {}
            TxBody::BridgeMint { to, deposit } => {
                if !sender.has(Role::Bridge) {
                    return Err(Error::Unauthorized("bridge role required".into()));
                }
                self.active_member(to)?;
                let devnet_ok = g.mode == NetworkMode::Devnet && deposit.dialect == "devnet";
                if !devnet_ok && deposit.network != g.bridge.network {
                    return Err(bad(format!("deposit on {} but bridge settles on {}", deposit.network, g.bridge.network)));
                }
                if g.mode == NetworkMode::Production && deposit.dialect == "devnet" {
                    return Err(bad("simulated deposits are not accepted on a production network"));
                }
                if deposit.amount < g.bridge.min_deposit || deposit.amount > g.bridge.max_deposit {
                    return Err(bad("deposit outside bridge limits"));
                }
                if deposit.settlement_tx.is_empty() || deposit.settlement_tx.len() > 128 {
                    return Err(bad("settlement reference required"));
                }
                if !self.used_deposits.insert(deposit.key()) {
                    return Err(bad("deposit already minted"));
                }
                self.credit(to, deposit.amount)?;
                self.supply = self.supply.checked_add(deposit.amount).ok_or_else(|| bad("supply overflow"))?;
                self.deposited = self.deposited.saturating_add(deposit.amount);
            }
            TxBody::BridgeBurn { amount, payout_to } => {
                if *amount == 0 {
                    return Err(bad("amount must be positive"));
                }
                if payout_to.0 == [0u8; 20] {
                    return Err(bad("payout address required"));
                }
                self.debit(&tx.from, *amount)?;
                self.supply = self.supply.checked_sub(*amount).ok_or_else(|| bad("supply underflow"))?;
                self.withdrawn = self.withdrawn.saturating_add(*amount);
                self.burns
                    .insert(tx_hash, Burn { id: tx_hash, owner: tx.from, amount: *amount, payout_to: *payout_to, height: h, status: BurnStatus::Pending });
            }
            TxBody::BridgePayoutSettled { burn_id, settlement_tx } => {
                if !sender.has(Role::Bridge) {
                    return Err(Error::Unauthorized("bridge role required".into()));
                }
                if settlement_tx.is_empty() || settlement_tx.len() > 128 {
                    return Err(bad("settlement reference required"));
                }
                let b = self.burns.get_mut(burn_id).ok_or_else(|| bad("unknown burn"))?;
                if b.status != BurnStatus::Pending {
                    return Err(bad("burn already paid"));
                }
                b.status = BurnStatus::Paid { settlement_tx: settlement_tx.clone() };
            }
            TxBody::AdmitMember { member } => {
                if !sender.has(Role::Admin) {
                    return Err(Error::Unauthorized("admin role required".into()));
                }
                member.validate(&g.chain_id, &g.erc8004)?;
                if member.has(Role::Validator) {
                    return Err(bad("the validator set is fixed at genesis"));
                }
                if self.members.contains_key(&member.address) {
                    return Err(bad("already a member"));
                }
                if self.member_by_handle(&member.handle).is_some() {
                    return Err(bad(format!("handle '{}' taken", member.handle)));
                }
                if let Some(b) = &member.erc8004 {
                    if self.members.values().any(|m| m.member.erc8004.as_ref().map(|x| x.agent_id) == Some(b.agent_id)) {
                        return Err(bad(format!("ERC-8004 agent {} already bound", b.agent_id)));
                    }
                }
                self.members.insert(member.address, MemberState { member: member.clone(), suspended: false, admitted_at: h });
                self.accounts.entry(member.address).or_default();
            }
            TxBody::SetSuspended { member, suspended } => {
                if !sender.has(Role::Admin) {
                    return Err(Error::Unauthorized("admin role required".into()));
                }
                if member == &tx.from {
                    return Err(bad("cannot change own suspension"));
                }
                let ms = self.members.get_mut(member).ok_or_else(|| bad("unknown member"))?;
                if ms.member.has(Role::Validator) {
                    return Err(bad("validators cannot be suspended by transaction"));
                }
                ms.suspended = *suspended;
            }
            TxBody::RwaIssue { asset_id, name, supply, document_hash, .. } => {
                if !(sender.has(Role::Issuer) || sender.has(Role::Admin)) {
                    return Err(Error::Unauthorized("issuer or admin role required".into()));
                }
                if asset_id.len() < 2 || asset_id.len() > 32 || !asset_id.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-') {
                    return Err(bad("asset_id must be 2-32 chars of [A-Z0-9-]"));
                }
                if name.is_empty() || name.len() > 128 {
                    return Err(bad("name must be 1-128 chars"));
                }
                if *supply == 0 {
                    return Err(bad("supply must be positive"));
                }
                if self.assets.contains_key(asset_id) {
                    return Err(bad("asset_id taken"));
                }
                let mut holders = BTreeMap::new();
                holders.insert(tx.from, *supply);
                self.assets.insert(
                    asset_id.clone(),
                    RwaAsset {
                        asset_id: asset_id.clone(),
                        name: name.clone(),
                        issuer: tx.from,
                        supply: *supply,
                        document_hash: *document_hash,
                        issued_at: h,
                        holders,
                    },
                );
            }
            TxBody::RwaTransfer { asset_id, to, amount, .. } => {
                self.active_member(to)?;
                if *amount == 0 || to == &tx.from {
                    return Err(bad("invalid RWA transfer"));
                }
                let asset = self.assets.get_mut(asset_id).ok_or_else(|| bad("unknown asset"))?;
                let held = asset.holders.get(&tx.from).copied().unwrap_or(0);
                let left = held.checked_sub(*amount).ok_or_else(|| bad("insufficient RWA balance"))?;
                if left == 0 {
                    asset.holders.remove(&tx.from);
                } else {
                    asset.holders.insert(tx.from, left);
                }
                let dest = asset.holders.entry(*to).or_insert(0);
                *dest = dest.checked_add(*amount).ok_or_else(|| bad("overflow"))?;
            }
            TxBody::AgentDispatch { to, agent_id, task, escrow, expires_in_blocks } => {
                self.active_member(to)?;
                if to == &tx.from {
                    return Err(bad("cannot dispatch to self"));
                }
                if *expires_in_blocks == 0 || *expires_in_blocks > MAX_DISPATCH_BLOCKS {
                    return Err(bad(format!("expires_in_blocks must be 1..={MAX_DISPATCH_BLOCKS}")));
                }
                if let Some(id) = agent_id {
                    let bound = sender.erc8004.as_ref().map(|b| b.agent_id) == Some(*id);
                    if !bound {
                        return Err(bad(format!("sender is not bound to ERC-8004 agent {id}")));
                    }
                }
                if *escrow > 0 {
                    self.debit(&tx.from, *escrow)?;
                }
                self.dispatches.insert(
                    tx_hash,
                    Dispatch {
                        id: tx_hash,
                        from: tx.from,
                        to: *to,
                        agent_id: *agent_id,
                        task_envelope: task.id,
                        escrow: *escrow,
                        created_at: h,
                        expires_at: h.saturating_add(*expires_in_blocks),
                        status: DispatchStatus::Open,
                        result_envelope: None,
                    },
                );
            }
            TxBody::AgentRespond { dispatch_id, accept, result } => {
                let d = self.dispatches.get(dispatch_id).ok_or_else(|| bad("unknown dispatch"))?.clone();
                if d.to != tx.from {
                    return Err(Error::Unauthorized("only the recipient can respond".into()));
                }
                if d.status != DispatchStatus::Open {
                    return Err(bad("dispatch is closed"));
                }
                if h > d.expires_at {
                    return Err(bad("dispatch expired"));
                }
                if d.escrow > 0 {
                    let payee = if *accept { d.to } else { d.from };
                    self.credit(&payee, d.escrow)?;
                }
                let entry = self.dispatches.get_mut(dispatch_id).expect("present");
                entry.status = if *accept { DispatchStatus::Accepted } else { DispatchStatus::Declined };
                entry.result_envelope = result.as_ref().map(|e| e.id);
            }
            TxBody::AgentReclaim { dispatch_id } => {
                let d = self.dispatches.get(dispatch_id).ok_or_else(|| bad("unknown dispatch"))?.clone();
                if d.from != tx.from {
                    return Err(Error::Unauthorized("only the sender can reclaim".into()));
                }
                if d.status != DispatchStatus::Open {
                    return Err(bad("dispatch is closed"));
                }
                if h <= d.expires_at {
                    return Err(bad(format!("dispatch open until block {}", d.expires_at)));
                }
                if d.escrow > 0 {
                    self.credit(&d.from, d.escrow)?;
                }
                self.dispatches.get_mut(dispatch_id).expect("present").status = DispatchStatus::Reclaimed;
            }
        }

        for env in tx.body.envelopes() {
            self.envelope_ids.insert(env.id);
        }
        self.accounts.entry(tx.from).or_default().nonce += 1;
        Ok(())
    }
}
