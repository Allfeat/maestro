//! GraphQL schema definition.
//!
//! This module provides the core GraphQL schema for the indexer,
//! handling blocks, extrinsics, and events (frame_system).

use std::sync::Arc;

use async_graphql::{Context, EmptyMutation, EmptySubscription, Object, Result, Schema, SchemaBuilder};
use chrono::{DateTime, Utc};

use maestro_core::ports::{
    BlockFilter, EventFilter, ExtrinsicFilter, OrderDirection, Pagination, Repositories,
};

use crate::types::MaestroSchema;

// -----------------------------------------------------------------------------
// Schema Configuration
// -----------------------------------------------------------------------------

/// Maximum query depth to prevent deeply nested queries (DoS protection).
/// Note: GraphQL introspection requires depth ~13, so we use 15 to allow it.
pub const MAX_QUERY_DEPTH: usize = 15;

/// Maximum query complexity score (DoS protection).
/// Each field has a default complexity of 1, nested objects multiply.
pub const MAX_QUERY_COMPLEXITY: usize = 500;

// -----------------------------------------------------------------------------
// Schema Builder
// -----------------------------------------------------------------------------

/// Build a GraphQL schema with just the core query (blocks, events, extrinsics).
///
/// Use this when no handler extensions are needed.
/// Includes query depth and complexity limits for DoS protection.
pub fn build_core_schema<R: Repositories + 'static>(repositories: Arc<R>) -> MaestroSchema {
    let repos: Arc<dyn Repositories> = repositories;
    Schema::build(CoreQuery, EmptyMutation, EmptySubscription)
        .data(repos)
        .limit_depth(MAX_QUERY_DEPTH)
        .limit_complexity(MAX_QUERY_COMPLEXITY)
        .finish()
}

/// Create a schema builder with repositories data.
///
/// Use this to build a schema with merged query types from bundles.
/// Remember to call `.limit_depth()` and `.limit_complexity()` before `.finish()`.
///
/// # Example
///
/// ```ignore
/// use async_graphql::MergedObject;
/// use maestro_graphql::{schema_builder, CoreQuery, MAX_QUERY_DEPTH, MAX_QUERY_COMPLEXITY};
/// use maestro_handlers::balances::BalancesQuery;
///
/// #[derive(MergedObject, Default)]
/// struct Query(CoreQuery, BalancesQuery);
///
/// let schema = schema_builder(repositories)
///     .data(balances_storage)
///     .limit_depth(MAX_QUERY_DEPTH)
///     .limit_complexity(MAX_QUERY_COMPLEXITY)
///     .finish();
/// ```
pub fn schema_builder<R: Repositories + 'static>(
    repositories: Arc<R>,
) -> SchemaBuilder<CoreQuery, EmptyMutation, EmptySubscription> {
    let repos: Arc<dyn Repositories> = repositories;
    Schema::build(CoreQuery, EmptyMutation, EmptySubscription)
        .data(repos)
}

/// Build a schema with a merged query type.
///
/// This is the preferred way to build a schema with bundle extensions.
/// Includes query depth and complexity limits for DoS protection.
pub fn build_schema_with_query<Q, R>(
    query: Q,
    repositories: Arc<R>,
) -> Schema<Q, EmptyMutation, EmptySubscription>
where
    Q: async_graphql::ObjectType + 'static,
    R: Repositories + 'static,
{
    let repos: Arc<dyn Repositories> = repositories;
    Schema::build(query, EmptyMutation, EmptySubscription)
        .data(repos)
        .limit_depth(MAX_QUERY_DEPTH)
        .limit_complexity(MAX_QUERY_COMPLEXITY)
        .finish()
}

// -----------------------------------------------------------------------------
// Core Query (frame_system)
// -----------------------------------------------------------------------------

/// Core query root for the indexer (blocks, extrinsics, events).
///
/// This provides access to frame_system data and can be merged with
/// bundle-specific queries using `#[derive(MergedObject)]`.
#[derive(Default)]
pub struct CoreQuery;

