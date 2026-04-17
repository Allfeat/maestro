//! Extension trait for SQLx error handling with context.
//!
//! Every `.query_err()` now captures both the human-readable context and
//! the original `sqlx::Error` as a `#[source]`. Log output keeps the old
//! format (because [`StorageError`]'s `Display` only renders `context`),
//! but `err.source()` now walks down to the sqlx metadata — pool state,
//! SQLSTATE codes, row counts etc.

use maestro_core::error::StorageError;

/// Extension trait for adding context to SQLx Results.
///
/// Instead of writing:
/// ```ignore
/// .await.map_err(|e| StorageError::query_with_source("get block", e))?;
/// ```
///
/// You can write:
/// ```ignore
/// .await.query_err("get block")?;
/// ```
pub trait SqlxResultExt<T> {
    /// Convert a SQLx error into a `StorageError::QueryError` with context.
    fn query_err(self, context: &str) -> Result<T, StorageError>;

    /// Convert a SQLx error into a `StorageError::TransactionError` with context.
    fn tx_err(self, context: &str) -> Result<T, StorageError>;

    /// Convert a SQLx error into a `StorageError::ConnectionError` with context.
    fn conn_err(self, context: &str) -> Result<T, StorageError>;
}

impl<T> SqlxResultExt<T> for Result<T, sqlx::Error> {
    fn query_err(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|e| StorageError::query_with_source(format!("{context}: {e}"), e))
    }

    fn tx_err(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|e| StorageError::transaction_with_source(format!("{context}: {e}"), e))
    }

    fn conn_err(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|e| StorageError::connection_with_source(format!("{context}: {e}"), e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn test_query_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.query_err("fetch block");

        match result {
            Err(e @ StorageError::QueryError { .. }) => {
                let msg = e.to_string();
                assert!(msg.starts_with("Query execution error: fetch block:"));
                assert!(msg.contains("no rows"));
                assert!(e.source().is_some(), "sqlx source must be preserved");
            }
            _ => panic!("Expected QueryError"),
        }
    }

    #[test]
    fn test_tx_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.tx_err("begin transaction");

        match result {
            Err(e @ StorageError::TransactionError { .. }) => {
                let msg = e.to_string();
                assert!(msg.starts_with("Transaction error: begin transaction:"));
                assert!(e.source().is_some());
            }
            _ => panic!("Expected TransactionError"),
        }
    }

    #[test]
    fn test_conn_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.conn_err("connect to database");

        match result {
            Err(e @ StorageError::ConnectionError { .. }) => {
                let msg = e.to_string();
                assert!(msg.starts_with("Database connection error: connect to database:"));
                assert!(e.source().is_some());
            }
            _ => panic!("Expected ConnectionError"),
        }
    }

    #[test]
    fn test_ok_result_passes_through() {
        let ok: Result<i32, sqlx::Error> = Ok(42);
        assert_eq!(ok.query_err("unused context").unwrap(), 42);
    }

    #[test]
    fn test_ok_result_passes_through_tx() {
        let ok: Result<String, sqlx::Error> = Ok("value".to_string());
        assert_eq!(ok.tx_err("unused context").unwrap(), "value");
    }

    #[test]
    fn test_context_preserved_in_error() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.query_err("insert block row #123");

        if let Err(e @ StorageError::QueryError { .. }) = result {
            assert!(e.to_string().contains("insert block row #123"));
        } else {
            panic!("Expected QueryError with context");
        }
    }
}
