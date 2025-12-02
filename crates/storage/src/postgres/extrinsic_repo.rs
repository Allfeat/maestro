//! Extrinsic repository implementation for PostgreSQL.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::{AccountId, BlockHash, Extrinsic, ExtrinsicStatus};
use maestro_core::ports::{
    Connection, Cursor, Edge, ExtrinsicFilter, ExtrinsicRepository, OrderDirection, PageInfo,
    Pagination,
};

use super::helpers::bytes_to_hash32;

// =============================================================================
// Repository Implementation
// =============================================================================

/// PostgreSQL implementation of ExtrinsicRepository.
pub struct PgExtrinsicRepository {
    pool: PgPool,
}

impl PgExtrinsicRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ExtrinsicRepository for PgExtrinsicRepository {
    async fn insert_extrinsics(&self, extrinsics: &[Extrinsic]) -> StorageResult<()> {
        if extrinsics.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        for ext in extrinsics {
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

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(())
    }

    async fn get_extrinsic(&self, id: &str) -> StorageResult<Option<Extrinsic>> {
        let row = sqlx::query_as::<_, ExtrinsicRow>(
            r#"
            SELECT id, block_number, block_hash, index, pallet, call,
                   signer, status, error, args, raw, tip::TEXT, nonce
            FROM extrinsics
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(ExtrinsicRow::into_extrinsic).transpose()
    }

    async fn list_extrinsics_for_block(&self, block_number: u64) -> StorageResult<Vec<Extrinsic>> {
        let rows = sqlx::query_as::<_, ExtrinsicRow>(
            r#"
            SELECT id, block_number, block_hash, index, pallet, call,
                   signer, status, error, args, raw, tip::TEXT, nonce
            FROM extrinsics
            WHERE block_number = $1
            ORDER BY index ASC
            "#,
        )
        .bind(block_number as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter()
            .map(ExtrinsicRow::into_extrinsic)
            .collect()
    }

    async fn list_extrinsics(
        &self,
        filter: ExtrinsicFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Extrinsic>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        // Build dynamic query.
        //
        // SAFETY: This dynamic SQL is safe from injection because:
        // 1. Column names are hardcoded (block_number, pallet, call, signer, status)
        // 2. Operators (=, AND) are hardcoded
        // 3. All VALUES are parameterized via $1, $2, etc. and bound separately
        // 4. Order direction comes from enum (ASC/DESC), not user strings
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.block_number.is_some() {
            conditions.push(format!("block_number = ${}", param_idx));
            param_idx += 1;
        }
        if filter.pallet.is_some() {
            conditions.push(format!("pallet = ${}", param_idx));
            param_idx += 1;
        }
        if filter.call.is_some() {
            conditions.push(format!("call = ${}", param_idx));
            param_idx += 1;
        }
        if filter.signer.is_some() {
            conditions.push(format!("signer = ${}", param_idx));
            param_idx += 1;
        }
        if filter.success.is_some() {
            conditions.push(format!("status = ${}", param_idx));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, block_number, block_hash, index, pallet, call,
                   signer, status, error, args, raw, tip::TEXT, nonce
            FROM extrinsics
            {}
            ORDER BY block_number {}, index {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            order_sql,
            limit + 1
        );

        let rows: Vec<ExtrinsicRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        } else {
            let mut q = sqlx::query_as::<_, ExtrinsicRow>(&query);
            if let Some(bn) = filter.block_number {
                q = q.bind(bn as i64);
            }
            if let Some(ref p) = filter.pallet {
                q = q.bind(p);
            }
            if let Some(ref c) = filter.call {
                q = q.bind(c);
            }
            if let Some(ref s) = filter.signer {
                q = q.bind(&s.0[..]);
            }
            if let Some(success) = filter.success {
                q = q.bind(if success { "success" } else { "failed" });
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        };

        let has_more = rows.len() > limit as usize;
        let extrinsics: Vec<Extrinsic> = rows
            .into_iter()
            .take(limit as usize)
            .map(ExtrinsicRow::into_extrinsic)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Extrinsic>> = extrinsics
            .into_iter()
            .map(|ext| Edge {
                cursor: Cursor {
                    value: ext.id.clone(),
                },
                node: ext,
            })
            .collect();

        Ok(Connection {
            edges: edges.clone(),
            page_info: PageInfo {
                has_next_page: has_more,
                has_previous_page: pagination.after.is_some(),
                start_cursor: edges.first().map(|e| e.cursor.clone()),
                end_cursor: edges.last().map(|e| e.cursor.clone()),
            },
            total_count: None,
        })
    }

    async fn delete_extrinsics_from(&self, from_block: u64) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM extrinsics WHERE block_number >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

// =============================================================================
// Row Mapping
// =============================================================================

#[derive(sqlx::FromRow)]
struct ExtrinsicRow {
    id: String,
    block_number: i64,
    block_hash: Vec<u8>,
    index: i32,
    pallet: String,
    call: String,
    signer: Option<Vec<u8>>,
    status: String,
    error: Option<String>,
    args: serde_json::Value,
    raw: String,
    tip: Option<String>,
    nonce: Option<i32>,
}

impl ExtrinsicRow {
    fn into_extrinsic(self) -> StorageResult<Extrinsic> {
        let block_hash = bytes_to_hash32(self.block_hash, "extrinsic.block_hash")?;

        let signer = self
            .signer
            .map(|s| bytes_to_hash32(s, "extrinsic.signer").map(AccountId))
            .transpose()?;

        Ok(Extrinsic {
            id: self.id,
            block_number: self.block_number as u64,
            block_hash: BlockHash(block_hash),
            index: self.index as u32,
            pallet: self.pallet,
            call: self.call,
            signer,
            status: str_to_status(&self.status),
            error: self.error,
            args: self.args,
            raw: self.raw,
            tip: self.tip.and_then(|t| t.parse::<u128>().ok()),
            nonce: self.nonce.map(|n| n as u32),
        })
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

fn str_to_status(s: &str) -> ExtrinsicStatus {
    if s == "success" {
        ExtrinsicStatus::Success
    } else {
        ExtrinsicStatus::Failed
    }
}
