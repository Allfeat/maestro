//! Storage layer for the Balances pallet.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::{AccountId, BlockHash};
use maestro_core::ports::{Connection, Cursor, Edge, OrderDirection, PageInfo, Pagination};

use super::models::Transfer;

/// Filter options for transfer queries.
#[derive(Debug, Clone, Default)]
pub struct TransferFilter {
    pub block_number_gte: Option<u64>,
    pub block_number_lte: Option<u64>,
    pub from: Option<AccountId>,
    pub to: Option<AccountId>,
    pub account: Option<AccountId>, // Either from or to
    pub amount_gte: Option<u128>,
    pub amount_lte: Option<u128>,
}

/// Storage trait for Balances pallet data.
#[async_trait]
pub trait BalancesStorage: Send + Sync {
    /// Insert a batch of transfers.
    async fn insert_transfers(&self, transfers: &[Transfer]) -> StorageResult<()>;

    /// Get transfer by ID.
    async fn get_transfer(&self, id: &str) -> StorageResult<Option<Transfer>>;

    /// List transfers for a block.
    async fn list_transfers_for_block(&self, block_number: u64) -> StorageResult<Vec<Transfer>>;

    /// List transfers with pagination and filtering.
    async fn list_transfers(
        &self,
        filter: TransferFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Transfer>>;

    /// Delete transfers from a given block number (for reorg handling).
    async fn delete_transfers_from(&self, from_block: u64) -> StorageResult<u64>;
}

/// PostgreSQL implementation of BalancesStorage.
pub struct PgBalancesStorage {
    pool: PgPool,
}

impl PgBalancesStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BalancesStorage for PgBalancesStorage {
    async fn insert_transfers(&self, transfers: &[Transfer]) -> StorageResult<()> {
        if transfers.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        for transfer in transfers {
            sqlx::query(
                r#"
                INSERT INTO transfers (
                    id, block_number, block_hash, event_index, extrinsic_index,
                    from_account, to_account, amount, success, timestamp
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8::NUMERIC, $9, $10)
                ON CONFLICT (block_number, event_index) DO NOTHING
                "#,
            )
            .bind(&transfer.id)
            .bind(transfer.block_number as i64)
            .bind(&transfer.block_hash.0[..])
            .bind(transfer.event_index as i32)
            .bind(transfer.extrinsic_index.map(|i| i as i32))
            .bind(&transfer.from.0[..])
            .bind(&transfer.to.0[..])
            .bind(transfer.amount.to_string())
            .bind(transfer.success)
            .bind(transfer.timestamp)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(())
    }

    async fn get_transfer(&self, id: &str) -> StorageResult<Option<Transfer>> {
        let row = sqlx::query_as::<_, TransferRow>(
            r#"
            SELECT id, block_number, block_hash, event_index, extrinsic_index,
                   from_account, to_account, amount::TEXT, success, timestamp
            FROM transfers
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(TransferRow::into_transfer).transpose()
    }

    async fn list_transfers_for_block(&self, block_number: u64) -> StorageResult<Vec<Transfer>> {
        let rows = sqlx::query_as::<_, TransferRow>(
            r#"
            SELECT id, block_number, block_hash, event_index, extrinsic_index,
                   from_account, to_account, amount::TEXT, success, timestamp
            FROM transfers
            WHERE block_number = $1
            ORDER BY event_index ASC
            "#,
        )
        .bind(block_number as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter()
            .map(TransferRow::into_transfer)
            .collect()
    }

    async fn list_transfers(
        &self,
        filter: TransferFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Transfer>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        // Build WHERE clause dynamically
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.block_number_gte.is_some() {
            conditions.push(format!("block_number >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.block_number_lte.is_some() {
            conditions.push(format!("block_number <= ${}", param_idx));
            param_idx += 1;
        }
        if filter.from.is_some() {
            conditions.push(format!("from_account = ${}", param_idx));
            param_idx += 1;
        }
        if filter.to.is_some() {
            conditions.push(format!("to_account = ${}", param_idx));
            param_idx += 1;
        }
        if filter.account.is_some() {
            conditions.push(format!(
                "(from_account = ${} OR to_account = ${})",
                param_idx, param_idx
            ));
            param_idx += 1;
        }
        if filter.amount_gte.is_some() {
            conditions.push(format!("amount >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.amount_lte.is_some() {
            conditions.push(format!("amount <= ${}", param_idx));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, block_number, block_hash, event_index, extrinsic_index,
                   from_account, to_account, amount::TEXT, success, timestamp
            FROM transfers
            {}
            ORDER BY block_number {}, event_index {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            order_sql,
            limit + 1
        );

        // Build query with bindings
        let rows: Vec<TransferRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        } else {
            let mut q = sqlx::query_as::<_, TransferRow>(&query);
            if let Some(bn) = filter.block_number_gte {
                q = q.bind(bn as i64);
            }
            if let Some(bn) = filter.block_number_lte {
                q = q.bind(bn as i64);
            }
            if let Some(ref from) = filter.from {
                q = q.bind(&from.0[..]);
            }
            if let Some(ref to) = filter.to {
                q = q.bind(&to.0[..]);
            }
            if let Some(ref account) = filter.account {
                q = q.bind(&account.0[..]);
            }
            if let Some(amount) = filter.amount_gte {
                q = q.bind(amount.to_string());
            }
            if let Some(amount) = filter.amount_lte {
                q = q.bind(amount.to_string());
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        };

        let has_more = rows.len() > limit as usize;
        let transfers: Vec<Transfer> = rows
            .into_iter()
            .take(limit as usize)
            .map(TransferRow::into_transfer)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Transfer>> = transfers
            .into_iter()
            .map(|t| Edge {
                cursor: Cursor {
                    value: t.id.clone(),
                },
                node: t,
            })
            .collect();

        let page_info = PageInfo {
            has_next_page: has_more,
            has_previous_page: pagination.after.is_some(),
            start_cursor: edges.first().map(|e| e.cursor.clone()),
            end_cursor: edges.last().map(|e| e.cursor.clone()),
        };

        Ok(Connection {
            edges,
            page_info,
            total_count: None,
        })
    }

    async fn delete_transfers_from(&self, from_block: u64) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM transfers WHERE block_number >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        Ok(result.rows_affected())
    }
}

// =============================================================================
// Row mapping
// =============================================================================

#[derive(sqlx::FromRow)]
struct TransferRow {
    id: String,
    block_number: i64,
    block_hash: Vec<u8>,
    event_index: i32,
    extrinsic_index: Option<i32>,
    from_account: Vec<u8>,
    to_account: Vec<u8>,
    amount: String,
    success: bool,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl TransferRow {
    fn into_transfer(self) -> StorageResult<Transfer> {
        Ok(Transfer {
            id: self.id,
            block_number: self.block_number as u64,
            block_hash: BlockHash(bytes_to_hash32(self.block_hash, "transfer.block_hash")?),
            event_index: self.event_index as u32,
            extrinsic_index: self.extrinsic_index.map(|i| i as u32),
            from: AccountId(bytes_to_hash32(self.from_account, "transfer.from")?),
            to: AccountId(bytes_to_hash32(self.to_account, "transfer.to")?),
            amount: parse_amount(&self.amount)?,
            success: self.success,
            timestamp: self.timestamp,
        })
    }
}

// =============================================================================
// Conversion helpers
// =============================================================================

/// Convert Vec<u8> to [u8; 32] with descriptive error.
fn bytes_to_hash32(bytes: Vec<u8>, field: &str) -> StorageResult<[u8; 32]> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        StorageError::SerializationError(format!(
            "{} has invalid length: expected 32, got {}",
            field,
            v.len()
        ))
    })
}

/// Parse amount string to u128.
fn parse_amount(s: &str) -> StorageResult<u128> {
    s.parse().map_err(|e| {
        StorageError::SerializationError(format!("amount parse error: {} (value: {})", e, s))
    })
}

/// SQL migrations for the balances bundle.
/// Each migration is tracked and only executed once.
pub const MIGRATIONS: &[&str] = &[
    // Migration 0: Create transfers table
    r#"
CREATE TABLE transfers (
    id TEXT PRIMARY KEY,
    block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    block_hash BYTEA NOT NULL,
    event_index INTEGER NOT NULL,
    extrinsic_index INTEGER,
    from_account BYTEA NOT NULL,
    to_account BYTEA NOT NULL,
    amount NUMERIC(39, 0) NOT NULL,
    success BOOLEAN NOT NULL DEFAULT TRUE,
    timestamp TIMESTAMPTZ,
    UNIQUE(block_number, event_index)
);

CREATE INDEX idx_transfers_block ON transfers(block_number);
CREATE INDEX idx_transfers_from ON transfers(from_account);
CREATE INDEX idx_transfers_to ON transfers(to_account);
CREATE INDEX idx_transfers_timestamp ON transfers(timestamp) WHERE timestamp IS NOT NULL;
CREATE INDEX idx_transfers_amount ON transfers(amount);
"#,
];
