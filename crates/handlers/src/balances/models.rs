//! Models for the Balances pallet.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use maestro_core::models::{AccountId, BlockHash};

/// A token transfer between two accounts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    /// Unique identifier: block_number-event_index.
    pub id: String,
    /// Block number containing this transfer.
    pub block_number: u64,
    /// Block hash containing this transfer.
    pub block_hash: BlockHash,
    /// Event index within the block.
    pub event_index: u32,
    /// Extrinsic index that triggered this transfer (if any).
    pub extrinsic_index: Option<u32>,
    /// Sender account.
    pub from: AccountId,
    /// Recipient account.
    pub to: AccountId,
    /// Amount transferred (in smallest unit).
    pub amount: u128,
    /// Whether the transfer was successful.
    pub success: bool,
    /// Block timestamp (if available).
    pub timestamp: Option<DateTime<Utc>>,
}
