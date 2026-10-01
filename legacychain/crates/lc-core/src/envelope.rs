//! Sealed envelopes: the unit of private information on LegacyChain.
//!
//! An envelope carries any payload (message, document, contract, payment
//! advice, RWA terms, agent task) encrypted so that only its recipients can
//! open it, even though the ciphertext is stored on every validator.
//!
//! Construction (one envelope, N recipients):
//! 1. Random 32-byte content key `K`; payload is padded to a size bucket and
//!    encrypted with XChaCha20-Poly1305 under `K`.
//! 2. One ephemeral X25519 key `e` per envelope. For each recipient `R`:
//!    `ss = X25519(e, R)`, `okm = HKDF-SHA256(salt = genesis_hash, ikm = ss,
//!    info = "lc/seal/v1" || id || e_pub || R_pub)`; the first 16 bytes are an
//!    opaque lookup tag, the next 32 bytes wrap `K` with ChaCha20-Poly1305.
//! 3. Stanzas are shuffled and padded with decoys to a power of two, so the
//!    chain learns neither who the recipients are nor exactly how many.
//! 4. The sender signs the whole envelope with their Ed25519 key.
//!
//! The genesis hash is the HKDF salt and part of every signed digest, so an
//! envelope is cryptographically bound to the chain it was sealed for: it
//! cannot be opened or replayed under any other genesis.

use crate::crypto::{random_bytes, verify, MemberKeys};
use crate::error::{Error, Result};
use crate::types::{canonical, tagged_hash, Address, Bytes, Nonce24, Pk32, Sig64, Tag16, H256};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{ChaCha20Poly1305, XChaCha20Poly1305};
use hkdf::Hkdf;
use rand::seq::SliceRandom;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use x25519_dalek::{PublicKey as XPublic, StaticSecret};
use zeroize::Zeroizing;

pub const ENVELOPE_VERSION: u8 = 1;
/// Largest padded plaintext accepted on chain. Larger material is stored off
/// chain and referenced by hash inside a sealed payload.
pub const MAX_PLAINTEXT: usize = 4 * 1024 * 1024;
pub const MAX_RECIPIENTS: usize = 64;
const MIN_BUCKET: usize = 256;
const MIN_STANZAS: usize = 2;
const WRAPPED_KEY_LEN: usize = 32 + 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PayloadKind {
    Message,
    Document,
    Contract,
    PaymentAdvice,
    RwaTerms,
    AgentTask,
    AgentResult,
}

impl std::str::FromStr for PayloadKind {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        Ok(match s {
            "message" => Self::Message,
            "document" => Self::Document,
            "contract" => Self::Contract,
            "payment_advice" | "payment-advice" => Self::PaymentAdvice,
            "rwa_terms" | "rwa-terms" => Self::RwaTerms,
            "agent_task" | "agent-task" => Self::AgentTask,
            "agent_result" | "agent-result" => Self::AgentResult,
            other => return Err(Error::Encoding(format!("unknown payload kind '{other}'"))),
        })
    }
}

/// The plaintext inside an envelope. Never stored on chain unencrypted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SealedPayload {
    pub kind: PayloadKind,
    pub title: String,
    pub content_type: String,
    pub body: Bytes,
    pub created_at_ms: u64,
    /// Links to other chain objects (a transfer, a dispatch, an RWA asset document hash).
    pub refs: Vec<H256>,
    pub meta: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stanza {
    pub tag: Tag16,
    pub wrapped_key: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u8,
    pub id: H256,
    pub sender: Address,
    pub ephemeral_pk: Pk32,
    pub stanzas: Vec<Stanza>,
    pub nonce: Nonce24,
    pub ciphertext: Bytes,
    pub sender_sig: Sig64,
}

/// A recipient as seen by a sender: their seal public key.
#[derive(Clone, Copy, Debug)]
pub struct Recipient {
    pub seal_pk: Pk32,
}

/// Result of successfully opening an envelope.
#[derive(Clone, Debug)]
pub struct Opened {
    pub envelope_id: H256,
    pub sender: Address,
    pub payload: SealedPayload,
}

