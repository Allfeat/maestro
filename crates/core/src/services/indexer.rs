//! Core indexer service - orchestrates block processing.
//!
//! This service is designed for chain head indexing only (v1).
//! It subscribes to finalized blocks and processes them in real-time.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::error::{IndexerError, IndexerResult};
use crate::events::{
    BackfillEvent, ChainEvent, ChainState, CursorState, EventBus, IndexerEvent, StopReason,
};
use crate::metrics::{
    ProcessingTimer, record_block_indexed, record_blocks_deleted, record_handler_error,
    record_reorg_detected,
};
use crate::models::{Block, BlockHash, Event, Extrinsic, ExtrinsicStatus, IndexerCursor};
use crate::ports::{
    BlockData, BlockMode, BlockSource, HandlerOutputs, HandlerRegistry, RawBlock, Repositories,
};
use crate::services::backfill::{BackfillPlan, BackfillRunner};

// =============================================================================
// Configuration
// =============================================================================

/// Whether a block is being processed from the live subscription or the
/// historical backfill loop. Drives metric labels and log distinction only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexMode {
    Live,
    Backfill,
}

impl IndexMode {
    pub fn as_label(&self) -> &'static str {
        match self {
            IndexMode::Live => "live",
            IndexMode::Backfill => "backfill",
        }
    }
}

/// Configuration for the indexer service.
#[derive(Debug, Clone)]
pub struct IndexerConfig {
    /// Chain identifier (usually genesis hash).
    pub chain_id: String,
    /// WebSocket URL of the Substrate node. Surfaced on `ChainEvent::RpcConnected`.
    pub ws_url: String,
    /// Polling interval when subscription fails.
    pub poll_interval: Duration,
    /// Maximum retries for block fetching.
    pub max_retries: u32,
    /// Delay between retries.
    pub retry_delay: Duration,
    /// Block subscription mode (finalized or best).
    pub block_mode: BlockMode,
    /// Historical backfill configuration.
    pub backfill: BackfillConfig,
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            chain_id: String::new(),
            ws_url: String::new(),
            poll_interval: Duration::from_secs(12),
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
            block_mode: BlockMode::Finalized,
            backfill: BackfillConfig::default(),
        }
    }
}

/// Historical backfill configuration.
#[derive(Debug, Clone)]
pub struct BackfillConfig {
    /// Lowest block number to index (inclusive).
    pub start_block: u64,
    /// Skip backfill entirely and start indexing at the current tip.
    pub live_only: bool,
    /// Maximum parallel block fetches during backfill.
    pub concurrency: usize,
    /// Maximum per-block fetch retries before aborting the run.
    pub max_fetch_retries: u32,
}

impl Default for BackfillConfig {
    fn default() -> Self {
        Self {
            start_block: 0,
            live_only: false,
            concurrency: 16,
            max_fetch_retries: 5,
        }
    }
}

// =============================================================================
// IndexerService
// =============================================================================

/// Main indexer service for chain head indexing.
///
/// # Design
///
/// This service subscribes to finalized block heads and processes each block
/// as it arrives. It does not support historical block indexing (v1 limitation).
///
/// # Flow
///
/// 1. Subscribe to finalized heads
/// 2. For each finalized head, fetch the full block
/// 3. Run pallet handlers to extract domain entities
/// 4. Persist block data and handler outputs
/// 5. Update cursor
pub struct IndexerService<S: BlockSource, R: Repositories> {
    config: IndexerConfig,
    block_source: Arc<S>,
    repositories: Arc<R>,
    handlers: Arc<HandlerRegistry>,
    event_bus: EventBus,
    /// In-memory mirror of the persisted cursor. Updated after each successful
    /// `persist_block_atomic` so `IndexerEvent::CursorAdvanced` and the
    /// `watch::CursorState` stay in sync without a post-persist DB round-trip.
    cursor_mirror: Arc<Mutex<Option<CursorState>>>,
}

/// Pure V14 metadata floor check. Returns `PreV14BlockRequested` if the
/// requested `start_block` is below the chain's earliest V14 block.
pub(crate) fn check_v14_floor(start_block: u64, earliest_v14: u64) -> IndexerResult<()> {
    if start_block < earliest_v14 {
        return Err(IndexerError::PreV14BlockRequested {
            requested: start_block,
            earliest_v14,
        });
    }
    Ok(())
}

