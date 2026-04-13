//! Layer-2 integration tests for historical backfill.
//!
//! Uses in-memory `MockBlockSource` and `MockRepositories` to exercise
//! end-to-end behavior of `BackfillRunner::run_range` + `persist_block_atomic`
//! semantics without a real node or Postgres.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tokio::sync::watch;

use maestro_core::error::{
    ChainError, ChainResult, IndexerError, IndexerResult, StorageError, StorageResult,
};
use maestro_core::models::{Block, BlockHash, Event, Extrinsic, IndexerCursor};
use maestro_core::ports::{
    BlockData, BlockFilter, BlockRepository, BlockSource, Connection, CursorRepository,
    EventFilter, EventRepository, ExtrinsicFilter, ExtrinsicRepository, FinalizedBlockStream,
    FinalizedHead, OrderDirection, Pagination, RawBlock, Repositories,
};
use maestro_core::services::backfill::BackfillRunner;
use maestro_core::services::{BackfillConfig, BackfillDirection, BackfillPlan, BackfillRange};

// ============================================================================
// Mock BlockSource
// ============================================================================

struct MockBlockSource {
    blocks: HashMap<u64, RawBlock>,
    fail_fetches: Mutex<HashMap<u64, u32>>, // block → remaining failures
    tip: u64,
}

impl MockBlockSource {
    fn new(blocks: Vec<RawBlock>) -> Self {
        let tip = blocks.iter().map(|b| b.number).max().unwrap_or(0);
        let map = blocks.into_iter().map(|b| (b.number, b)).collect();
        Self {
            blocks: map,
            fail_fetches: Mutex::new(HashMap::new()),
            tip,
        }
    }
}

fn mk_block(number: u64) -> RawBlock {
    let mut hash = [0u8; 32];
    hash[..8].copy_from_slice(&number.to_le_bytes());
    let mut parent = [0u8; 32];
    if number > 0 {
        parent[..8].copy_from_slice(&(number - 1).to_le_bytes());
    }
    RawBlock {
        number,
        hash,
        parent_hash: parent,
        state_root: [0u8; 32],
        extrinsics_root: [0u8; 32],
        extrinsics: vec![],
        events: vec![],
        timestamp: Some(1_600_000_000_000 + number * 6_000),
    }
}

#[async_trait]
impl BlockSource for MockBlockSource {
    async fn genesis_hash(&self) -> ChainResult<BlockHash> {
        Ok(BlockHash([0u8; 32]))
    }

    async fn finalized_head(&self) -> ChainResult<FinalizedHead> {
        Ok(FinalizedHead {
            number: self.tip,
            hash: [0u8; 32],
        })
    }

    async fn best_head(&self) -> ChainResult<FinalizedHead> {
        Ok(FinalizedHead {
            number: self.tip,
            hash: [0u8; 32],
        })
    }

    async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream> {
        unimplemented!("not needed for backfill tests")
    }

    async fn subscribe_best(&self) -> ChainResult<FinalizedBlockStream> {
        unimplemented!("not needed for backfill tests")
    }

    async fn runtime_version(&self) -> ChainResult<u32> {
        Ok(1)
    }

    async fn fetch_block_at(&self, number: u64) -> ChainResult<RawBlock> {
        let mut fails = self.fail_fetches.lock().unwrap();
        if let Some(n) = fails.get_mut(&number)
            && *n > 0
        {
            *n -= 1;
            return Err(ChainError::RpcError(format!("mock flaky at {number}")));
        }
        self.blocks
            .get(&number)
            .cloned()
            .ok_or_else(|| ChainError::RpcError(format!("no block {number}")))
    }

    async fn earliest_v14_block(&self) -> ChainResult<u64> {
        Ok(0)
    }
}

// ============================================================================
// Mock Repositories (in-memory, range-aware cursor)
// ============================================================================

#[derive(Default)]
struct MockStore {
    blocks: BTreeMap<u64, Block>,
    cursor: Option<IndexerCursor>,
}

struct MockRepositories {
    inner: Mutex<MockStore>,
}

impl MockRepositories {
    fn new() -> Self {
        Self {
            inner: Mutex::new(MockStore::default()),
        }
    }

    fn cursor_snapshot(&self) -> Option<IndexerCursor> {
        self.inner.lock().unwrap().cursor.clone()
    }

    fn indexed_block_numbers(&self) -> Vec<u64> {
        self.inner.lock().unwrap().blocks.keys().copied().collect()
    }

