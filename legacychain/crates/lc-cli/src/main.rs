//! `lc`: LegacyChain member CLI.

mod client;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, Subcommand};
use client::Client;
use lc_core::crypto::{eip191_digest, evm_address_of, evm_key_from_hex, evm_sign, random_bytes, MemberKeys, TransferAuthorization};
use lc_core::envelope::{Envelope, PayloadKind, SealedPayload};
use lc_core::genesis::*;
use lc_core::keyfile::KeyFile;
use lc_core::tx::TxBody;
use lc_core::types::{format_amount, parse_amount, tagged_hash, Address, Bytes, EvmAddress, H256};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "lc", version, about = "LegacyChain: private money, contracts and agents between members")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct Conn {
    /// Node base URL.
    #[arg(long, env = "LC_NODE", default_value = "http://127.0.0.1:7402")]
    node: String,
    /// Your key file. Encrypted files read LC_PASSPHRASE.
    #[arg(long, env = "LC_KEY")]
    key: PathBuf,
    /// Pin the genesis hash; the CLI refuses to talk to a node on any other chain.
    #[arg(long, env = "LC_GENESIS_HASH")]
    genesis_hash: Option<String>,
    /// Do not wait for the transaction to be committed.
    #[arg(long, default_value_t = false)]
    no_wait: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Create a new identity (<handle>.key.json and <handle>.member.json).
    Keygen {
        #[arg(long)]
        handle: String,
        #[arg(long, default_value = ".")]
        out_dir: PathBuf,
        /// Encrypt the key file with LC_PASSPHRASE (Argon2id + XChaCha20-Poly1305).
        #[arg(long, default_value_t = false)]
        encrypt: bool,
    },
    /// Bind a member record to an ERC-8004 agent on Base (owner signs EIP-191).
    Bind8004 {
        #[arg(long)]
        member: PathBuf,
        #[arg(long)]
        agent_id: u64,
        #[arg(long)]
        chain_id: String,
        /// Agent owner's EVM key (or use --signature from a hardware wallet).
        #[arg(long, env = "LC_EVM_KEY", hide_env_values = true)]
        evm_key: Option<String>,
        /// 65-byte personal_sign signature over the printed statement.
        #[arg(long)]
        signature: Option<String>,
        /// Agent owner address (required with --signature).
        #[arg(long)]
        owner: Option<String>,
    },
    /// Build a genesis file.
    Genesis {
        #[arg(long)]
        chain_id: String,
        #[arg(long, default_value = "devnet")]
        mode: String,
        /// Member record files (order of validator-role members = validator order).
        #[arg(long = "member", required = true)]
        members: Vec<PathBuf>,
        /// Role grants: handle=validator,admin,bridge,issuer (repeatable).
        #[arg(long = "role")]
        roles: Vec<String>,
        /// Treasury on Base that receives USDC deposits (BitGo / Safe multisig).
        #[arg(long)]
        pay_to: String,
        /// Devnet only: handle=amount opening balances.
        #[arg(long = "allocate")]
        allocations: Vec<String>,
        /// Founding charter; its sha256 is locked into the genesis.
        #[arg(long)]
        charter: Option<PathBuf>,
        #[arg(long)]
        quorum: Option<u32>,
        #[arg(long, default_value = "1")]
        min_deposit: String,
        #[arg(long, default_value = "1000000")]
        max_deposit: String,
        #[arg(long, default_value = "genesis.json")]
        out: PathBuf,
    },
    /// Your balance, holdings, dispatches and withdrawals.
    Me {
        #[command(flatten)]
        conn: Conn,
    },
    /// Members of the network.
    Directory {
        #[command(flatten)]
        conn: Conn,
    },
    /// Send money, optionally with a sealed memo only the recipient can read.
    Send {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        memo: Option<String>,
    },
    /// Seal information (message, document, contract, RWA terms) to members.
    Seal {
        #[command(flatten)]
        conn: Conn,
        /// Comma-separated handles or addresses.
        #[arg(long, value_delimiter = ',')]
        to: Vec<String>,
        #[arg(long, default_value = "message")]
        kind: String,
        #[arg(long, default_value = "")]
        title: String,
        #[arg(long, conflicts_with = "file")]
        text: Option<String>,
        #[arg(long)]
        file: Option<PathBuf>,
        /// Also seal a copy to yourself.
        #[arg(long, default_value_t = true)]
        keep_copy: bool,
    },
    /// Scan the chain and unwrap every envelope addressed to you.
    Inbox {
        #[command(flatten)]
        conn: Conn,
        #[arg(long, default_value_t = 0)]
        since: u64,
        /// Write each opened payload body into this directory.
        #[arg(long)]
        save_dir: Option<PathBuf>,
    },
    /// Send an agent: a sealed task to a member, with optional escrowed money.
    Dispatch {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        to: String,
        #[arg(long)]
        task: String,
        #[arg(long, default_value = "0")]
        escrow: String,
        /// Your ERC-8004 agent id, if your membership is bound to one.
        #[arg(long)]
        agent_id: Option<u64>,
        #[arg(long, default_value_t = 100_000)]
        expires_in_blocks: u64,
    },
    /// Accept or decline a dispatch addressed to you.
    Respond {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        dispatch: String,
        #[arg(long, default_value_t = false)]
        decline: bool,
        #[arg(long)]
        result: Option<String>,
    },
    /// Reclaim escrow from your expired, unanswered dispatch.
    Reclaim {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        dispatch: String,
    },
    /// Issue an RWA instrument; terms are sealed to the named holders.
    RwaIssue {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        asset_id: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        supply: u64,
        /// Offering document; its sha256 is public, its contents sealed.
        #[arg(long)]
        document: PathBuf,
        #[arg(long, value_delimiter = ',')]
        holders: Vec<String>,
    },
    /// Transfer RWA units.
    RwaSend {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        asset_id: String,
        #[arg(long)]
        to: String,
        #[arg(long)]
        amount: u64,
    },
    /// Deposit USDC on Base over x402 (default) or MPP and receive units.
    Deposit {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        amount: String,
        /// Payer's Base key. Without it the command only prints the 402 quote.
        #[arg(long, env = "LC_EVM_KEY", hide_env_values = true)]
        evm_key: Option<String>,
        /// Use the MPP `Payment` scheme instead of x402.
        #[arg(long, default_value_t = false)]
        mpp: bool,
        /// Refuse to sign above this amount.
        #[arg(long, default_value = "1000")]
        max: String,
    },
    /// Redeem units to USDC on Base.
    Withdraw {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        amount: String,
        #[arg(long)]
        payout_to: String,
    },
    /// Bridge role: record the Base transaction that paid a withdrawal.
    PayoutSettled {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        burn: String,
        #[arg(long)]
        settlement_tx: String,
    },
    /// Admin role: admit a member record.
    Admit {
        #[command(flatten)]
        conn: Conn,
        #[arg(long)]
        member: PathBuf,
        #[arg(long, value_delimiter = ',')]
        roles: Vec<String>,
    },
    /// Look up an ERC-8004 agent on Base through the node.
    Agent {
        #[command(flatten)]
        conn: Conn,
        agent_id: u64,
    },
    /// Status of a transaction.
    Tx {
        #[command(flatten)]
        conn: Conn,
        hash: String,
    },
}

