//! Port traits for data repositories.
//!
//! These traits define the storage interface used by the domain layer.
//! Implementations live in the infrastructure layer (e.g., `maestro-storage`).

use async_trait::async_trait;

use crate::error::StorageResult;
use crate::models::{AccountId, Block, BlockHash, Event, Extrinsic, IndexerCursor};

use super::pagination::{Connection, OrderDirection, Pagination};

// =============================================================================
// Filter Types
// =============================================================================

/// Filter options for block queries.
#[derive(Debug, Clone, Default)]
pub struct BlockFilter {
    pub number_gte: Option<u64>,
    pub number_lte: Option<u64>,
    pub hash: Option<BlockHash>,
    pub has_extrinsics: Option<bool>,
}

/// Filter options for extrinsic queries.
#[derive(Debug, Clone, Default)]
pub struct ExtrinsicFilter {
    pub block_number: Option<u64>,
    pub pallet: Option<String>,
    pub call: Option<String>,
    pub signer: Option<AccountId>,
    pub success: Option<bool>,
}

/// Filter options for event queries.
#[derive(Debug, Clone, Default)]
pub struct EventFilter {
    pub block_number: Option<u64>,
    pub extrinsic_index: Option<u32>,
    pub pallet: Option<String>,
    pub name: Option<String>,
}

// =============================================================================
// Repository Traits
// =============================================================================

/// Repository for block data.
#[async_trait]
pub trait BlockRepository: Send + Sync {
    /// Insert a batch of blocks.
    async fn insert_blocks(&self, blocks: &[Block]) -> StorageResult<()>;

    /// Get block by number.
    async fn get_block(&self, number: u64) -> StorageResult<Option<Block>>;

    /// Get block by hash.
    async fn get_block_by_hash(&self, hash: &BlockHash) -> StorageResult<Option<Block>>;

    /// List blocks with pagination and filtering.
    async fn list_blocks(
        &self,
        filter: BlockFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Block>>;

    /// Get latest indexed block number.
    async fn latest_block_number(&self) -> StorageResult<Option<u64>>;

    /// Delete blocks from a given number (for reorg handling).
    async fn delete_blocks_from(&self, from_number: u64) -> StorageResult<u64>;
}

/// Repository for extrinsic data.
#[async_trait]
pub trait ExtrinsicRepository: Send + Sync {
    /// Insert a batch of extrinsics.
    async fn insert_extrinsics(&self, extrinsics: &[Extrinsic]) -> StorageResult<()>;

    /// Get extrinsic by ID.
    async fn get_extrinsic(&self, id: &str) -> StorageResult<Option<Extrinsic>>;

    /// List extrinsics for a block.
    async fn list_extrinsics_for_block(&self, block_number: u64) -> StorageResult<Vec<Extrinsic>>;

    /// List extrinsics with pagination and filtering.
    async fn list_extrinsics(
        &self,
        filter: ExtrinsicFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Extrinsic>>;

    /// Delete extrinsics from a given block number.
    async fn delete_extrinsics_from(&self, from_block: u64) -> StorageResult<u64>;
}

/// Repository for event data.
#[async_trait]
pub trait EventRepository: Send + Sync {
    /// Insert a batch of events.
    async fn insert_events(&self, events: &[Event]) -> StorageResult<()>;

    /// Get event by ID.
    async fn get_event(&self, id: &str) -> StorageResult<Option<Event>>;

    /// List events for a block.
    async fn list_events_for_block(&self, block_number: u64) -> StorageResult<Vec<Event>>;

    /// List events for an extrinsic.
    async fn list_events_for_extrinsic(
        &self,
        block_number: u64,
        extrinsic_index: u32,
    ) -> StorageResult<Vec<Event>>;

    /// List events with pagination and filtering.
    async fn list_events(
        &self,
        filter: EventFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Event>>;

    /// Delete events from a given block number.
    async fn delete_events_from(&self, from_block: u64) -> StorageResult<u64>;
}

/// Repository for indexer cursor state.
#[async_trait]
pub trait CursorRepository: Send + Sync {
    /// Get current cursor for a chain.
    async fn get_cursor(&self, chain_id: &str) -> StorageResult<Option<IndexerCursor>>;

    /// Get any existing cursor (for chain mismatch detection).
    async fn get_any_cursor(&self) -> StorageResult<Option<IndexerCursor>>;

    /// Update cursor (upsert).
    async fn set_cursor(&self, cursor: &IndexerCursor) -> StorageResult<()>;
}

// =============================================================================
// Composite Repository
// =============================================================================

/// Data bundle for atomic block persistence.
#[derive(Debug)]
pub struct BlockData<'a> {
    pub block: &'a Block,
    pub extrinsics: &'a [Extrinsic],
    pub events: &'a [Event],
    pub cursor: &'a IndexerCursor,
}

/// Combined repository access for the indexer.
///
/// This trait provides access to all individual repositories and
/// atomic operations that span multiple tables.
#[async_trait]
pub trait Repositories: Send + Sync {
    /// Access the block repository.
    fn blocks(&self) -> &dyn BlockRepository;

    /// Access the extrinsic repository.
    fn extrinsics(&self) -> &dyn ExtrinsicRepository;

    /// Access the event repository.
    fn events(&self) -> &dyn EventRepository;

    /// Access the cursor repository.
    fn cursor(&self) -> &dyn CursorRepository;

    /// Persist block data atomically in a single transaction.
    ///
    /// This persists the block, its extrinsics, events, and updates
    /// the cursor. If any operation fails, everything is rolled back.
    async fn persist_block_atomic(&self, data: BlockData<'_>) -> StorageResult<()>;

    /// Delete all data from a given block number atomically.
    ///
    /// Used for chain reorganization recovery. Deletes blocks,
    /// extrinsics, events, and updates the cursor in a single transaction.
    async fn delete_from_block_atomic(
        &self,
        from_number: u64,
        chain_id: &str,
    ) -> StorageResult<u64>;
}