    fn seed_cursor(&self, first: u64, last: u64) {
        let mut g = self.inner.lock().unwrap();
        for n in first..=last {
            let mut hash = [0u8; 32];
            hash[..8].copy_from_slice(&n.to_le_bytes());
            g.blocks.insert(
                n,
                Block {
                    number: n,
                    hash: BlockHash(hash),
                    parent_hash: BlockHash([0u8; 32]),
                    state_root: BlockHash([0u8; 32]),
                    extrinsics_root: BlockHash([0u8; 32]),
                    author: None,
                    timestamp: None,
                    extrinsic_count: 0,
                    event_count: 0,
                    indexed_at: chrono::Utc::now(),
                },
            );
        }
        let last_hash = g.blocks.get(&last).map(|b| b.hash.clone()).unwrap();
        g.cursor = Some(IndexerCursor {
            chain_id: "test".into(),
            first_indexed_block: first,
            last_indexed_block: last,
            last_indexed_hash: last_hash,
            updated_at: chrono::Utc::now(),
        });
    }
}

// Minimal stubs — only methods called by the code paths under test are implemented.
struct NotImplementedRepo;

#[async_trait]
impl BlockRepository for NotImplementedRepo {
    async fn insert_blocks(&self, _: &[Block]) -> StorageResult<()> {
        unimplemented!()
    }
    async fn get_block(&self, _: u64) -> StorageResult<Option<Block>> {
        unimplemented!()
    }
    async fn get_block_by_hash(&self, _: &BlockHash) -> StorageResult<Option<Block>> {
        unimplemented!()
    }
    async fn list_blocks(
        &self,
        _: BlockFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> StorageResult<Connection<Block>> {
        unimplemented!()
    }
    async fn latest_block_number(&self) -> StorageResult<Option<u64>> {
        unimplemented!()
    }
    async fn delete_blocks_from(&self, _: u64) -> StorageResult<u64> {
        unimplemented!()
    }
}

#[async_trait]
impl ExtrinsicRepository for NotImplementedRepo {
    async fn insert_extrinsics(&self, _: &[Extrinsic]) -> StorageResult<()> {
        unimplemented!()
    }
    async fn get_extrinsic(&self, _: &str) -> StorageResult<Option<Extrinsic>> {
        unimplemented!()
    }
    async fn list_extrinsics_for_block(&self, _: u64) -> StorageResult<Vec<Extrinsic>> {
        unimplemented!()
    }
    async fn list_extrinsics(
        &self,
        _: ExtrinsicFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> StorageResult<Connection<Extrinsic>> {
        unimplemented!()
    }
    async fn delete_extrinsics_from(&self, _: u64) -> StorageResult<u64> {
        unimplemented!()
    }
}

#[async_trait]
impl EventRepository for NotImplementedRepo {
    async fn insert_events(&self, _: &[Event]) -> StorageResult<()> {
        unimplemented!()
    }
    async fn get_event(&self, _: &str) -> StorageResult<Option<Event>> {
        unimplemented!()
    }
    async fn list_events_for_block(&self, _: u64) -> StorageResult<Vec<Event>> {
        unimplemented!()
    }
    async fn list_events_for_extrinsic(&self, _: u64, _: u32) -> StorageResult<Vec<Event>> {
        unimplemented!()
    }
    async fn list_events(
        &self,
        _: EventFilter,
        _: Pagination,
        _: OrderDirection,
    ) -> StorageResult<Connection<Event>> {
        unimplemented!()
    }
    async fn delete_events_from(&self, _: u64) -> StorageResult<u64> {
        unimplemented!()
    }
}

#[async_trait]
impl CursorRepository for NotImplementedRepo {
    async fn get_cursor(&self, _: &str) -> StorageResult<Option<IndexerCursor>> {
        unimplemented!()
    }
    async fn get_any_cursor(&self) -> StorageResult<Option<IndexerCursor>> {
        unimplemented!()
    }
    async fn set_cursor(&self, _: &IndexerCursor) -> StorageResult<()> {
        unimplemented!()
    }
    async fn extend_upward(&self, _: &str, _: u64, _: &BlockHash) -> StorageResult<()> {
        unimplemented!()
    }
    async fn extend_downward(&self, _: &str, _: u64) -> StorageResult<()> {
        unimplemented!()
    }
}

static NOOP: NotImplementedRepo = NotImplementedRepo;

#[async_trait]
impl Repositories for MockRepositories {
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

