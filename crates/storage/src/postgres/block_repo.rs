//! Block repository implementation for PostgreSQL.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::{Block, BlockHash};
use maestro_core::ports::{
    BlockFilter, BlockRepository, Connection, Cursor, Edge, OrderDirection, PageInfo, Pagination,
};

use super::database::Database;
use super::helpers::{bytes_to_hash32_strict, bytes_to_optional_hash32};

/// PostgreSQL implementation of BlockRepository.
pub struct PgBlockRepository {
    pool: PgPool,
}

impl PgBlockRepository {
    pub fn new(db: &Database) -> Self {
        Self {
            pool: db.pool().clone(),
        }
    }
}

#[async_trait]
impl BlockRepository for PgBlockRepository {
    async fn insert_blocks(&self, blocks: &[Block]) -> StorageResult<()> {
        if blocks.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        for block in blocks {
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
            .bind(block.number as i64)
            .bind(&block.hash.0[..])
            .bind(&block.parent_hash.0[..])
            .bind(&block.state_root.0[..])
            .bind(&block.extrinsics_root.0[..])
            .bind(block.author.as_ref().map(|a| &a.0[..]))
            .bind(block.timestamp)
            .bind(block.extrinsic_count as i32)
            .bind(block.event_count as i32)
            .bind(block.indexed_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(())
    }

    async fn get_block(&self, number: u64) -> StorageResult<Option<Block>> {
        let row = sqlx::query_as::<_, BlockRow>(
            r#"
            SELECT number, hash, parent_hash, state_root, extrinsics_root,
                   author, timestamp, extrinsic_count, event_count, indexed_at
            FROM blocks
            WHERE number = $1
            "#,
        )
        .bind(number as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(BlockRow::into_block).transpose()
    }

    async fn get_block_by_hash(&self, hash: &BlockHash) -> StorageResult<Option<Block>> {
        let row = sqlx::query_as::<_, BlockRow>(
            r#"
            SELECT number, hash, parent_hash, state_root, extrinsics_root,
                   author, timestamp, extrinsic_count, event_count, indexed_at
            FROM blocks
            WHERE hash = $1
            "#,
        )
        .bind(&hash.0[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(BlockRow::into_block).transpose()
    }

    async fn list_blocks(
        &self,
        filter: BlockFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Block>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let (order_sql, cursor_op) = match order {
            OrderDirection::Asc => ("ASC", ">"),
            OrderDirection::Desc => ("DESC", "<"),
        };

        // Parse cursor if provided (cursor is block number as string)
        let after_number: Option<i64> =
            pagination.after.as_ref().and_then(|c| c.value.parse().ok());

        // Build WHERE clause dynamically.
        //
        // SAFETY: This dynamic SQL is safe from injection because:
        // 1. Column names (number) are hardcoded, never from user input
        // 2. Operators (>=, <=, >, <, AND) are hardcoded
        // 3. All VALUES are parameterized via $1, $2, etc. and bound separately
        // 4. Order direction comes from enum (ASC/DESC), not user strings
        let mut conditions = Vec::new();
        let mut params: Vec<i64> = Vec::new();

        // Cursor condition
        if let Some(cursor_num) = after_number {
            params.push(cursor_num);
            conditions.push(format!("number {} ${}", cursor_op, params.len()));
        }

        // Filter conditions
        if let Some(gte) = filter.number_gte {
            params.push(gte as i64);
            conditions.push(format!("number >= ${}", params.len()));
        }
        if let Some(lte) = filter.number_lte {
            params.push(lte as i64);
            conditions.push(format!("number <= ${}", params.len()));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        // Main query
        let query = format!(
            r#"
            SELECT number, hash, parent_hash, state_root, extrinsics_root,
                   author, timestamp, extrinsic_count, event_count, indexed_at
            FROM blocks
            {}
            ORDER BY number {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            limit + 1
        );

        let mut query_builder = sqlx::query_as::<_, BlockRow>(&query);
        for param in &params {
            query_builder = query_builder.bind(*param);
        }

        let rows: Vec<BlockRow> = query_builder
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let has_more = rows.len() > limit as usize;
        let blocks: Vec<Block> = rows
            .into_iter()
            .take(limit as usize)
            .map(BlockRow::into_block)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Block>> = blocks
            .into_iter()
            .map(|block| {
                let cursor = Cursor {
                    value: block.number.to_string(),
                };
                Edge {
                    node: block,
                    cursor,
                }
            })
            .collect();

        // Check if there are previous pages (only if cursor was used)
        let has_previous = after_number.is_some();

        let page_info = PageInfo {
            has_next_page: has_more,
            has_previous_page: has_previous,
            start_cursor: edges.first().map(|e| e.cursor.clone()),
            end_cursor: edges.last().map(|e| e.cursor.clone()),
        };

        Ok(Connection {
            edges,
            page_info,
            total_count: None,
        })
    }

    async fn latest_block_number(&self) -> StorageResult<Option<u64>> {
        // MAX returns NULL when table is empty, so we need Option<i64> in the tuple
        let row: (Option<i64>,) = sqlx::query_as("SELECT MAX(number) FROM blocks")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(row.0.map(|n| n as u64))
    }

    async fn delete_blocks_from(&self, from_number: u64) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM blocks WHERE number >= $1")
            .bind(from_number as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

/// Database row representation for Block.
#[derive(sqlx::FromRow)]
struct BlockRow {
    number: i64,
    hash: Vec<u8>,
    parent_hash: Vec<u8>,
    state_root: Vec<u8>,
    extrinsics_root: Vec<u8>,
    author: Option<Vec<u8>>,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
    extrinsic_count: i32,
    event_count: i32,
    indexed_at: chrono::DateTime<chrono::Utc>,
}

impl BlockRow {
    fn into_block(self) -> StorageResult<Block> {
        Ok(Block {
            number: self.number as u64,
            hash: BlockHash(bytes_to_hash32_strict(self.hash, "block.hash")?),
            parent_hash: BlockHash(bytes_to_hash32_strict(
                self.parent_hash,
                "block.parent_hash",
            )?),
            state_root: BlockHash(bytes_to_hash32_strict(self.state_root, "block.state_root")?),
            extrinsics_root: BlockHash(bytes_to_hash32_strict(
                self.extrinsics_root,
                "block.extrinsics_root",
            )?),
            author: bytes_to_optional_hash32(self.author, "block.author")?
                .map(maestro_core::models::AccountId),
            timestamp: self.timestamp,
            extrinsic_count: self.extrinsic_count as u32,
            event_count: self.event_count as u32,
            indexed_at: self.indexed_at,
        })
    }
}
