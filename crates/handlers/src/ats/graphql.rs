//! GraphQL types and queries for the ATS (Allfeat Timestamp) pallet.

use std::sync::Arc;

use async_graphql::{Context, Object, Result};
use chrono::{DateTime, Utc};

use maestro_core::ports::Pagination;
use maestro_graphql::{Order, PageInfo};

use super::models::{
    AtsOwnershipTransfer as AtsOwnershipTransferModel, AtsVerificationKeyUpdate as AtsVkUpdateModel,
    AtsVersion as AtsVersionModel, AtsWork as AtsWorkModel,
};
use super::storage::{AtsStorage, AtsTransferFilter, AtsWorkFilter};

// -----------------------------------------------------------------------------
// Helper functions
// -----------------------------------------------------------------------------

/// Maximum length for hash strings (64 hex chars + "0x" prefix).
const MAX_HASH_LENGTH: usize = 66;
/// Maximum page size for pagination.
const MAX_PAGE_SIZE: i32 = 100;
/// Default page size for pagination.
const DEFAULT_PAGE_SIZE: i32 = 20;

/// Parse and validate a hash string.
fn parse_hash(s: &str) -> Result<[u8; 32]> {
    if s.len() > MAX_HASH_LENGTH {
        return Err(async_graphql::Error::new(format!(
            "Hash too long: maximum {} characters allowed",
            MAX_HASH_LENGTH
        )));
    }

    let s = s.strip_prefix("0x").unwrap_or(s);

    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(async_graphql::Error::new(
            "Invalid hash: must contain only hexadecimal characters",
        ));
    }

    let bytes =
        hex::decode(s).map_err(|e| async_graphql::Error::new(format!("Invalid hash: {}", e)))?;

    bytes
        .try_into()
        .map_err(|_| async_graphql::Error::new("Hash must be exactly 32 bytes (64 hex characters)"))
}

/// Parse and validate an account address.
fn parse_account(s: &str) -> Result<maestro_core::models::AccountId> {
    let bytes = parse_hash(s)?;
    Ok(maestro_core::models::AccountId(bytes))
}

/// Validate and normalize pagination first parameter.
fn validate_pagination_first(first: Option<i32>) -> i32 {
    first.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

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
    /// All versions of this ATS.
    async fn versions<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Vec<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let versions = storage.list_versions_for_ats(self.id as u64).await?;
        Ok(versions.into_iter().map(AtsVersion::from).collect())
    }

    /// Ownership transfer history for this ATS.
    async fn transfers<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Vec<AtsOwnershipTransfer>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let transfers = storage.list_transfers_for_ats(self.id as u64).await?;
        Ok(transfers.into_iter().map(AtsOwnershipTransfer::from).collect())
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
    /// Hash commitment for this version (hex encoded).
    pub hash_commitment: String,
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
            hash_commitment: format!("0x{}", hex::encode(v.hash_commitment)),
            registered_at_block: v.registered_at_block as i64,
            registered_at_timestamp: v.registered_at_timestamp,
            event_index: v.event_index as i32,
        }
    }
}

/// An ownership transfer (claim) of an ATS.
#[derive(async_graphql::SimpleObject)]
pub struct AtsOwnershipTransfer {
    /// Unique identifier.
    pub id: String,
    /// ATS identifier that was transferred.
    pub ats_id: i64,
    /// Previous owner account (hex encoded).
    pub old_owner: String,
    /// New owner account (hex encoded).
    pub new_owner: String,
    /// Block number of the transfer.
    pub block_number: i64,
    /// Event index within the block.
    pub event_index: i32,
    /// Timestamp of the transfer.
    pub timestamp: Option<DateTime<Utc>>,
}

impl From<AtsOwnershipTransferModel> for AtsOwnershipTransfer {
    fn from(t: AtsOwnershipTransferModel) -> Self {
        Self {
            id: t.id,
            ats_id: t.ats_id as i64,
            old_owner: format!("0x{}", hex::encode(t.old_owner.0)),
            new_owner: format!("0x{}", hex::encode(t.new_owner.0)),
            block_number: t.block_number as i64,
            event_index: t.event_index as i32,
            timestamp: t.timestamp,
        }
    }
}

