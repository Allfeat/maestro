//! Historical backfill planning and execution.
//!
//! `BackfillPlan::compute` is a pure function that reconciles the requested
//! `start_block` against an existing cursor and the current tip, producing
//! 0–2 `BackfillRange`s. The downward-then-upward ordering preserves the
//! single-contiguous-range cursor invariant across crashes.

use crate::error::ChainError;
use crate::events::{BackfillEvent, EventBus};
use crate::metrics::record_backfill_fetch_retry;
use crate::models::IndexerCursor;
use crate::ports::{BlockSource, RawBlock};
use std::time::Duration;
use tracing::warn;

/// Retry wrapper around `BlockSource::fetch_block_at` with bounded exponential
/// backoff (250ms → 10s). Returns `Err((block_number, ChainError))` on budget
/// exhaustion so the caller can report the offending block. When an
/// `EventBus` is provided, each retry is echoed as `BackfillEvent::FetchRetried`.
pub async fn fetch_with_retry<S: BlockSource + ?Sized>(
    source: &S,
    block: u64,
    max_retries: u32,
    event_bus: Option<&EventBus>,
) -> Result<RawBlock, (u64, ChainError)> {
    let mut delay = Duration::from_millis(250);
    let mut attempt: u32 = 0;
    loop {
        match source.fetch_block_at(block).await {
            Ok(raw) => return Ok(raw),
            Err(e) if attempt < max_retries => {
                warn!(block, attempt, error = %e, "backfill fetch failed, retrying");
                record_backfill_fetch_retry();
                if let Some(bus) = event_bus {
                    bus.emit_backfill(BackfillEvent::FetchRetried {
                        number: block,
                        attempt: attempt + 1,
                        error: e.to_string(),
                    });
                }
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
                attempt += 1;
            }
            Err(e) => return Err((block, e)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillDirection {
    Upward,
    Downward,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackfillRange {
    /// Inclusive lower bound.
    pub from: u64,
    /// Inclusive upper bound. Always `from <= to`.
    pub to: u64,
    pub direction: BackfillDirection,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BackfillPlan {
    /// Ordered ranges to execute. Downward ranges always precede upward ranges
    /// to preserve the contiguous-range invariant under crashes.
    pub ranges: Vec<BackfillRange>,
}

impl BackfillPlan {
    pub fn compute(start_block: u64, existing_cursor: Option<&IndexerCursor>, tip: u64) -> Self {
        let mut ranges = Vec::new();

        match existing_cursor {
            None => {
                // Fresh DB. Single upward range from start_block to tip, if any.
                if start_block <= tip {
                    ranges.push(BackfillRange {
                        from: start_block,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
            Some(c) if start_block >= c.first_indexed_block => {
                // Already have as much or more history. Resume upward from cursor top.
                if c.last_indexed_block < tip {
                    ranges.push(BackfillRange {
                        from: c.last_indexed_block + 1,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
            Some(c) => {
                // start_block < c.first_indexed_block: two-phase extend.
                // Phase 1: gap-fill downward [start_block, first - 1].
                ranges.push(BackfillRange {
                    from: start_block,
                    to: c.first_indexed_block - 1,
                    direction: BackfillDirection::Downward,
                });
                // Phase 2: forward-extend upward [last + 1, tip] (if any).
                if c.last_indexed_block < tip {
                    ranges.push(BackfillRange {
                        from: c.last_indexed_block + 1,
                        to: tip,
                        direction: BackfillDirection::Upward,
                    });
                }
            }
        }

        BackfillPlan { ranges }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use crate::error::{ChainError, ChainResult};
    use crate::ports::{BlockSource, FinalizedBlockStream, FinalizedHead, RawBlock};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A tiny mock that fails `fail_count` times then succeeds.
    struct FlakySource {
        fail_count: AtomicU32,
    }

    fn stub_block(number: u64) -> RawBlock {
        RawBlock {
            number,
            hash: [0u8; 32],
            parent_hash: [0u8; 32],
            state_root: [0u8; 32],
            extrinsics_root: [0u8; 32],
            extrinsics: vec![],
            events: vec![],
            timestamp: None,
        }
    }

    #[async_trait]
    impl BlockSource for FlakySource {
        async fn genesis_hash(&self) -> ChainResult<crate::models::BlockHash> {
            Ok(crate::models::BlockHash([0u8; 32]))
        }
        async fn finalized_head(&self) -> ChainResult<FinalizedHead> {
            Ok(FinalizedHead {
                number: 0,
                hash: [0u8; 32],
            })
        }
        async fn best_head(&self) -> ChainResult<FinalizedHead> {
            Ok(FinalizedHead {
                number: 0,
                hash: [0u8; 32],
            })
        }
        async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream> {
            unimplemented!()
        }
        async fn subscribe_best(&self) -> ChainResult<FinalizedBlockStream> {
            unimplemented!()
        }
        async fn runtime_version(&self) -> ChainResult<u32> {
            Ok(1)
        }
        async fn fetch_block_at(&self, number: u64) -> ChainResult<RawBlock> {
            let remaining = self.fail_count.load(Ordering::SeqCst);
            if remaining > 0 {
                self.fail_count.fetch_sub(1, Ordering::SeqCst);
                return Err(ChainError::RpcError(format!("flaky at {number}")));
            }
            Ok(stub_block(number))
        }
        async fn earliest_v14_block(&self) -> ChainResult<u64> {
            Ok(0)
        }
    }

    #[tokio::test]
    async fn succeeds_after_retries() {
        let src = FlakySource {
            fail_count: AtomicU32::new(2),
        };
        let result = fetch_with_retry(&src, 42, 5, None).await;
        assert!(result.is_ok(), "should succeed on 3rd attempt");
        assert_eq!(result.unwrap().number, 42);
    }

    #[tokio::test]
    async fn errors_when_retry_budget_exhausted() {
        let src = FlakySource {
            fail_count: AtomicU32::new(10),
        };
        let result = fetch_with_retry(&src, 42, 3, None).await;
        match result {
            Err((block, _)) => assert_eq!(block, 42),
            Ok(_) => panic!("expected retry exhaustion"),
        }
    }
}

#[cfg(test)]
mod plan_tests {
    use super::*;
    use crate::models::BlockHash;
    use chrono::Utc;

    fn cursor(first: u64, last: u64) -> IndexerCursor {
        IndexerCursor {
            chain_id: "test".into(),
            first_indexed_block: first,
            last_indexed_block: last,
            last_indexed_hash: BlockHash([0u8; 32]),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn fresh_db_backfills_from_start_to_tip() {
        let plan = BackfillPlan::compute(0, None, 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn fresh_db_respects_non_zero_start_block() {
        let plan = BackfillPlan::compute(500, None, 1000);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 500);
        assert_eq!(plan.ranges[0].to, 1000);
    }

    #[test]
    fn resume_ignores_start_when_covered() {
        let plan = BackfillPlan::compute(10, Some(&cursor(0, 50)), 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 51);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn extend_below_triggers_two_phase_downward_then_upward() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 300);
        assert_eq!(plan.ranges.len(), 2);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 99);
        assert_eq!(plan.ranges[1].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[1].from, 201);
        assert_eq!(plan.ranges[1].to, 300);
    }

    #[test]
    fn already_at_tip_plans_nothing() {
        let plan = BackfillPlan::compute(0, Some(&cursor(0, 100)), 100);
        assert!(plan.ranges.is_empty());
    }

    #[test]
    fn downward_only_when_no_upward_needed() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 200);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 99);
    }
}

use crate::error::{IndexerError, IndexerResult};
use crate::services::BackfillConfig;
use futures::stream::StreamExt;
use std::sync::Arc;
use tokio::sync::watch;
use tracing::info;

/// Drives a `BackfillRange` by streaming per-block fetches through `buffered(K)`
/// and handing each decoded `RawBlock` to a caller-provided processor closure.
/// The closure shape avoids a circular ownership problem with `IndexerService`.
pub struct BackfillRunner<S: BlockSource> {
    block_source: Arc<S>,
    config: BackfillConfig,
    chain_id: String,
    event_bus: EventBus,
}

impl<S: BlockSource + 'static> BackfillRunner<S> {
    pub fn new(
        block_source: Arc<S>,
        config: BackfillConfig,
        chain_id: String,
        event_bus: EventBus,
    ) -> Self {
        Self {
            block_source,
            config,
            chain_id,
            event_bus,
        }
    }
}

impl<S: BlockSource + 'static> BackfillRunner<S> {
    pub async fn run_range<F, Fut>(
        &self,
        range: BackfillRange,
        shutdown_rx: &mut watch::Receiver<bool>,
        mut index_single_block: F,
    ) -> IndexerResult<()>
    where
        F: FnMut(RawBlock) -> Fut,
        Fut: std::future::Future<Output = IndexerResult<()>>,
    {
        let total = range.to - range.from + 1;
        info!(
            chain_id = %self.chain_id,
            from = range.from,
            to = range.to,
            direction = ?range.direction,
            total,
            "backfill range starting"
        );

        // Build block-number iterator (boxed so both branches have the same type).
        let block_numbers: Box<dyn Iterator<Item = u64> + Send> = match range.direction {
            BackfillDirection::Upward => Box::new(range.from..=range.to),
            BackfillDirection::Downward => Box::new((range.from..=range.to).rev()),
        };

        let source = self.block_source.clone();
        let max_retries = self.config.max_fetch_retries;
        let concurrency = self.config.concurrency.max(1);
        let fetch_bus = self.event_bus.clone();

        let mut fetched = futures::stream::iter(block_numbers)
            .map(move |n| {
                let source = source.clone();
                let bus = fetch_bus.clone();
                async move { fetch_with_retry(&*source, n, max_retries, Some(&bus)).await }
            })
            .buffered(concurrency);

        let mut indexed: u64 = 0;
        while let Some(result) = fetched.next().await {
            if *shutdown_rx.borrow() {
                info!(indexed, total, "backfill interrupted by shutdown");
                return Err(IndexerError::ShutdownRequested);
            }

            let raw_block = match result {
                Ok(raw) => raw,
                Err((block, err)) => {
                    crate::metrics::record_backfill_aborted();
                    let reason = err.to_string();
                    self.event_bus.emit_backfill(BackfillEvent::Aborted {
                        reason: reason.clone(),
                    });
                    return Err(IndexerError::BackfillAborted { block, reason });
                }
            };
            let block_number = raw_block.number;
            self.event_bus.emit_backfill(BackfillEvent::BlockFetched {
                number: block_number,
            });

            index_single_block(raw_block).await?;

            self.event_bus.emit_backfill(BackfillEvent::BlockPersisted {
                number: block_number,
            });

            indexed += 1;
            crate::metrics::record_backfill_block_indexed();
            if indexed.is_multiple_of(1000) {
                info!(indexed, remaining = total - indexed, "backfill progress");
                crate::metrics::record_backfill_progress(indexed, total - indexed);
            }
        }

        info!(indexed, total, "backfill range complete");
        crate::metrics::record_backfill_progress(indexed, total.saturating_sub(indexed));
        self.event_bus.emit_backfill(BackfillEvent::RangeCompleted {
            from: range.from,
            to: range.to,
        });
        Ok(())
    }
}
