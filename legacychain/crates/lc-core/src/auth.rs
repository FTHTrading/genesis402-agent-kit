//! Signed HTTP requests. Members and validators authenticate every API call
//! with `X-LC-Auth: <address>:<unix_ms>:<ed25519 sig hex>` over a digest of the
//! genesis hash, method, path+query, timestamp and body hash. Nothing on a
//! LegacyChain node is readable anonymously except its health line and the
//! 402 deposit quote.

use crate::crypto::{verify, MemberKeys};
use crate::error::{Error, Result};
use crate::types::{tagged_hash, Address, Pk32, Sig64, H256};

pub const HEADER: &str = "x-lc-auth";
/// Maximum clock skew accepted, milliseconds.
pub const MAX_SKEW_MS: u64 = 60_000;

pub fn digest(genesis_hash: &H256, method: &str, path_and_query: &str, ts_ms: u64, body: &[u8]) -> H256 {
    let body_hash = tagged_hash(b"lc/auth/body/v1", &[body]);
    tagged_hash(b"lc/auth/v1", &[&genesis_hash.0, method.to_ascii_uppercase().as_bytes(), path_and_query.as_bytes(), &ts_ms.to_be_bytes(), &body_hash.0])
}

pub fn sign(genesis_hash: &H256, keys: &MemberKeys, method: &str, path_and_query: &str, ts_ms: u64, body: &[u8]) -> String {
    let sig = keys.sign(&digest(genesis_hash, method, path_and_query, ts_ms, body));
    format!("{}:{}:{}", keys.address(), ts_ms, sig)
}

/// Parse the header without verifying it; returns (address, ts, sig).
pub fn parse(header: &str) -> Result<(Address, u64, Sig64)> {
    let mut it = header.trim().splitn(3, ':');
    let addr: Address = it.next().unwrap_or_default().parse()?;
    let ts: u64 = it.next().unwrap_or_default().parse().map_err(|_| Error::Unauthorized("bad auth timestamp".into()))?;
    let sig: Sig64 = it.next().unwrap_or_default().parse()?;
    Ok((addr, ts, sig))
}

#[allow(clippy::too_many_arguments)]
pub fn check(genesis_hash: &H256, sign_pk: &Pk32, method: &str, path_and_query: &str, ts_ms: u64, sig: &Sig64, body: &[u8], now_ms: u64) -> Result<()> {
    if ts_ms.abs_diff(now_ms) > MAX_SKEW_MS {
        return Err(Error::Unauthorized("request timestamp outside the allowed window".into()));
    }
    verify(sign_pk, &digest(genesis_hash, method, path_and_query, ts_ms, body), sig).map_err(|_| Error::Unauthorized("bad request signature".into()))
}
