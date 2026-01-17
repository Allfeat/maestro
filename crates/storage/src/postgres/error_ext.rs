//! Extension traits for SQLx error handling with context.
//!
//! Provides concise error mapping for database operations while preserving
//! meaningful context about what operation failed.

use maestro_core::error::StorageError;

/// Extension trait for adding context to SQLx Results.
///
/// Instead of writing:
/// ```ignore
/// .await.map_err(|e| StorageError::QueryError(e.to_string()))?;
/// ```
///
/// You can write:
/// ```ignore
/// .await.query_err("get block by number")?;
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
        self.map_err(|e| StorageError::QueryError(format!("{}: {}", context, e)))
    }

    fn tx_err(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|e| StorageError::TransactionError(format!("{}: {}", context, e)))
    }

    fn conn_err(self, context: &str) -> Result<T, StorageError> {
        self.map_err(|e| StorageError::ConnectionError(format!("{}: {}", context, e)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.query_err("fetch block");

        match result {
            Err(StorageError::QueryError(msg)) => {
                assert!(msg.starts_with("fetch block:"));
                assert!(msg.contains("no rows"));
            }
            _ => panic!("Expected QueryError"),
        }
    }

    #[test]
    fn test_tx_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.tx_err("begin transaction");

        match result {
            Err(StorageError::TransactionError(msg)) => {
                assert!(msg.starts_with("begin transaction:"));
            }
            _ => panic!("Expected TransactionError"),
        }
    }

    #[test]
    fn test_conn_err_formats_message() {
        let err: Result<(), sqlx::Error> = Err(sqlx::Error::RowNotFound);
        let result = err.conn_err("connect to database");

        match result {
            Err(StorageError::ConnectionError(msg)) => {
                assert!(msg.starts_with("connect to database:"));
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

        if let Err(StorageError::QueryError(msg)) = result {
            assert!(msg.contains("insert block row #123"));
        } else {
            panic!("Expected QueryError with context");
        }
    }
}
