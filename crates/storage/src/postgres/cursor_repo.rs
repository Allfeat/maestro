//! Cursor repository implementation for PostgreSQL.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::{BlockHash, IndexerCursor};
use maestro_core::ports::CursorRepository;

use super::database::Database;
use super::helpers::bytes_to_hash32;

/// PostgreSQL implementation of CursorRepository.
pub struct PgCursorRepository {
    pool: PgPool,
}

impl PgCursorRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl CursorRepository for PgCursorRepository {
    async fn get_cursor(&self, chain_id: &str) -> StorageResult<Option<IndexerCursor>> {
        let row = sqlx::query_as::<_, CursorRow>(
            r#"
            SELECT chain_id, last_indexed_block, last_indexed_hash, updated_at
            FROM indexer_cursor
            WHERE chain_id = $1
            "#,
        )
        .bind(chain_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(CursorRow::into_cursor).transpose()
    }

    async fn get_any_cursor(&self) -> StorageResult<Option<IndexerCursor>> {
        let row = sqlx::query_as::<_, CursorRow>(
            r#"
            SELECT chain_id, last_indexed_block, last_indexed_hash, updated_at
            FROM indexer_cursor
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(CursorRow::into_cursor).transpose()
    }

    async fn set_cursor(&self, cursor: &IndexerCursor) -> StorageResult<()> {
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
        .bind(&cursor.chain_id)
        .bind(cursor.last_indexed_block as i64)
        .bind(&cursor.last_indexed_hash.0[..])
        .bind(cursor.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct CursorRow {
    chain_id: String,
    last_indexed_block: i64,
    last_indexed_hash: Vec<u8>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl CursorRow {
    fn into_cursor(self) -> StorageResult<IndexerCursor> {
        Ok(IndexerCursor {
            chain_id: self.chain_id,
            last_indexed_block: self.last_indexed_block as u64,
            last_indexed_hash: BlockHash(bytes_to_hash32(
                self.last_indexed_hash,
                "cursor.last_indexed_hash",
            )?),
            updated_at: self.updated_at,
        })
    }
}
