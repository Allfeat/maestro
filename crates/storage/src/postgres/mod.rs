//! PostgreSQL storage adapter.
//!
//! This module implements the repository traits defined in `maestro-core`
//! using PostgreSQL as the backing store.
//!
//! # Architecture
//!
//! - [`Database`] - Connection pool and migrations
//! - [`PgRepositories`] - Composite repository implementing `Repositories` trait
//! - Individual repos: `PgBlockRepository`, `PgExtrinsicRepository`, etc.
//!
//! # Usage
//!
//! ```ignore
//! let config = DatabaseConfig::for_indexer(&database_url);
//! let db = Database::connect(&config).await?;
//! db.migrate().await?;
//!
//! let repositories = PgRepositories::new(Arc::new(db));
//! ```

mod block_repo;
mod cursor_repo;
mod database;
mod event_repo;
mod extrinsic_repo;
mod helpers;

pub use block_repo::PgBlockRepository;
pub use cursor_repo::PgCursorRepository;
pub use database::{Database, DatabaseConfig, PurgeStats};
pub use event_repo::PgEventRepository;
pub use extrinsic_repo::PgExtrinsicRepository;

use std::sync::Arc;

use async_trait::async_trait;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::ExtrinsicStatus;
use maestro_core::ports::{
    BlockData, BlockRepository, CursorRepository, EventRepository, ExtrinsicRepository,
    Repositories,
};

// =============================================================================
// Composite Repository
// =============================================================================

/// Aggregated PostgreSQL repositories implementing the `Repositories` trait.
///
/// This provides a single entry point for all storage operations and
/// implements atomic transactions that span multiple tables.
pub struct PgRepositories {
    db: Arc<Database>,
    blocks: PgBlockRepository,
    extrinsics: PgExtrinsicRepository,
    events: PgEventRepository,
    cursor: PgCursorRepository,
}

impl PgRepositories {
    /// Create a new repository aggregate from a database connection.
    pub fn new(db: Arc<Database>) -> Self {
        let pool = db.pool().clone();
        Self {
            blocks: PgBlockRepository::new(&db),
            extrinsics: PgExtrinsicRepository::new(pool.clone()),
            events: PgEventRepository::new(pool.clone()),
            cursor: PgCursorRepository::new(&db),
            db,
        }
    }
}

#[async_trait]
impl Repositories for PgRepositories {
    fn blocks(&self) -> &dyn BlockRepository {
        &self.blocks
    }

    fn extrinsics(&self) -> &dyn ExtrinsicRepository {
        &self.extrinsics
    }

    fn events(&self) -> &dyn EventRepository {
        &self.events
    }

    fn cursor(&self) -> &dyn CursorRepository {
        &self.cursor
    }

