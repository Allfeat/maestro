//! Cursor repository implementation for PostgreSQL.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::StorageResult;
use maestro_core::models::{BlockHash, IndexerCursor};
use maestro_core::ports::CursorRepository;

use super::SqlxResultExt;
use super::helpers::bytes_to_hash32;

/// PostgreSQL implementation of CursorRepository.
pub struct PgCursorRepository {
    pool: PgPool,
}

impl PgCursorRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CursorRepository for PgCursorRepository {
    async fn get_cursor(&self, chain_id: &str) -> StorageResult<Option<IndexerCursor>> {
        let row = sqlx::query_as::<_, CursorRow>(
            r#"
            SELECT chain_id, first_indexed_block, last_indexed_block, last_indexed_hash, updated_at
            FROM indexer_cursor
            WHERE chain_id = $1
            "#,
        )
        .bind(chain_id)
        .fetch_optional(&self.pool)
        .await
        .query_err("get cursor by chain_id")?;

        row.map(CursorRow::into_cursor).transpose()
    }

    async fn get_any_cursor(&self) -> StorageResult<Option<IndexerCursor>> {
        let row = sqlx::query_as::<_, CursorRow>(
            r#"
            SELECT chain_id, first_indexed_block, last_indexed_block, last_indexed_hash, updated_at
            FROM indexer_cursor
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .query_err("get any cursor")?;

        row.map(CursorRow::into_cursor).transpose()
    }

    async fn set_cursor(&self, cursor: &IndexerCursor) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO indexer_cursor (chain_id, first_indexed_block, last_indexed_block, last_indexed_hash, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (chain_id) DO UPDATE SET
                first_indexed_block = EXCLUDED.first_indexed_block,
                last_indexed_block = EXCLUDED.last_indexed_block,
                last_indexed_hash = EXCLUDED.last_indexed_hash,
                updated_at = EXCLUDED.updated_at
            "#,
        )
        .bind(&cursor.chain_id)
        .bind(cursor.first_indexed_block as i64)
        .bind(cursor.last_indexed_block as i64)
        .bind(&cursor.last_indexed_hash.0[..])
        .bind(cursor.updated_at)
        .execute(&self.pool)
        .await
        .query_err("set cursor")?;

        Ok(())
    }

    async fn extend_upward(
        &self,
        chain_id: &str,
        block: u64,
        hash: &BlockHash,
    ) -> StorageResult<()> {
        sqlx::query(
            r#"
            UPDATE indexer_cursor
            SET last_indexed_block = $1,
                last_indexed_hash = $2,
                updated_at = NOW()
            WHERE chain_id = $3
            "#,
        )
        .bind(block as i64)
        .bind(&hash.0[..])
        .bind(chain_id)
        .execute(&self.pool)
        .await
        .query_err("extend_upward cursor")?;
        Ok(())
    }

    async fn extend_downward(&self, chain_id: &str, block: u64) -> StorageResult<()> {
        sqlx::query(
            r#"
            UPDATE indexer_cursor
            SET first_indexed_block = $1,
                updated_at = NOW()
            WHERE chain_id = $2
            "#,
        )
        .bind(block as i64)
        .bind(chain_id)
        .execute(&self.pool)
        .await
        .query_err("extend_downward cursor")?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct CursorRow {
    chain_id: String,
    first_indexed_block: i64,
    last_indexed_block: i64,
    last_indexed_hash: Vec<u8>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl CursorRow {
    fn into_cursor(self) -> StorageResult<IndexerCursor> {
        Ok(IndexerCursor {
            chain_id: self.chain_id,
            first_indexed_block: self.first_indexed_block as u64,
            last_indexed_block: self.last_indexed_block as u64,
            last_indexed_hash: BlockHash(bytes_to_hash32(
                self.last_indexed_hash,
                "cursor.last_indexed_hash",
            )?),
            updated_at: self.updated_at,
        })
    }
}
