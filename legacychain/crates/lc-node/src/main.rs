//! legacychain-node: validator / member node for a private LegacyChain network.

mod api;
mod bridge;
mod chain;
mod node;
mod store;

use anyhow::{bail, Context, Result};
use clap::Parser;
use lc_core::genesis::{Genesis, Role};
use lc_core::keyfile::KeyFile;
use lc_core::types::tagged_hash;
use node::{BridgeConfig, Node};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::info;

#[derive(Parser)]
#[command(name = "legacychain-node", version, about = "LegacyChain private network node")]
struct Args {
    /// Genesis file.
    #[arg(long, env = "LC_GENESIS")]
    genesis: PathBuf,
    /// This node's key file (from `lc keygen`). Encrypted files read LC_PASSPHRASE.
    #[arg(long, env = "LC_KEY")]
    key: PathBuf,
    /// Data directory (append-only block log, vote record, deposit journal).
    #[arg(long, env = "LC_DATA", default_value = "./data")]
    data: PathBuf,
    /// Listen address.
    #[arg(long, env = "LC_LISTEN", default_value = "127.0.0.1:7402")]
    listen: String,
    /// Peer validator base URLs (repeat, or comma-separate in LC_PEERS).
    #[arg(long = "peer", env = "LC_PEERS", value_delimiter = ',')]
    peers: Vec<String>,
    /// Public base URL of this node (used in x402 resource URLs).
    #[arg(long, env = "LC_PUBLIC_URL", default_value = "https://chain.legacychain.app")]
    public_url: String,
    /// MPP realm.
    #[arg(long, env = "LC_REALM", default_value = "legacychain.app")]
    realm: String,
    /// x402 facilitator base URL that settles USDC on Base (POST {url}/settle).
    #[arg(long, env = "LC_FACILITATOR_URL")]
    facilitator_url: Option<String>,
    /// Value for the facilitator's Authorization header, if it needs one.
    #[arg(long, env = "LC_FACILITATOR_AUTH", hide_env_values = true)]
    facilitator_auth: Option<String>,
    /// Devnet only: accept verified payments without on-chain settlement.
    #[arg(long, env = "LC_SIMULATE_SETTLEMENT", default_value_t = false)]
    simulate_settlement: bool,
    /// Base JSON-RPC URL for ERC-8004 ownership checks.
    #[arg(long, env = "LC_BASE_RPC")]
    base_rpc: Option<String>,
    /// 32-byte hex key that encrypts the data directory at rest.
    #[arg(long, env = "LC_STORAGE_KEY", hide_env_values = true)]
    storage_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info,lc_node=info".into())).init();
    let args = Args::parse();

    let genesis: Genesis =
        serde_json::from_slice(&std::fs::read(&args.genesis).with_context(|| format!("read {}", args.genesis.display()))?).context("parse genesis")?;
    genesis.validate()?;
    let gh = genesis.hash();

    let kf: KeyFile = serde_json::from_slice(&std::fs::read(&args.key).with_context(|| format!("read {}", args.key.display()))?).context("parse key file")?;
    let pass = std::env::var("LC_PASSPHRASE").ok();
    let keys = kf.unlock(pass.as_deref())?;
    let me = keys.address();
    if !genesis.members.iter().any(|m| m.address == me) {
        info!(%me, "this key is not a genesis member; it must be admitted before the node can sign anything");
    }

    let storage_key = match &args.storage_key {
        None => None,
        Some(h) => {
            let raw = hex::decode(h.trim()).context("LC_STORAGE_KEY must be hex")?;
            let k: [u8; 32] = raw.as_slice().try_into().map_err(|_| anyhow::anyhow!("LC_STORAGE_KEY must be 32 bytes"))?;
            Some(k)
        }
    };
    if args.simulate_settlement && genesis.mode != lc_core::genesis::NetworkMode::Devnet {
        bail!("--simulate-settlement is only allowed on devnet genesis files");
    }

    let store = store::Store::open(&args.data, storage_key, gh)?;
    let chain = chain::Chain::open(genesis.clone(), store)?;
    info!(chain_id = %genesis.chain_id, genesis = %gh, height = chain.height(), me = %me, "chain loaded");

    // Per-node deposit challenge secret, derived from the node key so quotes
    // survive restarts without another secret to manage.
    let deposit_secret = tagged_hash(b"lc/deposit-secret/v1", &[keys.sign_seed(), &gh.0]).0.to_vec();
    let interval = genesis.consensus.block_interval_ms;
    let is_bridge = genesis.members.iter().any(|m| m.address == me && m.has(Role::Bridge));

    let node = Arc::new(Node {
        keys,
        me,
        chain: Mutex::new(chain),
        peers: args.peers.into_iter().filter(|p| !p.trim().is_empty()).collect(),
        http: reqwest::Client::builder().user_agent(concat!("legacychain-node/", env!("CARGO_PKG_VERSION"))).build()?,
        bridge: BridgeConfig {
            facilitator_url: args.facilitator_url,
            facilitator_auth: args.facilitator_auth,
            simulate_settlement: args.simulate_settlement,
            deposit_secret,
            public_url: args.public_url,
            realm: args.realm,
        },
        base_rpc: args.base_rpc,
        deposits_in_flight: Mutex::new(Default::default()),
    });
    if is_bridge {
        bridge::replay_journal(&node).await?;
    }
    node.spawn_loops(interval);

    let listener = tokio::net::TcpListener::bind(&args.listen).await.with_context(|| format!("bind {}", args.listen))?;
    info!(listen = %args.listen, peers = node.peers.len(), bridge = is_bridge, "serving");
    axum::serve(listener, api::router(node))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