    async fn persist_block_atomic(&self, data: BlockData<'_>) -> StorageResult<()> {
        let mut tx = self
            .db
            .pool()
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        // Insert block
        sqlx::query(
            r#"
            INSERT INTO blocks (
                number, hash, parent_hash, state_root, extrinsics_root,
                author, timestamp, extrinsic_count, event_count, indexed_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (number) DO UPDATE SET
                hash = EXCLUDED.hash,
                parent_hash = EXCLUDED.parent_hash,
                state_root = EXCLUDED.state_root,
                extrinsics_root = EXCLUDED.extrinsics_root,
                author = EXCLUDED.author,
                timestamp = EXCLUDED.timestamp,
                extrinsic_count = EXCLUDED.extrinsic_count,
                event_count = EXCLUDED.event_count,
                indexed_at = EXCLUDED.indexed_at
            "#,
        )
        .bind(data.block.number as i64)
        .bind(&data.block.hash.0[..])
        .bind(&data.block.parent_hash.0[..])
        .bind(&data.block.state_root.0[..])
        .bind(&data.block.extrinsics_root.0[..])
        .bind(data.block.author.as_ref().map(|a| &a.0[..]))
        .bind(data.block.timestamp)
        .bind(data.block.extrinsic_count as i32)
        .bind(data.block.event_count as i32)
        .bind(data.block.indexed_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        // Insert extrinsics
        for ext in data.extrinsics {
            sqlx::query(
                r#"
                INSERT INTO extrinsics (
                    id, block_number, block_hash, index, pallet, call,
                    signer, status, error, args, raw, tip, nonce
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12::NUMERIC, $13)
                ON CONFLICT (block_number, index) DO NOTHING
                "#,
            )
            .bind(&ext.id)
            .bind(ext.block_number as i64)
            .bind(&ext.block_hash.0[..])
            .bind(ext.index as i32)
            .bind(&ext.pallet)
            .bind(&ext.call)
            .bind(ext.signer.as_ref().map(|s| &s.0[..]))
            .bind(status_to_str(ext.status))
            .bind(&ext.error)
            .bind(&ext.args)
            .bind(&ext.raw)
            .bind(ext.tip.map(|t| t.to_string()))
            .bind(ext.nonce.map(|n| n as i32))
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        // Insert events
        for event in data.events {
            sqlx::query(
                r#"
                INSERT INTO events (
                    id, block_number, block_hash, index, extrinsic_index,
                    pallet, name, data, topics
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (block_number, index) DO NOTHING
                "#,
            )
            .bind(&event.id)
            .bind(event.block_number as i64)
            .bind(&event.block_hash.0[..])
            .bind(event.index as i32)
            .bind(event.extrinsic_index.map(|i| i as i32))
            .bind(&event.pallet)
            .bind(&event.name)
            .bind(&event.data)
            .bind(&event.topics)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        // Update cursor
        sqlx::query(
            r#"
            INSERT INTO indexer_cursor (chain_id, last_indexed_block, last_indexed_hash, updated_at)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (chain_id) DO UPDATE SET
                last_indexed_block = EXCLUDED.last_indexed_block,
                last_indexed_hash = EXCLUDED.last_indexed_hash,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&data.cursor.chain_id)
        .bind(data.cursor.last_indexed_block as i64)
        .bind(&data.cursor.last_indexed_hash.0[..])
        .bind(data.cursor.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(())
    }

    async fn delete_from_block_atomic(
        &self,
        from_number: u64,
        chain_id: &str,
    ) -> StorageResult<u64> {
        let mut tx = self
            .db
            .pool()
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        // Delete events first (child data)
        sqlx::query("DELETE FROM events WHERE block_number >= $1")
            .bind(from_number as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        // Delete extrinsics
        sqlx::query("DELETE FROM extrinsics WHERE block_number >= $1")
            .bind(from_number as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        // Delete blocks
        let result = sqlx::query("DELETE FROM blocks WHERE number >= $1")
            .bind(from_number as i64)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let blocks_deleted = result.rows_affected();

        // Update cursor
        if from_number > 0 {
            // Try to point cursor to previous block
            let prev_block: Option<(Vec<u8>,)> =
                sqlx::query_as("SELECT hash FROM blocks WHERE number = $1")
                    .bind((from_number - 1) as i64)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e| StorageError::QueryError(e.to_string()))?;

            if let Some((hash,)) = prev_block {
                sqlx::query(
                    r#"
                    UPDATE indexer_cursor
                    SET last_indexed_block = $1, last_indexed_hash = $2, updated_at = NOW()
                    WHERE chain_id = $3
                    "#,
                )
                .bind((from_number - 1) as i64)
                .bind(&hash[..])
                .bind(chain_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?;
            } else {
                // No previous block, delete cursor
                sqlx::query("DELETE FROM indexer_cursor WHERE chain_id = $1")
                    .bind(chain_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| StorageError::QueryError(e.to_string()))?;
            }
        } else {
            // Deleting from genesis, clear cursor
            sqlx::query("DELETE FROM indexer_cursor WHERE chain_id = $1")
                .bind(chain_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(blocks_deleted)
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn status_to_str(status: ExtrinsicStatus) -> &'static str {
    match status {
        ExtrinsicStatus::Success => "success",
        ExtrinsicStatus::Failed => "failed",
    }
}
