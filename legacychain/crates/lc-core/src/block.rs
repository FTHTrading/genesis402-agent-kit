//! Blocks and quorum certificates.
//!
//! Consensus is an authority quorum: the proposer for `(height, round)` orders
//! transactions, each validator re-executes the block and signs its hash at
//! most once per height, and a block is final once it carries signatures from
//! at least `floor(2n/3) + 1` validators. Finality is immediate; there are no
//! forks to wait out.

use crate::crypto::MemberKeys;
use crate::error::{Error, Result};
use crate::genesis::{verify_vote, vote_digest, Genesis};
use crate::state::State;
use crate::tx::Tx;
use crate::types::{canonical, tagged_hash, Address, Sig64, H256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockHeader {
    pub chain_id: String,
    pub height: u64,
    pub round: u32,
    pub prev_hash: H256,
    pub timestamp_ms: u64,
    pub tx_root: H256,
    pub state_root: H256,
    pub proposer: Address,
}

impl BlockHeader {
    pub fn hash(&self, genesis_hash: &H256) -> H256 {
        tagged_hash(b"lc/block/v1", &[&genesis_hash.0, &canonical(self)])
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vote {
    pub validator: Address,
    pub sig: Sig64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    pub txs: Vec<Tx>,
    /// Quorum certificate; empty while the block is only a proposal.
    pub votes: Vec<Vote>,
}

pub fn tx_root(txs: &[Tx]) -> H256 {
    let hashes: Vec<[u8; 32]> = txs.iter().map(|t| t.hash().0).collect();
    tagged_hash(b"lc/txroot/v1", &[&canonical(&hashes)])
}

/// Outcome of building a block from a mempool snapshot.
pub struct Built {
    pub block: Block,
    pub post_state: State,
    /// Transactions that failed validation, with the reason.
    pub rejected: Vec<(H256, String)>,
}

/// Proposer side: apply candidate transactions in order, keep the valid ones.
pub fn build_block(g: &Genesis, genesis_hash: &H256, state: &State, candidates: &[Tx], round: u32, timestamp_ms: u64, proposer: Address) -> Built {
    let mut post = state.clone();
    post.height = state.height + 1;
    let mut included = Vec::new();
    let mut rejected = Vec::new();
    for tx in candidates {
        if included.len() >= g.consensus.max_block_txs as usize {
            break;
        }
        match post.apply_tx(g, genesis_hash, tx) {
            Ok(()) => included.push(tx.clone()),
            Err(e) => rejected.push((tx.hash(), e.to_string())),
        }
    }
    let header = BlockHeader {
        chain_id: g.chain_id.clone(),
        height: state.height + 1,
        round,
        prev_hash: state.tip,
        timestamp_ms: timestamp_ms.max(1),
        tx_root: tx_root(&included),
        state_root: H256([0u8; 32]),
        proposer,
    };
    let mut block = Block { header, txs: included, votes: vec![] };
    // The state root commits to everything except the tip (which depends on
    // the header that contains the root).
    block.header.state_root = state_root_without_tip(&post);
    post.tip = block.header.hash(genesis_hash);
    Built { block, post_state: post, rejected }
}

fn state_root_without_tip(s: &State) -> H256 {
    let mut c = s.clone();
    c.tip = H256([0u8; 32]);
    c.root()
}

/// Re-execute a block on top of `state` and check every header field.
/// Returns the post-state. Does not check votes.
pub fn execute_block(g: &Genesis, genesis_hash: &H256, state: &State, block: &Block, prev_timestamp_ms: u64) -> Result<State> {
    let h = &block.header;
    if h.chain_id != g.chain_id {
        return Err(Error::InvalidBlock("wrong chain".into()));
    }
    if h.height != state.height + 1 {
        return Err(Error::InvalidBlock(format!("height {} does not follow {}", h.height, state.height)));
    }
    if h.prev_hash != state.tip {
        return Err(Error::InvalidBlock("prev_hash does not match tip".into()));
    }
    if h.proposer != g.proposer(h.height, h.round) {
        return Err(Error::InvalidBlock(format!("{} is not the proposer for height {} round {}", h.proposer, h.height, h.round)));
    }
    if h.timestamp_ms <= prev_timestamp_ms {
        return Err(Error::InvalidBlock("timestamp must increase".into()));
    }
    if block.txs.is_empty() {
        return Err(Error::InvalidBlock("empty blocks are not produced".into()));
    }
    if block.txs.len() > g.consensus.max_block_txs as usize {
        return Err(Error::InvalidBlock("too many transactions".into()));
    }
    if h.tx_root != tx_root(&block.txs) {
        return Err(Error::InvalidBlock("tx_root mismatch".into()));
    }
    let mut post = state.clone();
    post.height = h.height;
    for tx in &block.txs {
        post.apply_tx(g, genesis_hash, tx).map_err(|e| Error::InvalidBlock(format!("tx {}: {e}", tx.hash())))?;
    }
    if state_root_without_tip(&post) != h.state_root {
        return Err(Error::InvalidBlock("state_root mismatch".into()));
    }
    post.tip = h.hash(genesis_hash);
    Ok(post)
}

pub fn sign_vote(genesis_hash: &H256, block_hash: &H256, keys: &MemberKeys) -> Vote {
    Vote { validator: keys.address(), sig: keys.sign(&vote_digest(genesis_hash, block_hash)) }
}

/// Check that a block carries a valid quorum certificate.
pub fn verify_quorum(g: &Genesis, genesis_hash: &H256, state: &State, block: &Block) -> Result<()> {
    let bh = block.header.hash(genesis_hash);
    let mut signers = BTreeSet::new();
    for v in &block.votes {
        if !g.validators.contains(&v.validator) {
            return Err(Error::InvalidBlock(format!("{} is not a validator", v.validator)));
        }
        let m = state.member(&v.validator).ok_or_else(|| Error::InvalidBlock("validator not in state".into()))?;
        verify_vote(genesis_hash, &bh, &m.sign_pk, &v.sig).map_err(|_| Error::InvalidBlock(format!("bad vote from {}", v.validator)))?;
        signers.insert(v.validator);
    }
    if signers.len() < g.quorum() {
        return Err(Error::InvalidBlock(format!("{} of {} required votes", signers.len(), g.quorum())));
    }
    Ok(())
}
