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
    BlockData, BlockSource, HandlerOutputs, HandlerRegistry, RawBlock, Repositories,
};

// =============================================================================
// Configuration
// =============================================================================

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
}

impl Default for IndexerConfig {
    fn default() -> Self {
        Self {
            chain_id: String::new(),
            poll_interval: Duration::from_secs(12),
            max_retries: 3,
            retry_delay: Duration::from_secs(1),
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

impl<S: BlockSource, R: Repositories> IndexerService<S, R> {
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
    /// Subscribes to finalized blocks and processes them as they arrive.
    #[instrument(skip_all, fields(chain = %&self.config.chain_id[..16.min(self.config.chain_id.len())]))]
    pub async fn run(
        &self,
        mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> IndexerResult<()> {
        info!("⛓️  Starting indexer");

        // Verify we're connecting to the correct chain
        self.verify_chain_id().await?;

        let head = self.block_source.finalized_head().await?;
        debug!(head = head.number, "Chain head detected");

        self.follow_finalized(&mut shutdown_rx).await
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
    async fn verify_consistency_on_reconnect(&self) -> IndexerResult<Option<u64>> {
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
            Some(block) => {
                if block.hash != cursor.last_indexed_hash {
                    warn!(
                        block = cursor.last_indexed_block,
                        cursor_hash = %hex::encode(&cursor.last_indexed_hash.0[..8]),
                        stored_hash = %hex::encode(&block.hash.0[..8]),
                        "⚠️  Cursor hash mismatch, cleaning up"
                    );
                    let deleted = self
                        .repositories
                        .delete_from_block_atomic(cursor.last_indexed_block, &self.config.chain_id)
                        .await?;
                    info!(deleted = deleted, "🗑️  Cleaned inconsistent data");
                    record_blocks_deleted(deleted);
                    return Ok(None);
                }
                debug!(
                    block = cursor.last_indexed_block,
                    "Cursor verified, resuming"
                );
                Ok(Some(cursor.last_indexed_block))
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
                info!(deleted = deleted, "🗑️  Cleaned inconsistent data");
                record_blocks_deleted(deleted);
                Ok(None)
            }
        }
    }

    /// Follow finalized blocks via subscription.
    #[instrument(skip_all)]
    async fn follow_finalized(
        &self,
        shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
    ) -> IndexerResult<()> {
        use std::time::Duration;

        debug!("Subscribing to finalized blocks");
        let _last_indexed = self.verify_consistency_on_reconnect().await?;

        // Exponential backoff configuration
        const INITIAL_RETRY_DELAY: Duration = Duration::from_millis(500);
        const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);
        let mut retry_delay = INITIAL_RETRY_DELAY;

        loop {
            if *shutdown_rx.borrow() {
                debug!("Shutdown requested");
                return Err(IndexerError::ShutdownRequested);
            }

            match self.block_source.subscribe_finalized().await {
                Ok(mut stream) => {
                    debug!("📡 Subscription established");
                    retry_delay = INITIAL_RETRY_DELAY; // Reset backoff on success

                    while let Some(result) = stream.next().await {
                        if *shutdown_rx.borrow() {
                            debug!("Shutdown requested");
                            return Err(IndexerError::ShutdownRequested);
                        }

                        match result {
                            Ok(raw_block) => {
                                let block_number = raw_block.number;
                                match self.process_block(raw_block).await {
                                    Ok(true) => {
                                        info!(block = block_number, "⛓️  Block indexed");
                                    }
                                    Ok(false) => {
                                        trace!(
                                            block = block_number,
                                            "Block skipped (already indexed)"
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

    /// Process a single block through all handlers.
    /// Returns `Ok(true)` if processed, `Ok(false)` if skipped.
    #[instrument(skip(self, raw_block), fields(block = raw_block.number))]
    async fn process_block(&self, raw_block: RawBlock) -> IndexerResult<bool> {
        let block_number = raw_block.number;
        trace!("Processing block");

        // Skip already indexed blocks (happens on reconnect)
        if let Some(existing_block) = self.repositories.blocks().get_block(block_number).await? {
            let incoming_hash = BlockHash(raw_block.hash);
            if existing_block.hash == incoming_hash {
                trace!("Block already indexed, skipping");
                return Ok(false);
            }
            trace!("Block hash differs, checking for reorg");
        }

        // Check for reorg
        let reorg_handled = self.check_and_handle_reorg(&raw_block).await?;
        if reorg_handled {
            trace!("Reorg handled, continuing with block");
        }

        let _timer = ProcessingTimer::new();
        let block = self.transform_block(&raw_block);

        // Handler lifecycle: on_block_start
        for handler in self.handlers.all() {
            handler.on_block_start(&block).await?;
        }

        let extrinsics = self.transform_extrinsics(&raw_block, &block);
        let mut all_outputs = HandlerOutputs::new();

        // Process events through handlers
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

        // Process extrinsics through handlers
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

        let cursor = IndexerCursor {
            chain_id: self.config.chain_id.clone(),
            last_indexed_block: block.number,
            last_indexed_hash: block.hash.clone(),
            updated_at: chrono::Utc::now(),
        };

        let block_data = BlockData {
            block: &block,
            extrinsics: &extrinsics,
            events: &events,
            cursor: &cursor,
        };

        // Persist block data first (so foreign key constraints are satisfied)
        self.repositories.persist_block_atomic(block_data).await?;

        // Handler lifecycle: on_block_end (after block is persisted)
        for handler in self.handlers.all() {
            match handler.on_block_end(&block, &all_outputs).await {
                Ok(outputs) => all_outputs.merge(outputs),
                Err(e) => {
                    error!(error = ?e, "❌ Handler on_block_end failed");
                }
            }
        }

        record_block_indexed();
        trace!("Block processed successfully");
        Ok(true)
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
