//! Balances pallet GraphQL types for the Maestro blockchain indexer.
//!
//! This crate provides GraphQL schema types for the Substrate Balances pallet.
//!
//! # Example
//!
//! ```rust
//! use maestro_balances_schema::{Transfer, TransferConnection};
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use maestro_graphql_schema::PageInfo;

// =============================================================================
// Types
// =============================================================================

/// A token transfer event from the Balances pallet.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct Transfer {
    /// Unique identifier (block_number-event_index).
    pub id: String,
    /// Block number containing this transfer.
    pub block_number: i64,
    /// Block hash (hex encoded with 0x prefix).
    pub block_hash: String,
    /// Event index within the block.
    pub event_index: i32,
    /// Associated extrinsic index (if any).
    pub extrinsic_index: Option<i32>,
    /// Sender account (hex encoded with 0x prefix).
    #[graphql(name = "from")]
    pub from_account: String,
    /// Recipient account (hex encoded with 0x prefix).
    #[graphql(name = "to")]
    pub to_account: String,
    /// Transfer amount (as string for large numbers).
    pub amount: String,
    /// Whether the transfer succeeded.
    pub success: bool,
    /// Transfer timestamp.
    pub timestamp: Option<DateTime<Utc>>,
}

// =============================================================================
// Connection Types (Relay-style pagination)
// =============================================================================

/// A single transfer in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct TransferEdge {
    /// The transfer.
    pub node: Transfer,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of transfers.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct TransferConnection {
    /// List of transfer edges.
    pub edges: Vec<TransferEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of transfers (if available).
    pub total_count: Option<i64>,
}
