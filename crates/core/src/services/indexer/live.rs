//! Live subscription loop with exponential backoff + chain-state publishing.
//!
//! This module owns everything that happens *after* startup checks pass and
//! the service commits to streaming new blocks: subscription management,
//! reconnection, and the chain-watch publishing helpers.

use std::time::Duration;

use futures::StreamExt;
use tracing::{debug, error, info, instrument, trace, warn};

use crate::error::IndexerError;
use crate::error::IndexerResult;
use crate::events::{ChainEvent, ChainState};
use crate::ports::{BlockMode, BlockSource, Repositories};

use super::IndexerService;

impl<S: BlockSource + 'static, R: Repositories> IndexerService<S, R> {
    /// Follow blocks via subscription (finalized or best based on mode).
    #[instrument(skip_all, fields(mode = ?mode))]
    pub(super) async fn run_live_loop(
        &self,
        shutdown_rx: &mut tokio::sync::watch::Receiver<bool>,
        mode: BlockMode,
    ) -> IndexerResult<()> {
        let mode_label = match mode {
            BlockMode::Finalized => "finalized",
            BlockMode::Best => "best",
        };

        debug!(mode = mode_label, "Subscribing to blocks");

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

            let subscription = match mode {
                BlockMode::Finalized => self.block_source.subscribe_finalized().await,
                BlockMode::Best => self.block_source.subscribe_best().await,
            };

            match subscription {
                Ok(mut stream) => {
                    debug!(mode = mode_label, "📡 Subscription established");
                    retry_delay = INITIAL_RETRY_DELAY;
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
                    retry_delay = (retry_delay * 2).min(MAX_RETRY_DELAY);
                }
                _ = shutdown_rx.changed() => {
                    if *shutdown_rx.borrow() {
                        return Err(IndexerError::ShutdownRequested);
                    }
                }
            }
            trace!("Reconnect branch completed, looping");
        }
    }

    pub(super) fn publish_connected(&self) {
        // Scoped borrow: drop the read-lock BEFORE update_chain acquires
        // the write-lock — the `watch::Sender` uses an RwLock internally
        // and holding the receiver guard across the write call would
        // deadlock.
        let current = self.event_bus.watch_chain().borrow().clone();
        self.event_bus.update_chain(ChainState {
            connected: true,
            ..current
        });
    }

    pub(super) fn publish_disconnected(&self) {
        let current = self.event_bus.watch_chain().borrow().clone();
        self.event_bus.update_chain(ChainState {
            connected: false,
            ..current
        });
    }
}
