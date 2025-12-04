//! Storage layer for the ATS (Allfeat Timestamp) pallet.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::AccountId;
use maestro_core::ports::{Connection, Cursor, Edge, OrderDirection, PageInfo, Pagination};

use super::models::{AtsOwnershipTransfer, AtsVerificationKeyUpdate, AtsVersion, AtsWork};

/// Filter options for ATS work queries.
#[derive(Debug, Clone, Default)]
pub struct AtsWorkFilter {
    pub owner: Option<AccountId>,
    pub created_at_block_gte: Option<u64>,
    pub created_at_block_lte: Option<u64>,
}

/// Filter options for ATS version queries.
#[derive(Debug, Clone, Default)]
pub struct AtsVersionFilter {
    pub ats_id: Option<u64>,
    pub hash_commitment: Option<[u8; 32]>,
}

/// Filter options for ownership transfer queries.
#[derive(Debug, Clone, Default)]
pub struct AtsTransferFilter {
    pub ats_id: Option<u64>,
    pub old_owner: Option<AccountId>,
    pub new_owner: Option<AccountId>,
    /// Either old_owner or new_owner
    pub account: Option<AccountId>,
}

/// Storage trait for ATS pallet data.
#[async_trait]
pub trait AtsStorage: Send + Sync {
    // -------------------------------------------------------------------------
    // AtsWork operations
    // -------------------------------------------------------------------------

    /// Insert a new ATS work.
    async fn insert_ats_work(&self, work: &AtsWork) -> StorageResult<()>;

    /// Get an ATS work by ID.
    async fn get_ats_work(&self, id: u64) -> StorageResult<Option<AtsWork>>;

    /// Update the owner of an ATS work.
    async fn update_ats_owner(&self, id: u64, owner: &AccountId) -> StorageResult<()>;

    /// Update the latest version of an ATS work.
    async fn update_ats_latest_version(&self, id: u64, version: u32) -> StorageResult<()>;

