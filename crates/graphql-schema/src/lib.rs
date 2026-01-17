//! Core GraphQL types for the Maestro blockchain indexer.
//!
//! This crate provides the GraphQL schema types used by the Maestro indexer.
//! These types can be used by external consumers to build typed GraphQL clients.
//!
//! # Example
//!
//! ```rust
//! use maestro_graphql_schema::{Block, Event, PageInfo, Order};
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// Ordering
// =============================================================================

/// Ordering direction for queries.
#[derive(
    async_graphql::Enum, Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize,
)]
pub enum Order {
    /// Descending order (newest first).
    #[default]
    Desc,
    /// Ascending order (oldest first).
    Asc,
}

// =============================================================================
// Pagination
// =============================================================================

/// Page information for Relay-style cursor pagination.
#[derive(async_graphql::SimpleObject, Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageInfo {
    /// Whether there are more items after the current page.
    pub has_next_page: bool,
    /// Whether there are items before the current page.
    pub has_previous_page: bool,
    /// Cursor pointing to the first item in the current page.
    pub start_cursor: Option<String>,
    /// Cursor pointing to the last item in the current page.
    pub end_cursor: Option<String>,
}

// =============================================================================
// Core Types
// =============================================================================

/// Indexer status and statistics.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexerStatus {
    /// The latest indexed block number.
    pub latest_indexed_block: Option<u64>,
    /// When the indexer was last updated.
    pub last_updated: Option<DateTime<Utc>>,
}

/// A blockchain block.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    /// Block number (height).
    pub number: i64,
    /// Block hash (hex encoded with 0x prefix).
    pub hash: String,
    /// Parent block hash (hex encoded with 0x prefix).
    pub parent_hash: String,
    /// State root hash (hex encoded with 0x prefix).
    pub state_root: String,
    /// Extrinsics root hash (hex encoded with 0x prefix).
    pub extrinsics_root: String,
    /// Block author/validator (hex encoded with 0x prefix).
    pub author: Option<String>,
    /// Block timestamp.
    pub timestamp: Option<DateTime<Utc>>,
    /// Number of extrinsics in this block.
    pub extrinsic_count: i32,
    /// Number of events in this block.
    pub event_count: i32,
    /// When this block was indexed.
    pub indexed_at: DateTime<Utc>,
}

/// A blockchain extrinsic (transaction).
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extrinsic {
    /// Unique identifier (block_number-index).
    pub id: String,
    /// Block number containing this extrinsic.
    pub block_number: i64,
    /// Block hash (hex encoded with 0x prefix).
    pub block_hash: String,
    /// Index within the block.
    pub index: i32,
    /// Pallet name.
    pub pallet: String,
    /// Call name.
    pub call: String,
    /// Signer account (hex encoded with 0x prefix).
    pub signer: Option<String>,
    /// Whether the extrinsic succeeded.
    pub success: bool,
    /// Error message if failed.
    pub error: Option<String>,
    /// Call arguments as JSON.
    pub args: serde_json::Value,
    /// Tip amount (as string for large numbers).
    pub tip: Option<String>,
    /// Signer nonce.
    pub nonce: Option<i32>,
}

/// A blockchain event.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Unique identifier (block_number-index).
    pub id: String,
    /// Block number containing this event.
    pub block_number: i64,
    /// Block hash (hex encoded with 0x prefix).
    pub block_hash: String,
    /// Event index within the block.
    pub index: i32,
    /// Associated extrinsic index (if any).
    pub extrinsic_index: Option<i32>,
    /// Pallet name.
    pub pallet: String,
    /// Event name.
    pub name: String,
    /// Event data as JSON.
    pub data: serde_json::Value,
}

// =============================================================================
// Connection Types (Relay-style pagination)
// =============================================================================

/// A single block in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockEdge {
    /// The block.
    pub node: Block,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of blocks.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockConnection {
    /// List of block edges.
    pub edges: Vec<BlockEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of blocks (if available).
    pub total_count: Option<i64>,
}

/// A single extrinsic in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtrinsicEdge {
    /// The extrinsic.
    pub node: Extrinsic,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of extrinsics.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtrinsicConnection {
    /// List of extrinsic edges.
    pub edges: Vec<ExtrinsicEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of extrinsics (if available).
    pub total_count: Option<i64>,
}

/// A single event in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventEdge {
    /// The event.
    pub node: Event,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of events.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventConnection {
    /// List of event edges.
    pub edges: Vec<EventEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of events (if available).
    pub total_count: Option<i64>,
}
