//! Error types for the indexer domain layer.
//!
//! This module defines a hierarchy of error types:
//!
//! - [`DomainError`] - Business logic errors
//! - [`StorageError`] - Database/repository errors
//! - [`ChainError`] - Blockchain RPC errors
//! - [`IndexerError`] - Top-level orchestration errors
//!
//! Each String-wrapping variant carries an optional `#[source]` of the
//! underlying error (sqlx, subxt, serde…). `to_string()` remains stable
//! (only the context string is rendered), and `err.source()` walks the
//! chain for callers that need to inspect the original cause.

use thiserror::Error;

/// Boxed trait object used to carry originating errors across hexagonal
/// boundaries without pulling `sqlx` / `subxt` into the `core` crate.
pub type BoxedSource = Box<dyn std::error::Error + Send + Sync + 'static>;

// =============================================================================
// Domain Errors
// =============================================================================

/// Business logic and domain rule violations.
#[derive(Debug, Error)]
pub enum DomainError {
    /// Data decoding/deserialization failed.
    #[error("Decoding error: {message}")]
    DecodingError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Generic validation error.
    #[error("Validation error: {message}")]
    ValidationError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Storage operation failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl DomainError {
    pub fn decoding(message: impl Into<String>) -> Self {
        Self::DecodingError {
            message: message.into(),
            source: None,
        }
    }

    pub fn decoding_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::DecodingError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::ValidationError {
            message: message.into(),
            source: None,
        }
    }
}

// =============================================================================
// Storage Errors
// =============================================================================

/// Database and repository errors.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Failed to establish database connection.
    #[error("Database connection error: {message}")]
    ConnectionError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// SQL query execution failed.
    #[error("Query execution error: {message}")]
    QueryError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Database migration failed.
    #[error("Migration error: {message}")]
    MigrationError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Transaction commit/rollback failed.
    #[error("Transaction error: {message}")]
    TransactionError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Data serialization/deserialization failed.
    #[error("Serialization error: {message}")]
    SerializationError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// `persist_block_atomic` was given a block whose number neither extends the
    /// cursor range upward nor fills its downward boundary.
    #[error("Cursor gap violation: block {block} does not extend range [{first}, {last}]")]
    CursorGapViolation { block: u64, first: u64, last: u64 },
}

impl StorageError {
    pub fn connection(message: impl Into<String>) -> Self {
        Self::ConnectionError {
            message: message.into(),
            source: None,
        }
    }

    pub fn query(message: impl Into<String>) -> Self {
        Self::QueryError {
            message: message.into(),
            source: None,
        }
    }

    pub fn query_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::QueryError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn migration(message: impl Into<String>) -> Self {
        Self::MigrationError {
            message: message.into(),
            source: None,
        }
    }

    pub fn migration_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::MigrationError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn transaction_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::TransactionError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn connection_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::ConnectionError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn serialization(message: impl Into<String>) -> Self {
        Self::SerializationError {
            message: message.into(),
            source: None,
        }
    }

    pub fn serialization_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::SerializationError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

// =============================================================================
// Chain Errors
// =============================================================================

/// Blockchain RPC and connectivity errors.
#[derive(Debug, Error)]
pub enum ChainError {
    /// WebSocket connection failed.
    #[error("Connection failed: {message}")]
    ConnectionFailed {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// RPC request failed.
    #[error("RPC error: {message}")]
    RpcError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Block subscription failed or disconnected.
    #[error("Subscription error: {message}")]
    SubscriptionError {
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },

    /// Block could not be fetched.
    #[error("Block fetch error at hash {hash}: {message}")]
    BlockFetchError {
        /// Block hash that failed to fetch.
        hash: String,
        /// Error details.
        message: String,
        #[source]
        source: Option<BoxedSource>,
    },
}

impl ChainError {
    pub fn rpc(message: impl Into<String>) -> Self {
        Self::RpcError {
            message: message.into(),
            source: None,
        }
    }

    pub fn rpc_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::RpcError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn connection_failed(message: impl Into<String>) -> Self {
        Self::ConnectionFailed {
            message: message.into(),
            source: None,
        }
    }

