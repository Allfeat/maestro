//! Core indexer service - orchestrates block processing.
//!
//! This service is designed for chain head indexing only (v1).
//! It subscribes to finalized blocks and processes them in real-time.

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::error::{IndexerError, IndexerResult};
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
    ) -> Self {
        Self {
            config,
            block_source,
            repositories,
            handlers,
        }
    }

    /// Start the indexer.
    ///
    /// Subscribes to blocks and processes them as they arrive.
    /// The subscription mode is determined by `config.block_mode`.
    #[instrument(skip_all, fields(chain = %&self.config.chain_id[..16.min(self.config.chain_id.len())]))]
    pub async fn run(
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

        // Live-only escape hatch.
        if self.config.backfill.live_only {
            self.warn_if_cursor_gap(&existing_cursor).await;
            return self
                .run_live_loop(&mut shutdown_rx, self.config.block_mode)
                .await;
        }

        // Plan backfill.
        let tip = self.block_source.finalized_head().await?.number;
        let plan = BackfillPlan::compute(
            self.config.backfill.start_block,
            existing_cursor.as_ref(),
            tip,
        );

        if !plan.ranges.is_empty() {
            info!(ranges = ?plan.ranges, "🕰  Starting backfill");
            let runner = BackfillRunner::new(
                self.block_source.clone(),
                self.config.backfill.clone(),
                self.config.chain_id.clone(),
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

        self.run_live_loop(&mut shutdown_rx, self.config.block_mode)
            .await
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

                    while let Some(result) = stream.next().await {
                        if *shutdown_rx.borrow() {
                            debug!("Shutdown requested");
                            return Err(IndexerError::ShutdownRequested);
                        }

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
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(retry_delay) => {
                    debug!(retry_delay_ms = retry_delay.as_millis(), "🔄 Reconnecting to chain...");
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
