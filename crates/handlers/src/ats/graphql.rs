//! GraphQL types and queries for the ATS (Allfeat Timestamp) pallet.

use std::sync::Arc;

use async_graphql::{Context, Object, Result};
use chrono::{DateTime, Utc};

use maestro_core::ports::Pagination;
use maestro_graphql::{convert_order, Order, PageInfo};

use super::models::{AtsVersion as AtsVersionModel, AtsWork as AtsWorkModel};
use super::storage::{AtsStorage, AtsWorkFilter};
use crate::graphql_utils::{parse_account, parse_hash, validate_pagination_first};

// -----------------------------------------------------------------------------
// GraphQL Types
// -----------------------------------------------------------------------------

/// An ATS (Allfeat Timestamp) work registered on-chain.
#[derive(async_graphql::SimpleObject)]
#[graphql(complex)]
pub struct AtsWork {
    /// Unique ATS identifier.
    pub id: i64,
    /// Current owner account (hex encoded).
    pub owner: String,
    /// Block number when the ATS was first registered.
    pub created_at_block: i64,
    /// Timestamp when the ATS was first registered.
    pub created_at_timestamp: Option<DateTime<Utc>>,
    /// Latest version number for this ATS.
    pub latest_version: i32,
}

#[async_graphql::ComplexObject]
impl AtsWork {
    /// Commitment of the latest version.
    async fn latest_commitment<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Option<String>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let version = storage.get_ats_version(self.id as u64, self.latest_version as u32).await?;
        Ok(version.map(|v| format!("0x{}", hex::encode(v.commitment))))
    }

    /// All versions of this ATS.
    async fn versions<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Vec<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let versions = storage.list_versions_for_ats(self.id as u64).await?;
        Ok(versions.into_iter().map(AtsVersion::from).collect())
    }
}

impl From<AtsWorkModel> for AtsWork {
    fn from(w: AtsWorkModel) -> Self {
        Self {
            id: w.id as i64,
            owner: format!("0x{}", hex::encode(w.owner.0)),
            created_at_block: w.created_at_block as i64,
            created_at_timestamp: w.created_at_timestamp,
            latest_version: w.latest_version as i32,
        }
    }
}

/// A version of an ATS work.
#[derive(async_graphql::SimpleObject)]
pub struct AtsVersion {
    /// Unique identifier: "{ats_id}-{version}".
    pub id: String,
    /// Parent ATS identifier.
    pub ats_id: i64,
    /// Version number.
    pub version: i32,
    /// Commitment for this version (hex encoded).
    pub commitment: String,
    /// Protocol version for this ATS version.
    pub protocol_version: i32,
    /// Block number when this version was registered.
    pub registered_at_block: i64,
    /// Timestamp when this version was registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// Event index within the block.
    pub event_index: i32,
}

impl From<AtsVersionModel> for AtsVersion {
    fn from(v: AtsVersionModel) -> Self {
        Self {
            id: v.id,
            ats_id: v.ats_id as i64,
            version: v.version as i32,
            commitment: format!("0x{}", hex::encode(v.commitment)),
            protocol_version: v.protocol_version as i32,
            registered_at_block: v.registered_at_block as i64,
            registered_at_timestamp: v.registered_at_timestamp,
            event_index: v.event_index as i32,
        }
    }
}

// -----------------------------------------------------------------------------
// Connection Types (Relay-style pagination)
// -----------------------------------------------------------------------------