/// A verification key update event.
#[derive(async_graphql::SimpleObject)]
pub struct AtsVerificationKeyUpdate {
    /// Unique identifier.
    pub id: String,
    /// The verification key (hex encoded).
    pub vk: String,
    /// Block number when the key was updated.
    pub block_number: i64,
    /// Event index within the block.
    pub event_index: i32,
    /// Timestamp of the update.
    pub timestamp: Option<DateTime<Utc>>,
}

impl From<AtsVkUpdateModel> for AtsVerificationKeyUpdate {
    fn from(v: AtsVkUpdateModel) -> Self {
        Self {
            id: v.id,
            vk: format!("0x{}", hex::encode(&v.vk)),
            block_number: v.block_number as i64,
            event_index: v.event_index as i32,
            timestamp: v.timestamp,
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

#[derive(async_graphql::SimpleObject)]
pub struct AtsOwnershipTransferEdge {
    pub node: AtsOwnershipTransfer,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct AtsOwnershipTransferConnection {
    pub edges: Vec<AtsOwnershipTransferEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<AtsOwnershipTransferModel>>
    for AtsOwnershipTransferConnection
{
    fn from(conn: maestro_core::ports::Connection<AtsOwnershipTransferModel>) -> Self {
        Self {
            edges: conn
                .edges
                .into_iter()
                .map(|e| AtsOwnershipTransferEdge {
                    node: AtsOwnershipTransfer::from(e.node),
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
        #[graphql(default)] order: Order,
    ) -> Result<AtsWorkConnection> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;

        let filter = AtsWorkFilter {
            owner: owner.map(|s| parse_account(&s)).transpose()?,
            created_at_block_gte: created_at_block_gte.map(|n| n as u64),
            created_at_block_lte: created_at_block_lte.map(|n| n as u64),
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage.list_ats_works(filter, pagination, order.into()).await?;
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

    /// Find an ATS version by hash commitment.
    async fn ats_version_by_hash<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        hash_commitment: String,
    ) -> Result<Option<AtsVersion>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let hash = parse_hash(&hash_commitment)?;
        let version = storage.find_by_hash_commitment(&hash).await?;
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

    /// List ATS ownership transfers with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn ats_transfers<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        ats_id: Option<i64>,
        old_owner: Option<String>,
        new_owner: Option<String>,
        #[graphql(desc = "Filter by either old_owner or new_owner")]
        account: Option<String>,
        #[graphql(default)] order: Order,
    ) -> Result<AtsOwnershipTransferConnection> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;

        let filter = AtsTransferFilter {
            ats_id: ats_id.map(|n| n as u64),
            old_owner: old_owner.map(|s| parse_account(&s)).transpose()?,
            new_owner: new_owner.map(|s| parse_account(&s)).transpose()?,
            account: account.map(|s| parse_account(&s)).transpose()?,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage.list_transfers(filter, pagination, order.into()).await?;
        Ok(AtsOwnershipTransferConnection::from(connection))
    }

    /// Get the latest verification key.
    async fn ats_latest_verification_key<'ctx>(
        &self,
        ctx: &Context<'ctx>,
    ) -> Result<Option<AtsVerificationKeyUpdate>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let vk = storage.get_latest_vk().await?;
        Ok(vk.map(AtsVerificationKeyUpdate::from))
    }

    /// List all verification key updates.
    async fn ats_verification_key_history<'ctx>(
        &self,
        ctx: &Context<'ctx>,
    ) -> Result<Vec<AtsVerificationKeyUpdate>> {
        let storage = ctx.data::<Arc<dyn AtsStorage>>()?;
        let updates = storage.list_vk_updates().await?;
        Ok(updates.into_iter().map(AtsVerificationKeyUpdate::from).collect())
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