    async fn persist_block_atomic(&self, data: BlockData<'_>) -> StorageResult<()> {
        let mut g = self.inner.lock().unwrap();
        let n = data.block.number;
        g.blocks.insert(n, data.block.clone());

        match g.cursor.clone() {
            None => {
                g.cursor = Some(IndexerCursor {
                    chain_id: data.chain_id.into(),
                    first_indexed_block: n,
                    last_indexed_block: n,
                    last_indexed_hash: data.block.hash.clone(),
                    updated_at: chrono::Utc::now(),
                });
            }
            Some(c) if n == c.last_indexed_block + 1 => {
                g.cursor = Some(IndexerCursor {
                    chain_id: c.chain_id,
                    first_indexed_block: c.first_indexed_block,
                    last_indexed_block: n,
                    last_indexed_hash: data.block.hash.clone(),
                    updated_at: chrono::Utc::now(),
                });
            }
            Some(c) if n + 1 == c.first_indexed_block => {
                g.cursor = Some(IndexerCursor {
                    chain_id: c.chain_id,
                    first_indexed_block: n,
                    last_indexed_block: c.last_indexed_block,
                    last_indexed_hash: c.last_indexed_hash,
                    updated_at: chrono::Utc::now(),
                });
            }
            Some(c) if n >= c.first_indexed_block && n <= c.last_indexed_block => {
                // Idempotent re-process inside range. Do nothing.
            }
            Some(c) => {
                return Err(StorageError::CursorGapViolation {
                    block: n,
                    first: c.first_indexed_block,
                    last: c.last_indexed_block,
                });
            }
        }
        Ok(())
    }

    async fn delete_from_block_atomic(&self, _: u64, _: &str) -> StorageResult<u64> {
        Ok(0)
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn range(from: u64, to: u64, dir: BackfillDirection) -> BackfillRange {
    BackfillRange {
        from,
        to,
        direction: dir,
    }
}

fn mock_cfg(concurrency: usize, max_retries: u32) -> BackfillConfig {
    BackfillConfig {
        start_block: 0,
        live_only: false,
        concurrency,
        max_fetch_retries: max_retries,
    }
}

async fn run_plan(
    source: Arc<MockBlockSource>,
    repos: Arc<MockRepositories>,
    plan: BackfillPlan,
    cfg: BackfillConfig,
) -> IndexerResult<()> {
    let runner = BackfillRunner::new(source, cfg, "test".into());
    let (_tx, mut rx) = watch::channel(false);
    for r in plan.ranges {
        let repos = repos.clone();
        runner
            .run_range(r, &mut rx, move |raw| {
                let repos = repos.clone();
                async move {
                    let block = Block {
                        number: raw.number,
                        hash: BlockHash(raw.hash),
                        parent_hash: BlockHash(raw.parent_hash),
                        state_root: BlockHash(raw.state_root),
                        extrinsics_root: BlockHash(raw.extrinsics_root),
                        author: None,
                        timestamp: None,
                        extrinsic_count: 0,
                        event_count: 0,
                        indexed_at: chrono::Utc::now(),
                    };
                    repos
                        .persist_block_atomic(BlockData {
                            block: &block,
                            extrinsics: &[],
                            events: &[],
                            chain_id: "test",
                        })
                        .await
                        .map_err(IndexerError::from)
                }
            })
            .await?;
    }
    Ok(())
}

// ============================================================================
// Scenarios
// ============================================================================

#[tokio::test]
async fn end_to_end_fresh_backfill_0_to_100() {
    let source = Arc::new(MockBlockSource::new((0..=100).map(mk_block).collect()));
    let repos = Arc::new(MockRepositories::new());

    let plan = BackfillPlan::compute(0, None, 100);
    run_plan(source, repos.clone(), plan, mock_cfg(4, 3))
        .await
        .unwrap();

    let indexed = repos.indexed_block_numbers();
    assert_eq!(indexed.len(), 101, "all 101 blocks persisted");
    let cursor = repos.cursor_snapshot().unwrap();
    assert_eq!(cursor.first_indexed_block, 0);
    assert_eq!(cursor.last_indexed_block, 100);
}

#[tokio::test]
async fn downward_gap_fill_from_existing_range() {
    let source = Arc::new(MockBlockSource::new((0..=100).map(mk_block).collect()));
    let repos = Arc::new(MockRepositories::new());
    repos.seed_cursor(50, 100);

    let cursor = repos.cursor_snapshot();
    let plan = BackfillPlan::compute(0, cursor.as_ref(), 100);
    run_plan(source, repos.clone(), plan, mock_cfg(4, 3))
        .await
        .unwrap();

    let indexed = repos.indexed_block_numbers();
    assert_eq!(indexed.len(), 101);
    assert_eq!(indexed.first(), Some(&0));
    assert_eq!(indexed.last(), Some(&100));
    let cursor = repos.cursor_snapshot().unwrap();
    assert_eq!(cursor.first_indexed_block, 0);
    assert_eq!(cursor.last_indexed_block, 100);
}

#[tokio::test]
async fn concurrency_preserves_order() {
    let source = Arc::new(MockBlockSource::new((0..=50).map(mk_block).collect()));
    let repos = Arc::new(MockRepositories::new());

    let plan = BackfillPlan::compute(0, None, 50);
    run_plan(source, repos.clone(), plan, mock_cfg(16, 3))
        .await
        .unwrap();

    let indexed = repos.indexed_block_numbers();
    assert_eq!(
        indexed,
        (0..=50).collect::<Vec<_>>(),
        "strict ascending order"
    );
}

#[tokio::test]
async fn abort_on_retry_exhaustion() {
    let source = Arc::new(MockBlockSource::new((0..=50).map(mk_block).collect()));
    // Block 42 always fails.
    source.fail_fetches.lock().unwrap().insert(42, 1_000);
    let repos = Arc::new(MockRepositories::new());

    let plan = BackfillPlan::compute(0, None, 50);
    let err = run_plan(source, repos.clone(), plan, mock_cfg(1, 3))
        .await
        .unwrap_err();

    match err {
        IndexerError::BackfillAborted { block, .. } => assert_eq!(block, 42),
        other => panic!("expected BackfillAborted, got {other:?}"),
    }

    // With concurrency=1 blocks 0..=41 persisted.
    let indexed = repos.indexed_block_numbers();
    assert_eq!(indexed.first(), Some(&0));
    assert_eq!(indexed.last(), Some(&41));
    let cursor = repos.cursor_snapshot().unwrap();
    assert_eq!(cursor.last_indexed_block, 41);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_mid_backfill_leaves_valid_cursor() {
    let source = Arc::new(MockBlockSource::new((0..=500).map(mk_block).collect()));
    let repos = Arc::new(MockRepositories::new());

    let (tx, mut rx) = watch::channel(false);
    let runner = BackfillRunner::new(source, mock_cfg(1, 3), "test".into());

    // Spawn the shutdown after ~20 blocks. We use a synthetic delay: the
    // processor sleeps briefly on each block so the shutdown signal lands.
    let repos_c = repos.clone();
    let fut = runner.run_range(
        range(0, 500, BackfillDirection::Upward),
        &mut rx,
        move |raw| {
            let repos = repos_c.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_millis(1)).await;
                let block = Block {
                    number: raw.number,
                    hash: BlockHash(raw.hash),
                    parent_hash: BlockHash(raw.parent_hash),
                    state_root: BlockHash(raw.state_root),
                    extrinsics_root: BlockHash(raw.extrinsics_root),
                    author: None,
                    timestamp: None,
                    extrinsic_count: 0,
                    event_count: 0,
                    indexed_at: chrono::Utc::now(),
                };
                repos
                    .persist_block_atomic(BlockData {
                        block: &block,
                        extrinsics: &[],
                        events: &[],
                        chain_id: "test",
                    })
                    .await
                    .map_err(IndexerError::from)
            }
        },
    );

    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let _ = tx.send(true);
    });

