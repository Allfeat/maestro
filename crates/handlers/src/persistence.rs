//! Helpers for persisting handler outputs.
//!
//! This module provides utilities to reduce boilerplate when persisting
//! entities in handler `on_block_end` implementations.
//!
//! # Note on async_trait compatibility
//!
//! Due to lifetime constraints with `async_trait` (which boxes futures with `'static`
//! lifetime), these helpers cannot be directly used with storage trait methods that
//! take references. They are designed for use with:
//! - Concrete storage implementations (not trait objects)
//! - Non-async-trait contexts
//! - Future refactoring if storage traits are changed to avoid boxing

use maestro_core::error::{DomainResult, StorageError};
use std::future::Future;
use tracing::warn;

/// Persist a collection of entities with error handling and logging.
///
/// This helper reduces boilerplate for the common pattern of:
/// - Iterating over entities
/// - Calling an async storage operation for each
/// - Logging and returning on first error
///
/// # Arguments
/// * `entities` - The entities to persist
/// * `operation` - An async function that takes an entity reference and returns a Result
/// * `context` - Description for error logging (e.g., "ATS work")
/// * `block_number` - The current block number for logging
///
/// # Example
/// ```ignore
/// let works: Vec<AtsWork> = outputs.get_typed("ats", "works");
/// persist_each(&works, |w| self.storage.insert_ats_work(w), "ATS work", block.number).await?;
/// ```
pub async fn persist_each<T, F, Fut>(
    entities: &[T],
    operation: F,
    context: &str,
    block_number: u64,
) -> DomainResult<()>
where
    F: Fn(&T) -> Fut,
    Fut: Future<Output = Result<(), StorageError>>,
{
    for entity in entities {
        if let Err(e) = operation(entity).await {
            warn!(
                block = block_number,
                error = ?e,
                "Failed to persist {}", context
            );
            return Err(e.into());
        }
    }
    Ok(())
}

/// Persist a batch of entities in a single operation.
///
/// Use this when the storage layer supports batch inserts.
///
/// # Arguments
/// * `entities` - The entities to persist
/// * `operation` - An async function that takes a slice and returns a Result
/// * `context` - Description for error logging
/// * `block_number` - The current block number for logging
///
/// # Example
/// ```ignore
/// let transfers: Vec<Transfer> = outputs.get_typed("balances", "transfers");
/// persist_batch(&transfers, |t| self.storage.insert_transfers(t), "transfers", block.number).await?;
/// ```
pub async fn persist_batch<T, F, Fut>(
    entities: &[T],
    operation: F,
    context: &str,
    block_number: u64,
) -> DomainResult<()>
where
    F: FnOnce(&[T]) -> Fut,
    Fut: Future<Output = Result<(), StorageError>>,
{
    if entities.is_empty() {
        return Ok(());
    }

    if let Err(e) = operation(entities).await {
        warn!(
            block = block_number,
            count = entities.len(),
            error = ?e,
            "Failed to persist {}", context
        );
        return Err(e.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn test_persist_each_empty() {
        let entities: Vec<i32> = vec![];
        let result = persist_each(&entities, |_| async { Ok(()) }, "test", 100).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_persist_each_success() {
        let counter = Arc::new(AtomicUsize::new(0));
        let entities = vec![1, 2, 3];

        let counter_clone = counter.clone();
        let result = persist_each(
            &entities,
            |_| {
                let c = counter_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            "numbers",
            100,
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(counter.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_persist_each_stops_on_error() {
        let counter = Arc::new(AtomicUsize::new(0));
        let entities = vec![1, 2, 3];

        let result = persist_each(
            &entities,
            |n| {
                let c = counter.clone();
                let val = *n;
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    if val == 2 {
                        Err(StorageError::QueryError("test error".to_string()))
                    } else {
                        Ok(())
                    }
                }
            },
            "numbers",
            100,
        )
        .await;

        assert!(result.is_err());
        // Should have processed 1 and 2, then stopped
        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn test_persist_batch_empty() {
        let entities: Vec<i32> = vec![];
        let called = Arc::new(AtomicUsize::new(0));
        let called_clone = called.clone();

        let result = persist_batch(
            &entities,
            |_| {
                let c = called_clone.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            },
            "test",
            100,
        )
        .await;

        assert!(result.is_ok());
        // Should not have called the operation for empty slice
        assert_eq!(called.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn test_persist_batch_success() {
        let entities = vec![1, 2, 3];

        let result = persist_batch(
            &entities,
            |slice| {
                let len = slice.len();
                async move {
                    assert_eq!(len, 3);
                    Ok(())
                }
            },
            "numbers",
            100,
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_persist_batch_error() {
        let entities = vec![1, 2, 3];

        let result = persist_batch(
            &entities,
            |_| async { Err(StorageError::QueryError("batch failed".to_string())) },
            "numbers",
            100,
        )
        .await;

        assert!(result.is_err());
    }
}