#[Object]
impl CoreQuery {
    /// Get indexer status and statistics.
    async fn status<'ctx>(&self, ctx: &Context<'ctx>) -> Result<IndexerStatus> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let latest_block = repos.blocks().latest_block_number().await?;
        let cursor = repos.cursor().get_cursor("").await?;

        Ok(IndexerStatus {
            latest_indexed_block: latest_block,
            last_updated: cursor.map(|c| c.updated_at),
        })
    }

    /// Get a block by number.
    async fn block<'ctx>(&self, ctx: &Context<'ctx>, number: i64) -> Result<Option<Block>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let block = repos.blocks().get_block(number as u64).await?;
        Ok(block.map(Block::from))
    }

    /// Get a block by hash.
    async fn block_by_hash<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        hash: String,
    ) -> Result<Option<Block>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let hash_bytes = parse_hash(&hash)?;
        let block_hash = maestro_core::models::BlockHash(hash_bytes);
        let block = repos.blocks().get_block_by_hash(&block_hash).await?;
        Ok(block.map(Block::from))
    }

    /// List blocks with pagination.
    async fn blocks<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        number_gte: Option<i64>,
        number_lte: Option<i64>,
        #[graphql(default)] order: Order,
    ) -> Result<BlockConnection> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let filter = BlockFilter {
            number_gte: number_gte.map(|n| n as u64),
            number_lte: number_lte.map(|n| n as u64),
            ..Default::default()
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = repos
            .blocks()
            .list_blocks(filter, pagination, order.into())
            .await?;

        Ok(BlockConnection::from(connection))
    }

    /// Get an extrinsic by ID.
    async fn extrinsic<'ctx>(&self, ctx: &Context<'ctx>, id: String) -> Result<Option<Extrinsic>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let ext = repos.extrinsics().get_extrinsic(&id).await?;
        Ok(ext.map(Extrinsic::from))
    }

    /// List extrinsics with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn extrinsics<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        block_number: Option<i64>,
        pallet: Option<String>,
        call: Option<String>,
        signer: Option<String>,
        success: Option<bool>,
        #[graphql(default)] order: Order,
    ) -> Result<ExtrinsicConnection> {
        validate_filter_string(&pallet, "pallet")?;
        validate_filter_string(&call, "call")?;

        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let filter = ExtrinsicFilter {
            block_number: block_number.map(|n| n as u64),
            pallet,
            call,
            signer: signer.map(|s| parse_account(&s)).transpose()?,
            success,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = repos
            .extrinsics()
            .list_extrinsics(filter, pagination, order.into())
            .await?;

        Ok(ExtrinsicConnection::from(connection))
    }

    /// Get an event by ID.
    async fn event<'ctx>(&self, ctx: &Context<'ctx>, id: String) -> Result<Option<Event>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let event = repos.events().get_event(&id).await?;
        Ok(event.map(Event::from))
    }

    /// List events with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn events<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        block_number: Option<i64>,
        extrinsic_index: Option<i32>,
        pallet: Option<String>,
        name: Option<String>,
        #[graphql(default)] order: Order,
    ) -> Result<EventConnection> {
        validate_filter_string(&pallet, "pallet")?;
        validate_filter_string(&name, "name")?;

        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let filter = EventFilter {
            block_number: block_number.map(|n| n as u64),
            extrinsic_index: extrinsic_index.map(|i| i as u32),
            pallet,
            name,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = repos
            .events()
            .list_events(filter, pagination, order.into())
            .await?;

        Ok(EventConnection::from(connection))
    }

    /// List events for a specific block.
    async fn events_for_block<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        block_number: i64,
    ) -> Result<Vec<Event>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let events = repos
            .events()
            .list_events_for_block(block_number as u64)
            .await?;

        Ok(events.into_iter().map(Event::from).collect())
    }

    /// List extrinsics for a specific block.
    async fn extrinsics_for_block<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        block_number: i64,
    ) -> Result<Vec<Extrinsic>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let exts = repos
            .extrinsics()
            .list_extrinsics_for_block(block_number as u64)
            .await?;

        Ok(exts.into_iter().map(Extrinsic::from).collect())
    }
}

// -----------------------------------------------------------------------------
// GraphQL Types
// -----------------------------------------------------------------------------

/// Ordering direction.
#[derive(async_graphql::Enum, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    Desc,
    Asc,
}