    let err = fut.await.unwrap_err();
    let _ = matches!(err, IndexerError::ShutdownRequested);

    let cursor = repos.cursor_snapshot().unwrap();
    assert_eq!(cursor.first_indexed_block, 0);
    // Cursor must be a valid contiguous [0, N]; N is non-deterministic but positive.
    assert!(cursor.last_indexed_block >= 1);
    assert!(cursor.last_indexed_block < 500);
}

#[tokio::test]
async fn handoff_race_simulation() {
    // Backfill [0, 100] completes, then a "live" stream emits 99, 100, 101, 102.
    // The 99 and 100 emissions must be absorbed idempotently (inside-range arm
    // of persist_block_atomic) and 101, 102 must extend the cursor upward.
    let source = Arc::new(MockBlockSource::new((0..=100).map(mk_block).collect()));
    let repos = Arc::new(MockRepositories::new());

    let plan = BackfillPlan::compute(0, None, 100);
    run_plan(source, repos.clone(), plan, mock_cfg(4, 3))
        .await
        .unwrap();

    // Simulate the live stream by persisting 99, 100, 101, 102 directly through
    // the repo (the real path is live_process_block → index_single_block →
    // persist_block_atomic; for the mock we exercise the persist branching).
    for n in [99u64, 100, 101, 102] {
        let mut hash = [0u8; 32];
        hash[..8].copy_from_slice(&n.to_le_bytes());
        let block = Block {
            number: n,
            hash: BlockHash(hash),
            parent_hash: BlockHash([0u8; 32]),
            state_root: BlockHash([0u8; 32]),
            extrinsics_root: BlockHash([0u8; 32]),
            author: None,
            timestamp: None,
            extrinsic_count: 0,
            event_count: 0,
            indexed_at: chrono::Utc::now(),
        };
        repos
            .persist_block_atomic(BlockData {
                block: &block,
                extrinsics: &[],
                events: &[],
                chain_id: "test",
            })
            .await
            .unwrap();
    }

    let cursor = repos.cursor_snapshot().unwrap();
    assert_eq!(cursor.first_indexed_block, 0);
    assert_eq!(
        cursor.last_indexed_block, 102,
        "live stream extended cursor through 102"
    );
}
