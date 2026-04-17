//! GraphQL schema definition.
//!
//! This module provides the core GraphQL schema for the indexer,
//! handling blocks, extrinsics, and events (frame_system).

use std::sync::Arc;

use async_graphql::{Context, Object, Result};

use maestro_core::ports::{
    BlockFilter, EventFilter, ExtrinsicFilter, OrderDirection, Pagination, Repositories,
};

use crate::validation::{
    parse_account, parse_hash, validate_cursor, validate_filter_string, validate_pagination_first,
};

// Re-export schema types from the schema crate
pub use maestro_graphql_schema::{
    Block, BlockConnection, BlockEdge, Event, EventConnection, EventEdge, Extrinsic,
    ExtrinsicConnection, ExtrinsicEdge, IndexerStatus, Order, PageInfo,
};

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
        Ok(block.map(convert_block))
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
        Ok(block.map(convert_block))
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
        validate_cursor(&after, "after")?;

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
            .list_blocks(filter, pagination, convert_order(order))
            .await?;

        Ok(convert_block_connection(connection))
    }

    /// Get an extrinsic by ID.
    async fn extrinsic<'ctx>(&self, ctx: &Context<'ctx>, id: String) -> Result<Option<Extrinsic>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let ext = repos.extrinsics().get_extrinsic(&id).await?;
        Ok(ext.map(convert_extrinsic))
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
        validate_cursor(&after, "after")?;
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
            .list_extrinsics(filter, pagination, convert_order(order))
            .await?;

        Ok(convert_extrinsic_connection(connection))
    }

    /// Get an event by ID.
    async fn event<'ctx>(&self, ctx: &Context<'ctx>, id: String) -> Result<Option<Event>> {
        let repos = ctx.data::<Arc<dyn Repositories>>()?;

        let event = repos.events().get_event(&id).await?;
        Ok(event.map(convert_event))
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
        validate_cursor(&after, "after")?;
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
            .list_events(filter, pagination, convert_order(order))
            .await?;

        Ok(convert_event_connection(connection))
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

        Ok(events.into_iter().map(convert_event).collect())
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

        Ok(exts.into_iter().map(convert_extrinsic).collect())
    }
}

// -----------------------------------------------------------------------------
// Conversion Functions
// -----------------------------------------------------------------------------

/// Convert bytes to 0x-prefixed hex string.
fn to_hex(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(bytes))
}

/// Convert Order enum to OrderDirection.
pub fn convert_order(order: Order) -> OrderDirection {
    match order {
        Order::Asc => OrderDirection::Asc,
        Order::Desc => OrderDirection::Desc,
    }
}

/// Convert core Block model to GraphQL Block type.
pub fn convert_block(b: maestro_core::models::Block) -> Block {
    Block {
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

/// Convert core Extrinsic model to GraphQL Extrinsic type.
pub fn convert_extrinsic(e: maestro_core::models::Extrinsic) -> Extrinsic {
    Extrinsic {
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

/// Convert core Event model to GraphQL Event type.
pub fn convert_event(e: maestro_core::models::Event) -> Event {
    Event {
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

/// Convert a core `PageInfo` to the GraphQL `PageInfo` (shared across all connections).
fn convert_page_info(info: maestro_core::ports::PageInfo) -> PageInfo {
    PageInfo {
        has_next_page: info.has_next_page,
        has_previous_page: info.has_previous_page,
        start_cursor: info.start_cursor.map(|c| c.value),
        end_cursor: info.end_cursor.map(|c| c.value),
    }
}

/// Convert core Block connection to GraphQL BlockConnection.
pub fn convert_block_connection(
    conn: maestro_core::ports::Connection<maestro_core::models::Block>,
) -> BlockConnection {
    BlockConnection {
        edges: conn
            .edges
            .into_iter()
            .map(|e| BlockEdge {
                node: convert_block(e.node),
                cursor: e.cursor.value,
            })
            .collect(),
        page_info: convert_page_info(conn.page_info),
        total_count: conn.total_count,
    }
}

/// Convert core Extrinsic connection to GraphQL ExtrinsicConnection.
pub fn convert_extrinsic_connection(
    conn: maestro_core::ports::Connection<maestro_core::models::Extrinsic>,
) -> ExtrinsicConnection {
    ExtrinsicConnection {
        edges: conn
            .edges
            .into_iter()
            .map(|e| ExtrinsicEdge {
                node: convert_extrinsic(e.node),
                cursor: e.cursor.value,
            })
            .collect(),
        page_info: convert_page_info(conn.page_info),
        total_count: conn.total_count,
    }
}

/// Convert core Event connection to GraphQL EventConnection.
pub fn convert_event_connection(
    conn: maestro_core::ports::Connection<maestro_core::models::Event>,
) -> EventConnection {
    EventConnection {
        edges: conn
            .edges
            .into_iter()
            .map(|e| EventEdge {
                node: convert_event(e.node),
                cursor: e.cursor.value,
            })
            .collect(),
        page_info: convert_page_info(conn.page_info),
        total_count: conn.total_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

        assert!(convert_extrinsic(make_ext(ExtrinsicStatus::Success)).success);
        assert!(!convert_extrinsic(make_ext(ExtrinsicStatus::Failed)).success);
    }
}
