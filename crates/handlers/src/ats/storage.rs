//! Storage layer for the ATS (Allfeat Timestamp) pallet.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::AccountId;
use maestro_core::ports::{Connection, Cursor, Edge, OrderDirection, PageInfo, Pagination};

use super::models::{AtsVersion, AtsWork};

/// Filter options for ATS work queries.
#[derive(Debug, Clone, Default)]
pub struct AtsWorkFilter {
    pub owner: Option<AccountId>,
    pub created_at_block_gte: Option<u64>,
    pub created_at_block_lte: Option<u64>,
    pub created_at_timestamp_gte: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at_timestamp_lte: Option<chrono::DateTime<chrono::Utc>>,
}

/// Filter options for ATS version queries.
#[derive(Debug, Clone, Default)]
pub struct AtsVersionFilter {
    pub ats_id: Option<u64>,
    pub commitment: Option<[u8; 32]>,
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
    async fn get_ats_version(&self, ats_id: u64, version: u32)
    -> StorageResult<Option<AtsVersion>>;

    /// List all versions for an ATS.
    async fn list_versions_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsVersion>>;

    /// Find an ATS version by commitment.
    async fn find_by_commitment(&self, hash: &[u8; 32]) -> StorageResult<Option<AtsVersion>>;

    // -------------------------------------------------------------------------
    // Revocation
    // -------------------------------------------------------------------------

    /// Delete an ATS work by ID (CASCADE deletes versions).
    async fn delete_ats_work(&self, id: u64) -> StorageResult<()>;

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
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

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
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(AtsWorkRow::into_model).transpose()
    }

    async fn update_ats_latest_version(&self, id: u64, version: u32) -> StorageResult<()> {
        sqlx::query("UPDATE ats_works SET latest_version = $1 WHERE id = $2")
            .bind(version as i32)
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

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
            param_idx += 1;
        }
        if filter.created_at_timestamp_gte.is_some() {
            conditions.push(format!("created_at_timestamp >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_at_timestamp_lte.is_some() {
            conditions.push(format!("created_at_timestamp <= ${}", param_idx));
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
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
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
            if let Some(ts) = filter.created_at_timestamp_gte {
                q = q.bind(ts);
            }
            if let Some(ts) = filter.created_at_timestamp_lte {
                q = q.bind(ts);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
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
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        rows.into_iter().map(AtsWorkRow::into_model).collect()
    }

    async fn count_ats_works(&self) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ats_works")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(row.0 as u64)
    }

    async fn count_ats_by_owner(&self, owner: &AccountId) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM ats_works WHERE owner = $1")
            .bind(&owner.0[..])
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(row.0 as u64)
    }

    // -------------------------------------------------------------------------
    // AtsVersion operations
    // -------------------------------------------------------------------------

    async fn insert_ats_version(&self, version: &AtsVersion) -> StorageResult<()> {
        sqlx::query(
            r#"
            INSERT INTO ats_versions (
                id, ats_id, version, commitment, protocol_version,
                registered_at_block, registered_at_timestamp, event_index
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (ats_id, version) DO NOTHING
            "#,
        )
        .bind(&version.id)
        .bind(version.ats_id as i64)
        .bind(version.version as i32)
        .bind(&version.commitment[..])
        .bind(version.protocol_version as i16)
        .bind(version.registered_at_block as i64)
        .bind(version.registered_at_timestamp)
        .bind(version.event_index as i32)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(())
    }

    async fn get_ats_version(
        &self,
        ats_id: u64,
        version: u32,
    ) -> StorageResult<Option<AtsVersion>> {
        let row = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, commitment, protocol_version,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE ats_id = $1 AND version = $2
            "#,
        )
        .bind(ats_id as i64)
        .bind(version as i32)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(AtsVersionRow::into_model).transpose()
    }

    async fn list_versions_for_ats(&self, ats_id: u64) -> StorageResult<Vec<AtsVersion>> {
        let rows = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, commitment, protocol_version,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE ats_id = $1
            ORDER BY version ASC
            "#,
        )
        .bind(ats_id as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        rows.into_iter().map(AtsVersionRow::into_model).collect()
    }

    async fn find_by_commitment(&self, hash: &[u8; 32]) -> StorageResult<Option<AtsVersion>> {
        let row = sqlx::query_as::<_, AtsVersionRow>(
            r#"
            SELECT id, ats_id, version, commitment, protocol_version,
                   registered_at_block, registered_at_timestamp, event_index
            FROM ats_versions
            WHERE commitment = $1
            ORDER BY registered_at_block DESC
            LIMIT 1
            "#,
        )
        .bind(&hash[..])
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(AtsVersionRow::into_model).transpose()
    }

    // -------------------------------------------------------------------------
    // Revocation
    // -------------------------------------------------------------------------

    async fn delete_ats_work(&self, id: u64) -> StorageResult<()> {
        sqlx::query("DELETE FROM ats_works WHERE id = $1")
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Reorg handling
    // -------------------------------------------------------------------------

    async fn delete_from_block(&self, from_block: u64) -> StorageResult<u64> {
        let mut total_deleted = 0u64;

        // Delete in order of dependencies (children first)
        let versions_result =
            sqlx::query("DELETE FROM ats_versions WHERE registered_at_block >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        total_deleted += versions_result.rows_affected();

        let works_result = sqlx::query("DELETE FROM ats_works WHERE created_at_block >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
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
    commitment: Vec<u8>,
    protocol_version: i16,
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
            commitment: bytes_to_hash32(self.commitment, "ats_version.commitment")?,
            protocol_version: self.protocol_version as u8,
            registered_at_block: self.registered_at_block as u64,
            registered_at_timestamp: self.registered_at_timestamp,
            event_index: self.event_index as u32,
        })
    }
}

// =============================================================================
// Conversion helpers
// =============================================================================

/// Convert Vec<u8> to [u8; 32] with descriptive error.
fn bytes_to_hash32(bytes: Vec<u8>, field: &str) -> StorageResult<[u8; 32]> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        StorageError::serialization(format!(
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
    // Migration 1: Add index on created_at_timestamp for date range filtering
    r#"
CREATE INDEX IF NOT EXISTS idx_ats_works_created_at_ts ON ats_works(created_at_timestamp);
"#,
    // Migration 2: Adapt to new pallet-ats (remove ownership/VK, add protocol_version, rename hash_commitment)
    r#"
DROP TABLE IF EXISTS ats_verification_keys;
DROP TABLE IF EXISTS ats_ownership_transfers;
ALTER TABLE ats_versions ADD COLUMN IF NOT EXISTS protocol_version SMALLINT NOT NULL DEFAULT 1;
ALTER TABLE ats_versions RENAME COLUMN hash_commitment TO commitment;
"#,
];