fn derive_stanza_keys(genesis_hash: &H256, shared: &[u8; 32], id: &H256, eph: &Pk32, recipient: &Pk32) -> Result<(Tag16, Zeroizing<[u8; 32]>)> {
    if shared.iter().all(|b| *b == 0) {
        return Err(Error::Crypto("low-order X25519 point".into()));
    }
    let hk = Hkdf::<Sha256>::new(Some(&genesis_hash.0), shared);
    let mut info = Vec::with_capacity(10 + 96);
    info.extend_from_slice(b"lc/seal/v1");
    info.extend_from_slice(&id.0);
    info.extend_from_slice(&eph.0);
    info.extend_from_slice(&recipient.0);
    let mut okm = Zeroizing::new([0u8; 48]);
    hk.expand(&info, okm.as_mut()).map_err(|_| Error::Crypto("hkdf expand".into()))?;
    let mut tag = [0u8; 16];
    tag.copy_from_slice(&okm[..16]);
    let mut wrap = Zeroizing::new([0u8; 32]);
    wrap.copy_from_slice(&okm[16..48]);
    Ok((Tag16(tag), wrap))
}

fn bucket_len(n: usize) -> usize {
    let need = n + 4;
    if need <= MIN_BUCKET {
        MIN_BUCKET
    } else if need <= 1024 * 1024 {
        need.next_power_of_two()
    } else {
        need.div_ceil(1024 * 1024) * 1024 * 1024
    }
}

fn pad(plain: &[u8]) -> Result<Vec<u8>> {
    let len = bucket_len(plain.len());
    if len > MAX_PLAINTEXT {
        return Err(Error::Encoding(format!(
            "payload is {} bytes; on-chain limit is {} (store large files off chain and seal their hash)",
            plain.len(),
            MAX_PLAINTEXT - 4
        )));
    }
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(&(plain.len() as u32).to_le_bytes());
    out.extend_from_slice(plain);
    out.resize(len, 0);
    Ok(out)
}

fn unpad(padded: &[u8]) -> Result<&[u8]> {
    if padded.len() < 4 {
        return Err(Error::Crypto("truncated plaintext".into()));
    }
    let n = u32::from_le_bytes([padded[0], padded[1], padded[2], padded[3]]) as usize;
    padded.get(4..4 + n).ok_or_else(|| Error::Crypto("bad padding".into()))
}

impl Envelope {
    /// Digest bound into the payload AEAD (everything except ciphertext and signature).
    fn header_digest(&self, genesis_hash: &H256) -> H256 {
        tagged_hash(
            b"lc/seal/header/v1",
            &[&genesis_hash.0, &[self.version], &self.id.0, &self.sender.0, &self.ephemeral_pk.0, &canonical(&self.stanzas), &self.nonce.0],
        )
    }

    /// Digest the sender signs.
    pub fn signing_digest(&self, genesis_hash: &H256) -> H256 {
        let h = self.header_digest(genesis_hash);
        tagged_hash(b"lc/seal/sig/v1", &[&h.0, &self.ciphertext.0])
    }

    /// Seal `payload` from `sender` to `recipients` under `genesis_hash`.
    pub fn seal(genesis_hash: &H256, sender: &MemberKeys, recipients: &[Recipient], payload: &SealedPayload) -> Result<Self> {
        if recipients.is_empty() {
            return Err(Error::Encoding("envelope needs at least one recipient".into()));
        }
        if recipients.len() > MAX_RECIPIENTS {
            return Err(Error::Encoding(format!("at most {MAX_RECIPIENTS} recipients")));
        }
        let mut seen = std::collections::BTreeSet::new();
        for r in recipients {
            if !seen.insert(r.seal_pk) {
                return Err(Error::Encoding("duplicate recipient".into()));
            }
        }

        let id = H256(random_bytes::<32>());
        let eph_secret = StaticSecret::from(random_bytes::<32>());
        let eph_pk = Pk32(XPublic::from(&eph_secret).to_bytes());
        let content_key = Zeroizing::new(random_bytes::<32>());

        let mut stanzas = Vec::with_capacity(recipients.len().max(MIN_STANZAS).next_power_of_two());
        for r in recipients {
            let shared = Zeroizing::new(eph_secret.diffie_hellman(&XPublic::from(r.seal_pk.0)).to_bytes());
            let (tag, wrap_key) = derive_stanza_keys(genesis_hash, &shared, &id, &eph_pk, &r.seal_pk)?;
            let cipher = ChaCha20Poly1305::new(wrap_key.as_ref().into());
            // The wrap key is unique per (ephemeral key, recipient), so a fixed nonce is safe.
            let wrapped =
                cipher.encrypt(&[0u8; 12].into(), Payload { msg: content_key.as_ref(), aad: &id.0 }).map_err(|_| Error::Crypto("key wrap failed".into()))?;
            stanzas.push(Stanza { tag, wrapped_key: Bytes(wrapped) });
        }
        let target = recipients.len().max(MIN_STANZAS).next_power_of_two();
        while stanzas.len() < target {
            stanzas.push(Stanza { tag: Tag16(random_bytes::<16>()), wrapped_key: Bytes(random_bytes::<WRAPPED_KEY_LEN>().to_vec()) });
        }
        stanzas.shuffle(&mut rand::rngs::OsRng);

        let mut env = Envelope {
            version: ENVELOPE_VERSION,
            id,
            sender: sender.address(),
            ephemeral_pk: eph_pk,
            stanzas,
            nonce: Nonce24(random_bytes::<24>()),
            ciphertext: Bytes::default(),
            sender_sig: Sig64([0u8; 64]),
        };
        let plain = Zeroizing::new(pad(&canonical(payload))?);
        let aad = env.header_digest(genesis_hash);
        let cipher = XChaCha20Poly1305::new(content_key.as_ref().into());
        env.ciphertext = Bytes(
            cipher.encrypt(&env.nonce.0.into(), Payload { msg: plain.as_ref(), aad: &aad.0 }).map_err(|_| Error::Crypto("payload encryption failed".into()))?,
        );
        env.sender_sig = sender.sign(&env.signing_digest(genesis_hash));
        Ok(env)
    }