    pub fn connection_failed_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::ConnectionFailed {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn subscription(message: impl Into<String>) -> Self {
        Self::SubscriptionError {
            message: message.into(),
            source: None,
        }
    }

    pub fn subscription_with_source<E>(message: impl Into<String>, source: E) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::SubscriptionError {
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn block_fetch_with_source<E>(
        hash: impl Into<String>,
        message: impl Into<String>,
        source: E,
    ) -> Self
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        Self::BlockFetchError {
            hash: hash.into(),
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }
}

// =============================================================================
// Indexer Errors
// =============================================================================

/// Top-level indexer orchestration errors.
///
/// This is the main error type returned by [`crate::services::IndexerService`].
/// It wraps all lower-level errors and adds indexer-specific variants.
#[derive(Debug, Error)]
pub enum IndexerError {
    /// Domain logic error.
    #[error(transparent)]
    Domain(#[from] DomainError),

    /// Storage/database error.
    #[error(transparent)]
    Storage(#[from] StorageError),

    /// Blockchain connectivity error.
    #[error(transparent)]
    Chain(#[from] ChainError),

    /// Invalid configuration.
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Connected chain doesn't match stored data.
    ///
    /// This is a fatal error that requires manual intervention.
    #[error("Chain mismatch: connected to {connected} but database contains data for {expected}")]
    ChainMismatch {
        /// Genesis hash of connected chain.
        connected: String,
        /// Genesis hash expected by database.
        expected: String,
    },

    /// Graceful shutdown was requested.
    ///
    /// This is not really an error but uses the error type for control flow.
    #[error("Indexer shutdown requested")]
    ShutdownRequested,

    /// Unexpected internal error.
    #[error("Internal error: {0}")]
    Internal(String),

    /// Historical backfill aborted because retry budget for a block was exhausted.
    #[error("Backfill aborted at block {block}: {reason}")]
    BackfillAborted { block: u64, reason: String },

    /// `--start-block` is below the chain's earliest V14 metadata block.
    #[error(
        "Cannot backfill below block {earliest_v14} on this chain — its runtime \
         uses pre-V14 metadata, which Maestro does not decode. Re-run with \
         --start-block {earliest_v14} (or higher), or use --live-only to skip \
         backfill entirely."
    )]
    PreV14BlockRequested { requested: u64, earliest_v14: u64 },
}

// =============================================================================
// Result Type Aliases
// =============================================================================

/// Result type for indexer operations.
pub type IndexerResult<T> = Result<T, IndexerError>;

/// Result type for domain operations.
pub type DomainResult<T> = Result<T, DomainError>;

/// Result type for storage operations.
pub type StorageResult<T> = Result<T, StorageError>;

/// Result type for chain operations.
pub type ChainResult<T> = Result<T, ChainError>;

#[cfg(test)]
mod tests {
    use super::*;

    // Test critique: la chaîne de conversion d'erreurs fonctionne
    // Permet d'utiliser ? à travers les couches
    #[test]
    fn test_error_conversion_chain() {
        // Storage -> Domain -> Indexer
        let storage_err = StorageError::query("db failed");
        let domain_err: DomainError = storage_err.into();
        let indexer_err: IndexerError = domain_err.into();

        // Le message original est préservé
        assert!(indexer_err.to_string().contains("db failed"));

        // Chain -> Indexer
        let chain_err = ChainError::rpc("rpc failed");
        let indexer_err: IndexerError = chain_err.into();
        assert!(indexer_err.to_string().contains("rpc failed"));
    }

    // Test critique: ChainMismatch contient les infos de debug nécessaires
    #[test]
    fn test_chain_mismatch_includes_hashes() {
        let err = IndexerError::ChainMismatch {
            connected: "0xaaa".into(),
            expected: "0xbbb".into(),
        };
        let msg = err.to_string();
        // Les deux hashes doivent être visibles pour le debug
        assert!(msg.contains("0xaaa") && msg.contains("0xbbb"));
    }

    #[test]
    fn test_backfill_aborted_display_contains_block_and_reason() {
        let err = IndexerError::BackfillAborted {
            block: 12345,
            reason: "websocket disconnected".into(),
        };
        let msg = err.to_string();
        assert!(msg.contains("12345"));
        assert!(msg.contains("websocket disconnected"));
    }

    #[test]
    fn test_pre_v14_block_requested_display_names_both_knobs() {
        let err = IndexerError::PreV14BlockRequested {
            requested: 1000,
            earliest_v14: 473291,
        };
        let msg = err.to_string();
        assert!(msg.contains("473291"));
        assert!(msg.contains("--start-block"));
        assert!(msg.contains("--live-only"));
    }

    #[test]
    fn test_cursor_gap_violation_display_contains_range() {
        let err = StorageError::CursorGapViolation {
            block: 999,
            first: 100,
            last: 200,
        };
        let msg = err.to_string();
        assert!(msg.contains("999"));
        assert!(msg.contains("100"));
        assert!(msg.contains("200"));
    }

    // New: source() chain walks through wrapped errors.
    #[test]
    fn test_query_error_source_carries_underlying() {
        use std::error::Error;

        #[derive(Debug)]
        struct FakeSqlxErr;
        impl std::fmt::Display for FakeSqlxErr {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "rollback failed: connection lost")
            }
        }
        impl std::error::Error for FakeSqlxErr {}

        let err = StorageError::query_with_source("insert block", FakeSqlxErr);
        assert!(err.to_string().contains("insert block"));
        let src = err.source().expect("source should be present");
        assert!(src.to_string().contains("rollback failed"));
    }
}
