//! HTTP API. Everything except `/v1/health` and the deposit quote requires a
//! signed `X-LC-Auth` header from a member (or a validator, for consensus).

use crate::bridge::{self, DepositOutcome};
use crate::chain::EnvelopeRecord;
use crate::node::{now_ms, Node, Tip};
use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, Query, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use lc_core::auth;
use lc_core::block::Block;
use lc_core::genesis::{Member, Role};
use lc_core::state::BurnStatus;
use lc_core::tx::{Tx, TxBody};
use lc_core::types::{format_amount, parse_amount, Address, H256};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

type App = Arc<Node>;

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

fn err(code: StatusCode, msg: impl ToString) -> ApiError {
    ApiError(code, msg.to_string())
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        err(StatusCode::BAD_REQUEST, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<T, ApiError>;

/// Verify `X-LC-Auth` and return the calling member.
async fn caller(node: &Node, headers: &HeaderMap, method: &Method, uri: &OriginalUri, body: &[u8]) -> ApiResult<Member> {
    let h = headers.get(auth::HEADER).and_then(|v| v.to_str().ok()).ok_or_else(|| err(StatusCode::UNAUTHORIZED, "signed X-LC-Auth header required"))?;
    let (addr, ts, sig) = auth::parse(h).map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    let pq = uri.0.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let c = node.chain.lock().await;
    let ms = c.state.members.get(&addr).ok_or_else(|| err(StatusCode::FORBIDDEN, "not a member of this chain"))?;
    if ms.suspended {
        return Err(err(StatusCode::FORBIDDEN, "member suspended"));
    }
    auth::check(&c.gh, &ms.member.sign_pk, method.as_str(), pq, ts, &sig, body, now_ms()).map_err(|e| err(StatusCode::UNAUTHORIZED, e))?;
    Ok(ms.member.clone())
}

async fn validator(node: &Node, headers: &HeaderMap, method: &Method, uri: &OriginalUri, body: &[u8]) -> ApiResult<Member> {
    let m = caller(node, headers, method, uri, body).await?;
    if !m.has(Role::Validator) {
        return Err(err(StatusCode::FORBIDDEN, "validators only"));
    }
    Ok(m)
}

pub fn router(node: App) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/genesis", get(genesis))
        .route("/v1/directory", get(directory))
        .route("/v1/me", get(me))
        .route("/v1/tx", post(submit_tx))
        .route("/v1/tx/{hash}", get(tx_status))
        .route("/v1/envelopes", get(envelopes))
        .route("/v1/assets", get(assets))
        .route("/v1/bridge/deposit", post(deposit))
        .route("/v1/bridge/burns", get(burns))
        .route("/v1/erc8004/{agent_id}", get(erc8004))
        .route("/v1/consensus/tip", get(tip))
        .route("/v1/consensus/blocks", get(blocks))
        .route("/v1/consensus/propose", post(propose))
        .route("/v1/consensus/commit", post(commit))
        .route("/v1/consensus/tx", post(peer_tx))
        .with_state(node)
}

async fn health(State(node): State<App>) -> Json<Value> {
    let c = node.chain.lock().await;
    Json(json!({ "ok": true, "chain_id": c.genesis.chain_id, "genesis_hash": c.gh, "height": c.height() }))
}

async fn genesis(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Value>> {
    caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    Ok(Json(json!({ "genesis_hash": c.gh, "genesis": c.genesis })))
}

async fn directory(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Value>> {
    caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let members: Vec<Value> = c
        .state
        .members
        .values()
        .map(|ms| {
            json!({
                "handle": ms.member.handle,
                "address": ms.member.address,
                "sign_pk": ms.member.sign_pk,
                "seal_pk": ms.member.seal_pk,
                "roles": ms.member.roles,
                "erc8004_agent_id": ms.member.erc8004.as_ref().map(|b| b.agent_id),
                "suspended": ms.suspended,
            })
        })
        .collect();
    Ok(Json(json!({ "chain_id": c.genesis.chain_id, "genesis_hash": c.gh, "height": c.height(), "members": members })))
}

async fn me(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Value>> {
    let m = caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let pending = c.pending_state();
    let holdings: Vec<Value> = c
        .state
        .assets
        .values()
        .filter_map(|a| a.holders.get(&m.address).map(|q| json!({ "asset_id": a.asset_id, "name": a.name, "units": q, "of": a.supply })))
        .collect();
    let dispatches: Vec<Value> = c
        .state
        .dispatches
        .values()
        .filter(|d| d.from == m.address || d.to == m.address)
        .map(|d| json!({ "dispatch": d, "escrow_display": format_amount(d.escrow) }))
        .collect();
    let burns: Vec<_> = c.state.burns.values().filter(|b| b.owner == m.address).collect();
    Ok(Json(json!({
        "member": m,
        "height": c.height(),
        "balance": format_amount(c.state.balance(&m.address)),
        "balance_atomic": c.state.balance(&m.address),
        "symbol": c.genesis.asset.symbol,
        "next_nonce": pending.next_nonce(&m.address),
        "rwa": holdings,
        "dispatches": dispatches,
        "withdrawals": burns,
    })))
}

async fn submit_tx(State(node): State<App>, body: Bytes) -> ApiResult<Json<Value>> {
    let tx: Tx = serde_json::from_slice(&body).map_err(|e| err(StatusCode::BAD_REQUEST, format!("bad transaction JSON: {e}")))?;
    // Admission of an ERC-8004-bound member is checked against Base when an RPC is configured.
    if let TxBody::AdmitMember { member } = &tx.body {
        if let (Some(b), Some(_)) = (&member.erc8004, &node.base_rpc) {
            let registry = node.chain.lock().await.genesis.erc8004.registry;
            bridge::check_binding_onchain(&node, &registry, b.agent_id, &b.owner).await.map_err(|e| err(StatusCode::FORBIDDEN, format!("{e:#}")))?;
        }
    }
    let (h, gh) = {
        let mut c = node.chain.lock().await;
        let h = c.admit(tx.clone()).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")))?;
        (h, c.gh)
    };
    node.gossip_tx(gh, tx);
    Ok(Json(json!({ "tx_hash": h, "status": "pending" })))
}

async fn tx_status(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, Path(hash): Path<String>) -> ApiResult<Json<Value>> {
    caller(&node, &headers, &method, &uri, b"").await?;
    let h: H256 = hash.parse().map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let c = node.chain.lock().await;
    Ok(Json(serde_json::to_value(c.tx_status(&h)).unwrap_or(Value::Null)))
}

#[derive(Deserialize)]
struct SinceQuery {
    since: Option<u64>,
    limit: Option<usize>,
}

/// All envelopes since a height. Recipients are hidden from the chain, so a
/// member scans and trial-opens locally; outsiders cannot call this at all.
async fn envelopes(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, Query(q): Query<SinceQuery>) -> ApiResult<Json<Value>> {
    caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let since = q.since.unwrap_or(0);
    let limit = q.limit.unwrap_or(500).min(2000);
    let recs: Vec<&EnvelopeRecord> = c.envelopes.iter().filter(|r| r.height > since).take(limit).collect();
    let next = recs.last().map(|r| r.height).unwrap_or(since);
    Ok(Json(json!({ "height": c.height(), "next_since": next, "envelopes": recs })))
}

async fn assets(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Value>> {
    let m = caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let list: Vec<Value> = c
        .state
        .assets
        .values()
        .map(|a| {
            let visible = a.issuer == m.address || a.holders.contains_key(&m.address) || m.has(Role::Admin);
            json!({
                "asset_id": a.asset_id,
                "name": a.name,
                "issuer": a.issuer,
                "supply": a.supply,
                "document_hash": a.document_hash,
                "issued_at": a.issued_at,
                "holders": if visible { serde_json::to_value(&a.holders).unwrap_or(Value::Null) } else { Value::Null },
            })
        })
        .collect();
    Ok(Json(json!({ "assets": list })))
}

#[derive(Deserialize)]
struct DepositRequest {
    to: String,
    amount: String,
}

/// x402 / MPP deposit. Unpaid: 402 with both challenges. Paid: settle on
/// Base through the facilitator, then mint.
async fn deposit(State(node): State<App>, headers: HeaderMap, body: Bytes) -> Response {
    let req: DepositRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => return err(StatusCode::BAD_REQUEST, format!("body must be {{\"to\":\"<address|handle>\",\"amount\":\"25.00\"}}: {e}")).into_response(),
    };
    let to: Address = match req.to.parse() {
        Ok(a) => a,
        Err(_) => {
            let c = node.chain.lock().await;
            match c.state.member_by_handle(&req.to) {
                Some(m) => m.address,
                None => return err(StatusCode::BAD_REQUEST, format!("unknown recipient '{}'", req.to)).into_response(),
            }
        }
    };
    let amount = match parse_amount(&req.amount) {
        Ok(a) => a,
        Err(e) => return err(StatusCode::BAD_REQUEST, e).into_response(),
    };
    let hv = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).map(str::to_string);
    let x402 = hv("payment-signature").or_else(|| hv("x-payment"));
    let mpp = hv("authorization").and_then(|a| a.strip_prefix("Payment ").map(str::to_string));

    match bridge::deposit(&node, to, amount, x402.as_deref(), mpp.as_deref()).await {
        Ok(DepositOutcome::Challenge(pr, www)) => {
            let mut resp = (StatusCode::PAYMENT_REQUIRED, Json(pr.clone())).into_response();
            let h = resp.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&lc_core::payments::b64(&pr)) {
                h.insert(HeaderName::from_static("payment-required"), v);
            }
            if let Ok(v) = HeaderValue::from_str(&www) {
                h.insert(HeaderName::from_static("www-authenticate"), v);
            }
            resp
        }
        Ok(DepositOutcome::Settled { body, payment_response, mpp_receipt }) => {
            let mut resp = (StatusCode::OK, Json(body)).into_response();
            let h = resp.headers_mut();
            if let Ok(v) = HeaderValue::from_str(&payment_response) {
                h.insert(HeaderName::from_static("payment-response"), v);
            }
            if let Some(r) = mpp_receipt.and_then(|r| HeaderValue::from_str(&r).ok()) {
                h.insert(HeaderName::from_static("payment-receipt"), r);
            }
            resp
        }
        Err(e) => {
            let msg = format!("{e:#}");
            let code = if x402.is_some() || mpp.is_some() { StatusCode::PAYMENT_REQUIRED } else { StatusCode::BAD_REQUEST };
            err(code, msg).into_response()
        }
    }
}

async fn burns(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Value>> {
    let m = caller(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let all = m.has(Role::Bridge) || m.has(Role::Admin);
    let list: Vec<Value> = c
        .state
        .burns
        .values()
        .filter(|b| all || b.owner == m.address)
        .map(|b| json!({ "burn": b, "amount": format_amount(b.amount), "pending": b.status == BurnStatus::Pending }))
        .collect();
    Ok(Json(json!({
        "supply": format_amount(c.state.supply),
        "deposited": format_amount(c.state.deposited),
        "withdrawn": format_amount(c.state.withdrawn),
        "treasury": c.genesis.bridge.pay_to,
        "burns": list,
    })))
}

async fn erc8004(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, Path(agent_id): Path<u64>) -> ApiResult<Json<Value>> {
    caller(&node, &headers, &method, &uri, b"").await?;
    let (registry, bound) = {
        let c = node.chain.lock().await;
        let bound = c
            .state
            .members
            .values()
            .find(|m| m.member.erc8004.as_ref().map(|b| b.agent_id) == Some(agent_id))
            .map(|m| (m.member.handle.clone(), m.member.address));
        (c.genesis.erc8004.registry, bound)
    };
    let owner = bridge::erc8004_owner(&node, &registry, agent_id).await.map_err(|e| err(StatusCode::BAD_GATEWAY, format!("{e:#}")))?;
    let uri = bridge::erc8004_uri(&node, &registry, agent_id).await.ok();
    Ok(Json(json!({
        "agent_id": agent_id,
        "registry": registry,
        "owner": owner,
        "token_uri": uri,
        "bound_member": bound.map(|(h, a)| json!({ "handle": h, "address": a })),
    })))
}

// ---------------------------------------------------------------- consensus

async fn tip(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri) -> ApiResult<Json<Tip>> {
    validator(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    Ok(Json(Tip { height: c.height(), tip: c.state.tip }))
}

#[derive(Deserialize)]
struct BlocksQuery {
    from: u64,
    limit: Option<u64>,
}

async fn blocks(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, Query(q): Query<BlocksQuery>) -> ApiResult<Json<Vec<Block>>> {
    validator(&node, &headers, &method, &uri, b"").await?;
    let c = node.chain.lock().await;
    let from = q.from.max(1);
    let limit = q.limit.unwrap_or(100).min(500);
    let out: Vec<Block> = c.blocks.iter().skip((from - 1) as usize).take(limit as usize).cloned().collect();
    Ok(Json(out))
}

async fn propose(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, body: Bytes) -> ApiResult<Json<lc_core::block::Vote>> {
    let v = validator(&node, &headers, &method, &uri, &body).await?;
    let block: Block = serde_json::from_slice(&body).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let vote = node.on_proposal(v.address, block).await.map_err(|e| err(StatusCode::CONFLICT, format!("{e:#}")))?;
    Ok(Json(vote))
}

async fn commit(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, body: Bytes) -> ApiResult<Json<Value>> {
    validator(&node, &headers, &method, &uri, &body).await?;
    let block: Block = serde_json::from_slice(&body).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut c = node.chain.lock().await;
    let h = block.header.height;
    if h <= c.height() {
        return Ok(Json(json!({ "ok": true, "height": c.height() })));
    }
    if h != c.height() + 1 {
        return Err(err(StatusCode::CONFLICT, format!("behind: at {}, got {h}; will sync", c.height())));
    }
    c.commit(block).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")))?;
    Ok(Json(json!({ "ok": true, "height": c.height() })))
}

/// Transactions gossiped from other validators (not re-gossiped).
async fn peer_tx(State(node): State<App>, headers: HeaderMap, method: Method, uri: OriginalUri, body: Bytes) -> ApiResult<Json<Value>> {
    validator(&node, &headers, &method, &uri, &body).await?;
    let tx: Tx = serde_json::from_slice(&body).map_err(|e| err(StatusCode::BAD_REQUEST, e))?;
    let mut c = node.chain.lock().await;
    let h = c.admit(tx).map_err(|e| err(StatusCode::UNPROCESSABLE_ENTITY, format!("{e:#}")))?;
    Ok(Json(json!({ "tx_hash": h })))
}