fn read_json<T: serde::de::DeserializeOwned>(p: &Path) -> Result<T> {
    serde_json::from_slice(&std::fs::read(p).with_context(|| format!("read {}", p.display()))?).with_context(|| format!("parse {}", p.display()))
}

fn write_json<T: serde::Serialize>(p: &Path, v: &T, secret: bool) -> Result<()> {
    let data = serde_json::to_vec_pretty(v)?;
    #[cfg(unix)]
    if secret {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(p)
            .with_context(|| format!("create {} (refusing to overwrite)", p.display()))?;
        f.write_all(&data)?;
        return Ok(());
    }
    let _ = secret;
    std::fs::write(p, data).with_context(|| format!("write {}", p.display()))
}

fn parse_role(s: &str) -> Result<Role> {
    Ok(match s.trim() {
        "validator" => Role::Validator,
        "admin" => Role::Admin,
        "bridge" => Role::Bridge,
        "issuer" => Role::Issuer,
        other => bail!("unknown role '{other}'"),
    })
}

fn content_type_for(p: &Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("txt") => "text/plain",
        Some("md") => "text/markdown",
        Some("csv") => "text/csv",
        Some("json") => "application/json",
        Some("pdf") => "application/pdf",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("docx") => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        _ => "application/octet-stream",
    }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

