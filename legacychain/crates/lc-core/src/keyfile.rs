//! On-disk identity files.
//!
//! `<handle>.key.json`    secret key file, optionally passphrase-encrypted
//!                         (Argon2id -> XChaCha20-Poly1305).
//! `<handle>.member.json` public member record, safe to share; goes into the
//!                         genesis or an AdmitMember transaction.

use crate::crypto::{random_bytes, MemberKeys};
use crate::error::{Error, Result};
use crate::genesis::{Member, Role};
use crate::types::{Address, Bytes, Nonce24, Pk32};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretBox {
    Plain { sign_seed: Bytes, seal_secret: Bytes },
    Argon2idXchacha20poly1305 { salt: Bytes, m_cost_kib: u32, t_cost: u32, p_cost: u32, nonce: Nonce24, ciphertext: Bytes },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct KeyFile {
    pub version: u8,
    pub handle: String,
    pub address: Address,
    pub sign_pk: Pk32,
    pub seal_pk: Pk32,
    pub secret: SecretBox,
}

fn kdf(pass: &str, salt: &[u8], m: u32, t: u32, p: u32) -> Result<Zeroizing<[u8; 32]>> {
    let params = Params::new(m, t, p, Some(32)).map_err(|e| Error::Crypto(format!("argon2 params: {e}")))?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(pass.as_bytes(), salt, out.as_mut())
        .map_err(|e| Error::Crypto(format!("argon2: {e}")))?;
    Ok(out)
}

impl KeyFile {
    pub fn new(handle: &str, keys: &MemberKeys, passphrase: Option<&str>) -> Result<Self> {
        let secret = match passphrase {
            None => SecretBox::Plain { sign_seed: Bytes(keys.sign_seed().to_vec()), seal_secret: Bytes(keys.seal_secret_bytes().to_vec()) },
            Some(pass) => {
                let salt = random_bytes::<16>();
                let (m, t, p) = (64 * 1024, 3, 1);
                let k = kdf(pass, &salt, m, t, p)?;
                let nonce = Nonce24(random_bytes::<24>());
                let mut plain = Zeroizing::new(Vec::with_capacity(64));
                plain.extend_from_slice(keys.sign_seed());
                plain.extend_from_slice(keys.seal_secret_bytes());
                let ct = XChaCha20Poly1305::new(k.as_ref().into())
                    .encrypt(&nonce.0.into(), Payload { msg: &plain, aad: handle.as_bytes() })
                    .map_err(|_| Error::Crypto("key encryption failed".into()))?;
                SecretBox::Argon2idXchacha20poly1305 { salt: Bytes(salt.to_vec()), m_cost_kib: m, t_cost: t, p_cost: p, nonce, ciphertext: Bytes(ct) }
            }
        };
        Ok(KeyFile { version: 1, handle: handle.to_string(), address: keys.address(), sign_pk: keys.sign_pk(), seal_pk: keys.seal_pk(), secret })
    }

    pub fn is_encrypted(&self) -> bool {
        matches!(self.secret, SecretBox::Argon2idXchacha20poly1305 { .. })
    }

    pub fn unlock(&self, passphrase: Option<&str>) -> Result<MemberKeys> {
        let (sign, seal) = match &self.secret {
            SecretBox::Plain { sign_seed, seal_secret } => {
                let s: [u8; 32] = sign_seed.0.as_slice().try_into().map_err(|_| Error::Encoding("sign_seed".into()))?;
                let x: [u8; 32] = seal_secret.0.as_slice().try_into().map_err(|_| Error::Encoding("seal_secret".into()))?;
                (s, x)
            }
            SecretBox::Argon2idXchacha20poly1305 { salt, m_cost_kib, t_cost, p_cost, nonce, ciphertext } => {
                let pass = passphrase.ok_or_else(|| Error::Crypto("key file is encrypted; set LC_PASSPHRASE".into()))?;
                let k = kdf(pass, &salt.0, *m_cost_kib, *t_cost, *p_cost)?;
                let plain = Zeroizing::new(
                    XChaCha20Poly1305::new(k.as_ref().into())
                        .decrypt(&nonce.0.into(), Payload { msg: &ciphertext.0, aad: self.handle.as_bytes() })
                        .map_err(|_| Error::Crypto("wrong passphrase or corrupted key file".into()))?,
                );
                if plain.len() != 64 {
                    return Err(Error::Crypto("corrupted key file".into()));
                }
                let mut s = [0u8; 32];
                let mut x = [0u8; 32];
                s.copy_from_slice(&plain[..32]);
                x.copy_from_slice(&plain[32..]);
                (s, x)
            }
        };
        let keys = MemberKeys::from_secrets(sign, seal);
        if keys.address() != self.address || keys.seal_pk() != self.seal_pk {
            return Err(Error::Crypto("key file public keys do not match its secrets".into()));
        }
        Ok(keys)
    }

    pub fn member(&self, roles: BTreeSet<Role>) -> Member {
        Member { handle: self.handle.clone(), address: self.address, sign_pk: self.sign_pk, seal_pk: self.seal_pk, roles, erc8004: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_and_encrypted_round_trip() {
        let k = MemberKeys::generate();
        let plain = KeyFile::new("kevan", &k, None).unwrap();
        assert_eq!(plain.unlock(None).unwrap().address(), k.address());

        let enc = KeyFile::new("kevan", &k, Some("correct horse")).unwrap();
        let json = serde_json::to_string(&enc).unwrap();
        let back: KeyFile = serde_json::from_str(&json).unwrap();
        assert!(back.unlock(None).is_err());
        assert!(back.unlock(Some("wrong")).is_err());
        assert_eq!(back.unlock(Some("correct horse")).unwrap().seal_pk(), k.seal_pk());
    }
}
