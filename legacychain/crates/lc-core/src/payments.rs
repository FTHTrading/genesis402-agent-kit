//! Real-money ingress: x402 v2 and MPP (`Payment` HTTP auth scheme).
//!
//! A deposit into LegacyChain is an HTTP 402 exchange. The node quotes the
//! deposit as a USDC-on-Base payment to the treasury, the depositor signs an
//! EIP-3009 `transferWithAuthorization`, the node verifies it locally, a
//! facilitator settles it on Base, and only then does the bridge mint.
//!
//! Both dialects carry a server-issued, HMAC-protected challenge naming the
//! LegacyChain recipient and amount. The challenge is echoed back with the
//! payment, so a captured payment can never be redirected to another account.

use crate::crypto::{Eip712Domain, TransferAuthorization};
use crate::error::{Error, Result};
use crate::genesis::Genesis;
use crate::types::{Address, Bytes, EvmAddress, H256};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::Sha256;

pub const X402_VERSION: u32 = 2;
pub const MPP_METHOD: &str = "usdc";
pub const MPP_INTENT: &str = "charge";
/// Seconds a quote stays valid.
pub const QUOTE_TTL_SECS: u64 = 600;

/// The bound deposit order a challenge commits to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepositOrder {
    pub to: Address,
    pub amount: u64,
    pub expires: u64,
    pub mac: String,
}

impl DepositOrder {
    fn mac_for(secret: &[u8], genesis_hash: &H256, to: &Address, amount: u64, expires: u64) -> String {
        let mut m = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("hmac accepts any key length");
        m.update(b"lc/deposit-order/v1");
        m.update(&genesis_hash.0);
        m.update(&to.0);
        m.update(&amount.to_be_bytes());
        m.update(&expires.to_be_bytes());
        hex::encode(m.finalize().into_bytes())
    }

    pub fn new(secret: &[u8], genesis_hash: &H256, to: Address, amount: u64, now_secs: u64) -> Self {
        let expires = now_secs + QUOTE_TTL_SECS;
        let mac = Self::mac_for(secret, genesis_hash, &to, amount, expires);
        DepositOrder { to, amount, expires, mac }
    }

    pub fn check(&self, secret: &[u8], genesis_hash: &H256, now_secs: u64) -> Result<()> {
        let want = Self::mac_for(secret, genesis_hash, &self.to, self.amount, self.expires);
        let a = want.as_bytes();
        let b = self.mac.as_bytes();
        let same = a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0;
        if !same {
            return Err(Error::Payment("deposit challenge was not issued by this node".into()));
        }
        if now_secs > self.expires {
            return Err(Error::Payment("deposit quote expired; request a new one".into()));
        }
        Ok(())
    }
}

/// x402 v2 `PaymentRequirements` for the `exact` EVM scheme.
pub fn x402_requirements(g: &Genesis, order: &DepositOrder) -> Value {
    json!({
        "scheme": "exact",
        "network": g.bridge.network,
        "amount": order.amount.to_string(),
        "asset": g.bridge.usdc.to_string(),
        "payTo": g.bridge.pay_to.to_string(),
        "maxTimeoutSeconds": QUOTE_TTL_SECS,
        "extra": {
            "name": g.bridge.usdc_eip712_name,
            "version": g.bridge.usdc_eip712_version,
            "legacychain": order,
        }
    })
}

/// Full x402 v2 `PaymentRequired` body (base64 goes in `PAYMENT-REQUIRED`).
pub fn x402_payment_required(g: &Genesis, order: &DepositOrder, resource_url: &str) -> Value {
    json!({
        "x402Version": X402_VERSION,
        "error": "payment required",
        "resource": {
            "url": resource_url,
            "description": format!("Deposit {} {} into {} for {}", crate::types::format_amount(order.amount), g.asset.symbol, g.chain_id, order.to),
            "mimeType": "application/json",
        },
        "accepts": [x402_requirements(g, order)],
    })
}

pub fn b64(v: &Value) -> String {
    STANDARD.encode(serde_json::to_vec(v).expect("json"))
}

pub fn b64url(v: &Value) -> String {
    URL_SAFE_NO_PAD.encode(serde_json::to_vec(v).expect("json"))
}