    /// Structural and authenticity checks that every validator runs. Does not
    /// (and cannot) look inside the ciphertext.
    pub fn verify_structure(&self, genesis_hash: &H256, sender_sign_pk: &Pk32) -> Result<()> {
        if self.version != ENVELOPE_VERSION {
            return Err(Error::InvalidTx(format!("unsupported envelope version {}", self.version)));
        }
        let n = self.stanzas.len();
        if n < MIN_STANZAS || !n.is_power_of_two() || n > MAX_RECIPIENTS.next_power_of_two() {
            return Err(Error::InvalidTx("bad stanza count".into()));
        }
        if self.stanzas.iter().any(|s| s.wrapped_key.0.len() != WRAPPED_KEY_LEN) {
            return Err(Error::InvalidTx("bad stanza length".into()));
        }
        let ct = self.ciphertext.0.len();
        if !(MIN_BUCKET + 16..=MAX_PLAINTEXT + 16).contains(&ct) {
            return Err(Error::InvalidTx("bad ciphertext length".into()));
        }
        verify(sender_sign_pk, &self.signing_digest(genesis_hash), &self.sender_sig).map_err(|_| Error::InvalidTx("envelope sender signature invalid".into()))
    }

    /// Cheap check: is one of the stanzas addressed to this key?
    pub fn is_for(&self, genesis_hash: &H256, keys: &MemberKeys) -> bool {
        self.find_content_key(genesis_hash, keys).is_ok()
    }

    fn find_content_key(&self, genesis_hash: &H256, keys: &MemberKeys) -> Result<Zeroizing<[u8; 32]>> {
        let my_pk = keys.seal_pk();
        let shared = Zeroizing::new(keys.seal_secret().diffie_hellman(&XPublic::from(self.ephemeral_pk.0)).to_bytes());
        let (tag, wrap_key) = derive_stanza_keys(genesis_hash, &shared, &self.id, &self.ephemeral_pk, &my_pk)?;
        let stanza = self.stanzas.iter().find(|s| s.tag == tag).ok_or_else(|| Error::Unauthorized("envelope is not addressed to this key".into()))?;
        let cipher = ChaCha20Poly1305::new(wrap_key.as_ref().into());
        let k = Zeroizing::new(
            cipher
                .decrypt(&[0u8; 12].into(), Payload { msg: &stanza.wrapped_key.0, aad: &self.id.0 })
                .map_err(|_| Error::Crypto("key unwrap failed".into()))?,
        );
        let mut out = Zeroizing::new([0u8; 32]);
        out.copy_from_slice(&k);
        Ok(out)
    }

