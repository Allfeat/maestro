//! Error types for the indexer domain layer.
//!
//! This module defines a hierarchy of error types:
//!
//! - [`DomainError`] - Business logic errors
//! - [`StorageError`] - Database/repository errors
//! - [`ChainError`] - Blockchain RPC errors
//! - [`IndexerError`] - Top-level orchestration errors
//!
//! Error conversion is automatic via `From` implementations,
//! allowing `?` to work across error boundaries.

use thiserror::Error;

// =============================================================================
// Domain Errors
// =============================================================================

/// Business logic and domain rule violations.
///
/// These errors represent problems in the indexer's domain logic,
/// such as data validation failures or missing required data.
#[derive(Debug, Error)]
pub enum DomainError {
    /// Block was not found in storage.
    #[error("Block not found: {0}")]
    BlockNotFound(u64),

    /// Block hash failed validation.
    #[error("Invalid block hash: {0}")]
    InvalidBlockHash(String),

    /// Account ID failed validation.
    #[error("Invalid account ID: {0}")]
    InvalidAccountId(String),

    /// Data decoding/deserialization failed.
    #[error("Decoding error: {0}")]
    DecodingError(String),

    /// Chain reorganization was detected.
    #[error("Chain reorg detected at block {0}")]
    ReorgDetected(u64),

    /// No handler registered for a pallet.
    #[error("Handler not found for pallet: {0}")]
    HandlerNotFound(String),

    /// Generic validation error.
    #[error("Validation error: {0}")]
    ValidationError(String),

    /// Storage operation failed.
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),
}

// =============================================================================
// Storage Errors
// =============================================================================

/// Database and repository errors.
///
/// These errors originate from storage operations like queries,
/// transactions, and data serialization.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Failed to establish database connection.
    #[error("Database connection error: {0}")]
    ConnectionError(String),

    /// SQL query execution failed.
    #[error("Query execution error: {0}")]
    QueryError(String),

    /// Requested record was not found.
    #[error("Record not found: {0}")]
    NotFound(String),

    /// Database constraint was violated (unique, foreign key, etc.).
    #[error("Constraint violation: {0}")]
    ConstraintViolation(String),

    /// Database migration failed.
    #[error("Migration error: {0}")]
    MigrationError(String),

    /// Transaction commit/rollback failed.
    #[error("Transaction error: {0}")]
    TransactionError(String),

    /// Data serialization/deserialization failed.
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

// =============================================================================
// Chain Errors
// =============================================================================

/// Blockchain RPC and connectivity errors.
///
/// These errors occur when communicating with the Substrate node
/// via WebSocket RPC.
#[derive(Debug, Error)]
pub enum ChainError {
    /// WebSocket connection failed.
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    /// RPC request failed.
    #[error("RPC error: {0}")]
    RpcError(String),

    /// Block subscription failed or disconnected.
    #[error("Subscription error: {0}")]
    SubscriptionError(String),

    /// Runtime metadata could not be fetched or parsed.
    #[error("Metadata error: {0}")]
    MetadataError(String),

    /// Block could not be fetched.
    #[error("Block fetch error at hash {hash}: {message}")]
    BlockFetchError {
        /// Block hash that failed to fetch.
        hash: String,
        /// Error details.
        message: String,
    },

    /// Operation timed out waiting for block.
    #[error("Timeout waiting for block {0}")]
    Timeout(u64),
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
    #[error("Domain error: {0}")]
    Domain(#[from] DomainError),

    /// Storage/database error.
    #[error("Storage error: {0}")]
    Storage(#[from] StorageError),

    /// Blockchain connectivity error.
    #[error("Chain error: {0}")]
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

    /// Indexer is already running.
    #[error("Indexer already running")]
    AlreadyRunning,

    /// Graceful shutdown was requested.
    ///
    /// This is not really an error but uses the error type for control flow.
    #[error("Indexer shutdown requested")]
    ShutdownRequested,

    /// Unexpected internal error.
    #[error("Internal error: {0}")]
    Internal(String),
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
        let storage_err = StorageError::QueryError("db failed".into());
        let domain_err: DomainError = storage_err.into();
        let indexer_err: IndexerError = domain_err.into();

        // Le message original est préservé
        assert!(indexer_err.to_string().contains("db failed"));

        // Chain -> Indexer
        let chain_err = ChainError::RpcError("rpc failed".into());
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
}
