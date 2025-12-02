//! Domain models representing indexed blockchain data.
//!
//! These models are storage-agnostic and represent the canonical
//! form of indexed data within the domain layer.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// =============================================================================
// 32-byte Hash Types
// =============================================================================

/// Macro to generate 32-byte hash newtypes with common functionality.
///
/// Generates:
/// - `from_hex()` - Parse from hex string (with or without 0x prefix)
/// - `to_hex()` - Convert to 0x-prefixed hex string
/// - `Display` trait implementation
/// - `From<[u8; 32]>` implementation
macro_rules! hash32_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub [u8; 32]);

        impl $name {
            /// Parse from hex string (with or without 0x prefix).
            pub fn from_hex(s: &str) -> Result<Self, hex::FromHexError> {
                let s = s.strip_prefix("0x").unwrap_or(s);
                let bytes = hex::decode(s)?;
                let arr: [u8; 32] = bytes
                    .try_into()
                    .map_err(|_| hex::FromHexError::InvalidStringLength)?;
                Ok(Self(arr))
            }

            /// Convert to 0x-prefixed hex string.
            pub fn to_hex(&self) -> String {
                format!("0x{}", hex::encode(self.0))
            }

            /// Get the inner bytes.
            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.to_hex())
            }
        }

        impl From<[u8; 32]> for $name {
            fn from(bytes: [u8; 32]) -> Self {
                Self(bytes)
            }
        }

        impl AsRef<[u8]> for $name {
            fn as_ref(&self) -> &[u8] {
                &self.0
            }
        }
    };
}

hash32_newtype!(
    /// 32-byte block hash (Blake2-256).
    BlockHash
);

hash32_newtype!(
    /// 32-byte account identifier (SS58 decoded public key).
    AccountId
);

// =============================================================================
// Block Identification
// =============================================================================

/// Unique identifier for a block, combining number and hash for fork safety.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BlockId {
    pub number: u64,
    pub hash: BlockHash,
}

// =============================================================================
// Block & Chain Data
// =============================================================================

/// Indexed block with all relevant metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    /// Block number (height).
    pub number: u64,
    /// Block hash.
    pub hash: BlockHash,
    /// Parent block hash.
    pub parent_hash: BlockHash,
    /// State root after executing this block.
    pub state_root: BlockHash,
    /// Extrinsics root (merkle root of extrinsics).
    pub extrinsics_root: BlockHash,
    /// Block author/validator (if available).
    pub author: Option<AccountId>,
    /// Timestamp from `pallet_timestamp` (if available).
    pub timestamp: Option<DateTime<Utc>>,
    /// Number of extrinsics in this block.
    pub extrinsic_count: u32,
    /// Number of events in this block.
    pub event_count: u32,
    /// When this block was indexed.
    pub indexed_at: DateTime<Utc>,
}

// =============================================================================
// Extrinsics
// =============================================================================

/// Execution status of an extrinsic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtrinsicStatus {
    Success,
    Failed,
}

/// Indexed extrinsic (transaction or inherent).
///
/// An extrinsic is either a signed transaction or an unsigned inherent
/// that was included in a block.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Extrinsic {
    /// Unique identifier: block_number-extrinsic_index.
    pub id: String,
    /// Block number containing this extrinsic.
    pub block_number: u64,
    /// Block hash containing this extrinsic.
    pub block_hash: BlockHash,
    /// Index within the block (0-based).
    pub index: u32,
    /// Pallet name (e.g., "Balances").
    pub pallet: String,
    /// Call name (e.g., "transfer_keep_alive").
    pub call: String,
    /// Signer account (None for unsigned/inherent).
    pub signer: Option<AccountId>,
    /// Execution status.
    pub status: ExtrinsicStatus,
    /// Error message if failed.
    pub error: Option<String>,
    /// Call arguments as JSON.
    pub args: serde_json::Value,
    /// Raw SCALE-encoded bytes (hex).
    pub raw: String,
    /// Tip paid (in smallest unit).
    pub tip: Option<u128>,
    /// Nonce (if signed).
    pub nonce: Option<u32>,
}

// =============================================================================
// Events
// =============================================================================

/// Indexed event emitted during block execution.
///
/// Events are emitted by pallets during extrinsic execution or as
/// part of block initialization/finalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    /// Unique identifier: block_number-event_index.
    pub id: String,
    /// Block number containing this event.
    pub block_number: u64,
    /// Block hash containing this event.
    pub block_hash: BlockHash,
    /// Index within the block (0-based).
    pub index: u32,
    /// Extrinsic index that triggered this event (None for system events).
    pub extrinsic_index: Option<u32>,
    /// Pallet name (e.g., "Balances").
    pub pallet: String,
    /// Event variant name (e.g., "Transfer").
    pub name: String,
    /// Event data as JSON.
    pub data: serde_json::Value,
    /// Topics for filtering (if applicable).
    pub topics: Vec<String>,
}

// =============================================================================
// Indexer State
// =============================================================================

/// Indexer cursor tracking progress.
///
/// The cursor tracks the last successfully indexed block for each chain,
/// enabling the indexer to resume from where it left off.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexerCursor {
    /// Chain identifier (genesis hash or name).
    pub chain_id: String,
    /// Last fully indexed block number.
    pub last_indexed_block: u64,
    /// Last indexed block hash (for reorg detection).
    pub last_indexed_hash: BlockHash,
    /// Last update timestamp.
    pub updated_at: DateTime<Utc>,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_hash_hex_roundtrip() {
        let hex = "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let hash = BlockHash::from_hex(hex).unwrap();
        assert_eq!(hash.to_hex(), hex);
    }

    #[test]
    fn block_hash_without_prefix() {
        let hex = "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let hash = BlockHash::from_hex(hex).unwrap();
        assert_eq!(hash.to_hex(), format!("0x{}", hex));
    }

    #[test]
    fn account_id_hex_roundtrip() {
        let hex = "0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d";
        let account = AccountId::from_hex(hex).unwrap();
        assert_eq!(account.to_hex(), hex);
    }

    #[test]
    fn hash32_from_bytes() {
        let bytes = [0xab; 32];
        let hash = BlockHash::from(bytes);
        assert_eq!(hash.as_bytes(), &bytes);
    }

    #[test]
    fn hash32_invalid_length() {
        let hex = "0x1234"; // Too short
        assert!(BlockHash::from_hex(hex).is_err());
    }
}