impl<S: BlockSource + 'static, R: Repositories> IndexerService<S, R> {
    pub fn new(
        config: IndexerConfig,
        block_source: Arc<S>,
        repositories: Arc<R>,
        handlers: Arc<HandlerRegistry>,
        event_bus: EventBus,
    ) -> Self {
        Self {
            config,
            block_source,
            repositories,
            handlers,
            event_bus,
            cursor_mirror: Arc::new(Mutex::new(None)),
        }
    }

    /// Start the indexer.
    ///
    /// Subscribes to blocks and processes them as they arrive.
    /// The subscription mode is determined by `config.block_mode`.
    #[instrument(skip_all, fields(chain = %&self.config.chain_id[..16.min(self.config.chain_id.len())]))]
    pub async fn run(&self, shutdown_rx: tokio::sync::watch::Receiver<bool>) -> IndexerResult<()> {
        let result = self.run_inner(shutdown_rx).await;
        let reason = match &result {
            Ok(()) => StopReason::ShutdownRequested,
            Err(IndexerError::ShutdownRequested) => StopReason::ShutdownRequested,
            Err(e) => StopReason::Fatal(e.to_string()),
        };
        self.event_bus
            .emit_indexer(IndexerEvent::Stopped { reason });
        // Read current chain state via a scoped borrow so the read-lock is
        // dropped before `update_chain` acquires the write-lock. Without
        // this, the temporary from `watch_chain().borrow()` survives until
        // the end of the enclosing statement and deadlocks the RwLock.
        let current_chain = self.event_bus.watch_chain().borrow().clone();
        self.event_bus.update_chain(ChainState {
            connected: false,
            ..current_chain
        });
        result
    }

    async fn run_inner(
        &self,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> IndexerResult<()> {
        info!(mode = ?self.config.block_mode, "⛓️  Starting indexer");

        // Warn if running in best block mode
        if self.config.block_mode == BlockMode::Best {
            warn!("⚠️  Running in BEST BLOCK mode - data may be reorged!");
        }

        // R5: all startup checks consolidated here.
        self.verify_chain_id().await?;
        let existing_cursor = self.verify_consistency_on_startup().await?;

        // V14 floor enforcement (§4).
        let earliest_v14 = self.block_source.earliest_v14_block().await?;
        self.enforce_v14_floor(earliest_v14)?;

        // Publish initial chain state: we're connected to the node and know
        // both its finalized head and runtime version. Subsequent transitions
        // are pushed from the live loop.
        let initial_head = self.block_source.finalized_head().await?;
        let spec_version = self.block_source.runtime_version().await?;
        self.event_bus.update_chain(ChainState {
            connected: true,
            finalized_head: initial_head.number,
            spec_version,
        });

        // Seed the cursor mirror and publish the initial CursorState snapshot.
        self.seed_cursor_mirror(existing_cursor.as_ref()).await;

        // Determine the initial mode + start block for the Started event.
        let start_mode = if self.config.backfill.live_only {
            IndexMode::Live
        } else {
            IndexMode::Backfill
        };
        self.event_bus.emit_indexer(IndexerEvent::Started {
            mode: start_mode,
            start_block: self.config.backfill.start_block,
        });

        // Live-only escape hatch.
        if self.config.backfill.live_only {
            self.warn_if_cursor_gap(&existing_cursor).await;
            let from_block = existing_cursor
                .as_ref()
                .map(|c| c.last_indexed_block)
                .unwrap_or(self.config.backfill.start_block);
            self.event_bus
                .emit_indexer(IndexerEvent::LiveModeEntered { from_block });
            return self
                .run_live_loop(&mut shutdown_rx, self.config.block_mode)
                .await;
        }

        // Plan backfill.
        let tip = initial_head.number;
        let plan = BackfillPlan::compute(
            self.config.backfill.start_block,
            existing_cursor.as_ref(),
            tip,
        );

        if !plan.ranges.is_empty() {
            // Aggregate total across every planned range so the TUI gauge
            // tracks the whole backfill, not just the current slice.
            let plan_from = plan.ranges.iter().map(|r| r.from).min().unwrap_or(0);
            let plan_to = plan.ranges.iter().map(|r| r.to).max().unwrap_or(0);
            let plan_total: u64 = plan.ranges.iter().map(|r| r.to - r.from + 1).sum();
            self.event_bus.emit_backfill(BackfillEvent::Planned {
                from: plan_from,
                to: plan_to,
                total: plan_total,
            });

            info!(ranges = ?plan.ranges, "🕰  Starting backfill");
            let runner = BackfillRunner::new(
                self.block_source.clone(),
                self.config.backfill.clone(),
                self.config.chain_id.clone(),
                self.event_bus.clone(),
            );
            for range in plan.ranges {
                runner
                    .run_range(range, &mut shutdown_rx, |raw| {
                        self.backfill_process_block(raw)
                    })
                    .await?;
            }
            info!("✅ Backfill complete, switching to live stream");
        }

        let from_block = {
            let mirror = self.cursor_mirror.lock().await;
            mirror.as_ref().map(|c| c.head).unwrap_or(tip)
        };
        self.event_bus
            .emit_indexer(IndexerEvent::LiveModeEntered { from_block });

        self.run_live_loop(&mut shutdown_rx, self.config.block_mode)
            .await
    }

    /// Initialize the in-memory cursor mirror from the persisted cursor
    /// (if any) and publish an initial `CursorState` on the watch channel so
    /// consumers like the TUI render a meaningful head/tail on first frame.
    async fn seed_cursor_mirror(&self, existing: Option<&IndexerCursor>) {
        let state = existing.map(|c| CursorState {
            head: c.last_indexed_block,
            tail: c.first_indexed_block,
        });
        if let Some(s) = state.clone() {
            self.event_bus.update_cursor(s);
        }
        *self.cursor_mirror.lock().await = state;
    }

    fn enforce_v14_floor(&self, earliest_v14: u64) -> IndexerResult<()> {
        check_v14_floor(self.config.backfill.start_block, earliest_v14)
    }

    async fn warn_if_cursor_gap(&self, cursor: &Option<IndexerCursor>) {
        let Some(c) = cursor else { return };

        // Tip-side gap: blocks between cursor top and current chain head.
        if let Ok(head) = self.block_source.finalized_head().await {
            let gap = head.number.saturating_sub(c.last_indexed_block);
            if gap > 1 {
                warn!(
                    cursor_top = c.last_indexed_block,
                    tip = head.number,
                    gap,
                    "⚠️  --live-only with a cursor gap; {gap} blocks will never be filled"
                );
            }
        }

        // Floor-side gap: blocks below cursor floor that --start-block wanted.
        let start_block = self.config.backfill.start_block;
        if c.first_indexed_block > start_block {
            let gap = c.first_indexed_block - start_block;
            warn!(
                start_block,
                cursor_floor = c.first_indexed_block,
                gap,
                "⚠️  --live-only with a floor-side cursor gap; {gap} blocks below {} will never be filled",
                c.first_indexed_block
            );
        }
    }

    /// Verify the connected chain matches any existing indexed data.
    /// Returns error if database contains data from a different chain.
    async fn verify_chain_id(&self) -> IndexerResult<()> {
        let existing_cursor = self.repositories.cursor().get_any_cursor().await?;

        if let Some(cursor) = existing_cursor {
            if cursor.chain_id != self.config.chain_id {
                let connected_short = &self.config.chain_id[..16.min(self.config.chain_id.len())];
                let expected_short = &cursor.chain_id[..16.min(cursor.chain_id.len())];

                error!(
                    connected = connected_short,
                    expected = expected_short,
                    "❌ Chain mismatch! Database contains data from a different chain"
                );
                error!(
                    "   Manual action required: either connect to the correct chain or clear the database"
                );

                return Err(IndexerError::ChainMismatch {
                    connected: self.config.chain_id.clone(),
                    expected: cursor.chain_id,
                });
            }
            debug!("Chain ID verified");
        }

        Ok(())
    }

    /// Verify consistency between stored cursor and chain state on reconnection.
    #[instrument(skip(self))]
    async fn verify_consistency_on_startup(&self) -> IndexerResult<Option<IndexerCursor>> {
        let cursor = self
            .repositories
            .cursor()
            .get_cursor(&self.config.chain_id)
            .await?;

        let Some(cursor) = cursor else {
            debug!("No cursor found, starting fresh");
            return Ok(None);
        };

        debug!(
            block = cursor.last_indexed_block,
            "Verifying cursor consistency"
        );

        let stored_block = self
            .repositories
            .blocks()
            .get_block(cursor.last_indexed_block)
            .await?;

        match stored_block {
            Some(block) if block.hash == cursor.last_indexed_hash => {
                debug!(
                    block = cursor.last_indexed_block,
                    "Cursor verified, resuming"
                );
                Ok(Some(cursor))
            }
            Some(_) => {
                warn!(
                    block = cursor.last_indexed_block,
                    "⚠️  Cursor hash mismatch, cleaning up"
                );
                let deleted = self
                    .repositories
                    .delete_from_block_atomic(cursor.last_indexed_block, &self.config.chain_id)
                    .await?;
                info!(deleted, "🗑️  Cleaned inconsistent data");
                record_blocks_deleted(deleted);
                Ok(None)
            }
            None => {
                warn!(
                    block = cursor.last_indexed_block,
                    "⚠️  Cursor points to missing block"
                );
                let deleted = self
                    .repositories
                    .delete_from_block_atomic(0, &self.config.chain_id)
                    .await?;
                info!(deleted, "🗑️  Cleaned inconsistent data");
                record_blocks_deleted(deleted);
                Ok(None)
            }
        }
    }

    /// Follow blocks via subscription (finalized or best based on mode).
    #[instrument(skip_all, fields(mode = ?mode))]
    async fn run_live_loop(
        &self,
        shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
        mode: BlockMode,
    ) -> IndexerResult<()> {
        let mode_label = match mode {
            BlockMode::Finalized => "finalized",
            BlockMode::Best => "best",
        };

        debug!(mode = mode_label, "Subscribing to blocks");
        // Consistency check lives in `run` now (R5); removed from here.

        // Exponential backoff configuration
        const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);
        const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
        let mut retry_delay = INITIAL_RETRY_DELAY;
        let mut reconnect_attempt: u32 = 0;

        loop {
            if *shutdown_rx.borrow() {
                debug!("Shutdown requested");
                return Err(IndexerError::ShutdownRequested);
            }

            // Subscribe based on mode
            let subscription = match mode {
                BlockMode::Finalized => self.block_source.subscribe_finalized().await,
                BlockMode::Best => self.block_source.subscribe_best().await,
            };

            match subscription {
                Ok(mut stream) => {
                    debug!(mode = mode_label, "📡 Subscription established");
                    retry_delay = INITIAL_RETRY_DELAY; // Reset backoff on success
                    reconnect_attempt = 0;
                    self.publish_connected();
                    self.event_bus.emit_chain(ChainEvent::RpcConnected {
                        url: self.config.ws_url.clone(),
                    });

                    loop {
                        // Race the next block against shutdown so the live
                        // loop stays responsive even when the node is quiet
                        // and `stream.next()` would otherwise park for many
                        // seconds between blocks.
                        let next = tokio::select! {
                            biased;
                            _ = shutdown_rx.changed() => {
                                if *shutdown_rx.borrow() {
                                    debug!("Shutdown requested");
                                    return Err(IndexerError::ShutdownRequested);
                                }
                                continue;
                            }
                            item = stream.next() => item,
                        };
                        let Some(result) = next else {
                            break;
                        };

                        match result {
                            Ok(raw_block) => {
                                let block_number = raw_block.number;
                                match self.live_process_block(raw_block).await {
                                    Ok(true) => info!(block = block_number, "⛓️  Block indexed"),
                                    Ok(false) => {
                                        // L2: visibility on the backfill→live handoff race.
                                        debug!(
                                            block = block_number,
                                            "live: skipped already-indexed block (handoff overlap)"
                                        );
                                    }
                                    Err(e) => {
                                        error!(block = block_number, error = ?e, "❌ Block processing failed");
                                    }
                                }
                            }
                            Err(e) => {
                                warn!(error = ?e, "⚠️  Subscription error, reconnecting...");
                                self.publish_disconnected();
                                self.event_bus.emit_chain(ChainEvent::RpcDisconnected {
                                    reason: e.to_string(),
                                });
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        error = ?e,
                        retry_in_ms = retry_delay.as_millis(),
                        "⚠️  Failed to subscribe, retrying..."
                    );
                    self.publish_disconnected();
                    self.event_bus.emit_chain(ChainEvent::RpcDisconnected {
                        reason: e.to_string(),
                    });
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(retry_delay) => {
                    debug!(retry_delay_ms = retry_delay.as_millis(), "🔄 Reconnecting to chain...");
                    reconnect_attempt = reconnect_attempt.saturating_add(1);
                    self.event_bus.emit_chain(ChainEvent::RpcReconnecting {
                        attempt: reconnect_attempt,
                    });
                    // Exponential backoff: double the delay, up to max
                    retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return Err(IndexerError::ShutdownRequested);
                    }
                }
            }
        }
    }

    fn publish_connected(&self) {
        // Scoped borrow: drop the read-lock BEFORE update_chain acquires
        // the write-lock — see `run()` for the same pattern + rationale.
        let current = self.event_bus.watch_chain().borrow().clone();
        self.event_bus.update_chain(ChainState {
            connected: true,
            ..current
        });
    }

    fn publish_disconnected(&self) {
        let current = self.event_bus.watch_chain().borrow().clone();
        self.event_bus.update_chain(ChainState {
            connected: false,
            ..current
        });
    }

    /// Check for chain reorganization by comparing parent hash.
    #[instrument(skip(self, raw_block), fields(block = raw_block.number))]
    async fn check_and_handle_reorg(&self, raw_block: &RawBlock) -> IndexerResult<bool> {
        let block_number = raw_block.number;

        if block_number == 0 {
            return Ok(false);
        }

        let stored_prev = self
            .repositories
            .blocks()
            .get_block(block_number - 1)
            .await?;

        match stored_prev {
            Some(prev_block) => {
                let expected_parent = BlockHash(raw_block.parent_hash);
                if prev_block.hash != expected_parent {
                    warn!(
                        block = block_number,
                        expected = %hex::encode(&expected_parent.0[..8]),
                        stored = %hex::encode(&prev_block.hash.0[..8]),
                        "🔄 Reorg detected! Parent hash mismatch"
                    );

                    record_reorg_detected(block_number);

                    let deleted = self
                        .repositories
                        .delete_from_block_atomic(block_number - 1, &self.config.chain_id)
                        .await?;

                    info!(block = block_number, deleted = deleted, "🔄 Reorg handled");
                    record_blocks_deleted(deleted);

                    return Ok(true);
                }
            }
            None => {
                let latest = self.repositories.blocks().latest_block_number().await?;
                if let Some(latest_num) = latest
                    && block_number > latest_num + 1
                {
                    warn!(
                        block = block_number,
                        latest = latest_num,
                        gap = block_number - latest_num - 1,
                        "⚠️  Gap detected in block sequence"
                    );
                }
            }
        }

        Ok(false)
    }

    /// Pure per-block processor. Runs handlers and persists atomically.
    /// Does NOT check for reorgs, does NOT skip already-indexed blocks.
    /// Both live and backfill loops funnel through this.
    #[instrument(skip(self, raw_block), fields(block = raw_block.number, mode = mode.as_label()))]
    async fn index_single_block(&self, raw_block: RawBlock, mode: IndexMode) -> IndexerResult<()> {
        let _timer = ProcessingTimer::new();
        let started_at = Instant::now();
        let block = self.transform_block(&raw_block);

        // Handler lifecycle: on_block_start
        for handler in self.handlers.all() {
            handler.on_block_start(&block).await?;
        }

        let extrinsics = self.transform_extrinsics(&raw_block, &block);
        let mut all_outputs = HandlerOutputs::new();

        for raw_event in &raw_block.events {
            if let Some(handler) = self.handlers.get(&raw_event.pallet) {
                let extrinsic = raw_event
                    .extrinsic_index
                    .and_then(|idx| raw_block.extrinsics.iter().find(|e| e.index == idx));

                match handler.handle_event(raw_event, &block, extrinsic).await {
                    Ok(outputs) => all_outputs.merge(outputs),
                    Err(e) => {
                        debug!(
                            event = raw_event.index,
                            pallet = %raw_event.pallet,
                            name = %raw_event.name,
                            error = ?e,
                            "Handler failed for event"
                        );
                        record_handler_error("event", &raw_event.pallet);
                    }
                }
            }
        }

        for raw_ext in &raw_block.extrinsics {
            if let Some(handler) = self.handlers.get(&raw_ext.pallet) {
                match handler.handle_extrinsic(raw_ext, &block).await {
                    Ok(outputs) => all_outputs.merge(outputs),
                    Err(e) => {
                        debug!(
                            ext = raw_ext.index,
                            pallet = %raw_ext.pallet,
                            call = %raw_ext.call,
                            error = ?e,
                            "Handler failed for extrinsic"
                        );
                        record_handler_error("extrinsic", &raw_ext.pallet);
                    }
                }
            }
        }

        let events = self.transform_events(&raw_block, &block);

        let block_data = BlockData {
            block: &block,
            extrinsics: &extrinsics,
            events: &events,
            chain_id: &self.config.chain_id,
        };
        self.repositories.persist_block_atomic(block_data).await?;

        for handler in self.handlers.all() {
            match handler.on_block_end(&block, &all_outputs).await {
                Ok(outputs) => all_outputs.merge(outputs),
                Err(e) => {
                    error!(error = ?e, "❌ Handler on_block_end failed");
                }
            }
        }

        record_block_indexed(mode);

        // Event bus emission: now that persistence and on_block_end have
        // succeeded, notify consumers (TUI, logger, metrics bridge).
        let duration_ms = started_at.elapsed().as_millis().min(u32::MAX as u128) as u32;
        let block_number = block.number;
        let block_hash = block.hash.clone();
        let extrinsic_count = block.extrinsic_count;
        let event_count = block.event_count;

        self.event_bus.emit_indexer(IndexerEvent::BlockIndexed {
            number: block_number,
            hash: block_hash,
            extrinsics: extrinsic_count,
            events: event_count,
            duration_ms,
        });

        // Update in-memory cursor mirror + publish watch + emit CursorAdvanced.
        let new_state = {
            let mut mirror = self.cursor_mirror.lock().await;
            let next = match mirror.as_ref() {
                Some(c) => CursorState {
                    head: c.head.max(block_number),
                    tail: c.tail.min(block_number),
                },
                None => CursorState {
                    head: block_number,
                    tail: block_number,
                },
            };
            *mirror = Some(next.clone());
            next
        };
        self.event_bus.update_cursor(new_state.clone());
        self.event_bus.emit_indexer(IndexerEvent::CursorAdvanced {
            head: new_state.head,
            tail: new_state.tail,
        });

        trace!("Block processed successfully");
        Ok(())
    }

    /// Live-loop wrapper. Skips already-indexed blocks, checks reorg, calls `index_single_block`.
    /// Returns `Ok(true)` if the block was indexed, `Ok(false)` if it was skipped.
    async fn live_process_block(&self, raw_block: RawBlock) -> IndexerResult<bool> {
        let block_number = raw_block.number;
        trace!(block = block_number, "live: processing block");

        if let Some(existing_block) = self.repositories.blocks().get_block(block_number).await? {
            let incoming_hash = BlockHash(raw_block.hash);
            if existing_block.hash == incoming_hash {
                trace!(block = block_number, "live: already indexed, skipping");
                return Ok(false);
            }
            trace!(
                block = block_number,
                "live: hash differs, checking for reorg"
            );
        }

        if self.check_and_handle_reorg(&raw_block).await? {
            trace!(block = block_number, "live: reorg handled, continuing");
        }

        self.index_single_block(raw_block, IndexMode::Live).await?;
        Ok(true)
    }

    /// Backfill-loop wrapper. No skip, no reorg. Exists for symmetry with
    /// `live_process_block` and to make the intent obvious at the call site.
    async fn backfill_process_block(&self, raw_block: RawBlock) -> IndexerResult<()> {
        self.index_single_block(raw_block, IndexMode::Backfill)
            .await
    }

    /// Transform raw block to domain model.
    fn transform_block(&self, raw: &RawBlock) -> Block {
        Block {
            number: raw.number,
            hash: BlockHash(raw.hash),
            parent_hash: BlockHash(raw.parent_hash),
            state_root: BlockHash(raw.state_root),
            extrinsics_root: BlockHash(raw.extrinsics_root),
            author: None,
            timestamp: raw.timestamp.map(|ts| {
                chrono::DateTime::from_timestamp_millis(ts as i64).unwrap_or_else(chrono::Utc::now)
            }),
            extrinsic_count: raw.extrinsics.len() as u32,
            event_count: raw.events.len() as u32,
            indexed_at: chrono::Utc::now(),
        }
    }

    /// Transform raw extrinsics to domain models.
    fn transform_extrinsics(&self, raw: &RawBlock, block: &Block) -> Vec<Extrinsic> {
        raw.extrinsics
            .iter()
            .map(|ext| Extrinsic {
                id: format!("{}-{}", block.number, ext.index),
                block_number: block.number,
                block_hash: block.hash.clone(),
                index: ext.index,
                pallet: ext.pallet.clone(),
                call: ext.call.clone(),
                signer: ext.signer.map(crate::models::AccountId),
                status: if ext.success {
                    ExtrinsicStatus::Success
                } else {
                    ExtrinsicStatus::Failed
                },
                error: ext.error.clone(),
                args: ext.args.clone(),
                raw: hex::encode(&ext.bytes),
                tip: ext.tip,
                nonce: ext.nonce,
            })
            .collect()
    }

    /// Transform raw events to domain models.
    fn transform_events(&self, raw: &RawBlock, block: &Block) -> Vec<Event> {
        raw.events
            .iter()
            .map(|evt| Event {
                id: format!("{}-{}", block.number, evt.index),
                block_number: block.number,
                block_hash: block.hash.clone(),
                index: evt.index,
                extrinsic_index: evt.extrinsic_index,
                pallet: evt.pallet.clone(),
                name: evt.name.clone(),
                data: evt.data.clone(),
                topics: evt.topics.iter().map(hex::encode).collect(),
            })
            .collect()
    }
}

#[cfg(test)]
mod v14_floor_tests {
    use super::*;

    #[test]
    fn start_at_or_above_floor_passes() {
        assert!(check_v14_floor(0, 0).is_ok());
        assert!(check_v14_floor(100, 100).is_ok());
        assert!(check_v14_floor(200, 100).is_ok());
    }

    #[test]
    fn start_below_floor_errors_with_both_numbers() {
        let err = check_v14_floor(50, 473291).unwrap_err();
        match err {
            IndexerError::PreV14BlockRequested {
                requested,
                earliest_v14,
            } => {
                assert_eq!(requested, 50);
                assert_eq!(earliest_v14, 473291);
            }
            other => panic!("expected PreV14BlockRequested, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod event_emission_tests {
    //! Phase 5 wiring: drive `index_single_block` directly and assert the
    //! right events land on the bus + watches. Tests `run_live_loop`
    //! lifecycle separately via `live_loop_shutdown_is_observed_promptly`.
    use super::*;
    use crate::error::{ChainResult, StorageResult};
    use crate::events::{BackfillEvent, ChainEvent};
    use crate::ports::{
        BlockFilter, BlockRepository, Connection, CursorRepository, EventFilter, EventRepository,
        ExtrinsicFilter, ExtrinsicRepository, FinalizedBlockStream, FinalizedHead, OrderDirection,
        Pagination, RawBlock,
    };
    use async_trait::async_trait;
    use futures::stream;
    use std::pin::Pin;
    use std::sync::Mutex as StdMutex;

    fn mk_raw(number: u64) -> RawBlock {
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

    struct StubBlockSource {
        tip: u64,
        /// Whether `subscribe_finalized` should keep yielding blocks forever
        /// (for the shutdown-responsiveness test) or only yield one then end.
        keep_streaming: bool,
    }

    #[async_trait]
    impl BlockSource for StubBlockSource {
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
            self.finalized_head().await
        }
        async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream> {
            if self.keep_streaming {
                // pending stream: never yields so the live loop parks on
                // stream.next() — exactly the scenario where the fix must
                // still observe shutdown.
                let s = stream::pending::<ChainResult<RawBlock>>();
                Ok(Box::pin(s)
                    as Pin<
                        Box<dyn futures::Stream<Item = ChainResult<RawBlock>> + Send>,
                    >)
            } else {
                let s = stream::empty::<ChainResult<RawBlock>>();
                Ok(Box::pin(s)
                    as Pin<
                        Box<dyn futures::Stream<Item = ChainResult<RawBlock>> + Send>,
                    >)
            }
        }
        async fn subscribe_best(&self) -> ChainResult<FinalizedBlockStream> {
            self.subscribe_finalized().await
        }
        async fn runtime_version(&self) -> ChainResult<u32> {
            Ok(42)
        }
        async fn fetch_block_at(&self, number: u64) -> ChainResult<RawBlock> {
            Ok(mk_raw(number))
        }
        async fn earliest_v14_block(&self) -> ChainResult<u64> {
            Ok(0)
        }
    }

    #[derive(Default)]
    struct StubRepos {
        blocks: StdMutex<Vec<Block>>,
    }

    struct NoopRepo;

    #[async_trait]
    impl BlockRepository for NoopRepo {
        async fn insert_blocks(&self, _: &[Block]) -> StorageResult<()> {
            Ok(())
        }
        async fn get_block(&self, _: u64) -> StorageResult<Option<Block>> {
            Ok(None)
        }
        async fn get_block_by_hash(&self, _: &BlockHash) -> StorageResult<Option<Block>> {
            Ok(None)
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
            Ok(None)
        }
        async fn delete_blocks_from(&self, _: u64) -> StorageResult<u64> {
            Ok(0)
        }
    }
    #[async_trait]
    impl ExtrinsicRepository for NoopRepo {
        async fn insert_extrinsics(&self, _: &[Extrinsic]) -> StorageResult<()> {
            Ok(())
        }
        async fn get_extrinsic(&self, _: &str) -> StorageResult<Option<Extrinsic>> {
            Ok(None)
        }
        async fn list_extrinsics_for_block(&self, _: u64) -> StorageResult<Vec<Extrinsic>> {
            Ok(vec![])
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
            Ok(0)
        }
    }
    #[async_trait]
    impl EventRepository for NoopRepo {
        async fn insert_events(&self, _: &[Event]) -> StorageResult<()> {
            Ok(())
        }
        async fn get_event(&self, _: &str) -> StorageResult<Option<Event>> {
            Ok(None)
        }
        async fn list_events_for_block(&self, _: u64) -> StorageResult<Vec<Event>> {
            Ok(vec![])
        }
        async fn list_events_for_extrinsic(&self, _: u64, _: u32) -> StorageResult<Vec<Event>> {
            Ok(vec![])
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
            Ok(0)
        }
    }
    #[async_trait]
    impl CursorRepository for NoopRepo {
        async fn get_cursor(&self, _: &str) -> StorageResult<Option<IndexerCursor>> {
            Ok(None)
        }
        async fn get_any_cursor(&self) -> StorageResult<Option<IndexerCursor>> {
            Ok(None)
        }
        async fn set_cursor(&self, _: &IndexerCursor) -> StorageResult<()> {
            Ok(())
        }
        async fn extend_upward(&self, _: &str, _: u64, _: &BlockHash) -> StorageResult<()> {
            Ok(())
        }
        async fn extend_downward(&self, _: &str, _: u64) -> StorageResult<()> {
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
        async fn persist_block_atomic(&self, data: BlockData<'_>) -> StorageResult<()> {
            self.blocks.lock().unwrap().push(data.block.clone());
            Ok(())
        }
        async fn delete_from_block_atomic(&self, _: u64, _: &str) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn build_service(
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

    #[tokio::test(flavor = "current_thread")]
    async fn index_single_block_emits_block_indexed_and_cursor_advanced() {
        let bus = EventBus::default();
        let mut indexer_rx = bus.subscribe_indexer();
        // Subscribe to the watch BEFORE the emit — `watch::Sender::send()`
        // returns Err(SendError) and drops the value on the floor when
        // called with zero receivers, so an active subscription must exist
        // at send time for the value to be retained.
        let cursor_rx = bus.watch_cursor();
        let svc = build_service(bus.clone(), false);

        svc.index_single_block(mk_raw(42), IndexMode::Live)
            .await
            .expect("index_single_block failed");

        // First event must be BlockIndexed.
        match indexer_rx.try_recv().expect("no BlockIndexed emitted") {
            IndexerEvent::BlockIndexed {
                number,
                extrinsics,
                events,
                ..
            } => {
                assert_eq!(number, 42);
                assert_eq!(extrinsics, 0);
                assert_eq!(events, 0);
            }
            other => panic!("expected BlockIndexed, got {other:?}"),
        }

        // Then CursorAdvanced with head=tail=42 (seeded from the single block).
        match indexer_rx.try_recv().expect("no CursorAdvanced emitted") {
            IndexerEvent::CursorAdvanced { head, tail } => {
                assert_eq!(head, 42);
                assert_eq!(tail, 42);
            }
            other => panic!("expected CursorAdvanced, got {other:?}"),
        }

        // Cursor watch should have been published with head=tail=42.
        let state = cursor_rx.borrow().clone();
        assert_eq!(state.head, 42);
        assert_eq!(state.tail, 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn run_inner_publishes_chain_state_and_started_before_live_loop() {
        let bus = EventBus::default();
        let mut indexer_rx = bus.subscribe_indexer();
        // Hold a chain-watch receiver for the whole test so `update_chain`
        // calls aren't dropped for lack of subscribers.
        let _chain_rx = bus.watch_chain();
        let _cursor_rx = bus.watch_cursor();
        let svc = build_service(bus.clone(), true);

        let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

        // Run the service in a task and immediately request shutdown so the
        // live-loop exits via the select! we added. We only care about the
        // events emitted during startup here.
        let handle = tokio::spawn(async move {
            let _ = svc.run(shutdown_rx).await;
        });

        // Give the task a chance to run past startup + into the live loop.
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        // Signal shutdown and wait for termination — this is the regression
        // guard for the `q → hang` bug: the live loop must observe shutdown
        // while parked on `stream.next()`.
        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("indexer did not terminate promptly on shutdown")
            .expect("indexer task panicked");

        // Startup chain state must have been published with connected=true.
        // Note: `run()` republishes `connected=false` on exit, so we can't
        // rely on the watch's final value — inspect the broadcast trace.
        let mut saw_started = false;
        let mut saw_live_mode_entered = false;
        let mut saw_stopped = false;
        while let Ok(ev) = indexer_rx.try_recv() {
            match ev {
                IndexerEvent::Started { start_block, .. } => {
                    assert_eq!(start_block, 42);
                    saw_started = true;
                }
                IndexerEvent::LiveModeEntered { .. } => saw_live_mode_entered = true,
                IndexerEvent::Stopped { .. } => saw_stopped = true,
                _ => {}
            }
        }
        assert!(saw_started, "Started event never emitted");
        assert!(saw_live_mode_entered, "LiveModeEntered never emitted");
        assert!(
            saw_stopped,
            "Stopped never emitted (live loop ignored shutdown)"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backfill_event_variants_are_re_exported() {
        // Lightweight sanity check — not strictly event-emission but ensures
        // the submodule path keeps compiling for consumers of these imports.
        let _ = BackfillEvent::Planned {
            from: 0,
            to: 0,
            total: 1,
        };
        let _ = ChainEvent::RpcConnected {
            url: "ws://".into(),
        };
    }
}
