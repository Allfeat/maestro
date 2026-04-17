//! Core indexer service - orchestrates block processing.
//!
//! This module is the orchestration layer. Work lives in submodules:
//! - [`transforms`]: pure RawBlock → domain projections
//! - [`startup`]: V14 floor, chain-id, cursor verification
//! - [`reorg`]: parent-hash reorg detection on the live path
//! - [`live`]: subscription loop, reconnection, chain-state publishing

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::error::{IndexerError, IndexerResult};
use crate::events::{BackfillEvent, ChainState, CursorState, EventBus, IndexerEvent, StopReason};
use crate::metrics::{ProcessingTimer, record_block_indexed, record_handler_error};
use crate::models::BlockHash;
use crate::ports::{
    BlockData, BlockMode, BlockSource, HandlerOutputs, HandlerRegistry, RawBlock, Repositories,
};
use crate::services::backfill::{BackfillPlan, BackfillRunner};

mod live;
mod reorg;
mod startup;
mod transforms;

#[cfg(test)]
mod test_utils;

use transforms::{transform_block, transform_events, transform_extrinsics};

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

/// Main indexer service.
///
/// Subscribes to blocks and processes them as they arrive. Historical
/// backfill runs first (unless `--live-only`), then the live loop takes
/// over.
pub struct IndexerService<S: BlockSource, R: Repositories> {
    pub(super) config: IndexerConfig,
    pub(super) block_source: Arc<S>,
    pub(super) repositories: Arc<R>,
    pub(super) handlers: Arc<HandlerRegistry>,
    pub(super) event_bus: EventBus,
    /// In-memory mirror of the persisted cursor. Updated after each successful
    /// `persist_block_atomic` so `IndexerEvent::CursorAdvanced` and the
    /// `watch::CursorState` stay in sync without a post-persist DB round-trip.
    pub(super) cursor_mirror: Arc<Mutex<Option<CursorState>>>,
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
        // both its finalized head and runtime version.
        let initial_head = self.block_source.finalized_head().await?;
        let spec_version = self.block_source.runtime_version().await?;
        self.event_bus.update_chain(ChainState {
            connected: true,
            finalized_head: initial_head.number,
            spec_version,
        });

        self.seed_cursor_mirror(existing_cursor.as_ref()).await;

        let start_mode = if self.config.backfill.live_only {
            IndexMode::Live
        } else {
            IndexMode::Backfill
        };
        self.event_bus.emit_indexer(IndexerEvent::Started {
            mode: start_mode,
            start_block: self.config.backfill.start_block,
        });

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

    /// Pure per-block processor. Runs handlers and persists atomically.
    /// Does NOT check for reorgs, does NOT skip already-indexed blocks.
    /// Both live and backfill loops funnel through this.
    #[instrument(skip(self, raw_block), fields(block = raw_block.number, mode = mode.as_label()))]
    async fn index_single_block(&self, raw_block: RawBlock, mode: IndexMode) -> IndexerResult<()> {
        let _timer = ProcessingTimer::new();
        let started_at = Instant::now();
        let block = transform_block(&raw_block);

        for handler in self.handlers.all() {
            handler.on_block_start(&block).await?;
        }

        let extrinsics = transform_extrinsics(&raw_block, &block);
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

        let events = transform_events(&raw_block, &block);

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
        let block_hash = block.hash;
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
            *mirror = Some(next);
            next
        };
        self.event_bus.update_cursor(new_state);
        self.event_bus.emit_indexer(IndexerEvent::CursorAdvanced {
            head: new_state.head,
            tail: new_state.tail,
        });

        trace!("Block processed successfully");
        Ok(())
    }

    /// Live-loop wrapper. Skips already-indexed blocks, checks reorg, calls
    /// `index_single_block`. Returns `Ok(true)` if the block was indexed,
    /// `Ok(false)` if it was skipped.
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
}

#[cfg(test)]
mod event_emission_tests {
    //! Phase 5 wiring: drive `index_single_block` directly and assert the
    //! right events land on the bus + watches. Tests `run_live_loop`
    //! lifecycle separately via `run_inner_publishes_chain_state_…`.
    use super::*;
    use crate::events::{BackfillEvent, ChainEvent};

    use super::test_utils::{build_service, mk_raw};

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

        match indexer_rx.try_recv().expect("no CursorAdvanced emitted") {
            IndexerEvent::CursorAdvanced { head, tail } => {
                assert_eq!(head, 42);
                assert_eq!(tail, 42);
            }
            other => panic!("expected CursorAdvanced, got {other:?}"),
        }

        let state = *cursor_rx.borrow();
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

        let handle = tokio::spawn(async move {
            let _ = svc.run(shutdown_rx).await;
        });

        for _ in 0..8 {
            tokio::task::yield_now().await;
        }

        shutdown_tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(2), handle)
            .await
            .expect("indexer did not terminate promptly on shutdown")
            .expect("indexer task panicked");

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
