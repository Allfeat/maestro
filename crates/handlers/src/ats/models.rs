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
/// Each ATS can have multiple versions, each with its own hash commitment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsVersion {
    /// Unique identifier: "{ats_id}-{version}".
    pub id: String,
    /// Parent ATS identifier.
    pub ats_id: u64,
    /// Version number (starts at 1).
    pub version: u32,
    /// Hash commitment for this version (32 bytes).
    pub hash_commitment: [u8; 32],
    /// Block number when this version was registered.
    pub registered_at_block: u64,
    /// Timestamp when this version was registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// Event index within the block.
    pub event_index: u32,
}

/// An ownership transfer (claim) of an ATS.
///
/// Records when an ATS ownership changes via ZKP claim.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsOwnershipTransfer {
    /// Unique identifier: "{block_number}-{event_index}".
    pub id: String,
    /// ATS identifier that was transferred.
    pub ats_id: u64,
    /// Previous owner account.
    pub old_owner: AccountId,
    /// New owner account.
    pub new_owner: AccountId,
    /// Block number of the transfer.
    pub block_number: u64,
    /// Event index within the block.
    pub event_index: u32,
    /// Timestamp of the transfer.
    pub timestamp: Option<DateTime<Utc>>,
}

/// A verification key update event.
///
/// Records changes to the ZKP verification key used for claims.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AtsVerificationKeyUpdate {
    /// Unique identifier: "{block_number}-{event_index}".
    pub id: String,
    /// The new verification key bytes.
    pub vk: Vec<u8>,
    /// Block number when the key was updated.
    pub block_number: u64,
    /// Event index within the block.
    pub event_index: u32,
    /// Timestamp of the update.
    pub timestamp: Option<DateTime<Utc>>,
}
