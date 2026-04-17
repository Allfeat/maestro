//! Shared stubs for submodule tests: a minimal `BlockSource` and a noop
//! `Repositories` impl. Kept in one place so every submodule that tests the
//! indexer doesn't redefine them.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use async_trait::async_trait;
use futures::stream;

use crate::events::EventBus;
use crate::models::{Block, BlockHash, Event, Extrinsic, IndexerCursor};
use crate::ports::{
    BlockData, BlockFilter, BlockMode, BlockRepository, BlockSource, Connection, CursorRepository,
    EventFilter, EventRepository, ExtrinsicFilter, ExtrinsicRepository, FinalizedBlockStream,
    FinalizedHead, HandlerRegistry, OrderDirection, Pagination, RawBlock, Repositories,
};

use super::{BackfillConfig, IndexerConfig, IndexerService};

pub fn mk_raw(number: u64) -> RawBlock {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&number.to_le_bytes());
    RawBlock {
        number,
        hash,
        parent_hash: [0u8; 32],
        state_root: [0u8; 32],
        extrinsics_root: [0u8; 32],
        extrinsics: vec![],
        events: vec![],
        timestamp: Some(1_700_000_000_000),
    }
}

pub struct StubBlockSource {
    pub tip: u64,
    /// Whether `subscribe_finalized` should keep yielding blocks forever
    /// (for the shutdown-responsiveness test) or only yield one then end.
    pub keep_streaming: bool,
}

#[async_trait]
impl BlockSource for StubBlockSource {
    async fn genesis_hash(&self) -> crate::error::ChainResult<BlockHash> {
        Ok(BlockHash([0u8; 32]))
    }
    async fn finalized_head(&self) -> crate::error::ChainResult<FinalizedHead> {
        Ok(FinalizedHead {
            number: self.tip,
            hash: [0u8; 32],
        })
    }
    async fn best_head(&self) -> crate::error::ChainResult<FinalizedHead> {
        self.finalized_head().await
    }
    async fn subscribe_finalized(&self) -> crate::error::ChainResult<FinalizedBlockStream> {
        if self.keep_streaming {
            // pending stream: never yields so the live loop parks on
            // stream.next() — exactly the scenario where shutdown must
            // still be observed.
            let s = stream::pending::<crate::error::ChainResult<RawBlock>>();
            Ok(Box::pin(s)
                as Pin<
                    Box<dyn futures::Stream<Item = crate::error::ChainResult<RawBlock>> + Send>,
                >)
        } else {
            let s = stream::empty::<crate::error::ChainResult<RawBlock>>();
            Ok(Box::pin(s)
                as Pin<
                    Box<dyn futures::Stream<Item = crate::error::ChainResult<RawBlock>> + Send>,
                >)
        }
    }
    async fn subscribe_best(&self) -> crate::error::ChainResult<FinalizedBlockStream> {
        self.subscribe_finalized().await
    }
    async fn runtime_version(&self) -> crate::error::ChainResult<u32> {
        Ok(42)
    }
    async fn fetch_block_at(&self, number: u64) -> crate::error::ChainResult<RawBlock> {
        Ok(mk_raw(number))
    }
    async fn earliest_v14_block(&self) -> crate::error::ChainResult<u64> {
        Ok(0)
    }
}

#[derive(Default)]
pub struct StubRepos {
    pub blocks: StdMutex<Vec<Block>>,
}

pub struct NoopRepo;

