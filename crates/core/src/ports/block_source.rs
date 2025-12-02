//! Port trait for blockchain data source.
//!
//! This trait defines the interface for fetching blocks and subscribing
//! to new blocks from a Substrate chain. Implementations live in the
//! infrastructure layer (e.g., `maestro-substrate`).

use async_trait::async_trait;
use futures::Stream;
use std::pin::Pin;

use crate::error::ChainResult;
use crate::models::BlockHash;

/// Raw block data from the chain before domain transformation.
#[derive(Debug, Clone)]
pub struct RawBlock {
    /// Block number.
    pub number: u64,
    /// Block hash.
    pub hash: [u8; 32],
    /// Parent hash.
    pub parent_hash: [u8; 32],
    /// State root.
    pub state_root: [u8; 32],
    /// Extrinsics root.
    pub extrinsics_root: [u8; 32],
    /// Raw extrinsics (SCALE encoded).
    pub extrinsics: Vec<RawExtrinsic>,
    /// Raw events (SCALE encoded).
    pub events: Vec<RawEvent>,
    /// Block timestamp (from Timestamp pallet).
    pub timestamp: Option<u64>,
}

/// Raw extrinsic data.
#[derive(Debug, Clone)]
pub struct RawExtrinsic {
    /// Index in block.
    pub index: u32,
    /// SCALE-encoded bytes.
    pub bytes: Vec<u8>,
    /// Decoded pallet name.
    pub pallet: String,
    /// Decoded call name.
    pub call: String,
    /// Signer (if signed).
    pub signer: Option<[u8; 32]>,
    /// Arguments as JSON.
    pub args: serde_json::Value,
    /// Success flag.
    pub success: bool,
    /// Error info if failed.
    pub error: Option<String>,
    /// Tip.
    pub tip: Option<u128>,
    /// Nonce.
    pub nonce: Option<u32>,
}

/// Raw event data.
#[derive(Debug, Clone)]
pub struct RawEvent {
    /// Index in block.
    pub index: u32,
    /// Extrinsic index (if applicable).
    pub extrinsic_index: Option<u32>,
    /// Pallet name.
    pub pallet: String,
    /// Event variant name.
    pub name: String,
    /// Event data as JSON.
    pub data: serde_json::Value,
    /// Event topics.
    pub topics: Vec<[u8; 32]>,
}

/// Notification when a new block is finalized.
#[derive(Debug, Clone)]
pub struct FinalizedHead {
    pub number: u64,
    pub hash: [u8; 32],
}

/// Stream of finalized block.
pub type FinalizedBlockStream = Pin<Box<dyn Stream<Item = ChainResult<RawBlock>> + Send>>;

/// Port trait for blockchain data source.
///
/// Designed for chain head indexing only (no historical block support in v1).
#[async_trait]
pub trait BlockSource: Send + Sync {
    /// Get the genesis hash of the connected chain.
    async fn genesis_hash(&self) -> ChainResult<BlockHash>;

    /// Get the current finalized block head.
    async fn finalized_head(&self) -> ChainResult<FinalizedHead>;

    /// Subscribe to finalized block.
    ///
    /// This is the primary method for chain head indexing.
    async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream>;

    /// Get current runtime version.
    async fn runtime_version(&self) -> ChainResult<u32>;
}
