//! Genesis: the founding record of a LegacyChain network.
//!
//! The genesis fixes the chain id, validator set, founding members, the
//! settlement asset and its Base USDC backing, the ERC-8004 registry members
//! bind to, and an optional charter hash (the founding agreement, whose text is
//! sealed to the founders in block 1). Its hash is mixed into every signature,
//! every vote and every envelope key, so nothing produced on this chain is
//! valid anywhere else.

use crate::crypto::{ecrecover, eip191_digest, verify};
use crate::error::{Error, Result};
use crate::types::{canonical, tagged_hash, Address, Bytes, EvmAddress, Pk32, Sig64, H256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Base mainnet chain id and contracts.
pub const BASE_CHAIN_ID: u64 = 8453;
pub const BASE_USDC: &str = "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913";
/// ERC-8004 Identity Registry on Base (the registry the UnyKorn agents live in).
pub const ERC8004_IDENTITY_REGISTRY: &str = "0x8004a169fb4a3325136eb29fa0ceb6d2e539a432";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// Test network: genesis allocations allowed, deposits may be simulated.
    Devnet,
    /// Real money: supply only enters through settled Base USDC deposits.
    Production,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// Signs blocks. Fixed at genesis.
    Validator,
    /// Admits and suspends members.
    Admin,
    /// Mints against settled deposits and records payouts.
    Bridge,
    /// May issue RWA instruments.
    Issuer,
}

/// Link from a LegacyChain member to an ERC-8004 agent identity on Base.
///
/// The EVM account that owns the agent NFT signs (EIP-191) a statement naming
/// this chain, the agent, and the member's two public keys. Validators verify
/// the signature deterministically; nodes additionally check `ownerOf` against
/// a Base RPC before admitting a member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc8004Binding {
    pub agent_id: u64,
    pub owner: EvmAddress,
    pub signature: Bytes,
}