impl From<Order> for OrderDirection {
    fn from(order: Order) -> Self {
        match order {
            Order::Asc => OrderDirection::Asc,
            Order::Desc => OrderDirection::Desc,
        }
    }
}

/// Indexer status.
#[derive(async_graphql::SimpleObject)]
pub struct IndexerStatus {
    pub latest_indexed_block: Option<u64>,
    pub last_updated: Option<DateTime<Utc>>,
}

/// Block type.
#[derive(async_graphql::SimpleObject)]
pub struct Block {
    pub number: i64,
    pub hash: String,
    pub parent_hash: String,
    pub state_root: String,
    pub extrinsics_root: String,
    pub author: Option<String>,
    pub timestamp: Option<DateTime<Utc>>,
    pub extrinsic_count: i32,
    pub event_count: i32,
    pub indexed_at: DateTime<Utc>,
}

impl From<maestro_core::models::Block> for Block {
    fn from(b: maestro_core::models::Block) -> Self {
        Self {
            number: b.number as i64,
            hash: to_hex(&b.hash.0),
            parent_hash: to_hex(&b.parent_hash.0),
            state_root: to_hex(&b.state_root.0),
            extrinsics_root: to_hex(&b.extrinsics_root.0),
            author: b.author.map(|a| to_hex(&a.0)),
            timestamp: b.timestamp,
            extrinsic_count: b.extrinsic_count as i32,
            event_count: b.event_count as i32,
            indexed_at: b.indexed_at,
        }
    }
}

/// Extrinsic type.
#[derive(async_graphql::SimpleObject)]
pub struct Extrinsic {
    pub id: String,
    pub block_number: i64,
    pub block_hash: String,
    pub index: i32,
    pub pallet: String,
    pub call: String,
    pub signer: Option<String>,
    pub success: bool,
    pub error: Option<String>,
    pub args: serde_json::Value,
    pub tip: Option<String>,
    pub nonce: Option<i32>,
}

impl From<maestro_core::models::Extrinsic> for Extrinsic {
    fn from(e: maestro_core::models::Extrinsic) -> Self {
        Self {
            id: e.id,
            block_number: e.block_number as i64,
            block_hash: to_hex(&e.block_hash.0),
            index: e.index as i32,
            pallet: e.pallet,
            call: e.call,
            signer: e.signer.map(|s| to_hex(&s.0)),
            success: matches!(e.status, maestro_core::models::ExtrinsicStatus::Success),
            error: e.error,
            args: e.args,
            tip: e.tip.map(|t| t.to_string()),
            nonce: e.nonce.map(|n| n as i32),
        }
    }
}

/// Event type.
#[derive(async_graphql::SimpleObject)]
pub struct Event {
    pub id: String,
    pub block_number: i64,
    pub block_hash: String,
    pub index: i32,
    pub extrinsic_index: Option<i32>,
    pub pallet: String,
    pub name: String,
    pub data: serde_json::Value,
}

impl From<maestro_core::models::Event> for Event {
    fn from(e: maestro_core::models::Event) -> Self {
        Self {
            id: e.id,
            block_number: e.block_number as i64,
            block_hash: to_hex(&e.block_hash.0),
            index: e.index as i32,
            extrinsic_index: e.extrinsic_index.map(|i| i as i32),
            pallet: e.pallet,
            name: e.name,
            data: e.data,
        }
    }
}

// -----------------------------------------------------------------------------
// Connection Types (Relay-style pagination)
// -----------------------------------------------------------------------------

#[derive(async_graphql::SimpleObject)]
pub struct PageInfo {
    pub has_next_page: bool,
    pub has_previous_page: bool,
    pub start_cursor: Option<String>,
    pub end_cursor: Option<String>,
}

