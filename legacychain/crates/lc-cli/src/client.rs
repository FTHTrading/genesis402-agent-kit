//! Signed HTTP client for a LegacyChain node.

use crate::Conn;
use anyhow::{anyhow, bail, Context, Result};
use lc_core::auth;
use lc_core::crypto::{Eip712Domain, MemberKeys};
use lc_core::envelope::{Envelope, Opened, Recipient};
use lc_core::genesis::Genesis;
use lc_core::keyfile::KeyFile;
use lc_core::tx::{Tx, TxBody};
use lc_core::types::{Address, Pk32, H256};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Deserialize)]
pub struct DirEntry {
    pub handle: String,
    pub address: Address,
    pub sign_pk: Pk32,
    pub seal_pk: Pk32,
}

pub struct Client {
    base: String,
    http: reqwest::Client,
    pub keys: MemberKeys,
    pub gh: H256,
    dir: Vec<DirEntry>,
    no_wait: bool,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

async fn json_or_error(resp: reqwest::Response) -> Result<Value> {
    let status = resp.status();
    let v: Value = resp.json().await.unwrap_or(Value::Null);
    if !status.is_success() {
        bail!("{status}: {}", v.get("error").and_then(|e| e.as_str()).unwrap_or("request failed"));
    }
    Ok(v)
}

impl Client {
    pub async fn connect(conn: &Conn) -> Result<Self> {
        let kf: KeyFile = serde_json::from_slice(&std::fs::read(&conn.key).with_context(|| format!("read {}", conn.key.display()))?)?;
        let keys = kf.unlock(std::env::var("LC_PASSPHRASE").ok().as_deref())?;
        let http = reqwest::Client::builder().timeout(Duration::from_secs(90)).build()?;
        let base = conn.node.trim_end_matches('/').to_string();
        let health = json_or_error(http.get(format!("{base}/v1/health")).send().await.with_context(|| format!("node {base} unreachable"))?).await?;
        let gh: H256 = health["genesis_hash"].as_str().ok_or_else(|| anyhow!("node did not report a genesis hash"))?.parse()?;
        if let Some(pin) = &conn.genesis_hash {
            if pin.parse::<H256>()? != gh {
                bail!("node is on genesis {gh}, not the pinned {pin}");
            }
        }
        let mut c = Client { base, http, keys, gh, dir: vec![], no_wait: conn.no_wait };
        let d = c.get("/v1/directory").await?;
        c.dir = serde_json::from_value(d["members"].clone())?;
        Ok(c)
    }

    pub async fn get(&self, path: &str) -> Result<Value> {
        let h = auth::sign(&self.gh, &self.keys, "GET", path, now_ms(), b"");
        json_or_error(self.http.get(format!("{}{}", self.base, path)).header(auth::HEADER, h).send().await?).await
    }

    pub async fn me(&self) -> Result<Value> {
        self.get("/v1/me").await
    }

    pub fn resolve(&self, who: &str) -> Result<DirEntry> {
        if let Ok(a) = who.parse::<Address>() {
            return self.dir.iter().find(|m| m.address == a).cloned().ok_or_else(|| anyhow!("{a} is not a member"));
        }
        self.dir.iter().find(|m| m.handle.eq_ignore_ascii_case(who)).cloned().ok_or_else(|| anyhow!("no member '{who}'"))
    }

    pub fn handle_of(&self, a: &Address) -> String {
        self.dir.iter().find(|m| &m.address == a).map(|m| m.handle.clone()).unwrap_or_else(|| a.to_string())
    }

    pub fn recipients(&self, addrs: &[Address], keep_copy: bool) -> Result<Vec<Recipient>> {
        let mut list: Vec<Address> = addrs.to_vec();
        if keep_copy {
            list.push(self.keys.address());
        }
        list.sort();
        list.dedup();
        list.iter().map(|a| self.resolve(&a.to_string()).map(|m| Recipient { seal_pk: m.seal_pk })).collect()
    }

    pub async fn submit(&self, body: TxBody) -> Result<H256> {
        let nonce = self.me().await?["next_nonce"].as_u64().ok_or_else(|| anyhow!("node did not report a nonce"))?;
        let tx = Tx::sign(&self.gh, &self.keys, nonce, body);
        let v = json_or_error(self.http.post(format!("{}/v1/tx", self.base)).json(&tx).send().await?).await?;
        let h: H256 = v["tx_hash"].as_str().ok_or_else(|| anyhow!("no tx hash"))?.parse()?;
        println!("submitted {} {h}", tx.body.kind());
        if !self.no_wait {
            self.wait(&h).await?;
        }
        Ok(h)
    }

    pub async fn wait(&self, h: &H256) -> Result<()> {
        for _ in 0..120 {
            let s = self.get(&format!("/v1/tx/{h}")).await?;
            match s["status"].as_str() {
                Some("committed") => {
                    println!("committed in block {} ({})", s["height"], s["block_hash"].as_str().unwrap_or(""));
                    return Ok(());
                }
                Some("rejected") => bail!("rejected: {}", s["reason"].as_str().unwrap_or("")),
                _ => tokio::time::sleep(Duration::from_millis(250)).await,
            }
        }
        bail!("not committed after 30s; check `lc tx {h}`")
    }

    /// Scan envelopes since `since` and open the ones addressed to us.
    pub async fn inbox(&self, since: u64) -> Result<Vec<(Value, Opened)>> {
        let mut out = Vec::new();
        let mut cursor = since;
        loop {
            let page = self.get(&format!("/v1/envelopes?since={cursor}&limit=1000")).await?;
            let recs = page["envelopes"].as_array().cloned().unwrap_or_default();
            if recs.is_empty() {
                break;
            }
            for r in &recs {
                let env: Envelope = serde_json::from_value(r["envelope"].clone())?;
                if !env.is_for(&self.gh, &self.keys) {
                    continue;
                }
                let sender = self.dir.iter().find(|m| m.address == env.sender).ok_or_else(|| anyhow!("unknown sender {}", env.sender))?;
                out.push((r.clone(), env.open(&self.gh, &self.keys, &sender.sign_pk)?));
            }
            let next = page["next_since"].as_u64().unwrap_or(cursor);
            if next <= cursor {
                break;
            }
            cursor = next;
        }
        Ok(out)
    }

    pub async fn deposit(&self, body: &Value, header: Option<(&str, String)>) -> Result<(u16, Value, HashMap<String, String>)> {
        let mut rb = self.http.post(format!("{}/v1/bridge/deposit", self.base)).json(body);
        if let Some((k, v)) = header {
            rb = rb.header(k, v);
        }
        let resp = rb.send().await?;
        let status = resp.status().as_u16();
        let headers = resp.headers().iter().filter_map(|(k, v)| v.to_str().ok().map(|s| (k.as_str().to_string(), s.to_string()))).collect();
        let v = resp.json().await.unwrap_or(Value::Null);
        Ok((status, v, headers))
    }

    pub async fn genesis_domain(&self) -> Result<Eip712Domain> {
        let v = self.get("/v1/genesis").await?;
        let g: Genesis = serde_json::from_value(v["genesis"].clone())?;
        if g.hash() != self.gh {
            bail!("node served a genesis that does not hash to {}", self.gh);
        }
        Ok(g.eip712_domain())
    }
}
