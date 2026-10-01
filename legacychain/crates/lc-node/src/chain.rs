//! In-memory chain: committed state, indexes, mempool, and vote safety.

use crate::store::{LastVote, Store};
use anyhow::{anyhow, bail, Result};
use lc_core::block::{execute_block, verify_quorum, Block, BlockHeader};
use lc_core::crypto::verify;
use lc_core::envelope::Envelope;
use lc_core::genesis::Genesis;
use lc_core::state::State;
use lc_core::tx::{Tx, MAX_TX_BYTES};
use lc_core::types::{Address, H256};
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

const MAX_MEMPOOL: usize = 10_000;
const MAX_REJECTED: usize = 10_000;

#[derive(Clone, Serialize)]
pub struct EnvelopeRecord {
    pub height: u64,
    pub tx_hash: H256,
    pub tx_kind: &'static str,
    pub from: Address,
    pub envelope: Envelope,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TxStatus {
    Committed { height: u64, block_hash: H256 },
    Pending,
    Rejected { reason: String },
    Unknown,
}

pub struct Chain {
    pub genesis: Genesis,
    pub gh: H256,
    pub state: State,
    pub blocks: Vec<Block>,
    tx_index: HashMap<H256, u64>,
    pub envelopes: Vec<EnvelopeRecord>,
    pub mempool: Vec<Tx>,
    mempool_ids: HashSet<H256>,
    rejected: HashMap<H256, String>,
    rejected_order: VecDeque<H256>,
    pub last_vote: Option<LastVote>,
    pub round: u32,
    pub round_started: Instant,
    pub proposed: Option<(u64, u32)>,
    pub store: Store,
}

impl Chain {
    pub fn open(genesis: Genesis, store: Store) -> Result<Self> {
        let gh = genesis.hash();
        let state = State::from_genesis(&genesis)?;
        let last_vote = store.load_vote()?;
        let mut c = Chain {
            genesis,
            gh,
            state,
            blocks: Vec::new(),
            tx_index: HashMap::new(),
            envelopes: Vec::new(),
            mempool: Vec::new(),
            mempool_ids: HashSet::new(),
            rejected: HashMap::new(),
            rejected_order: VecDeque::new(),
            last_vote,
            round: 0,
            round_started: Instant::now(),
            proposed: None,
            store,
        };
        for b in c.store.load_blocks()? {
            c.apply_verified(b, false)?;
        }
        Ok(c)
    }

    pub fn height(&self) -> u64 {
        self.state.height
    }

    pub fn last_timestamp(&self) -> u64 {
        self.blocks.last().map(|b| b.header.timestamp_ms).unwrap_or(self.genesis.created_at_ms)
    }

    pub fn header_at(&self, height: u64) -> Option<&BlockHeader> {
        height.checked_sub(1).and_then(|i| self.blocks.get(i as usize)).map(|b| &b.header)
    }

    /// Verify a block's quorum certificate and execution, then commit it.
    pub fn commit(&mut self, b: Block) -> Result<()> {
        self.apply_verified(b, true)
    }

    fn apply_verified(&mut self, b: Block, persist: bool) -> Result<()> {
        verify_quorum(&self.genesis, &self.gh, &self.state, &b)?;
        let post = execute_block(&self.genesis, &self.gh, &self.state, &b, self.last_timestamp())?;
        if persist {
            self.store.append_block(&b)?;
        }
        let bh = b.header.hash(&self.gh);
        for tx in &b.txs {
            let h = tx.hash();
            self.tx_index.insert(h, b.header.height);
            for env in tx.body.envelopes() {
                self.envelopes.push(EnvelopeRecord { height: b.header.height, tx_hash: h, tx_kind: tx.body.kind(), from: tx.from, envelope: env.clone() });
            }
        }
        self.state = post;
        debug_assert_eq!(self.state.tip, bh);
        let included: HashSet<H256> = b.txs.iter().map(|t| t.hash()).collect();
        self.mempool.retain(|t| !included.contains(&t.hash()));
        self.mempool_ids.retain(|h| !included.contains(h));
        self.blocks.push(b);
        self.round = 0;
        self.round_started = Instant::now();
        self.proposed = None;
        // Drop mempool entries that can no longer apply (stale nonces).
        let state = &self.state;
        let stale: Vec<H256> = self.mempool.iter().filter(|t| t.nonce < state.next_nonce(&t.from)).map(|t| t.hash()).collect();
        for h in stale {
            self.reject(h, "nonce already used".into());
        }
        Ok(())
    }

    /// State with every pending transaction applied, for admission checks and
    /// nonce suggestions.
    pub fn pending_state(&self) -> State {
        let mut s = self.state.clone();
        s.height += 1;
        for t in &self.mempool {
            let _ = s.apply_tx(&self.genesis, &self.gh, t);
        }
        s
    }

    pub fn admit(&mut self, tx: Tx) -> Result<H256> {
        let h = tx.hash();
        if self.tx_index.contains_key(&h) {
            bail!("transaction already committed");
        }
        if self.mempool_ids.contains(&h) {
            return Ok(h);
        }
        if self.mempool.len() >= MAX_MEMPOOL {
            bail!("mempool full; retry shortly");
        }
        // Cheap checks first: this path is reachable from the internet.
        if tx.encoded_len() > MAX_TX_BYTES {
            bail!("transaction too large");
        }
        let ms = self.state.members.get(&tx.from).ok_or_else(|| anyhow!("{} is not a member", tx.from))?;
        if ms.suspended {
            bail!("sender is suspended");
        }
        verify(&ms.member.sign_pk, &Tx::signing_digest(&self.gh, &tx.from, tx.nonce, &tx.body), &tx.sig).map_err(|_| anyhow!("bad signature"))?;
        if tx.nonce < self.state.next_nonce(&tx.from) {
            bail!("nonce {} already used", tx.nonce);
        }
        let mut s = self.pending_state();
        s.apply_tx(&self.genesis, &self.gh, &tx)?;
        self.rejected.remove(&h);
        self.mempool_ids.insert(h);
        self.mempool.push(tx);
        Ok(h)
    }

    pub fn reject(&mut self, h: H256, reason: String) {
        if self.mempool_ids.remove(&h) {
            self.mempool.retain(|t| t.hash() != h);
        }
        if self.rejected.insert(h, reason).is_none() {
            self.rejected_order.push_back(h);
            while self.rejected_order.len() > MAX_REJECTED {
                if let Some(old) = self.rejected_order.pop_front() {
                    self.rejected.remove(&old);
                }
            }
        }
    }

    pub fn tx_status(&self, h: &H256) -> TxStatus {
        if let Some(height) = self.tx_index.get(h) {
            let block_hash = self.header_at(*height).map(|hd| hd.hash(&self.gh)).unwrap_or(H256([0u8; 32]));
            return TxStatus::Committed { height: *height, block_hash };
        }
        if self.mempool_ids.contains(h) {
            return TxStatus::Pending;
        }
        if let Some(r) = self.rejected.get(h) {
            return TxStatus::Rejected { reason: r.clone() };
        }
        TxStatus::Unknown
    }

    /// Vote safety rule: at most one block hash per height, durable across restarts.
    pub fn may_vote(&self, height: u64, block_hash: &H256) -> bool {
        match &self.last_vote {
            Some(v) if v.height > height => false,
            Some(v) if v.height == height => &v.block_hash == block_hash,
            _ => true,
        }
    }

    pub fn record_vote(&mut self, height: u64, block_hash: H256) -> Result<()> {
        let v = LastVote { height, block_hash };
        self.store.save_vote(&v)?;
        self.last_vote = Some(v);
        Ok(())
    }
}
