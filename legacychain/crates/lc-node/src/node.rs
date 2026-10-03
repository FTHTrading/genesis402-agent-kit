//! Node runtime: identity, peer transport, block production and sync.

use crate::chain::Chain;
use anyhow::{anyhow, bail, Context, Result};
use lc_core::auth;
use lc_core::block::{build_block, execute_block, sign_vote, Block, Vote};
use lc_core::crypto::MemberKeys;
use lc_core::genesis::{verify_vote, Role};
use lc_core::tx::Tx;
use lc_core::types::{Address, H256};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::Mutex;
use tracing::{debug, info, warn};

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub struct BridgeConfig {
    pub facilitator_url: Option<String>,
    pub facilitator_auth: Option<String>,
    pub simulate_settlement: bool,
    pub deposit_secret: Vec<u8>,
    pub public_url: String,
    pub realm: String,
}

pub struct Node {
    pub keys: MemberKeys,
    pub me: Address,
    pub chain: Mutex<Chain>,
    pub peers: Vec<String>,
    pub http: reqwest::Client,
    pub bridge: BridgeConfig,
    pub base_rpc: Option<String>,
    pub deposits_in_flight: Mutex<std::collections::HashSet<H256>>,
}

#[derive(Serialize, Deserialize)]
pub struct Tip {
    pub height: u64,
    pub tip: H256,
}

impl Node {
    pub fn is_validator(&self, chain: &Chain) -> bool {
        chain.genesis.validators.contains(&self.me)
    }

    pub fn has_role(&self, chain: &Chain, r: Role) -> bool {
        chain.state.member(&self.me).map(|m| m.has(r)).unwrap_or(false)
    }

    /// Signed request to a peer.
    async fn peer_request(&self, gh: &H256, peer: &str, method: reqwest::Method, path: &str, body: Option<Vec<u8>>) -> Result<reqwest::Response> {
        let ts = now_ms();
        let bytes = body.unwrap_or_default();
        let header = auth::sign(gh, &self.keys, method.as_str(), path, ts, &bytes);
        let url = format!("{}{}", peer.trim_end_matches('/'), path);
        let mut rb = self.http.request(method, &url).header(auth::HEADER, header).timeout(Duration::from_secs(5));
        if !bytes.is_empty() {
            rb = rb.header("content-type", "application/json").body(bytes);
        }
        Ok(rb.send().await?)
    }

    /// Forward a client transaction to every peer (best effort).
    pub fn gossip_tx(self: &Arc<Self>, gh: H256, tx: Tx) {
        let node = self.clone();
        tokio::spawn(async move {
            let body = match serde_json::to_vec(&tx) {
                Ok(b) => b,
                Err(_) => return,
            };
            for p in &node.peers {
                if let Err(e) = node.peer_request(&gh, p, reqwest::Method::POST, "/v1/consensus/tx", Some(body.clone())).await {
                    debug!(peer = %p, "tx gossip failed: {e}");
                }
            }
        });
    }

    /// One production step: if this validator is the proposer and there is
    /// work, build a block, gather votes, and commit on quorum.
    pub async fn produce_once(self: &Arc<Self>) -> Result<()> {
        let (block, gh, quorum, height) = {
            let mut c = self.chain.lock().await;
            if !self.is_validator(&c) || c.mempool.is_empty() {
                c.round_started = std::time::Instant::now();
                c.round = 0;
                return Ok(());
            }
            let timeout = Duration::from_millis(c.genesis.consensus.round_timeout_ms);
            if c.round_started.elapsed() > timeout {
                c.round = c.round.saturating_add(1);
                c.round_started = std::time::Instant::now();
                info!(height = c.height() + 1, round = c.round, "round timeout; rotating proposer");
            }
            let height = c.height() + 1;
            let round = c.round;
            if c.genesis.proposer(height, round) != self.me || c.proposed == Some((height, round)) {
                return Ok(());
            }
            let ts = now_ms().max(c.last_timestamp() + 1);
            let candidates = c.mempool.clone();
            let built = build_block(&c.genesis, &c.gh, &c.state, &candidates, round, ts, self.me);
            for (h, reason) in built.rejected {
                warn!(tx = %h, %reason, "dropping invalid transaction");
                c.reject(h, reason);
            }
            if built.block.txs.is_empty() {
                return Ok(());
            }
            let bh = built.block.header.hash(&c.gh);
            if !c.may_vote(height, &bh) {
                c.proposed = Some((height, round));
                warn!(height, "already voted for a different block at this height; waiting for it to commit or sync");
                return Ok(());
            }
            c.record_vote(height, bh)?;
            c.proposed = Some((height, round));
            let mut block = built.block;
            block.votes.push(sign_vote(&c.gh, &bh, &self.keys));
            (block, c.gh, c.genesis.quorum(), height)
        };

        let bh = block.header.hash(&gh);
        let body = serde_json::to_vec(&block)?;
        let mut tasks = Vec::new();
        for p in self.peers.clone() {
            let node = self.clone();
            let body = body.clone();
            tasks.push(tokio::spawn(async move {
                let r = node.peer_request(&gh, &p, reqwest::Method::POST, "/v1/consensus/propose", Some(body)).await;
                match r {
                    Ok(resp) if resp.status().is_success() => resp.json::<Vote>().await.ok(),
                    Ok(resp) => {
                        debug!(peer = %p, status = %resp.status(), "peer declined proposal");
                        None
                    }
                    Err(e) => {
                        debug!(peer = %p, "proposal failed: {e}");
                        None
                    }
                }
            }));
        }
        let mut block = block;
        for t in tasks {
            if let Ok(Some(v)) = t.await {
                if block.votes.iter().any(|x| x.validator == v.validator) {
                    continue;
                }
                let c = self.chain.lock().await;
                if let Some(m) = c.state.member(&v.validator) {
                    if c.genesis.validators.contains(&v.validator) && verify_vote(&gh, &bh, &m.sign_pk, &v.sig).is_ok() {
                        block.votes.push(v);
                    }
                }
            }
        }
        if block.votes.len() < quorum {
            debug!(height, votes = block.votes.len(), quorum, "no quorum yet");
            return Ok(());
        }
        {
            let mut c = self.chain.lock().await;
            if c.height() + 1 != height {
                return Ok(());
            }
            c.commit(block.clone())?;
            info!(height, txs = block.txs.len(), votes = block.votes.len(), hash = %bh, "block committed");
        }
        let body = serde_json::to_vec(&block)?;
        for p in self.peers.clone() {
            let node = self.clone();
            let body = body.clone();
            tokio::spawn(async move {
                if let Err(e) = node.peer_request(&gh, &p, reqwest::Method::POST, "/v1/consensus/commit", Some(body)).await {
                    debug!(peer = %p, "commit broadcast failed: {e}");
                }
            });
        }
        Ok(())
    }

