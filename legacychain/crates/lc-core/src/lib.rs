//! LegacyChain core.
//!
//! A permissioned, zero-fee ledger whose members are identified by Ed25519 /
//! X25519 keys (optionally bound to ERC-8004 agent identities on Base), whose
//! settlement unit is redeemable 1:1 for USDC on Base via x402 / MPP, and
//! whose payloads (messages, documents, contracts, RWA terms, agent tasks)
//! travel as sealed envelopes only their recipients can open.

pub mod auth;
pub mod block;
pub mod crypto;
pub mod envelope;
pub mod error;
pub mod genesis;
pub mod keyfile;
pub mod payments;
pub mod state;
pub mod tx;
pub mod types;

pub use error::{Error, Result};
