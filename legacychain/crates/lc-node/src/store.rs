//! Append-only, fsync'd storage. Committed blocks go to `blocks.log` one per
//! line; with a storage key every line is sealed with XChaCha20-Poly1305 so a
//! copied disk reveals nothing. The validator's last vote and the bridge's
//! settled-deposit journal live beside it.

use anyhow::{bail, Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use lc_core::block::Block;
use lc_core::tx::DepositProof;
use lc_core::types::{Address, H256};
use rand::RngCore;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

const ENC_PREFIX: &str = "e1:";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastVote {
    pub height: u64,
    pub block_hash: H256,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettledDeposit {
    pub to: Address,
    pub deposit: DepositProof,
}

pub struct Store {
    dir: PathBuf,
    key: Option<[u8; 32]>,
    genesis_hash: H256,
    blocks: File,
    deposits: File,
}

fn open_append(p: &Path) -> Result<File> {
    OpenOptions::new().create(true).append(true).open(p).with_context(|| format!("open {}", p.display()))
}

impl Store {
    pub fn open(dir: &Path, key: Option<[u8; 32]>, genesis_hash: H256) -> Result<Self> {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
        let marker = dir.join("GENESIS");
        match fs::read_to_string(&marker) {
            Ok(existing) if existing.trim() != genesis_hash.to_string() => {
                bail!("data dir {} belongs to genesis {}, not {}", dir.display(), existing.trim(), genesis_hash)
            }
            Ok(_) => {}
            Err(_) => fs::write(&marker, genesis_hash.to_string())?,
        }
        Ok(Store {
            dir: dir.to_path_buf(),
            key,
            genesis_hash,
            blocks: open_append(&dir.join("blocks.log"))?,
            deposits: open_append(&dir.join("deposits.log"))?,
        })
    }

    fn seal_line(&self, json: &[u8]) -> Result<String> {
        match &self.key {
            None => Ok(String::from_utf8(json.to_vec())?),
            Some(k) => {
                let mut nonce = [0u8; 24];
                rand::rngs::OsRng.fill_bytes(&mut nonce);
                let ct = XChaCha20Poly1305::new(k.into())
                    .encrypt(&nonce.into(), Payload { msg: json, aad: &self.genesis_hash.0 })
                    .map_err(|_| anyhow::anyhow!("storage encryption failed"))?;
                let mut buf = nonce.to_vec();
                buf.extend_from_slice(&ct);
                Ok(format!("{ENC_PREFIX}{}", STANDARD.encode(buf)))
            }
        }
    }

    fn open_line<T: DeserializeOwned>(&self, line: &str) -> Result<T> {
        if let Some(b) = line.strip_prefix(ENC_PREFIX) {
            let k = self.key.as_ref().context("storage is encrypted; set LC_STORAGE_KEY")?;
            let raw = STANDARD.decode(b)?;
            if raw.len() < 24 {
                bail!("truncated record");
            }
            let pt = XChaCha20Poly1305::new(k.into())
                .decrypt(raw[..24].into(), Payload { msg: &raw[24..], aad: &self.genesis_hash.0 })
                .map_err(|_| anyhow::anyhow!("wrong LC_STORAGE_KEY or corrupted record"))?;
            Ok(serde_json::from_slice(&pt)?)
        } else {
            Ok(serde_json::from_str(line)?)
        }
    }

    fn read_all<T: DeserializeOwned>(&self, name: &str) -> Result<Vec<T>> {
        let p = self.dir.join(name);
        let f = match File::open(&p) {
            Ok(f) => f,
            Err(_) => return Ok(vec![]),
        };
        let mut out = Vec::new();
        for (i, line) in BufReader::new(f).lines().enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(self.open_line(&line).with_context(|| format!("{name} line {}", i + 1))?);
        }
        Ok(out)
    }

    fn append(file: &mut File, line: String) -> Result<()> {
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    }

    pub fn load_blocks(&self) -> Result<Vec<Block>> {
        self.read_all("blocks.log")
    }

    pub fn append_block(&mut self, b: &Block) -> Result<()> {
        let line = self.seal_line(&serde_json::to_vec(b)?)?;
        Self::append(&mut self.blocks, line)
    }

    pub fn load_deposits(&self) -> Result<Vec<SettledDeposit>> {
        self.read_all("deposits.log")
    }

    pub fn append_deposit(&mut self, d: &SettledDeposit) -> Result<()> {
        let line = self.seal_line(&serde_json::to_vec(d)?)?;
        Self::append(&mut self.deposits, line)
    }

    pub fn load_vote(&self) -> Result<Option<LastVote>> {
        match fs::read_to_string(self.dir.join("vote.json")) {
            Ok(s) => Ok(Some(serde_json::from_str(&s)?)),
            Err(_) => Ok(None),
        }
    }

    /// Durably record a vote before it leaves the process, so a restarted
    /// validator can never sign two different blocks at one height.
    pub fn save_vote(&self, v: &LastVote) -> Result<()> {
        let tmp = self.dir.join("vote.json.tmp");
        {
            let mut f = File::create(&tmp)?;
            f.write_all(&serde_json::to_vec(v)?)?;
            f.sync_all()?;
        }
        fs::rename(&tmp, self.dir.join("vote.json"))?;
        if let Ok(d) = File::open(&self.dir) {
            let _ = d.sync_all();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_core::block::BlockHeader;
    use lc_core::types::Address;

    fn block() -> Block {
        Block {
            header: BlockHeader {
                chain_id: "t".into(),
                height: 1,
                round: 0,
                prev_hash: H256([1u8; 32]),
                timestamp_ms: 5,
                tx_root: H256([2u8; 32]),
                state_root: H256([3u8; 32]),
                proposer: Address([4u8; 20]),
            },
            txs: vec![],
            votes: vec![],
        }
    }

    #[test]
    fn encrypted_log_round_trip_and_wrong_key() {
        let dir = std::env::temp_dir().join(format!("lc-store-{}", hex::encode(rand::random::<[u8; 8]>())));
        let gh = H256([9u8; 32]);
        {
            let mut s = Store::open(&dir, Some([7u8; 32]), gh).unwrap();
            s.append_block(&block()).unwrap();
            s.save_vote(&LastVote { height: 1, block_hash: H256([5u8; 32]) }).unwrap();
        }
        let raw = std::fs::read_to_string(dir.join("blocks.log")).unwrap();
        assert!(raw.starts_with(ENC_PREFIX) && !raw.contains("proposer"), "block log must be ciphertext at rest");
        let s = Store::open(&dir, Some([7u8; 32]), gh).unwrap();
        assert_eq!(s.load_blocks().unwrap(), vec![block()]);
        assert_eq!(s.load_vote().unwrap().unwrap().height, 1);
        assert!(Store::open(&dir, Some([8u8; 32]), gh).unwrap().load_blocks().is_err());
        assert!(Store::open(&dir, None, gh).unwrap().load_blocks().is_err());
        assert!(Store::open(&dir, Some([7u8; 32]), H256([0u8; 32])).is_err(), "data dir is pinned to its genesis");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
