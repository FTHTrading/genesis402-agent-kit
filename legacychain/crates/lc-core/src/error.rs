use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Error {
    #[error("encoding: {0}")]
    Encoding(String),
    #[error("crypto: {0}")]
    Crypto(String),
    #[error("invalid transaction: {0}")]
    InvalidTx(String),
    #[error("invalid block: {0}")]
    InvalidBlock(String),
    #[error("invalid genesis: {0}")]
    InvalidGenesis(String),
    #[error("not authorized: {0}")]
    Unauthorized(String),
    #[error("payment: {0}")]
    Payment(String),
}

pub type Result<T> = std::result::Result<T, Error>;
