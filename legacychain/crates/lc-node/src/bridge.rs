//! Real-money bridge: x402 / MPP deposits settled on Base, then minted.
//! Also the ERC-8004 `ownerOf` check against a Base RPC.

use crate::node::{now_ms, Node};
use crate::store::SettledDeposit;
use anyhow::{anyhow, bail, Context, Result};
use lc_core::genesis::{NetworkMode, Role};
use lc_core::payments::{self, VerifiedPayment};
use lc_core::tx::{DepositProof, TxBody};
use lc_core::types::{Address, EvmAddress};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tracing::{info, warn};

#[derive(Serialize)]
pub struct Settlement {
    pub success: bool,
    pub transaction: String,
    pub network: String,
    pub payer: String,
}

/// Settle a verified payment through the configured x402 facilitator.
async fn settle(node: &Node, network: &str, vp: &VerifiedPayment, mode: NetworkMode) -> Result<(Settlement, &'static str)> {
    match &node.bridge.facilitator_url {
        Some(url) => {
            let body = json!({
                "x402Version": payments::X402_VERSION,
                "paymentPayload": vp.payment_payload,
                "paymentRequirements": vp.requirements,
            });
            let mut rb = node.http.post(format!("{}/settle", url.trim_end_matches('/'))).json(&body).timeout(std::time::Duration::from_secs(60));
            if let Some(a) = &node.bridge.facilitator_auth {
                rb = rb.header("authorization", a);
            }
            let resp = rb.send().await.context("facilitator unreachable")?;
            let status = resp.status();
            let v: Value = resp.json().await.context("facilitator returned non-JSON")?;
            let ok = v.get("success").and_then(|x| x.as_bool()).unwrap_or(false);
            let txh = v.get("transaction").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if !status.is_success() || !ok || txh.is_empty() {
                let reason = v.get("errorReason").and_then(|x| x.as_str()).unwrap_or("settlement failed");
                bail!("facilitator: {reason}");
            }
            Ok((
                Settlement {
                    success: true,
                    transaction: txh,
                    network: v.get("network").and_then(|x| x.as_str()).unwrap_or(network).to_string(),
                    payer: vp.authorization.from.to_string(),
                },
                vp.dialect,
            ))
        }
        None if node.bridge.simulate_settlement && mode == NetworkMode::Devnet => Ok((
            Settlement {
                success: true,
                transaction: format!("devnet-sim:{}", vp.authorization.nonce),
                network: network.to_string(),
                payer: vp.authorization.from.to_string(),
            },
            "devnet",
        )),
        None => bail!("no x402 facilitator configured on this node (set --facilitator-url)"),
    }
}

pub enum DepositOutcome {
    /// 402 challenge: (x402 PAYMENT-REQUIRED json, MPP WWW-Authenticate value).
    Challenge(Value, String),
    Settled {
        body: Value,
        payment_response: String,
        mpp_receipt: Option<String>,
    },
}

pub async fn deposit(node: &Arc<Node>, to: Address, amount: u64, x402_header: Option<&str>, mpp_credential: Option<&str>) -> Result<DepositOutcome> {
    let now = now_ms() / 1000;
    let (g, gh) = {
        let c = node.chain.lock().await;
        if !node.has_role(&c, Role::Bridge) {
            bail!("this node does not hold the bridge role; deposit through the bridge node");
        }
        let m = c.state.members.get(&to).ok_or_else(|| anyhow!("{to} is not a member of this chain"))?;
        if m.suspended {
            bail!("{to} is suspended");
        }
        if amount < c.genesis.bridge.min_deposit || amount > c.genesis.bridge.max_deposit {
            bail!(
                "deposit must be between {} and {}",
                lc_core::types::format_amount(c.genesis.bridge.min_deposit),
                lc_core::types::format_amount(c.genesis.bridge.max_deposit)
            );
        }
        (c.genesis.clone(), c.gh)
    };
    let secret = &node.bridge.deposit_secret;

    let vp = match (x402_header, mpp_credential) {
        (Some(h), _) => payments::verify_x402(&g, &gh, secret, h, now)?,
        (None, Some(c)) => payments::verify_mpp(&g, &gh, secret, c, now)?,
        (None, None) => {
            let order = payments::DepositOrder::new(secret, &gh, to, amount, now);
            let url = format!("{}/v1/bridge/deposit", node.bridge.public_url.trim_end_matches('/'));
            return Ok(DepositOutcome::Challenge(payments::x402_payment_required(&g, &order, &url), payments::mpp_challenge(&g, &order, &node.bridge.realm)));
        }
    };

    let probe = DepositProof {
        dialect: vp.dialect.to_string(),
        network: g.bridge.network.clone(),
        payer: vp.authorization.from,
        auth_nonce: vp.authorization.nonce,
        settlement_tx: String::new(),
        amount: vp.order.amount,
    };
    let key = probe.key();
    {
        let c = node.chain.lock().await;
        if c.state.used_deposits.contains(&key) {
            bail!("this payment was already credited");
        }
    }
    if !node.deposits_in_flight.lock().await.insert(key) {
        bail!("this payment is already being settled");
    }
    let result = settle_and_mint(node, &g.bridge.network, g.mode, vp, probe).await;
    node.deposits_in_flight.lock().await.remove(&key);
    result
}

