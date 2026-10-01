//! Key material and signature primitives.
//!
//! Every LegacyChain identity holds two keys:
//! * an Ed25519 signing key (authorizes transactions, votes, API calls);
//! * an X25519 sealing key (receives sealed envelopes).
//!
//! EVM primitives (keccak, EIP-191 recovery, EIP-712 / EIP-3009 digests) live
//! here too, because ERC-8004 identity binding and x402 deposits are verified
//! against Base accounts.

use crate::error::{Error, Result};
use crate::types::{Address, EvmAddress, Pk32, Sig64, H256};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use k256::ecdsa::{RecoveryId, Signature as EcdsaSignature, SigningKey as EcdsaSigningKey, VerifyingKey as EcdsaVerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use sha3::{Digest, Keccak256};
use x25519_dalek::{PublicKey as XPublic, StaticSecret};
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Secret material for one member. Zeroized on drop.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct MemberKeys {
    sign_seed: [u8; 32],
    seal_secret: [u8; 32],
}

impl MemberKeys {
    pub fn generate() -> Self {
        let mut sign_seed = [0u8; 32];
        let mut seal_secret = [0u8; 32];
        OsRng.fill_bytes(&mut sign_seed);
        OsRng.fill_bytes(&mut seal_secret);
        Self { sign_seed, seal_secret }
    }

    pub fn from_secrets(sign_seed: [u8; 32], seal_secret: [u8; 32]) -> Self {
        Self { sign_seed, seal_secret }
    }

    pub fn sign_seed(&self) -> &[u8; 32] {
        &self.sign_seed
    }

    pub fn seal_secret_bytes(&self) -> &[u8; 32] {
        &self.seal_secret
    }

    fn signing_key(&self) -> SigningKey {
        SigningKey::from_bytes(&self.sign_seed)
    }

    pub fn sign_pk(&self) -> Pk32 {
        Pk32(self.signing_key().verifying_key().to_bytes())
    }

    pub fn seal_secret(&self) -> StaticSecret {
        StaticSecret::from(self.seal_secret)
    }

    pub fn seal_pk(&self) -> Pk32 {
        Pk32(XPublic::from(&self.seal_secret()).to_bytes())
    }

    pub fn address(&self) -> Address {
        Address::from_sign_key(&self.sign_pk())
    }

    pub fn sign(&self, msg: &H256) -> Sig64 {
        Sig64(self.signing_key().sign(&msg.0).to_bytes())
    }
}

/// Verify an Ed25519 signature over a 32-byte digest. Uses strict verification
/// (rejects non-canonical and small-order encodings).
pub fn verify(pk: &Pk32, msg: &H256, sig: &Sig64) -> Result<()> {
    let vk = VerifyingKey::from_bytes(&pk.0).map_err(|e| Error::Crypto(format!("bad public key: {e}")))?;
    let s = ed25519_dalek::Signature::from_bytes(&sig.0);
    vk.verify_strict(&msg.0, &s).map_err(|_| Error::Crypto("signature does not verify".into()))
}

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    OsRng.fill_bytes(&mut b);
    b
}

// ---------------------------------------------------------------- EVM side

pub fn keccak256(data: &[u8]) -> [u8; 32] {
    Keccak256::digest(data).into()
}

pub fn evm_address_of(vk: &EcdsaVerifyingKey) -> EvmAddress {
    let point = vk.to_encoded_point(false);
    let h = keccak256(&point.as_bytes()[1..]);
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    EvmAddress(a)
}

/// EIP-191 `personal_sign` digest.
pub fn eip191_digest(message: &[u8]) -> [u8; 32] {
    let mut buf = format!("\x19Ethereum Signed Message:\n{}", message.len()).into_bytes();
    buf.extend_from_slice(message);
    keccak256(&buf)
}

/// Recover the EVM address that produced a 65-byte `r || s || v` signature
/// over `digest`. Accepts v in {0,1,27,28}; rejects high-s signatures.
pub fn ecrecover(digest: &[u8; 32], sig65: &[u8]) -> Result<EvmAddress> {
    if sig65.len() != 65 {
        return Err(Error::Crypto("EVM signature must be 65 bytes".into()));
    }
    let v = match sig65[64] {
        0 | 27 => 0u8,
        1 | 28 => 1u8,
        other => return Err(Error::Crypto(format!("bad recovery id {other}"))),
    };
    let sig = EcdsaSignature::from_slice(&sig65[..64]).map_err(|e| Error::Crypto(format!("bad signature: {e}")))?;
    if sig.normalize_s().is_some() {
        return Err(Error::Crypto("non-canonical (high-s) signature".into()));
    }
    let rid = RecoveryId::from_byte(v).ok_or_else(|| Error::Crypto("bad recovery id".into()))?;
    let vk = EcdsaVerifyingKey::recover_from_prehash(digest, &sig, rid).map_err(|e| Error::Crypto(format!("recovery failed: {e}")))?;
    Ok(evm_address_of(&vk))
}

/// Sign a digest with a secp256k1 key, returning `r || s || v` (v = 27/28).
pub fn evm_sign(key: &EcdsaSigningKey, digest: &[u8; 32]) -> Result<[u8; 65]> {
    let (sig, rid) = key.sign_prehash_recoverable(digest).map_err(|e| Error::Crypto(format!("signing failed: {e}")))?;
    let mut out = [0u8; 65];
    out[..64].copy_from_slice(&sig.to_bytes());
    out[64] = 27 + rid.to_byte();
    Ok(out)
}

pub fn evm_key_from_hex(s: &str) -> Result<EcdsaSigningKey> {
    let raw = hex::decode(s.trim().trim_start_matches("0x")).map_err(|e| Error::Encoding(format!("EVM key: {e}")))?;
    EcdsaSigningKey::from_slice(&raw).map_err(|e| Error::Crypto(format!("EVM key: {e}")))
}

