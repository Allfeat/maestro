//! ATS (Allfeat Timestamp) pallet GraphQL types for the Maestro blockchain indexer.
//!
//! This crate provides GraphQL schema types for the Allfeat ATS pallet,
//! which handles timestamping and versioning of digital works.
//!
//! # Example
//!
//! ```rust
//! use maestro_ats_schema::{AtsWork, AtsVersion, AtsOwnershipTransfer};
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use maestro_graphql_schema::PageInfo;

// =============================================================================
// Core Types
// =============================================================================

/// An ATS (Allfeat Timestamp) work registered on-chain.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsWork {
    /// Unique ATS identifier.
    pub id: i64,
    /// Current owner account (hex encoded with 0x prefix).
    pub owner: String,
    /// Block number when the ATS was first registered.
    pub created_at_block: i64,
    /// Timestamp when the ATS was first registered.
    pub created_at_timestamp: Option<DateTime<Utc>>,
    /// Latest version number for this ATS.
    pub latest_version: i32,
}

/// A version of an ATS work.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsVersion {
    /// Unique identifier: "{ats_id}-{version}".
    pub id: String,
    /// Parent ATS identifier.
    pub ats_id: i64,
    /// Version number.
    pub version: i32,
    /// Hash commitment for this version (hex encoded with 0x prefix).
    pub hash_commitment: String,
    /// Block number when this version was registered.
    pub registered_at_block: i64,
    /// Timestamp when this version was registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// Event index within the block.
    pub event_index: i32,
}

/// An ownership transfer (claim) of an ATS.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsOwnershipTransfer {
    /// Unique identifier.
    pub id: String,
    /// ATS identifier that was transferred.
    pub ats_id: i64,
    /// Previous owner account (hex encoded with 0x prefix).
    pub old_owner: String,
    /// New owner account (hex encoded with 0x prefix).
    pub new_owner: String,
    /// Block number of the transfer.
    pub block_number: i64,
    /// Event index within the block.
    pub event_index: i32,
    /// Timestamp of the transfer.
    pub timestamp: Option<DateTime<Utc>>,
}

/// A verification key update event.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsVerificationKeyUpdate {
    /// Unique identifier.
    pub id: String,
    /// The verification key (hex encoded with 0x prefix).
    pub vk: String,
    /// Block number when the key was updated.
    pub block_number: i64,
    /// Event index within the block.
    pub event_index: i32,
    /// Timestamp of the update.
    pub timestamp: Option<DateTime<Utc>>,
}

/// Statistics about ATS on-chain data.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsStats {
    /// Total number of ATS works registered.
    pub total_ats_count: i64,
    /// Number of ATS works owned by a specific account (if queried).
    pub owner_ats_count: Option<i64>,
}

// =============================================================================
// Connection Types (Relay-style pagination)
// =============================================================================

/// A single ATS work in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsWorkEdge {
    /// The ATS work.
    pub node: AtsWork,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of ATS works.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsWorkConnection {
    /// List of ATS work edges.
    pub edges: Vec<AtsWorkEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of ATS works (if available).
    pub total_count: Option<i64>,
}

/// A single ownership transfer in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsOwnershipTransferEdge {
    /// The ownership transfer.
    pub node: AtsOwnershipTransfer,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of ownership transfers.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AtsOwnershipTransferConnection {
    /// List of ownership transfer edges.
    pub edges: Vec<AtsOwnershipTransferEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of transfers (if available).
    pub total_count: Option<i64>,
}
