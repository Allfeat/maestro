//! ATS (Allfeat Timestamp) pallet GraphQL types for the Maestro blockchain indexer.
//!
//! This crate provides GraphQL schema types for the Allfeat ATS pallet,
//! which handles timestamping and versioning of digital works.
//!
//! # Example
//!
//! ```rust
//! use maestro_ats_schema::{AtsWork, AtsVersion};
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
    /// Commitment for this version (hex encoded with 0x prefix).
    pub commitment: String,
    /// Protocol version for this ATS version.
    pub protocol_version: i32,
    /// Block number when this version was registered.
    pub registered_at_block: i64,
    /// Timestamp when this version was registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// Event index within the block.
    pub event_index: i32,
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