fn payload(kind: PayloadKind, title: &str, content_type: &str, body: Vec<u8>, refs: Vec<H256>) -> SealedPayload {
    SealedPayload { kind, title: title.to_string(), content_type: content_type.to_string(), body: Bytes(body), created_at_ms: now_ms(), refs, meta: vec![] }
}

fn print(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Keygen { handle, out_dir, encrypt } => {
            let keys = MemberKeys::generate();
            let pass = if encrypt { Some(std::env::var("LC_PASSPHRASE").context("--encrypt needs LC_PASSPHRASE")?) } else { None };
            let kf = KeyFile::new(&handle, &keys, pass.as_deref())?;
            std::fs::create_dir_all(&out_dir)?;
            let kp = out_dir.join(format!("{handle}.key.json"));
            let mp = out_dir.join(format!("{handle}.member.json"));
            write_json(&kp, &kf, true)?;
            write_json(&mp, &kf.member(BTreeSet::new()), false)?;
            print(&json!({ "handle": handle, "address": kf.address, "key_file": kp, "member_file": mp, "encrypted": kf.is_encrypted() }));
        }
        Cmd::Bind8004 { member, agent_id, chain_id, evm_key, signature, owner } => {
            let mut m: Member = read_json(&member)?;
            let spec = Erc8004Spec { chain_id: BASE_CHAIN_ID, registry: ERC8004_IDENTITY_REGISTRY.parse()? };
            let stmt = Erc8004Binding::statement(&chain_id, &spec, agent_id, &m.address, &m.sign_pk, &m.seal_pk);
            let (owner, sig) = match (evm_key, signature) {
                (Some(k), _) => {
                    let key = evm_key_from_hex(&k)?;
                    (evm_address_of(key.verifying_key()), evm_sign(&key, &eip191_digest(stmt.as_bytes()))?.to_vec())
                }
                (None, Some(s)) => {
                    let o: EvmAddress = owner.ok_or_else(|| anyhow!("--owner is required with --signature"))?.parse()?;
                    (o, hex::decode(s.trim_start_matches("0x"))?)
                }
                (None, None) => {
                    println!("Have the owner of ERC-8004 agent {agent_id} personal_sign exactly this text, then rerun with --signature and --owner:\n\n{stmt}");
                    return Ok(());
                }
            };
            m.erc8004 = Some(Erc8004Binding { agent_id, owner, signature: Bytes(sig) });
            m.validate(&chain_id, &spec)?;
            std::fs::write(&member, serde_json::to_vec_pretty(&m)?)?;
            print(&json!({ "member": m.handle, "agent_id": agent_id, "owner": owner, "bound": true }));
        }
        Cmd::Genesis { chain_id, mode, members, roles, pay_to, allocations, charter, quorum, min_deposit, max_deposit, out } => {
            let mode = match mode.as_str() {
                "devnet" => NetworkMode::Devnet,
                "production" => NetworkMode::Production,
                other => bail!("mode must be devnet or production, not '{other}'"),
            };
            let mut ms: Vec<Member> = members.iter().map(|p| read_json(p)).collect::<Result<_>>()?;
            for r in &roles {
                let (h, list) = r.split_once('=').ok_or_else(|| anyhow!("--role must be handle=role[,role]"))?;
                let m = ms.iter_mut().find(|m| m.handle == h).ok_or_else(|| anyhow!("--role: no member '{h}'"))?;
                for x in list.split(',') {
                    m.roles.insert(parse_role(x)?);
                }
            }
            let validators: Vec<Address> = ms.iter().filter(|m| m.has(Role::Validator)).map(|m| m.address).collect();
            let mut allocs = Vec::new();
            for a in &allocations {
                let (h, amt) = a.split_once('=').ok_or_else(|| anyhow!("--allocate must be handle=amount"))?;
                let m = ms.iter().find(|m| m.handle == h).ok_or_else(|| anyhow!("--allocate: no member '{h}'"))?;
                allocs.push((m.address, parse_amount(amt)?));
            }
            let charter_hash = match charter {
                Some(p) => Some(tagged_hash(b"lc/charter/v1", &[&std::fs::read(&p)?])),
                None => None,
            };
            let g = Genesis {
                chain_id,
                mode,
                created_at_ms: now_ms(),
                asset: AssetSpec { symbol: "LUSD".into(), decimals: 6, description: "LegacyChain settlement unit, redeemable 1:1 for USDC on Base".into() },
                bridge: BridgeSpec {
                    network: format!("eip155:{BASE_CHAIN_ID}"),
                    evm_chain_id: BASE_CHAIN_ID,
                    usdc: BASE_USDC.parse()?,
                    usdc_eip712_name: "USD Coin".into(),
                    usdc_eip712_version: "2".into(),
                    pay_to: pay_to.parse()?,
                    min_deposit: parse_amount(&min_deposit)?,
                    max_deposit: parse_amount(&max_deposit)?,
                },
                erc8004: Erc8004Spec { chain_id: BASE_CHAIN_ID, registry: ERC8004_IDENTITY_REGISTRY.parse()? },
                consensus: ConsensusParams {
                    block_interval_ms: 400,
                    round_timeout_ms: 4000,
                    max_block_txs: 500,
                    quorum: quorum.unwrap_or_else(|| default_quorum(validators.len())),
                },
                validators,
                members: ms,
                allocations: allocs,
                charter_hash,
            };
            g.validate()?;
            std::fs::write(&out, serde_json::to_vec_pretty(&g)?)?;
            print(&json!({ "genesis": out, "genesis_hash": g.hash(), "validators": g.validators.len(), "quorum": g.quorum(), "members": g.members.len() }));
        }
        Cmd::Me { conn } => print(&Client::connect(&conn).await?.get("/v1/me").await?),
        Cmd::Directory { conn } => print(&Client::connect(&conn).await?.get("/v1/directory").await?),
        Cmd::Send { conn, to, amount, memo } => {
            let c = Client::connect(&conn).await?;
            let r = c.resolve(&to)?;
            let amt = parse_amount(&amount)?;
            let memo = match memo {
                Some(t) => Some(Envelope::seal(
                    &c.gh,
                    &c.keys,
                    &c.recipients(&[r.address], true)?,
                    &payload(PayloadKind::PaymentAdvice, "memo", "text/plain", t.into_bytes(), vec![]),
                )?),
                None => None,
            };
            c.submit(TxBody::Transfer { to: r.address, amount: amt, memo }).await?;
        }
        Cmd::Seal { conn, to, kind, title, text, file, keep_copy } => {
            let c = Client::connect(&conn).await?;
            let kind: PayloadKind = kind.parse()?;
            let addrs: Vec<Address> = to.iter().map(|t| c.resolve(t).map(|m| m.address)).collect::<Result<_>>()?;
            let (body, ct, title) = match (text, file) {
                (Some(t), None) => (t.into_bytes(), "text/plain".to_string(), title),
                (None, Some(p)) => {
                    let name = p.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                    (std::fs::read(&p)?, content_type_for(&p).to_string(), if title.is_empty() { name } else { title })
                }
                _ => bail!("give --text or --file"),
            };
            let env = Envelope::seal(&c.gh, &c.keys, &c.recipients(&addrs, keep_copy)?, &payload(kind, &title, &ct, body, vec![]))?;
            let id = env.id;
            c.submit(TxBody::Seal { envelope: env }).await?;
            println!("envelope {id}");
        }
        Cmd::Inbox { conn, since, save_dir } => {
            let c = Client::connect(&conn).await?;
            let opened = c.inbox(since).await?;
            if let Some(d) = &save_dir {
                std::fs::create_dir_all(d)?;
            }
            for (rec, o) in &opened {
                let p = &o.payload;
                let from = c.handle_of(&o.sender);
                println!("── {} · {:?} · from {} · block {} · {}", o.envelope_id, p.kind, from, rec["height"], rec["tx_kind"].as_str().unwrap_or(""));
                if !p.title.is_empty() {
                    println!("   title: {}", p.title);
                }
                if p.content_type.starts_with("text/") {
                    let s = String::from_utf8_lossy(&p.body.0);
                    println!("   {}", if s.len() > 2000 { format!("{}…", &s[..2000]) } else { s.into_owned() });
                } else {
                    println!("   {} bytes ({})", p.body.0.len(), p.content_type);
                }
                if let Some(d) = &save_dir {
                    let path = d.join(o.envelope_id.to_string());
                    std::fs::write(&path, &p.body.0)?;
                    println!("   saved {}", path.display());
                }
            }
            println!("{} envelope(s) opened", opened.len());
        }
        Cmd::Dispatch { conn, to, task, escrow, agent_id, expires_in_blocks } => {
            let c = Client::connect(&conn).await?;
            let r = c.resolve(&to)?;
            let env = Envelope::seal(
                &c.gh,
                &c.keys,
                &c.recipients(&[r.address], true)?,
                &payload(PayloadKind::AgentTask, "agent task", "text/plain", task.into_bytes(), vec![]),
            )?;
            let h = c.submit(TxBody::AgentDispatch { to: r.address, agent_id, task: env, escrow: parse_amount(&escrow)?, expires_in_blocks }).await?;
            println!("dispatch id {h}");
        }
        Cmd::Respond { conn, dispatch, decline, result } => {
            let c = Client::connect(&conn).await?;
            let id: H256 = dispatch.parse()?;
            let me = c.me().await?;
            let from: Address = me["dispatches"]
                .as_array()
                .and_then(|a| a.iter().find(|d| d["dispatch"]["id"] == json!(id.to_string())))
                .and_then(|d| d["dispatch"]["from"].as_str())
                .ok_or_else(|| anyhow!("dispatch {id} not found among yours"))?
                .parse()?;
            let result = match result {
                Some(t) => Some(Envelope::seal(
                    &c.gh,
                    &c.keys,
                    &c.recipients(&[from], true)?,
                    &payload(PayloadKind::AgentResult, "agent result", "text/plain", t.into_bytes(), vec![id]),
                )?),
                None => None,
            };
            c.submit(TxBody::AgentRespond { dispatch_id: id, accept: !decline, result }).await?;
        }
        Cmd::Reclaim { conn, dispatch } => {
            let c = Client::connect(&conn).await?;
            c.submit(TxBody::AgentReclaim { dispatch_id: dispatch.parse()? }).await?;
        }
        Cmd::RwaIssue { conn, asset_id, name, supply, document, holders } => {
            let c = Client::connect(&conn).await?;
            let doc = std::fs::read(&document)?;
            let document_hash = tagged_hash(b"lc/rwa-document/v1", &[&doc]);
            let addrs: Vec<Address> = holders.iter().map(|t| c.resolve(t).map(|m| m.address)).collect::<Result<_>>()?;
            let terms = if addrs.is_empty() {
                None
            } else {
                let fname = document.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                Some(Envelope::seal(
                    &c.gh,
                    &c.keys,
                    &c.recipients(&addrs, true)?,
                    &payload(PayloadKind::RwaTerms, &fname, content_type_for(&document), doc, vec![document_hash]),
                )?)
            };
            c.submit(TxBody::RwaIssue { asset_id, name, supply, document_hash, terms }).await?;
            println!("document hash {document_hash}");
        }
        Cmd::RwaSend { conn, asset_id, to, amount } => {
            let c = Client::connect(&conn).await?;
            let r = c.resolve(&to)?;
            c.submit(TxBody::RwaTransfer { asset_id, to: r.address, amount, memo: None }).await?;
        }
        Cmd::Deposit { conn, to, amount, evm_key, mpp, max } => {
            let c = Client::connect(&conn).await?;
            let to = match to {
                Some(t) => c.resolve(&t)?.address,
                None => c.keys.address(),
            };
            let amt = parse_amount(&amount)?;
            let quote = c.deposit(&json!({ "to": to, "amount": amount }), None).await?;
            let (status, body, headers) = quote;
            if status != 402 {
                bail!("expected a 402 quote, got {status}: {body}");
            }
            let req = body["accepts"][0].clone();
            println!(
                "402 quote: {} USDC on {} -> {} (credits {})",
                format_amount(amt),
                req["network"].as_str().unwrap_or(""),
                req["payTo"].as_str().unwrap_or(""),
                to
            );
            let Some(k) = evm_key else {
                println!("Quote only. Set LC_EVM_KEY (a Base wallet holding USDC) to pay.");
                return Ok(());
            };
            if amt > parse_amount(&max)? {
                bail!("quote {} exceeds --max {max}", format_amount(amt));
            }
            let key = evm_key_from_hex(&k)?;
            let now = now_ms() / 1000;
            let auth = TransferAuthorization {
                from: evm_address_of(key.verifying_key()),
                to: req["payTo"].as_str().ok_or_else(|| anyhow!("quote missing payTo"))?.parse()?,
                value: amt as u128,
                valid_after: now.saturating_sub(60),
                valid_before: now + 300,
                nonce: H256(random_bytes::<32>()),
            };
            let domain = c.genesis_domain().await?;
            let sig = evm_sign(&key, &auth.digest(&domain))?;
            let header = if mpp {
                let www = headers.get("www-authenticate").cloned().ok_or_else(|| anyhow!("node sent no MPP challenge"))?;
                let field = |name: &str| -> Result<String> {
                    let pat = format!("{name}=\"");
                    let s = www.find(&pat).ok_or_else(|| anyhow!("MPP challenge missing {name}"))? + pat.len();
                    Ok(www[s..s + www[s..].find('"').unwrap_or(0)].to_string())
                };
                let cred = json!({
                    "challenge": { "id": field("id")?, "realm": field("realm")?, "method": field("method")?, "intent": field("intent")?, "request": field("request")?, "expires": field("expires")? },
                    "source": format!("did:pkh:eip155:{}:{}", BASE_CHAIN_ID, auth.from),
                    "payload": {
                        "signature": format!("0x{}", hex::encode(sig)),
                        "authorization": { "from": auth.from.to_string(), "to": auth.to.to_string(), "value": auth.value.to_string(), "validAfter": auth.valid_after.to_string(), "validBefore": auth.valid_before.to_string(), "nonce": format!("0x{}", auth.nonce) }
                    }
                });
                ("authorization", format!("Payment {}", lc_core::payments::b64url(&cred)))
            } else {
                ("payment-signature", lc_core::payments::build_x402_payment(&req, body.get("resource"), &auth, &sig))
            };
            let (status, body, _) = c.deposit(&json!({ "to": to, "amount": amount }), Some(header)).await?;
            if status != 200 {
                bail!("deposit failed ({status}): {body}");
            }
            print(&body);
            if let Some(h) = body["mint_tx"].as_str() {
                if !conn.no_wait {
                    c.wait(&h.parse()?).await?;
                }
            }
        }
        Cmd::Withdraw { conn, amount, payout_to } => {
            let c = Client::connect(&conn).await?;
            let h = c.submit(TxBody::BridgeBurn { amount: parse_amount(&amount)?, payout_to: payout_to.parse()? }).await?;
            println!("withdrawal id {h} (the bridge pays out from the treasury and records the Base transaction)");
        }
        Cmd::PayoutSettled { conn, burn, settlement_tx } => {
            let c = Client::connect(&conn).await?;
            c.submit(TxBody::BridgePayoutSettled { burn_id: burn.parse()?, settlement_tx }).await?;
        }
        Cmd::Admit { conn, member, roles } => {
            let c = Client::connect(&conn).await?;
            let mut m: Member = read_json(&member)?;
            for r in roles.iter().filter(|r| !r.is_empty()) {
                m.roles.insert(parse_role(r)?);
            }
            c.submit(TxBody::AdmitMember { member: m }).await?;
        }
        Cmd::Agent { conn, agent_id } => print(&Client::connect(&conn).await?.get(&format!("/v1/erc8004/{agent_id}")).await?),
        Cmd::Tx { conn, hash } => print(&Client::connect(&conn).await?.get(&format!("/v1/tx/{hash}")).await?),
    }
    Ok(())
}
