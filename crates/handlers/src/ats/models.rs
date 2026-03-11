//! Models for the ATS (Allfeat Timestamp) pallet.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use maestro_core::models::AccountId;

/// An ATS work registered on-chain.
///
/// Represents a timestamped creative work with ownership tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsWork {
    /// Unique ATS identifier (on-chain AtsId).
    pub id: u64,
    /// Current owner account.
    pub owner: AccountId,
    /// Block number when the ATS was first registered.
    pub created_at_block: u64,
    /// Timestamp when the ATS was first registered.
    pub created_at_timestamp: Option<DateTime<Utc>>,
    /// Latest version number for this ATS.
    pub latest_version: u32,
}

/// A version of an ATS work.
///
/// Each ATS can have multiple versions, each with its own commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsVersion {
    /// Unique identifier: "{ats_id}-{version}".
    pub id: String,
    /// Parent ATS identifier.
    pub ats_id: u64,
    /// Version number (starts at 1).
    pub version: u32,
    /// Commitment for this version (32 bytes).
    pub commitment: [u8; 32],
    /// Protocol version for this ATS version.
    pub protocol_version: u8,
    /// Block number when this version was registered.
    pub registered_at_block: u64,
    /// Timestamp when this version was registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// Event index within the block.
    pub event_index: u32,
}