    /// List ATS works with pagination and filtering.
    async fn list_ats_works(
        &self,
        filter: AtsWorkFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<AtsWork>>;

    /// List all ATS works owned by an account.
    async fn list_ats_by_owner(&self, owner: &AccountId) -> StorageResult<Vec<AtsWork>>;

    /// Get the total count of ATS works.
    async fn count_ats_works(&self) -> StorageResult<u64>;

    /// Get the count of ATS works per owner.
    async fn count_ats_by_owner(&self, owner: &AccountId) -> StorageResult<u64>;

    // -------------------------------------------------------------------------
    // AtsVersion operations
    // -------------------------------------------------------------------------

    /// Insert a new ATS version.
    async fn insert_ats_version(&self, version: &AtsVersion) -> StorageResult<()>;

    /// Get a specific version of an ATS.
    async fn get_ats_version(&self, ats_id: u64, version: u32) -> StorageResult<Option<AtsVersion>>;

    /// List all versions for an ATS.
    async fn list_versions_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsVersion>>;

    /// Find an ATS version by hash commitment.
    async fn find_by_hash_commitment(
        &self,
        hash: &[u8; 32],
    ) -> StorageResult<Option<AtsVersion>>;

    // -------------------------------------------------------------------------
    // Ownership transfer operations
    // -------------------------------------------------------------------------

    /// Insert an ownership transfer record.
    async fn insert_ownership_transfer(
        &self,
        transfer: &AtsOwnershipTransfer,
    ) -> StorageResult<()>;

    /// List ownership transfers for an ATS.
    async fn list_transfers_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsOwnershipTransfer>>;

    /// List ownership transfers with pagination and filtering.
    async fn list_transfers(
        &self,
        filter: AtsTransferFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<AtsOwnershipTransfer>>;

    // -------------------------------------------------------------------------
    // Verification key operations
    // -------------------------------------------------------------------------

    /// Insert a verification key update record.
    async fn insert_vk_update(&self, update: &AtsVerificationKeyUpdate) -> StorageResult<()>;

    /// Get the latest verification key.
    async fn get_latest_vk(&self) -> StorageResult<Option<AtsVerificationKeyUpdate>>;

    /// List all verification key updates.
    async fn list_vk_updates(&self) -> StorageResult<Vec<AtsVerificationKeyUpdate>>;

    // -------------------------------------------------------------------------
    // Reorg handling
    // -------------------------------------------------------------------------

    /// Delete all ATS data from a given block number (for reorg handling).
    async fn delete_from_block(&self, from_block: u64) -> StorageResult<u64>;
}

/// PostgreSQL implementation of AtsStorage.
pub struct PgAtsStorage {
    pool: PgPool,
}

impl PgAtsStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl AtsStorage for PgAtsStorage {
    // -------------------------------------------------------------------------
    // AtsWork operations
    // -------------------------------------------------------------------------

    async fn insert_ats_work(&self, work: &AtsWork) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ats_works (id, owner, created_at_block, created_at_timestamp, latest_version)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(work.id as i64)
        .bind(&work.owner.0[..])
        .bind(work.created_at_block as i64)
        .bind(work.created_at_timestamp)
        .bind(work.latest_version as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn get_ats_work(&self, id: u64) -> StorageResult<Option<AtsWork>> {
        let row = sqlx::query_as::<_, AtsWorkRow>(
            r#"
            SELECT id, owner, created_at_block, created_at_timestamp, latest_version
            FROM ats_works
            WHERE id = $1
            "#,
        )
        .bind(id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(AtsWorkRow::into_model).transpose()
    }

    async fn update_ats_owner(&self, id: u64, owner: &AccountId) -> StorageResult<()> {
        sqlx::query("UPDATE ats_works SET owner = $1 WHERE id = $2")
            .bind(&owner.0[..])
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn update_ats_latest_version(&self, id: u64, version: u32) -> StorageResult<()> {
        sqlx::query("UPDATE ats_works SET latest_version = $1 WHERE id = $2")
            .bind(version as i32)
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn list_ats_works(
        &self,
        filter: AtsWorkFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<AtsWork>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.owner.is_some() {
            conditions.push(format!("owner = ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_at_block_gte.is_some() {
            conditions.push(format!("created_at_block >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_at_block_lte.is_some() {
            conditions.push(format!("created_at_block <= ${}", param_idx));
            // param_idx += 1; // not needed, last param
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, owner, created_at_block, created_at_timestamp, latest_version
            FROM ats_works
            {}
            ORDER BY id {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            limit + 1
        );

        let rows: Vec<AtsWorkRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        } else {
            let mut q = sqlx::query_as::<_, AtsWorkRow>(&query);
            if let Some(ref owner) = filter.owner {
                q = q.bind(&owner.0[..]);
            }
            if let Some(block) = filter.created_at_block_gte {
                q = q.bind(block as i64);
            }
            if let Some(block) = filter.created_at_block_lte {
                q = q.bind(block as i64);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        };

        let has_more = rows.len() > limit as usize;
        let works: Vec<AtsWork> = rows
            .into_iter()
            .take(limit as usize)
            .map(AtsWorkRow::into_model)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<AtsWork>> = works
            .into_iter()
            .map(|w| Edge {
                cursor: Cursor {
                    value: w.id.to_string(),
                },
                node: w,
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

    async fn list_ats_by_owner(&self, owner: &AccountId) -> StorageResult<Vec<AtsWork>> {
        let rows = sqlx::query_as::<_, AtsWorkRow>(
            r#"
            SELECT id, owner, created_at_block, created_at_timestamp, latest_version
            FROM ats_works
            WHERE owner = $1
            ORDER BY id ASC
            "#,
        )
        .bind(&owner.0[..])
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter().map(AtsWorkRow::into_model).collect()
    }

    async fn count_ats_works(&self) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ats_works")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(row.0 as u64)
    }

    async fn count_ats_by_owner(&self, owner: &AccountId) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ats_works WHERE owner = $1")
            .bind(&owner.0[..])
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(row.0 as u64)
    }

    // -------------------------------------------------------------------------
    // AtsVersion operations
    // -------------------------------------------------------------------------

    async fn insert_ats_version(&self, version: &AtsVersion) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ats_versions (
                id, ats_id, version, hash_commitment,
                registered_at_block, registered_at_timestamp, event_index
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (ats_id, version) DO NOTHING
            "#,
        )
        .bind(&version.id)
        .bind(version.ats_id as i64)
        .bind(version.version as i32)
        .bind(&version.hash_commitment[..])
        .bind(version.registered_at_block as i64)
        .bind(version.registered_at_timestamp)
        .bind(version.event_index as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn get_ats_version(&self, ats_id: u64, version: u32) -> StorageResult<Option<AtsVersion>> {
        let row = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, hash_commitment,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE ats_id = $1 AND version = $2
            "#,
        )
        .bind(ats_id as i64)
        .bind(version as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(AtsVersionRow::into_model).transpose()
    }

    async fn list_versions_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsVersion>> {
        let rows = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, hash_commitment,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE ats_id = $1
            ORDER BY version ASC
            "#,
        )
        .bind(ats_id as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter().map(AtsVersionRow::into_model).collect()
    }

    async fn find_by_hash_commitment(
        &self,
        hash: &[u8; 32],
    ) -> StorageResult<Option<AtsVersion>> {
        let row = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, hash_commitment,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE hash_commitment = $1
            ORDER BY registered_at_block DESC
            LIMIT 1
            "#,
        )
        .bind(&hash[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(AtsVersionRow::into_model).transpose()
    }

    // -------------------------------------------------------------------------
    // Ownership transfer operations
    // -------------------------------------------------------------------------

    async fn insert_ownership_transfer(
        &self,
        transfer: &AtsOwnershipTransfer,
    ) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ats_ownership_transfers (
                id, ats_id, old_owner, new_owner, block_number, event_index, timestamp
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (block_number, event_index) DO NOTHING
            "#,
        )
        .bind(&transfer.id)
        .bind(transfer.ats_id as i64)
        .bind(&transfer.old_owner.0[..])
        .bind(&transfer.new_owner.0[..])
        .bind(transfer.block_number as i64)
        .bind(transfer.event_index as i32)
        .bind(transfer.timestamp)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn list_transfers_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsOwnershipTransfer>> {
        let rows = sqlx::query_as::<_, AtsTransferRow>(
            r#"
            SELECT id, ats_id, old_owner, new_owner, block_number, event_index, timestamp
            FROM ats_ownership_transfers
            WHERE ats_id = $1
            ORDER BY block_number ASC, event_index ASC
            "#,
        )
        .bind(ats_id as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter().map(AtsTransferRow::into_model).collect()
    }

    async fn list_transfers(
        &self,
        filter: AtsTransferFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<AtsOwnershipTransfer>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.ats_id.is_some() {
            conditions.push(format!("ats_id = ${}", param_idx));
            param_idx += 1;
        }
        if filter.old_owner.is_some() {
            conditions.push(format!("old_owner = ${}", param_idx));
            param_idx += 1;
        }
        if filter.new_owner.is_some() {
            conditions.push(format!("new_owner = ${}", param_idx));
            param_idx += 1;
        }
        if filter.account.is_some() {
            conditions.push(format!(
                "(old_owner = ${} OR new_owner = ${})",
                param_idx, param_idx
            ));
            // param_idx += 1; // not needed, last param
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, ats_id, old_owner, new_owner, block_number, event_index, timestamp
            FROM ats_ownership_transfers
            {}
            ORDER BY block_number {}, event_index {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            order_sql,
            limit + 1
        );

        let rows: Vec<AtsTransferRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        } else {
            let mut q = sqlx::query_as::<_, AtsTransferRow>(&query);
            if let Some(ats_id) = filter.ats_id {
                q = q.bind(ats_id as i64);
            }
            if let Some(ref owner) = filter.old_owner {
                q = q.bind(&owner.0[..]);
            }
            if let Some(ref owner) = filter.new_owner {
                q = q.bind(&owner.0[..]);
            }
            if let Some(ref account) = filter.account {
                q = q.bind(&account.0[..]);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        };

        let has_more = rows.len() > limit as usize;
        let transfers: Vec<AtsOwnershipTransfer> = rows
            .into_iter()
            .take(limit as usize)
            .map(AtsTransferRow::into_model)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<AtsOwnershipTransfer>> = transfers
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

    // -------------------------------------------------------------------------
    // Verification key operations
    // -------------------------------------------------------------------------

    async fn insert_vk_update(&self, update: &AtsVerificationKeyUpdate) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ats_verification_keys (id, vk, block_number, event_index, timestamp)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (block_number, event_index) DO NOTHING
            "#,
        )
        .bind(&update.id)
        .bind(&update.vk)
        .bind(update.block_number as i64)
        .bind(update.event_index as i32)
        .bind(update.timestamp)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(())
    }

    async fn get_latest_vk(&self) -> StorageResult<Option<AtsVerificationKeyUpdate>> {
        let row = sqlx::query_as::<_, AtsVkRow>(
            r#"
            SELECT id, vk, block_number, event_index, timestamp
            FROM ats_verification_keys
            ORDER BY block_number DESC, event_index DESC
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(row.map(AtsVkRow::into_model))
    }

    async fn list_vk_updates(&self) -> StorageResult<Vec<AtsVerificationKeyUpdate>> {
        let rows = sqlx::query_as::<_, AtsVkRow>(
            r#"
            SELECT id, vk, block_number, event_index, timestamp
            FROM ats_verification_keys
            ORDER BY block_number ASC, event_index ASC
            "#,
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(rows.into_iter().map(AtsVkRow::into_model).collect())
    }

    // -------------------------------------------------------------------------
    // Reorg handling
    // -------------------------------------------------------------------------

    async fn delete_from_block(&self, from_block: u64) -> StorageResult<u64> {
        let mut total_deleted = 0u64;

        // Delete in order of dependencies (children first)
        let vk_result = sqlx::query("DELETE FROM ats_verification_keys WHERE block_number >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        total_deleted += vk_result.rows_affected();

        let transfers_result =
            sqlx::query("DELETE FROM ats_ownership_transfers WHERE block_number >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?;
        total_deleted += transfers_result.rows_affected();

        let versions_result =
            sqlx::query("DELETE FROM ats_versions WHERE registered_at_block >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?;
        total_deleted += versions_result.rows_affected();

        let works_result = sqlx::query("DELETE FROM ats_works WHERE created_at_block >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        total_deleted += works_result.rows_affected();

        Ok(total_deleted)
    }
}

// =============================================================================
// Row mapping
// =============================================================================

#[derive(sqlx::FromRow)]
struct AtsWorkRow {
    id: i64,
    owner: Vec<u8>,
    created_at_block: i64,
    created_at_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    latest_version: i32,
}

impl AtsWorkRow {
    fn into_model(self) -> StorageResult<AtsWork> {
        Ok(AtsWork {
            id: self.id as u64,
            owner: AccountId(bytes_to_hash32(self.owner, "ats_work.owner")?),
            created_at_block: self.created_at_block as u64,
            created_at_timestamp: self.created_at_timestamp,
            latest_version: self.latest_version as u32,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AtsVersionRow {
    id: String,
    ats_id: i64,
    version: i32,
    hash_commitment: Vec<u8>,
    registered_at_block: i64,
    registered_at_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    event_index: i32,
}

impl AtsVersionRow {
    fn into_model(self) -> StorageResult<AtsVersion> {
        Ok(AtsVersion {
            id: self.id,
            ats_id: self.ats_id as u64,
            version: self.version as u32,
            hash_commitment: bytes_to_hash32(self.hash_commitment, "ats_version.hash_commitment")?,
            registered_at_block: self.registered_at_block as u64,
            registered_at_timestamp: self.registered_at_timestamp,
            event_index: self.event_index as u32,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AtsTransferRow {
    id: String,
    ats_id: i64,
    old_owner: Vec<u8>,
    new_owner: Vec<u8>,
    block_number: i64,
    event_index: i32,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl AtsTransferRow {
    fn into_model(self) -> StorageResult<AtsOwnershipTransfer> {
        Ok(AtsOwnershipTransfer {
            id: self.id,
            ats_id: self.ats_id as u64,
            old_owner: AccountId(bytes_to_hash32(self.old_owner, "ats_transfer.old_owner")?),
            new_owner: AccountId(bytes_to_hash32(self.new_owner, "ats_transfer.new_owner")?),
            block_number: self.block_number as u64,
            event_index: self.event_index as u32,
            timestamp: self.timestamp,
        })
    }
}

#[derive(sqlx::FromRow)]
struct AtsVkRow {
    id: String,
    vk: Vec<u8>,
    block_number: i64,
    event_index: i32,
    timestamp: Option<chrono::DateTime<chrono::Utc>>,
}

impl AtsVkRow {
    fn into_model(self) -> AtsVerificationKeyUpdate {
        AtsVerificationKeyUpdate {
            id: self.id,
            vk: self.vk,
            block_number: self.block_number as u64,
            event_index: self.event_index as u32,
            timestamp: self.timestamp,
        }
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

/// SQL migrations for the ATS bundle.
/// Each migration is tracked and only executed once.
pub const MIGRATIONS: &[&str] = &[
    // Migration 0: Create ATS tables
    r#"
-- Main ATS works table
CREATE TABLE ats_works (
    id BIGINT PRIMARY KEY,
    owner BYTEA NOT NULL,
    created_at_block BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    created_at_timestamp TIMESTAMPTZ,
    latest_version INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_ats_works_owner ON ats_works(owner);
CREATE INDEX idx_ats_works_created_at ON ats_works(created_at_block);

-- ATS versions history
CREATE TABLE ats_versions (
    id TEXT PRIMARY KEY,
    ats_id BIGINT NOT NULL REFERENCES ats_works(id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    hash_commitment BYTEA NOT NULL,
    registered_at_block BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    registered_at_timestamp TIMESTAMPTZ,
    event_index INTEGER NOT NULL,
    UNIQUE(ats_id, version)
);

CREATE INDEX idx_ats_versions_ats_id ON ats_versions(ats_id);
CREATE INDEX idx_ats_versions_hash ON ats_versions(hash_commitment);
CREATE INDEX idx_ats_versions_block ON ats_versions(registered_at_block);

-- Ownership transfers (claims)
CREATE TABLE ats_ownership_transfers (
    id TEXT PRIMARY KEY,
    ats_id BIGINT NOT NULL REFERENCES ats_works(id) ON DELETE CASCADE,
    old_owner BYTEA NOT NULL,
    new_owner BYTEA NOT NULL,
    block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    event_index INTEGER NOT NULL,
    timestamp TIMESTAMPTZ,
    UNIQUE(block_number, event_index)
);

CREATE INDEX idx_ats_transfers_ats_id ON ats_ownership_transfers(ats_id);
CREATE INDEX idx_ats_transfers_old_owner ON ats_ownership_transfers(old_owner);
CREATE INDEX idx_ats_transfers_new_owner ON ats_ownership_transfers(new_owner);
CREATE INDEX idx_ats_transfers_block ON ats_ownership_transfers(block_number);

-- Verification key history
CREATE TABLE ats_verification_keys (
    id TEXT PRIMARY KEY,
    vk BYTEA NOT NULL,
    block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    event_index INTEGER NOT NULL,
    timestamp TIMESTAMPTZ,
    UNIQUE(block_number, event_index)
);

CREATE INDEX idx_ats_vk_block ON ats_verification_keys(block_number);
"#,
];