fn decode_json(s: &str) -> Result<Value> {
    let s = s.trim();
    let raw = STANDARD
        .decode(s)
        .or_else(|_| URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')))
        .map_err(|e| Error::Payment(format!("payment header is not base64: {e}")))?;
    serde_json::from_slice(&raw).map_err(|e| Error::Payment(format!("payment header is not JSON: {e}")))
}

/// MPP challenge: the `WWW-Authenticate: Payment ...` header value.
pub fn mpp_challenge(g: &Genesis, order: &DepositOrder, realm: &str) -> String {
    let request = json!({
        "amount": order.amount.to_string(),
        "currency": g.bridge.usdc.to_string(),
        "recipient": g.bridge.pay_to.to_string(),
        "network": g.bridge.network,
        "eip712": { "name": g.bridge.usdc_eip712_name, "version": g.bridge.usdc_eip712_version, "chainId": g.bridge.evm_chain_id },
        "legacychain": order,
    });
    format!(
        "Payment id=\"{}\", realm=\"{}\", method=\"{}\", intent=\"{}\", request=\"{}\", expires=\"{}\"",
        &order.mac[..32],
        realm,
        MPP_METHOD,
        MPP_INTENT,
        b64url(&request),
        order.expires
    )
}

/// A verified payment, ready to hand to a facilitator for settlement.
#[derive(Clone, Debug)]
pub struct VerifiedPayment {
    pub dialect: &'static str,
    pub order: DepositOrder,
    pub authorization: TransferAuthorization,
    pub signature: Bytes,
    /// x402 v2 `PaymentPayload` (normalized for the facilitator).
    pub payment_payload: Value,
    /// x402 v2 `PaymentRequirements` the payment satisfies.
    pub requirements: Value,
}

fn field_str<'a>(v: &'a Value, k: &str) -> Result<&'a str> {
    v.get(k).and_then(|x| x.as_str()).ok_or_else(|| Error::Payment(format!("missing field '{k}'")))
}

fn parse_authorization(exact: &Value) -> Result<(TransferAuthorization, Bytes)> {
    let auth = exact.get("authorization").ok_or_else(|| Error::Payment("missing authorization".into()))?;
    let num = |k: &str| -> Result<u128> {
        let v = auth.get(k).ok_or_else(|| Error::Payment(format!("missing authorization.{k}")))?;
        match v {
            Value::String(s) => s.parse().map_err(|_| Error::Payment(format!("authorization.{k} is not an integer"))),
            Value::Number(n) => n.as_u64().map(|x| x as u128).ok_or_else(|| Error::Payment(format!("authorization.{k} is not an integer"))),
            _ => Err(Error::Payment(format!("authorization.{k} is not an integer"))),
        }
    };
    let a = TransferAuthorization {
        from: field_str(auth, "from")?.parse()?,
        to: field_str(auth, "to")?.parse()?,
        value: num("value")?,
        valid_after: u64::try_from(num("validAfter")?).map_err(|_| Error::Payment("validAfter out of range".into()))?,
        valid_before: u64::try_from(num("validBefore")?).map_err(|_| Error::Payment("validBefore out of range".into()))?,
        nonce: field_str(auth, "nonce")?.trim_start_matches("0x").parse()?,
    };
    let sig_hex = field_str(exact, "signature")?;
    let sig = Bytes(hex::decode(sig_hex.trim_start_matches("0x")).map_err(|_| Error::Payment("signature is not hex".into()))?);
    Ok((a, sig))
}

fn check_authorization(g: &Genesis, domain: &Eip712Domain, order: &DepositOrder, a: &TransferAuthorization, sig: &Bytes, now_secs: u64) -> Result<()> {
    if a.to != g.bridge.pay_to {
        return Err(Error::Payment(format!("authorization pays {}, treasury is {}", a.to, g.bridge.pay_to)));
    }
    if a.value != order.amount as u128 {
        return Err(Error::Payment(format!("authorization value {} != quoted {}", a.value, order.amount)));
    }
    if a.valid_after > now_secs {
        return Err(Error::Payment("authorization not yet valid".into()));
    }
    if a.valid_before <= now_secs + 6 {
        return Err(Error::Payment("authorization expires too soon to settle".into()));
    }
    if a.from == EvmAddress([0u8; 20]) {
        return Err(Error::Payment("payer missing".into()));
    }
    a.verify_signature(domain, &sig.0)
}

fn order_from(v: Option<&Value>) -> Result<DepositOrder> {
    let v = v.ok_or_else(|| Error::Payment("payment does not echo the LegacyChain deposit challenge".into()))?;
    serde_json::from_value(v.clone()).map_err(|e| Error::Payment(format!("bad deposit challenge: {e}")))
}