    /// Open (unwrap) the envelope with the recipient's keys. The caller must
    /// pass the sender's registered signing key so authenticity is checked
    /// before any plaintext is returned.
    pub fn open(&self, genesis_hash: &H256, keys: &MemberKeys, sender_sign_pk: &Pk32) -> Result<Opened> {
        if Address::from_sign_key(sender_sign_pk) != self.sender {
            return Err(Error::Crypto("sender key does not match envelope sender".into()));
        }
        verify(sender_sign_pk, &self.signing_digest(genesis_hash), &self.sender_sig)?;
        let k = self.find_content_key(genesis_hash, keys)?;
        let aad = self.header_digest(genesis_hash);
        let cipher = XChaCha20Poly1305::new(k.as_ref().into());
        let padded = Zeroizing::new(
            cipher
                .decrypt(&self.nonce.0.into(), Payload { msg: &self.ciphertext.0, aad: &aad.0 })
                .map_err(|_| Error::Crypto("payload decryption failed".into()))?,
        );
        let payload: SealedPayload = bincode::deserialize(unpad(&padded)?).map_err(|e| Error::Encoding(format!("payload: {e}")))?;
        Ok(Opened { envelope_id: self.id, sender: self.sender, payload })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(body: &[u8]) -> SealedPayload {
        SealedPayload {
            kind: PayloadKind::Contract,
            title: "Share purchase agreement".into(),
            content_type: "text/plain".into(),
            body: Bytes(body.to_vec()),
            created_at_ms: 1,
            refs: vec![],
            meta: vec![("jurisdiction".into(), "England & Wales".into())],
        }
    }

    #[test]
    fn only_recipients_can_open() {
        let g = H256([9u8; 32]);
        let kevan = MemberKeys::generate();
        let london = MemberKeys::generate();
        let alaska = MemberKeys::generate();
        let outsider = MemberKeys::generate();
        let p = payload(b"terms: 1,000,000 units at par");
        let env = Envelope::seal(&g, &kevan, &[Recipient { seal_pk: london.seal_pk() }, Recipient { seal_pk: alaska.seal_pk() }], &p).unwrap();

        env.verify_structure(&g, &kevan.sign_pk()).unwrap();
        assert_eq!(env.open(&g, &london, &kevan.sign_pk()).unwrap().payload, p);
        assert_eq!(env.open(&g, &alaska, &kevan.sign_pk()).unwrap().payload, p);
        assert!(env.open(&g, &outsider, &kevan.sign_pk()).is_err());
        assert!(env.open(&g, &kevan, &kevan.sign_pk()).is_err(), "sender not listed as recipient");
        assert!(!env.is_for(&g, &outsider));
    }

    #[test]
    fn bound_to_genesis() {
        let g = H256([1u8; 32]);
        let other = H256([2u8; 32]);
        let a = MemberKeys::generate();
        let b = MemberKeys::generate();
        let env = Envelope::seal(&g, &a, &[Recipient { seal_pk: b.seal_pk() }], &payload(b"x")).unwrap();
        assert!(env.open(&other, &b, &a.sign_pk()).is_err());
        assert!(env.verify_structure(&other, &a.sign_pk()).is_err());
    }

    #[test]
    fn tamper_is_detected() {
        let g = H256([3u8; 32]);
        let a = MemberKeys::generate();
        let b = MemberKeys::generate();
        let mut env = Envelope::seal(&g, &a, &[Recipient { seal_pk: b.seal_pk() }], &payload(b"pay 10")).unwrap();
        env.ciphertext.0[5] ^= 1;
        assert!(env.open(&g, &b, &a.sign_pk()).is_err());
        assert!(env.verify_structure(&g, &a.sign_pk()).is_err());
    }

    #[test]
    fn forged_sender_rejected() {
        let g = H256([4u8; 32]);
        let mallory = MemberKeys::generate();
        let kevan = MemberKeys::generate();
        let b = MemberKeys::generate();
        let mut env = Envelope::seal(&g, &mallory, &[Recipient { seal_pk: b.seal_pk() }], &payload(b"x")).unwrap();
        env.sender = kevan.address();
        assert!(env.open(&g, &b, &kevan.sign_pk()).is_err());
    }

    #[test]
    fn sizes_and_counts_are_padded() {
        let g = H256([5u8; 32]);
        let a = MemberKeys::generate();
        let b = MemberKeys::generate();
        let small = Envelope::seal(&g, &a, &[Recipient { seal_pk: b.seal_pk() }], &payload(b"1")).unwrap();
        let bigger = Envelope::seal(&g, &a, &[Recipient { seal_pk: b.seal_pk() }], &payload(b"1000000")).unwrap();
        assert_eq!(small.ciphertext.0.len(), bigger.ciphertext.0.len());
        assert_eq!(small.stanzas.len(), 2);
    }

    #[test]
    fn oversize_rejected() {
        let g = H256([6u8; 32]);
        let a = MemberKeys::generate();
        let b = MemberKeys::generate();
        let huge = vec![0u8; MAX_PLAINTEXT];
        assert!(Envelope::seal(&g, &a, &[Recipient { seal_pk: b.seal_pk() }], &payload(&huge)).is_err());
    }
}