#[async_trait]
impl BlockRepository for NoopRepo {
    async fn insert_blocks(&self, _: &[Block]) -> crate::error::StorageResult<()> {
        Ok(())
    }
    async fn get_block(&self, _: u64) -> crate::error::StorageResult<Option<Block>> {
        Ok(None)
    }
    async fn get_block_by_hash(&self, _: &BlockHash) -> crate::error::StorageResult<Option<Block>> {
        Ok(None)
    }
    async fn list_blocks(
        &self,
        _: BlockFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> crate::error::StorageResult<Connection<Block>> {
        unimplemented!()
    }
    async fn latest_block_number(&self) -> crate::error::StorageResult<Option<u64>> {
        Ok(None)
    }
    async fn delete_blocks_from(&self, _: u64) -> crate::error::StorageResult<u64> {
        Ok(0)
    }
}

#[async_trait]
impl ExtrinsicRepository for NoopRepo {
    async fn insert_extrinsics(&self, _: &[Extrinsic]) -> crate::error::StorageResult<()> {
        Ok(())
    }
    async fn get_extrinsic(&self, _: &str) -> crate::error::StorageResult<Option<Extrinsic>> {
        Ok(None)
    }
    async fn list_extrinsics_for_block(
        &self,
        _: u64,
    ) -> crate::error::StorageResult<Vec<Extrinsic>> {
        Ok(vec![])
    }
    async fn list_extrinsics(
        &self,
        _: ExtrinsicFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> crate::error::StorageResult<Connection<Extrinsic>> {
        unimplemented!()
    }
    async fn delete_extrinsics_from(&self, _: u64) -> crate::error::StorageResult<u64> {
        Ok(0)
    }
}

#[async_trait]
impl EventRepository for NoopRepo {
    async fn insert_events(&self, _: &[Event]) -> crate::error::StorageResult<()> {
        Ok(())
    }
    async fn get_event(&self, _: &str) -> crate::error::StorageResult<Option<Event>> {
        Ok(None)
    }
    async fn list_events_for_block(&self, _: u64) -> crate::error::StorageResult<Vec<Event>> {
        Ok(vec![])
    }
    async fn list_events_for_extrinsic(
        &self,
        _: u64,
        _: u32,
    ) -> crate::error::StorageResult<Vec<Event>> {
        Ok(vec![])
    }
    async fn list_events(
        &self,
        _: EventFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> crate::error::StorageResult<Connection<Event>> {
        unimplemented!()
    }
    async fn delete_events_from(&self, _: u64) -> crate::error::StorageResult<u64> {
        Ok(0)
    }
}

#[async_trait]
impl CursorRepository for NoopRepo {
    async fn get_cursor(&self, _: &str) -> crate::error::StorageResult<Option<IndexerCursor>> {
        Ok(None)
    }
    async fn get_any_cursor(&self) -> crate::error::StorageResult<Option<IndexerCursor>> {
        Ok(None)
    }
    async fn set_cursor(&self, _: &IndexerCursor) -> crate::error::StorageResult<()> {
        Ok(())
    }
    async fn extend_upward(
        &self,
        _: &str,
        _: u64,
        _: &BlockHash,
    ) -> crate::error::StorageResult<()> {
        Ok(())
    }
    async fn extend_downward(&self, _: &str, _: u64) -> crate::error::StorageResult<()> {
        Ok(())
    }
}

static NOOP: NoopRepo = NoopRepo;

#[async_trait]
impl Repositories for StubRepos {
    fn blocks(&self) -> &dyn BlockRepository {
        &NOOP
    }
    fn extrinsics(&self) -> &dyn ExtrinsicRepository {
        &NOOP
    }
    fn events(&self) -> &dyn EventRepository {
        &NOOP
    }
    fn cursor(&self) -> &dyn CursorRepository {
        &NOOP
    }
    async fn persist_block_atomic(&self, data: BlockData<'_>) -> crate::error::StorageResult<()> {
        self.blocks.lock().unwrap().push(data.block.clone());
        Ok(())
    }
    async fn delete_from_block_atomic(&self, _: u64, _: &str) -> crate::error::StorageResult<u64> {
        Ok(0)
    }
}

pub fn build_service(
    bus: EventBus,
    keep_streaming: bool,
) -> IndexerService<StubBlockSource, StubRepos> {
    let config = IndexerConfig {
        chain_id: "test-chain".into(),
        ws_url: "ws://mock".into(),
        block_mode: BlockMode::Finalized,
        backfill: BackfillConfig {
            start_block: 42,
            live_only: true,
            concurrency: 1,
            max_fetch_retries: 0,
        },
        ..Default::default()
    };
    IndexerService::new(
        config,
        Arc::new(StubBlockSource {
            tip: 42,
            keep_streaming,
        }),
        Arc::new(StubRepos::default()),
        Arc::new(HandlerRegistry::new()),
        bus,
    )
}