    /// Follower side of a proposal: re-execute and vote at most once per height.
    pub async fn on_proposal(&self, proposer: Address, block: Block) -> Result<Vote> {
        let mut c = self.chain.lock().await;
        if !self.is_validator(&c) {
            bail!("not a validator");
        }
        if block.header.proposer != proposer {
            bail!("proposal not signed by its proposer");
        }
        let height = block.header.height;
        if height != c.height() + 1 {
            bail!("proposal for height {height}, this node is at {}", c.height());
        }
        let skew = block.header.timestamp_ms.abs_diff(now_ms());
        if skew > 30_000 {
            bail!("proposal timestamp {skew} ms from local clock");
        }
        execute_block(&c.genesis, &c.gh, &c.state, &block, c.last_timestamp())?;
        let bh = block.header.hash(&c.gh);
        if !c.may_vote(height, &bh) {
            bail!("already voted for a different block at height {height}");
        }
        c.record_vote(height, bh)?;
        if block.header.round > c.round {
            c.round = block.header.round;
        }
        c.round_started = std::time::Instant::now();
        Ok(sign_vote(&c.gh, &bh, &self.keys))
    }

    /// Pull missing blocks from peers.
    pub async fn sync_once(self: &Arc<Self>) -> Result<()> {
        let gh = { self.chain.lock().await.gh };
        for p in self.peers.clone() {
            let mine = self.chain.lock().await.height();
            let tip: Tip = match self.peer_request(&gh, &p, reqwest::Method::GET, "/v1/consensus/tip", None).await {
                Ok(r) if r.status().is_success() => match r.json().await {
                    Ok(t) => t,
                    Err(_) => continue,
                },
                _ => continue,
            };
            if tip.height <= mine {
                continue;
            }
            let path = format!("/v1/consensus/blocks?from={}&limit=200", mine + 1);
            let resp = self.peer_request(&gh, &p, reqwest::Method::GET, &path, None).await?;
            if !resp.status().is_success() {
                continue;
            }
            let blocks: Vec<Block> = resp.json().await.context("decode blocks")?;
            let mut c = self.chain.lock().await;
            for b in blocks {
                if b.header.height != c.height() + 1 {
                    continue;
                }
                let h = b.header.height;
                c.commit(b).map_err(|e| anyhow!("sync from {p} failed at height {h}: {e}"))?;
                info!(height = h, peer = %p, "synced block");
            }
        }
        Ok(())
    }

    pub fn spawn_loops(self: &Arc<Self>, interval_ms: u64) {
        let producer = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(interval_ms));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = producer.produce_once().await {
                    warn!("production: {e:#}");
                }
            }
        });
        if self.peers.is_empty() {
            return;
        }
        let syncer = self.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                if let Err(e) = syncer.sync_once().await {
                    warn!("sync: {e:#}");
                }
            }
        });
    }

    /// Sign a transaction from this node's own key at the next pending nonce,
    /// admit it, and gossip it. Holds the chain lock across sign and admit so
    /// concurrent callers never reuse a nonce.
    pub async fn submit_own(self: &Arc<Self>, body: lc_core::tx::TxBody) -> Result<H256> {
        let (tx, gh, h) = {
            let mut c = self.chain.lock().await;
            let nonce = c.pending_state().next_nonce(&self.me);
            let tx = Tx::sign(&c.gh, &self.keys, nonce, body);
            let h = c.admit(tx.clone())?;
            (tx, c.gh, h)
        };
        self.gossip_tx(gh, tx);
        Ok(h)
    }
}