fn abi_word_u256(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn abi_word_addr(a: &EvmAddress) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(&a.0);
    w
}

/// EIP-712 domain for an ERC-20 implementing EIP-3009 (USDC: name "USD Coin", version "2").
#[derive(Clone, Debug)]
pub struct Eip712Domain {
    pub name: String,
    pub version: String,
    pub chain_id: u64,
    pub verifying_contract: EvmAddress,
}

impl Eip712Domain {
    pub fn separator(&self) -> [u8; 32] {
        let type_hash = keccak256(b"EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)");
        let mut enc = Vec::with_capacity(32 * 5);
        enc.extend_from_slice(&type_hash);
        enc.extend_from_slice(&keccak256(self.name.as_bytes()));
        enc.extend_from_slice(&keccak256(self.version.as_bytes()));
        enc.extend_from_slice(&abi_word_u256(self.chain_id as u128));
        enc.extend_from_slice(&abi_word_addr(&self.verifying_contract));
        keccak256(&enc)
    }
}

/// EIP-3009 `TransferWithAuthorization` message, as carried in an x402
/// `exact` EVM payment payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransferAuthorization {
    pub from: EvmAddress,
    pub to: EvmAddress,
    pub value: u128,
    pub valid_after: u64,
    pub valid_before: u64,
    pub nonce: H256,
}

impl TransferAuthorization {
    pub fn digest(&self, domain: &Eip712Domain) -> [u8; 32] {
        let type_hash = keccak256(b"TransferWithAuthorization(address from,address to,uint256 value,uint256 validAfter,uint256 validBefore,bytes32 nonce)");
        let mut enc = Vec::with_capacity(32 * 7);
        enc.extend_from_slice(&type_hash);
        enc.extend_from_slice(&abi_word_addr(&self.from));
        enc.extend_from_slice(&abi_word_addr(&self.to));
        enc.extend_from_slice(&abi_word_u256(self.value));
        enc.extend_from_slice(&abi_word_u256(self.valid_after as u128));
        enc.extend_from_slice(&abi_word_u256(self.valid_before as u128));
        enc.extend_from_slice(&self.nonce.0);
        let struct_hash = keccak256(&enc);
        let mut msg = Vec::with_capacity(66);
        msg.extend_from_slice(&[0x19, 0x01]);
        msg.extend_from_slice(&domain.separator());
        msg.extend_from_slice(&struct_hash);
        keccak256(&msg)
    }

    /// Verify the authorization was signed by `from`.
    pub fn verify_signature(&self, domain: &Eip712Domain, sig65: &[u8]) -> Result<()> {
        let signer = ecrecover(&self.digest(domain), sig65)?;
        if signer != self.from {
            return Err(Error::Payment(format!("authorization signed by {signer}, not by payer {}", self.from)));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::tagged_hash;

    #[test]
    fn ed25519_sign_verify() {
        let k = MemberKeys::generate();
        let m = tagged_hash(b"test", &[b"hello"]);
        let s = k.sign(&m);
        verify(&k.sign_pk(), &m, &s).unwrap();
        let other = tagged_hash(b"test", &[b"hellp"]);
        assert!(verify(&k.sign_pk(), &other, &s).is_err());
    }

    #[test]
    fn keccak_known_vector() {
        assert_eq!(hex::encode(keccak256(b"")), "c5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470");
    }

    #[test]
    fn evm_address_known_vector() {
        // Private key 0x...01 controls 0x7E5F4552091A69125d5DfCb7b8C2659029395Bdf.
        let k = evm_key_from_hex("0000000000000000000000000000000000000000000000000000000000000001").unwrap();
        assert_eq!(evm_address_of(k.verifying_key()).to_string(), "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf");
    }

    /// Vector produced by viem 2 `hashTypedData` / `signTypedData` (what x402
    /// clients use) for a USDC-on-Base TransferWithAuthorization.
    #[test]
    fn eip3009_digest_matches_viem() {
        let domain = Eip712Domain {
            name: "USD Coin".into(),
            version: "2".into(),
            chain_id: 8453,
            verifying_contract: "0x833589fcd6edb6e08f4c7c32d4f71b54bda02913".parse().unwrap(),
        };
        let auth = TransferAuthorization {
            from: "0x2c7536e3605d9c16a7a3d7b1898e529396a65c23".parse().unwrap(),
            to: "0x1111111111111111111111111111111111111111".parse().unwrap(),
            value: 25_000_000,
            valid_after: 0,
            valid_before: 1_800_000_300,
            nonce: H256([0x42; 32]),
        };
        assert_eq!(hex::encode(auth.digest(&domain)), "b1be5a97815915de3c0c590c26c8c996a4be552ce86fff59a45ce21d5677184b");
        let viem_sig =
            hex::decode("316d066422781f9f96e6ad4ebd5bd1bbbfdd1955283e42f95c6b44c0f8beb2f650547b3d6bc27994441884f6044b231918c12e1610a6d885d461bcacec41bea41c")
                .unwrap();
        auth.verify_signature(&domain, &viem_sig).unwrap();
        let k = evm_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318").unwrap();
        assert_eq!(evm_sign(&k, &auth.digest(&domain)).unwrap().to_vec(), viem_sig, "RFC6979 deterministic signature matches viem");
    }

    #[test]
    fn personal_sign_round_trip() {
        let k = evm_key_from_hex("4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318").unwrap();
        let d = eip191_digest(b"Some data");
        let sig = evm_sign(&k, &d).unwrap();
        assert_eq!(ecrecover(&d, &sig).unwrap(), evm_address_of(k.verifying_key()));
    }
}