/// Generate Relay-style connection types (Edge + Connection) with From impl.
macro_rules! define_connection {
    ($node:ty, $core_model:ty, $edge:ident, $connection:ident) => {
        #[derive(async_graphql::SimpleObject)]
        pub struct $edge {
            pub node: $node,
            pub cursor: String,
        }

        #[derive(async_graphql::SimpleObject)]
        pub struct $connection {
            pub edges: Vec<$edge>,
            pub page_info: PageInfo,
            pub total_count: Option<i64>,
        }

        impl From<maestro_core::ports::Connection<$core_model>> for $connection {
            fn from(conn: maestro_core::ports::Connection<$core_model>) -> Self {
                Self {
                    edges: conn
                        .edges
                        .into_iter()
                        .map(|e| $edge {
                            node: <$node>::from(e.node),
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
    };
}

define_connection!(Block, maestro_core::models::Block, BlockEdge, BlockConnection);
define_connection!(Extrinsic, maestro_core::models::Extrinsic, ExtrinsicEdge, ExtrinsicConnection);
define_connection!(Event, maestro_core::models::Event, EventEdge, EventConnection);

// -----------------------------------------------------------------------------
// Helpers & Validation
// -----------------------------------------------------------------------------

/// Convert bytes to 0x-prefixed hex string.
fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Maximum length for hash strings (64 hex chars + "0x" prefix).
const MAX_HASH_LENGTH: usize = 66;
/// Maximum length for string filter parameters.
const MAX_FILTER_STRING_LENGTH: usize = 128;
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

/// Validate a filter string parameter.
fn validate_filter_string(s: &Option<String>, field_name: &str) -> Result<()> {
    if let Some(value) = s {
        if value.len() > MAX_FILTER_STRING_LENGTH {
            return Err(async_graphql::Error::new(format!(
                "{} too long: maximum {} characters allowed",
                field_name, MAX_FILTER_STRING_LENGTH
            )));
        }
        if value.is_empty() {
            return Err(async_graphql::Error::new(format!(
                "{} cannot be empty",
                field_name
            )));
        }
    }
    Ok(())
}

/// Validate and normalize pagination first parameter.
fn validate_pagination_first(first: Option<i32>) -> i32 {
    first.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Tests de validation critiques - protègent contre les injections/DoS

    #[test]
    fn test_parse_hash_rejects_invalid_input() {
        // Trop long (DoS prevention)
        assert!(parse_hash(&"ab".repeat(100)).is_err());
        // Caractères non-hex (injection prevention)
        assert!(parse_hash("0x<script>alert(1)</script>").is_err());
        // Mauvaise longueur
        assert!(parse_hash(&"ab".repeat(16)).is_err());
    }

    #[test]
    fn test_parse_hash_accepts_both_formats() {
        let with_prefix = parse_hash(&("0x".to_string() + &"ab".repeat(32)));
        let without_prefix = parse_hash(&"ab".repeat(32));
        assert!(with_prefix.is_ok());
        assert!(without_prefix.is_ok());
        assert_eq!(with_prefix.unwrap(), without_prefix.unwrap());
    }

    #[test]
    fn test_validate_filter_string_boundaries() {
        // Vide = erreur (évite les requêtes inutiles)
        assert!(validate_filter_string(&Some("".into()), "x").is_err());
        // Trop long = erreur (DoS prevention)
        assert!(validate_filter_string(&Some("x".repeat(200)), "x").is_err());
        // None = OK (optionnel)
        assert!(validate_filter_string(&None, "x").is_ok());
    }

    #[test]
    fn test_pagination_clamping() {
        // Valeurs négatives/zéro clampées à 1
        assert_eq!(validate_pagination_first(Some(-100)), 1);
        assert_eq!(validate_pagination_first(Some(0)), 1);
        // Valeurs trop grandes clampées à MAX
        assert_eq!(validate_pagination_first(Some(10000)), MAX_PAGE_SIZE);
    }

    // Test de conversion critique - vérifie le format de sortie GraphQL

    #[test]
    fn test_extrinsic_status_conversion() {
        use maestro_core::models::{BlockHash, Extrinsic as CoreExtrinsic, ExtrinsicStatus};

        let make_ext = |status| CoreExtrinsic {
            id: "1-0".into(),
            block_number: 1,
            block_hash: BlockHash([0; 32]),
            index: 0,
            pallet: "Test".into(),
            call: "test".into(),
            signer: None,
            status,
            error: None,
            args: serde_json::json!({}),
            raw: "".into(),
            tip: None,
            nonce: None,
        };

        // Vérifie que le mapping success est correct
        assert!(Extrinsic::from(make_ext(ExtrinsicStatus::Success)).success);
        assert!(!Extrinsic::from(make_ext(ExtrinsicStatus::Failed)).success);
    }
}
