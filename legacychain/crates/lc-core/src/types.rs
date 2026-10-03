//! Primitive value types shared by every LegacyChain component.
//!
//! All fixed-size byte values serialize as lowercase hex strings in JSON and as
//! the same strings under bincode, so the signed encoding is identical no
//! matter which side produced it.

use crate::error::{Error, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

macro_rules! hex_bytes {
    ($name:ident, $len:expr, $prefix:expr) => {
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub [u8; $len]);

        impl $name {
            pub const LEN: usize = $len;

            pub fn from_slice(b: &[u8]) -> Result<Self> {
                let arr: [u8; $len] = b.try_into().map_err(|_| Error::Encoding(format!("{} must be {} bytes", stringify!($name), $len)))?;
                Ok(Self(arr))
            }

            pub fn as_bytes(&self) -> &[u8; $len] {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, hex::encode(self.0))
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(s: &str) -> Result<Self> {
                let body = s.strip_prefix($prefix).unwrap_or(s);
                let raw = hex::decode(body.to_ascii_lowercase()).map_err(|e| Error::Encoding(format!("{}: {e}", stringify!($name))))?;
                Self::from_slice(&raw)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
                s.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
                let s = String::deserialize(d)?;
                s.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

hex_bytes!(H256, 32, "");
hex_bytes!(Pk32, 32, "");
hex_bytes!(Sig64, 64, "");
hex_bytes!(Tag16, 16, "");
hex_bytes!(Nonce24, 24, "");
hex_bytes!(EvmAddress, 20, "0x");

/// Variable-length bytes, hex in JSON.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Bytes(pub Vec<u8>);

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({} bytes)", self.0.len())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let body = s.strip_prefix("0x").unwrap_or(&s);
        hex::decode(body).map(Bytes).map_err(serde::de::Error::custom)
    }
}

/// A LegacyChain account address: the first 20 bytes of
/// `sha256("lc/addr/v1" || ed25519_public_key)`, rendered `lc` + 40 hex.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Address(pub [u8; 20]);

impl Address {
    pub fn from_sign_key(pk: &Pk32) -> Self {
        let mut h = Sha256::new();
        h.update(b"lc/addr/v1");
        h.update(pk.0);
        let d = h.finalize();
        let mut a = [0u8; 20];
        a.copy_from_slice(&d[..20]);
        Address(a)
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lc{}", hex::encode(self.0))
    }
}

impl fmt::Debug for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl FromStr for Address {
    type Err = Error;
    fn from_str(s: &str) -> Result<Self> {
        let body = s.strip_prefix("lc").ok_or_else(|| Error::Encoding("address must start with 'lc'".into()))?;
        let raw = hex::decode(body.to_ascii_lowercase()).map_err(|e| Error::Encoding(format!("address: {e}")))?;
        let arr: [u8; 20] = raw.as_slice().try_into().map_err(|_| Error::Encoding("address must be 20 bytes".into()))?;
        Ok(Address(arr))
    }
}

impl Serialize for Address {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Address {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Settlement unit decimals. One LUSD atomic unit equals one USDC atomic unit.
pub const DECIMALS: u32 = 6;
const UNIT: u64 = 10u64.pow(DECIMALS);

/// Parse a decimal string ("250", "250.5", "0.000001") into atomic units.
/// Rejects more than six fractional digits instead of rounding.
pub fn parse_amount(s: &str) -> Result<u64> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('-') || s.starts_with('+') {
        return Err(Error::Encoding(format!("invalid amount '{s}'")));
    }
    let (whole, frac) = match s.split_once('.') {
        Some((w, f)) => (w, f),
        None => (s, ""),
    };
    if (whole.is_empty() && frac.is_empty()) || !whole.chars().all(|c| c.is_ascii_digit()) || !frac.chars().all(|c| c.is_ascii_digit()) {
        return Err(Error::Encoding(format!("invalid amount '{s}'")));
    }
    if frac.len() > DECIMALS as usize {
        return Err(Error::Encoding(format!("amount '{s}' has more than {DECIMALS} decimals")));
    }
    let w: u64 = if whole.is_empty() { 0 } else { whole.parse().map_err(|_| Error::Encoding(format!("amount '{s}' too large")))? };
    let mut f_str = frac.to_string();
    while f_str.len() < DECIMALS as usize {
        f_str.push('0');
    }
    let f: u64 = f_str.parse().unwrap_or(0);
    w.checked_mul(UNIT).and_then(|v| v.checked_add(f)).ok_or_else(|| Error::Encoding(format!("amount '{s}' too large")))
}

/// Render atomic units as a fixed six-decimal string.
pub fn format_amount(atomic: u64) -> String {
    format!("{}.{:06}", atomic / UNIT, atomic % UNIT)
}

/// SHA-256 with a domain tag, used for every hash that is signed or compared.
pub fn tagged_hash(tag: &[u8], parts: &[&[u8]]) -> H256 {
    let mut h = Sha256::new();
    h.update((tag.len() as u32).to_le_bytes());
    h.update(tag);
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    H256(h.finalize().into())
}

/// Canonical binary encoding for anything that is hashed or signed.
pub fn canonical<T: Serialize>(v: &T) -> Vec<u8> {
    bincode::serialize(v).expect("bincode serialization of in-memory value cannot fail")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_round_trip() {
        assert_eq!(parse_amount("250").unwrap(), 250_000_000);
        assert_eq!(parse_amount("250.5").unwrap(), 250_500_000);
        assert_eq!(parse_amount("0.000001").unwrap(), 1);
        assert_eq!(parse_amount(".25").unwrap(), 250_000);
        assert_eq!(format_amount(250_500_000), "250.500000");
        assert!(parse_amount("1.0000001").is_err());
        assert!(parse_amount("-1").is_err());
        assert!(parse_amount("1e5").is_err());
        assert!(parse_amount(".").is_err());
        assert!(parse_amount("99999999999999999999").is_err());
    }

    #[test]
    fn address_text_round_trip() {
        let a = Address::from_sign_key(&Pk32([7u8; 32]));
        let s = a.to_string();
        assert!(s.starts_with("lc") && s.len() == 42);
        assert_eq!(s.parse::<Address>().unwrap(), a);
        let j = serde_json::to_string(&a).unwrap();
        assert_eq!(serde_json::from_str::<Address>(&j).unwrap(), a);
    }

    #[test]
    fn tagged_hash_is_length_prefixed() {
        assert_ne!(tagged_hash(b"t", &[b"ab", b"c"]), tagged_hash(b"t", &[b"a", b"bc"]));
    }
}
