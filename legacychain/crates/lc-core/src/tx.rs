//! Transactions. Every transaction is signed by a member's Ed25519 key over a
//! digest that includes the genesis hash, so it is valid on exactly one chain.

use crate::crypto::MemberKeys;
use crate::envelope::Envelope;
use crate::genesis::Member;
use crate::types::{canonical, tagged_hash, Address, EvmAddress, Sig64, H256};
use serde::{Deserialize, Serialize};

/// Largest encoded transaction accepted.
pub const MAX_TX_BYTES: usize = 5 * 1024 * 1024;
/// Longest an agent dispatch may stay open, in blocks.
pub const MAX_DISPATCH_BLOCKS: u64 = 1_000_000;

/// Evidence that real USDC reached the treasury on Base before a mint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositProof {
    /// "x402", "mpp", or "devnet".
    pub dialect: String,
    /// CAIP-2 network the money moved on.
    pub network: String,
    pub payer: EvmAddress,
    /// EIP-3009 authorization nonce (unique per payer, enforced by USDC too).
    pub auth_nonce: H256,
    /// Settlement transaction hash returned by the facilitator.
    pub settlement_tx: String,
    pub amount: u64,
}

impl DepositProof {
    /// Replay key: one mint per (payer, authorization nonce).
    pub fn key(&self) -> H256 {
        tagged_hash(b"lc/deposit/v1", &[&self.payer.0, &self.auth_nonce.0])
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxBody {
    /// Move settlement units to another member, optionally with a sealed memo
    /// (invoice, remittance advice, contract reference).
    Transfer { to: Address, amount: u64, memo: Option<Envelope> },
    /// Anchor a sealed envelope: message, document, contract, anything.
    Seal { envelope: Envelope },
    /// Bridge role: credit a member for a settled Base USDC deposit.
    BridgeMint { to: Address, deposit: DepositProof },
    /// Redeem units back to USDC on Base. Units are destroyed immediately;
    /// the bridge pays out from the treasury and records the payout.
    BridgeBurn { amount: u64, payout_to: EvmAddress },
    /// Bridge role: record the Base transaction that paid a burn out.
    BridgePayoutSettled { burn_id: H256, settlement_tx: String },
    /// Admin role: admit a new member.
    AdmitMember { member: Member },
    /// Admin role: suspend or reinstate a member.
    SetSuspended { member: Address, suspended: bool },
    /// Admin or issuer role: issue a real-world-asset instrument. Terms are
    /// sealed to the intended holders; only the document hash is public.
    RwaIssue { asset_id: String, name: String, supply: u64, document_hash: H256, terms: Option<Envelope> },
    /// Transfer RWA units between members.
    RwaTransfer { asset_id: String, to: Address, amount: u64, memo: Option<Envelope> },
    /// Send an agent: a sealed task to another member, optionally carrying
    /// escrowed money that releases when the recipient accepts.
    AgentDispatch { to: Address, agent_id: Option<u64>, task: Envelope, escrow: u64, expires_in_blocks: u64 },
    /// Recipient accepts (escrow released to them) or declines (refunded).
    AgentRespond { dispatch_id: H256, accept: bool, result: Option<Envelope> },
    /// Sender reclaims escrow from an expired, unanswered dispatch.
    AgentReclaim { dispatch_id: H256 },
}

impl TxBody {
    /// Every envelope carried by this body.
    pub fn envelopes(&self) -> Vec<&Envelope> {
        match self {
            TxBody::Transfer { memo, .. } | TxBody::RwaTransfer { memo, .. } => memo.iter().collect(),
            TxBody::Seal { envelope } => vec![envelope],
            TxBody::RwaIssue { terms, .. } => terms.iter().collect(),
            TxBody::AgentDispatch { task, .. } => vec![task],
            TxBody::AgentRespond { result, .. } => result.iter().collect(),
            _ => vec![],
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            TxBody::Transfer { .. } => "transfer",
            TxBody::Seal { .. } => "seal",
            TxBody::BridgeMint { .. } => "bridge_mint",
            TxBody::BridgeBurn { .. } => "bridge_burn",
            TxBody::BridgePayoutSettled { .. } => "bridge_payout_settled",
            TxBody::AdmitMember { .. } => "admit_member",
            TxBody::SetSuspended { .. } => "set_suspended",
            TxBody::RwaIssue { .. } => "rwa_issue",
            TxBody::RwaTransfer { .. } => "rwa_transfer",
            TxBody::AgentDispatch { .. } => "agent_dispatch",
            TxBody::AgentRespond { .. } => "agent_respond",
            TxBody::AgentReclaim { .. } => "agent_reclaim",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tx {
    pub from: Address,
    pub nonce: u64,
    pub body: TxBody,
    pub sig: Sig64,
}

impl Tx {
    pub fn signing_digest(genesis_hash: &H256, from: &Address, nonce: u64, body: &TxBody) -> H256 {
        tagged_hash(b"lc/tx/v1", &[&genesis_hash.0, &canonical(&(from, nonce, body))])
    }

    pub fn sign(genesis_hash: &H256, keys: &MemberKeys, nonce: u64, body: TxBody) -> Tx {
        let from = keys.address();
        let sig = keys.sign(&Self::signing_digest(genesis_hash, &from, nonce, &body));
        Tx { from, nonce, body, sig }
    }

    pub fn hash(&self) -> H256 {
        tagged_hash(b"lc/txid/v1", &[&canonical(self)])
    }

    pub fn encoded_len(&self) -> usize {
        canonical(self).len()
    }
}