impl Erc8004Binding {
    pub fn statement(chain_id: &str, registry: &Erc8004Spec, agent_id: u64, address: &Address, sign_pk: &Pk32, seal_pk: &Pk32) -> String {
        format!(
            "LegacyChain identity binding\nchain: {chain_id}\nregistry: eip155:{}:{}\nagent: {agent_id}\nmember: {address}\nsign_key: {sign_pk}\nseal_key: {seal_pk}",
            registry.chain_id, registry.registry
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub handle: String,
    pub address: Address,
    pub sign_pk: Pk32,
    pub seal_pk: Pk32,
    pub roles: BTreeSet<Role>,
    pub erc8004: Option<Erc8004Binding>,
}

impl Member {
    pub fn has(&self, r: Role) -> bool {
        self.roles.contains(&r)
    }

    /// Deterministic validity checks (run at genesis and on admission).
    pub fn validate(&self, chain_id: &str, registry: &Erc8004Spec) -> Result<()> {
        if self.handle.is_empty() || self.handle.len() > 32 || !self.handle.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(Error::InvalidTx(format!("handle '{}' must be 1-32 chars of [A-Za-z0-9_-]", self.handle)));
        }
        if Address::from_sign_key(&self.sign_pk) != self.address {
            return Err(Error::InvalidTx(format!("address {} does not match signing key", self.address)));
        }
        if let Some(b) = &self.erc8004 {
            let msg = Erc8004Binding::statement(chain_id, registry, b.agent_id, &self.address, &self.sign_pk, &self.seal_pk);
            let signer = ecrecover(&eip191_digest(msg.as_bytes()), &b.signature.0).map_err(|e| Error::InvalidTx(format!("ERC-8004 binding: {e}")))?;
            if signer != b.owner {
                return Err(Error::InvalidTx(format!("ERC-8004 binding signed by {signer}, expected agent owner {}", b.owner)));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetSpec {
    pub symbol: String,
    pub decimals: u8,
    pub description: String,
}

/// Where real money enters and leaves: USDC on Base, paid over x402 / MPP.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeSpec {
    /// CAIP-2 network id, e.g. "eip155:8453".
    pub network: String,
    pub evm_chain_id: u64,
    pub usdc: EvmAddress,
    /// EIP-712 domain of the USDC contract (USDC on Base: "USD Coin", "2").
    pub usdc_eip712_name: String,
    pub usdc_eip712_version: String,
    /// Treasury that receives deposits (a BitGo / Safe multisig in production).
    pub pay_to: EvmAddress,
    /// Smallest and largest single deposit, atomic units.
    pub min_deposit: u64,
    pub max_deposit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Erc8004Spec {
    pub chain_id: u64,
    pub registry: EvmAddress,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsensusParams {
    /// Target block interval when transactions are waiting.
    pub block_interval_ms: u64,
    /// Time a validator waits for a proposer before moving to the next round.
    pub round_timeout_ms: u64,
    pub max_block_txs: u32,
    /// Votes needed to finalize a block. Must be a strict majority of the
    /// validator set (so any two quorums share a validator). `2n/3 + 1`
    /// tolerates Byzantine validators when n >= 4; a plain majority tolerates
    /// crashed validators in small, trusted sets (3 nodes: 2 of 3).
    pub quorum: u32,
}

/// Recommended quorum for `n` validators: BFT `floor(2n/3) + 1` from four
/// validators up, strict majority below that.
pub fn default_quorum(n: usize) -> u32 {
    if n >= 4 {
        (n * 2 / 3 + 1) as u32
    } else {
        (n / 2 + 1) as u32
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Genesis {
    pub chain_id: String,
    pub mode: NetworkMode,
    pub created_at_ms: u64,
    pub asset: AssetSpec,
    pub bridge: BridgeSpec,
    pub erc8004: Erc8004Spec,
    pub consensus: ConsensusParams,
    /// Ordered validator set; proposer for (height, round) is
    /// `validators[(height + round) % n]`.
    pub validators: Vec<Address>,
    pub members: Vec<Member>,
    /// Devnet only: opening balances.
    pub allocations: Vec<(Address, u64)>,
    /// sha256 of the founding charter / operating agreement.
    pub charter_hash: Option<H256>,
}

impl Genesis {
    pub fn hash(&self) -> H256 {
        tagged_hash(b"lc/genesis/v1", &[&canonical(self)])
    }

    pub fn quorum(&self) -> usize {
        self.consensus.quorum as usize
    }

    pub fn proposer(&self, height: u64, round: u32) -> Address {
        let n = self.validators.len() as u64;
        self.validators[((height + round as u64) % n) as usize]
    }

    pub fn eip712_domain(&self) -> crate::crypto::Eip712Domain {
        crate::crypto::Eip712Domain {
            name: self.bridge.usdc_eip712_name.clone(),
            version: self.bridge.usdc_eip712_version.clone(),
            chain_id: self.bridge.evm_chain_id,
            verifying_contract: self.bridge.usdc,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.chain_id.is_empty() || self.chain_id.len() > 64 {
            return Err(Error::InvalidGenesis("chain_id must be 1-64 chars".into()));
        }
        if self.asset.decimals as u32 != crate::types::DECIMALS {
            return Err(Error::InvalidGenesis(format!("asset decimals must be {} (USDC parity)", crate::types::DECIMALS)));
        }
        if self.bridge.min_deposit == 0 || self.bridge.min_deposit > self.bridge.max_deposit {
            return Err(Error::InvalidGenesis("bridge deposit limits invalid".into()));
        }
        if self.consensus.max_block_txs == 0 || self.consensus.block_interval_ms == 0 || self.consensus.round_timeout_ms < self.consensus.block_interval_ms {
            return Err(Error::InvalidGenesis("consensus params invalid".into()));
        }
        let mut addrs = BTreeSet::new();
        let mut handles = BTreeSet::new();
        for m in &self.members {
            m.validate(&self.chain_id, &self.erc8004).map_err(|e| Error::InvalidGenesis(e.to_string()))?;
            if !addrs.insert(m.address) {
                return Err(Error::InvalidGenesis(format!("duplicate member {}", m.address)));
            }
            if !handles.insert(m.handle.to_ascii_lowercase()) {
                return Err(Error::InvalidGenesis(format!("duplicate handle {}", m.handle)));
            }
        }
        if self.validators.is_empty() {
            return Err(Error::InvalidGenesis("at least one validator required".into()));
        }
        let n = self.validators.len();
        let q = self.consensus.quorum as usize;
        if q <= n / 2 || q > n {
            return Err(Error::InvalidGenesis(format!("quorum {q} must be a strict majority of {n} validators and at most {n}")));
        }
        let mut vset = BTreeSet::new();
        for v in &self.validators {
            let m = self.members.iter().find(|m| &m.address == v).ok_or_else(|| Error::InvalidGenesis(format!("validator {v} is not a member")))?;
            if !m.has(Role::Validator) {
                return Err(Error::InvalidGenesis(format!("validator {v} lacks the validator role")));
            }
            if !vset.insert(*v) {
                return Err(Error::InvalidGenesis(format!("duplicate validator {v}")));
            }
        }
        for m in &self.members {
            if m.has(Role::Validator) && !vset.contains(&m.address) {
                return Err(Error::InvalidGenesis(format!("{} has the validator role but is not in the validator set", m.handle)));
            }
        }
        if !self.members.iter().any(|m| m.has(Role::Admin)) {
            return Err(Error::InvalidGenesis("at least one admin required".into()));
        }
        match self.mode {
            NetworkMode::Production if !self.allocations.is_empty() => {
                return Err(Error::InvalidGenesis("production networks cannot pre-allocate balances; supply enters only through settled deposits".into()))
            }
            _ => {}
        }
        let mut total: u64 = 0;
        for (a, amt) in &self.allocations {
            if !addrs.contains(a) {
                return Err(Error::InvalidGenesis(format!("allocation to non-member {a}")));
            }
            total = total.checked_add(*amt).ok_or_else(|| Error::InvalidGenesis("allocation overflow".into()))?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------- block votes

/// Digest a validator signs to vote for a block.
pub fn vote_digest(genesis_hash: &H256, block_hash: &H256) -> H256 {
    tagged_hash(b"lc/vote/v1", &[&genesis_hash.0, &block_hash.0])
}

pub fn verify_vote(genesis_hash: &H256, block_hash: &H256, pk: &Pk32, sig: &Sig64) -> Result<()> {
    verify(pk, &vote_digest(genesis_hash, block_hash), sig)
}