async fn settle_and_mint(node: &Arc<Node>, network: &str, mode: NetworkMode, vp: VerifiedPayment, mut proof: DepositProof) -> Result<DepositOutcome> {
    let (settlement, dialect) = settle(node, network, &vp, mode).await?;
    proof.settlement_tx = settlement.transaction.clone();
    proof.dialect = dialect.to_string();
    let to = vp.order.to;
    // Journal before minting: money has moved on Base, so the credit must
    // survive a crash. Unminted journal entries are replayed at boot.
    {
        let mut c = node.chain.lock().await;
        c.store.append_deposit(&SettledDeposit { to, deposit: proof.clone() })?;
    }
    let tx_hash = node.submit_own(TxBody::BridgeMint { to, deposit: proof.clone() }).await?;
    info!(%to, amount = proof.amount, settlement = %proof.settlement_tx, dialect, "deposit settled; mint submitted");
    let pr = json!({ "success": true, "transaction": settlement.transaction, "network": settlement.network, "payer": settlement.payer });
    Ok(DepositOutcome::Settled {
        body: json!({
            "status": "settled",
            "to": to,
            "amount": lc_core::types::format_amount(proof.amount),
            "mint_tx": tx_hash,
            "settlement": pr,
            "dialect": vp.dialect,
        }),
        payment_response: payments::b64(&pr),
        mpp_receipt: (vp.dialect == "mpp").then(|| payments::mpp_receipt(&settlement.transaction, now_ms() / 1000)),
    })
}

/// Re-submit mints for deposits that settled on Base but never reached a block.
pub async fn replay_journal(node: &Arc<Node>) -> Result<()> {
    let pending: Vec<SettledDeposit> = {
        let c = node.chain.lock().await;
        if !node.has_role(&c, Role::Bridge) {
            return Ok(());
        }
        c.store.load_deposits()?.into_iter().filter(|d| !c.state.used_deposits.contains(&d.deposit.key())).collect()
    };
    for d in pending {
        match node.submit_own(TxBody::BridgeMint { to: d.to, deposit: d.deposit.clone() }).await {
            Ok(h) => info!(tx = %h, settlement = %d.deposit.settlement_tx, "re-submitted mint for journaled deposit"),
            Err(e) => warn!(settlement = %d.deposit.settlement_tx, "journaled deposit could not be re-submitted: {e:#}"),
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- ERC-8004

async fn eth_call(node: &Node, to: &EvmAddress, data: String) -> Result<Vec<u8>> {
    let rpc = node.base_rpc.as_ref().ok_or_else(|| anyhow!("no --base-rpc configured"))?;
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "eth_call", "params": [{ "to": to.to_string(), "data": data }, "latest"] });
    let v: Value = node.http.post(rpc).json(&body).timeout(std::time::Duration::from_secs(10)).send().await?.json().await?;
    if let Some(err) = v.get("error") {
        bail!("eth_call reverted: {err}");
    }
    let hexs = v.get("result").and_then(|r| r.as_str()).ok_or_else(|| anyhow!("eth_call: no result"))?;
    Ok(hex::decode(hexs.trim_start_matches("0x"))?)
}

fn u256_word(v: u64) -> String {
    format!("{:064x}", v)
}

/// `ownerOf(agentId)` on the ERC-8004 Identity Registry.
pub async fn erc8004_owner(node: &Node, registry: &EvmAddress, agent_id: u64) -> Result<EvmAddress> {
    let out = eth_call(node, registry, format!("0x6352211e{}", u256_word(agent_id))).await?;
    if out.len() < 32 {
        bail!("agent {agent_id} not found in registry");
    }
    EvmAddress::from_slice(&out[12..32]).map_err(|e| anyhow!("{e}"))
}

/// `tokenURI(agentId)`: the agent's registration file.
pub async fn erc8004_uri(node: &Node, registry: &EvmAddress, agent_id: u64) -> Result<String> {
    let out = eth_call(node, registry, format!("0xc87b56dd{}", u256_word(agent_id))).await?;
    if out.len() < 64 {
        bail!("tokenURI: short response");
    }
    let len = u64::from_be_bytes(out[56..64].try_into().unwrap()) as usize;
    let s = out.get(64..64 + len).ok_or_else(|| anyhow!("tokenURI: bad length"))?;
    Ok(String::from_utf8_lossy(s).into_owned())
}

/// Admission guard: a member claiming an ERC-8004 agent must still own it on Base.
pub async fn check_binding_onchain(node: &Node, registry: &EvmAddress, agent_id: u64, claimed_owner: &EvmAddress) -> Result<()> {
    let owner = erc8004_owner(node, registry, agent_id).await?;
    if &owner != claimed_owner {
        bail!("ERC-8004 agent {agent_id} is owned by {owner} on Base, not {claimed_owner}");
    }
    Ok(())
}