#[derive(async_graphql::SimpleObject)]
pub struct AtsWorkEdge {
    pub node: AtsWork,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct AtsWorkConnection {
    pub edges: Vec<AtsWorkEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<AtsWorkModel>> for AtsWorkConnection {
    fn from(conn: maestro_core::ports::Connection<AtsWorkModel>) -> Self {
        Self {
            edges: conn
                .edges
                .into_iter()
                .map(|e| AtsWorkEdge {
                    node: AtsWork::from(e.node),
                    cursor: e.cursor.value,
                })
                .collect(),
            page_info: PageInfo {
                has_next_page: conn.page_info.has_next_page,
                has_previous_page: conn.page_info.has_previous_page,
                start_cursor: conn.page_info.start_cursor.map(|c| c.value),
                end_cursor: conn.page_info.end_cursor.map(|c| c.value),
            },
            total_count: conn.total_count,
        }
    }
}

// -----------------------------------------------------------------------------
// Statistics Types
// -----------------------------------------------------------------------------

/// Statistics about ATS on-chain data.
#[derive(async_graphql::SimpleObject)]
pub struct AtsStats {
    /// Total number of ATS works registered.
    pub total_ats_count: i64,
    /// Number of ATS works owned by a specific account (if queried).
    pub owner_ats_count: Option<i64>,
}

// -----------------------------------------------------------------------------
// ATS Query
// -----------------------------------------------------------------------------

/// GraphQL query root for the ATS (Allfeat Timestamp) pallet.
///
/// This can be merged with other query types using `#[derive(MergedObject)]`.
#[derive(Default)]
pub struct AtsQuery;

#[Object]
impl AtsQuery {
    /// Get an ATS work by ID.
    async fn ats_work<'ctx>(&self, ctx: &Context<'ctx>, id: i64) -> Result<Option<AtsWork>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let work = storage.get_ats_work(id as u64).await?;
        Ok(work.map(AtsWork::from))
    }

    /// List ATS works with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn ats_works<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        owner: Option<String>,
        created_at_block_gte: Option<i64>,
        created_at_block_lte: Option<i64>,
        created_at_timestamp_gte: Option<DateTime<Utc>>,
        created_at_timestamp_lte: Option<DateTime<Utc>>,
        #[graphql(default)] order: Order,
    ) -> Result<AtsWorkConnection> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;

        let filter = AtsWorkFilter {
            owner: owner.map(|s| parse_account(&s)).transpose()?,
            created_at_block_gte: created_at_block_gte.map(|n| n as u64),
            created_at_block_lte: created_at_block_lte.map(|n| n as u64),
            created_at_timestamp_gte,
            created_at_timestamp_lte,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage.list_ats_works(filter, pagination, convert_order(order)).await?;
        Ok(AtsWorkConnection::from(connection))
    }

    /// List all ATS works owned by an account.
    async fn ats_works_by_owner<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        owner: String,
    ) -> Result<Vec<AtsWork>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let account = parse_account(&owner)?;
        let works = storage.list_ats_by_owner(&account).await?;
        Ok(works.into_iter().map(AtsWork::from).collect())
    }

    /// Find an ATS version by commitment.
    async fn ats_version_by_commitment<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        commitment: String,
    ) -> Result<Option<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let hash = parse_hash(&commitment)?;
        let version = storage.find_by_commitment(&hash).await?;
        Ok(version.map(AtsVersion::from))
    }

    /// Get a specific version of an ATS.
    async fn ats_version<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        ats_id: i64,
        version: i32,
    ) -> Result<Option<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let v = storage.get_ats_version(ats_id as u64, version as u32).await?;
        Ok(v.map(AtsVersion::from))
    }

    /// List all versions for an ATS.
    async fn ats_versions<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        ats_id: i64,
    ) -> Result<Vec<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let versions = storage.list_versions_for_ats(ats_id as u64).await?;
        Ok(versions.into_iter().map(AtsVersion::from).collect())
    }

    /// Get ATS statistics.
    async fn ats_stats<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        owner: Option<String>,
    ) -> Result<AtsStats> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;

        let total_count = storage.count_ats_works().await?;

        let owner_count = if let Some(owner_str) = owner {
            let account = parse_account(&owner_str)?;
            Some(storage.count_ats_by_owner(&account).await? as i64)
        } else {
            None
        };

        Ok(AtsStats {
            total_ats_count: total_count as i64,
            owner_ats_count: owner_count,
        })
    }
}