/// Verify an x402 v2 `PAYMENT-SIGNATURE` / `X-PAYMENT` header.
pub fn verify_x402(g: &Genesis, genesis_hash: &H256, secret: &[u8], header: &str, now_secs: u64) -> Result<VerifiedPayment> {
    let p = decode_json(header)?;
    let version = p.get("x402Version").and_then(|v| v.as_u64()).unwrap_or(0);
    if version != X402_VERSION as u64 {
        return Err(Error::Payment(format!("x402Version {version} unsupported; use 2")));
    }
    let accepted = p.get("accepted").ok_or_else(|| Error::Payment("missing 'accepted' requirements".into()))?;
    let order = order_from(accepted.get("extra").and_then(|e| e.get("legacychain")))?;
    order.check(secret, genesis_hash, now_secs)?;
    let want = x402_requirements(g, &order);
    for k in ["scheme", "network", "amount", "payTo"] {
        if accepted.get(k) != want.get(k) {
            return Err(Error::Payment(format!("accepted.{k} does not match the quote")));
        }
    }
    let asset_ok = accepted.get("asset").and_then(|a| a.as_str()).map(|a| a.eq_ignore_ascii_case(&g.bridge.usdc.to_string())).unwrap_or(false);
    if !asset_ok {
        return Err(Error::Payment("accepted.asset is not the bridge USDC".into()));
    }
    let exact = p.get("payload").ok_or_else(|| Error::Payment("missing payload".into()))?;
    let (authorization, signature) = parse_authorization(exact)?;
    check_authorization(g, &g.eip712_domain(), &order, &authorization, &signature, now_secs)?;
    Ok(VerifiedPayment { dialect: "x402", order, authorization, signature, payment_payload: p.clone(), requirements: want })
}

/// Verify an MPP `Authorization: Payment <credential>` header value (the part
/// after the scheme name).
pub fn verify_mpp(g: &Genesis, genesis_hash: &H256, secret: &[u8], credential: &str, now_secs: u64) -> Result<VerifiedPayment> {
    let c = decode_json(credential)?;
    let challenge = c.get("challenge").ok_or_else(|| Error::Payment("credential missing challenge".into()))?;
    if challenge.get("method").and_then(|m| m.as_str()) != Some(MPP_METHOD) || challenge.get("intent").and_then(|m| m.as_str()) != Some(MPP_INTENT) {
        return Err(Error::Payment("unsupported MPP method or intent".into()));
    }
    let request = decode_json(field_str(challenge, "request")?)?;
    let order = order_from(request.get("legacychain"))?;
    order.check(secret, genesis_hash, now_secs)?;
    if challenge.get("id").and_then(|v| v.as_str()) != Some(&order.mac[..32]) {
        return Err(Error::Payment("challenge id does not match".into()));
    }
    let exact = c.get("payload").ok_or_else(|| Error::Payment("credential missing payload".into()))?;
    let (authorization, signature) = parse_authorization(exact)?;
    check_authorization(g, &g.eip712_domain(), &order, &authorization, &signature, now_secs)?;
    let requirements = x402_requirements(g, &order);
    // Normalize into an x402 v2 PaymentPayload so one facilitator settles both dialects.
    let payment_payload = json!({
        "x402Version": X402_VERSION,
        "accepted": requirements,
        "payload": exact,
    });
    Ok(VerifiedPayment { dialect: "mpp", order, authorization, signature, payment_payload, requirements })
}

/// `Payment-Receipt` header value for a settled MPP payment.
pub fn mpp_receipt(reference: &str, now_secs: u64) -> String {
    b64url(&json!({ "status": "success", "method": MPP_METHOD, "timestamp": now_secs, "reference": reference }))
}

/// Client side: build the x402 v2 `PaymentPayload` header for a quote.
pub fn build_x402_payment(requirements: &Value, resource: Option<&Value>, auth: &TransferAuthorization, sig65: &[u8]) -> String {
    let mut p = json!({
        "x402Version": X402_VERSION,
        "accepted": requirements,
        "payload": {
            "signature": format!("0x{}", hex::encode(sig65)),
            "authorization": {
                "from": auth.from.to_string(),
                "to": auth.to.to_string(),
                "value": auth.value.to_string(),
                "validAfter": auth.valid_after.to_string(),
                "validBefore": auth.valid_before.to_string(),
                "nonce": format!("0x{}", auth.nonce),
            }
        }
    });
    if let Some(r) = resource {
        p["resource"] = r.clone();
    }
    b64(&p)
}
