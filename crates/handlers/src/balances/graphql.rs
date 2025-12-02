//! GraphQL types and queries for the Balances pallet.

use std::sync::Arc;

use async_graphql::{Context, Object, Result};
use chrono::{DateTime, Utc};

use maestro_core::ports::Pagination;
use maestro_graphql::{Order, PageInfo};

use super::models::Transfer as TransferModel;
use super::storage::{BalancesStorage, TransferFilter};

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

/// Transfer type (Balances pallet).
#[derive(async_graphql::SimpleObject)]
pub struct Transfer {
    pub id: String,
    pub block_number: i64,
    pub block_hash: String,
    pub event_index: i32,
    pub extrinsic_index: Option<i32>,
    #[graphql(name = "from")]
    pub from_account: String,
    #[graphql(name = "to")]
    pub to_account: String,
    pub amount: String,
    pub success: bool,
    pub timestamp: Option<DateTime<Utc>>,
}

impl From<TransferModel> for Transfer {
    fn from(t: TransferModel) -> Self {
        Self {
            id: t.id,
            block_number: t.block_number as i64,
            block_hash: format!("0x{}", hex::encode(t.block_hash.0)),
            event_index: t.event_index as i32,
            extrinsic_index: t.extrinsic_index.map(|i| i as i32),
            from_account: format!("0x{}", hex::encode(t.from.0)),
            to_account: format!("0x{}", hex::encode(t.to.0)),
            amount: t.amount.to_string(),
            success: t.success,
            timestamp: t.timestamp,
        }
    }
}

// -----------------------------------------------------------------------------
// Connection Types (Relay-style pagination)
// -----------------------------------------------------------------------------

#[derive(async_graphql::SimpleObject)]
pub struct TransferEdge {
    pub node: Transfer,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct TransferConnection {
    pub edges: Vec<TransferEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<TransferModel>> for TransferConnection {
    fn from(conn: maestro_core::ports::Connection<TransferModel>) -> Self {
        Self {
            edges: conn
                .edges
                .into_iter()
                .map(|e| TransferEdge {
                    node: Transfer::from(e.node),
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
// Balances Query
// -----------------------------------------------------------------------------

/// GraphQL query root for the Balances pallet.
///
/// This can be merged with other query types using `#[derive(MergedObject)]`.
#[derive(Default)]
pub struct BalancesQuery;

#[Object]
impl BalancesQuery {
    /// Get a transfer by ID.
    async fn transfer<'ctx>(&self, ctx: &Context<'ctx>, id: String) -> Result<Option<Transfer>> {
        let balances = ctx.data::<Arc<dyn BalancesStorage>>()?;

        let transfer = balances.get_transfer(&id).await?;
        Ok(transfer.map(Transfer::from))
    }

    /// List transfers with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn transfers<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        block_number_gte: Option<i64>,
        block_number_lte: Option<i64>,
        from: Option<String>,
        to: Option<String>,
        account: Option<String>,
        #[graphql(default)] order: Order,
    ) -> Result<TransferConnection> {
        let balances = ctx.data::<Arc<dyn BalancesStorage>>()?;

        let filter = TransferFilter {
            block_number_gte: block_number_gte.map(|n| n as u64),
            block_number_lte: block_number_lte.map(|n| n as u64),
            from: from.map(|s| parse_account(&s)).transpose()?,
            to: to.map(|s| parse_account(&s)).transpose()?,
            account: account.map(|s| parse_account(&s)).transpose()?,
            ..Default::default()
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = balances
            .list_transfers(filter, pagination, order.into())
            .await?;

        Ok(TransferConnection::from(connection))
    }

    /// List transfers for a specific block.
    async fn transfers_for_block<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        block_number: i64,
    ) -> Result<Vec<Transfer>> {
        let balances = ctx.data::<Arc<dyn BalancesStorage>>()?;

        let transfers = balances
            .list_transfers_for_block(block_number as u64)
            .await?;

        Ok(transfers.into_iter().map(Transfer::from).collect())
    }
}
