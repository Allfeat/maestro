//! Aggregated indexer state rendered by the TUI.
//!
//! [`AppState`] folds every `EventBus` channel into a single observable
//! snapshot so the render loop can paint a dashboard without re-querying
//! the bus. It owns no I/O — the `run` loop feeds it events and then hands
//! a `&AppState` to the draw function each frame.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use maestro_core::events::{
    BackfillEvent, ChainEvent, ChainState, CursorState, HandlerEvent, IndexerEvent, StopReason,
};
use maestro_core::services::IndexMode;
use tracing::Level;

use super::log_layer::LogBuffer;

const LATENCY_WINDOW: usize = 64;
/// Max samples kept for the backfill rate estimator.
const BACKFILL_RATE_WINDOW: usize = 64;
/// Samples older than this window are evicted even if under the max count,
/// so a stalled backfill doesn't keep reporting an ancient (now stale) rate.
const BACKFILL_RATE_MAX_AGE: Duration = Duration::from_secs(30);
/// How many blocks to keep in the animated tape. Rendering truncates to what
/// actually fits on screen — this is only the ceiling for the in-memory deque.
const BLOCKS_TAPE_CAPACITY: usize = 64;

/// Minimum level displayed in the logs panel. `All` leaves the EnvFilter
/// output untouched; the others progressively hide lower-severity lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFilter {
    #[default]
    All,
    Info,
    Warn,
    Error,
}

impl LogFilter {
    pub fn as_label(self) -> &'static str {
        match self {
            LogFilter::All => "all",
            LogFilter::Info => "info+",
            LogFilter::Warn => "warn+",
            LogFilter::Error => "error",
        }
    }

    /// Lowest severity (in terms of `Level` ordering) kept by this filter.
    /// `tracing::Level` is ordered so that ERROR < WARN < INFO < DEBUG < TRACE,
    /// therefore "min level" means "levels `<=` this value are kept".
    pub fn min_level(self) -> Option<Level> {
        match self {
            LogFilter::All => None,
            LogFilter::Info => Some(Level::INFO),
            LogFilter::Warn => Some(Level::WARN),
            LogFilter::Error => Some(Level::ERROR),
        }
    }

    pub fn next(self) -> Self {
        match self {
            LogFilter::All => LogFilter::Info,
            LogFilter::Info => LogFilter::Warn,
            LogFilter::Warn => LogFilter::Error,
            LogFilter::Error => LogFilter::All,
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct HandlerStats {
    pub events_processed: u64,
    pub persisted: u64,
    pub errors: u64,
}

#[derive(Clone, Copy)]
pub struct LastBlockInfo {
    pub number: u64,
    pub extrinsics: u32,
    pub events: u32,
    pub duration_ms: u32,
}

/// A block that appeared in the animated blockchain tape. `inserted_at`
/// drives the pulse colouring — new cells flash bright, older ones fade
/// to the muted palette.
#[derive(Clone, Copy)]
pub struct BlockAnim {
    pub number: u64,
    pub inserted_at: Instant,
}

#[derive(Clone, Copy)]
pub struct BackfillPlan {
    pub from: u64,
    pub to: u64,
    pub total: u64,
    pub persisted: u64,
}

pub struct AppState {
    started_at: Instant,
    pub mode: Option<IndexMode>,
    pub start_block: Option<u64>,
    pub cursor: CursorState,
    pub chain: ChainState,
    pub handlers: BTreeMap<&'static str, HandlerStats>,
    pub last_block: Option<LastBlockInfo>,
    latency_samples: VecDeque<u32>,
    pub backfill: Option<BackfillPlan>,
    /// Sliding window of `(tick, cumulative persisted)` pairs used to compute
    /// the blocks/sec rate and ETA. Reset whenever a new plan is published.
    backfill_history: VecDeque<(Instant, u64)>,
    /// Ring buffer of recently indexed blocks used to animate the blockchain
    /// tape. Newest block is at the back.
    blocks_tape: VecDeque<BlockAnim>,
    pub stop_reason: Option<String>,
    pub logs: Option<LogBuffer>,
    pub log_filter: LogFilter,
    /// Scroll offset in lines from the tail. `0` follows the newest line.
    pub log_scroll: usize,
}

impl AppState {
    pub fn new(logs: Option<LogBuffer>) -> Self {
        Self {
            started_at: Instant::now(),
            mode: None,
            start_block: None,
            cursor: CursorState::default(),
            chain: ChainState::default(),
            handlers: BTreeMap::new(),
            last_block: None,
            latency_samples: VecDeque::with_capacity(LATENCY_WINDOW),
            backfill: None,
            backfill_history: VecDeque::with_capacity(BACKFILL_RATE_WINDOW),
            blocks_tape: VecDeque::with_capacity(BLOCKS_TAPE_CAPACITY),
            stop_reason: None,
            logs,
            log_filter: LogFilter::default(),
            log_scroll: 0,
        }
    }

    pub fn cycle_log_filter(&mut self) {
        self.log_filter = self.log_filter.next();
        self.log_scroll = 0;
    }

    pub fn scroll_logs_up(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_add(amount);
    }

    pub fn scroll_logs_down(&mut self, amount: usize) {
        self.log_scroll = self.log_scroll.saturating_sub(amount);
    }

    pub fn scroll_logs_to_tail(&mut self) {
        self.log_scroll = 0;
    }

    pub fn set_cursor(&mut self, cursor: CursorState) {
        self.cursor = cursor;
    }

    pub fn set_chain_state(&mut self, chain: ChainState) {
        self.chain = chain;
    }

    pub fn on_indexer(&mut self, ev: IndexerEvent) {
        self.on_indexer_at(ev, Instant::now());
    }

    /// Test-friendly variant: accepts the current `Instant` so the block-tape
    /// animation can be exercised without sleeping.
    pub fn on_indexer_at(&mut self, ev: IndexerEvent, now: Instant) {
        match ev {
            IndexerEvent::Started { mode, start_block } => {
                self.mode = Some(mode);
                self.start_block = Some(start_block);
                self.stop_reason = None;
            }
            IndexerEvent::BlockIndexed {
                number,
                extrinsics,
                events,
                duration_ms,
                ..
            } => {
                self.last_block = Some(LastBlockInfo {
                    number,
                    extrinsics,
                    events,
                    duration_ms,
                });
                if self.latency_samples.len() == LATENCY_WINDOW {
                    self.latency_samples.pop_front();
                }
                self.latency_samples.push_back(duration_ms);

                if self.blocks_tape.len() == BLOCKS_TAPE_CAPACITY {
                    self.blocks_tape.pop_front();
                }
                self.blocks_tape.push_back(BlockAnim {
                    number,
                    inserted_at: now,
                });
            }
            IndexerEvent::CursorAdvanced { head, tail } => {
                self.cursor = CursorState { head, tail };
            }
            IndexerEvent::LiveModeEntered { .. } => {
                self.mode = Some(IndexMode::Live);
                self.backfill = None;
                self.backfill_history.clear();
            }
            IndexerEvent::Stopped { reason } => {
                self.stop_reason = Some(match reason {
                    StopReason::ShutdownRequested => "shutdown requested".to_string(),
                    StopReason::Fatal(msg) => format!("fatal: {msg}"),
                });
            }
        }
    }

    pub fn on_backfill(&mut self, ev: BackfillEvent) {
        self.on_backfill_at(ev, Instant::now());
    }

    /// Test-friendly variant that accepts a caller-provided timestamp so
    /// rate/ETA logic can be exercised without sleeping.
    pub fn on_backfill_at(&mut self, ev: BackfillEvent, now: Instant) {
        match ev {
            BackfillEvent::Planned { from, to, total } => {
                self.backfill = Some(BackfillPlan {
                    from,
                    to,
                    total,
                    persisted: 0,
                });
                self.backfill_history.clear();
            }
            BackfillEvent::BlockPersisted { .. } => {
                if let Some(bf) = self.backfill.as_mut() {
                    bf.persisted = bf.persisted.saturating_add(1).min(bf.total);
                    let persisted = bf.persisted;
                    self.push_backfill_sample(now, persisted);
                }
            }
            BackfillEvent::Aborted { reason } => {
                self.stop_reason = Some(format!("backfill aborted: {reason}"));
                self.backfill = None;
                self.backfill_history.clear();
            }
            BackfillEvent::BlockFetched { .. }
            | BackfillEvent::RangeCompleted { .. }
            | BackfillEvent::FetchRetried { .. } => {}
        }
    }

    fn push_backfill_sample(&mut self, now: Instant, persisted: u64) {
        while let Some((t0, _)) = self.backfill_history.front() {
            if now.duration_since(*t0) > BACKFILL_RATE_MAX_AGE {
                self.backfill_history.pop_front();
            } else {
                break;
            }
        }
        if self.backfill_history.len() == BACKFILL_RATE_WINDOW {
            self.backfill_history.pop_front();
        }
        self.backfill_history.push_back((now, persisted));
    }

    pub fn on_handler(&mut self, ev: HandlerEvent) {
        match ev {
            HandlerEvent::EventProcessed { pallet, .. } => {
                self.handlers.entry(pallet).or_default().events_processed += 1;
            }
            HandlerEvent::Persisted { pallet, count, .. } => {
                self.handlers.entry(pallet).or_default().persisted += count as u64;
            }
            HandlerEvent::Error { pallet, .. } => {
                self.handlers.entry(pallet).or_default().errors += 1;
            }
        }
    }

    pub fn on_chain(&mut self, ev: ChainEvent) {
        match ev {
            ChainEvent::RpcConnected { .. } => self.chain.connected = true,
            ChainEvent::RpcDisconnected { .. } | ChainEvent::RpcReconnecting { .. } => {
                self.chain.connected = false;
            }
            ChainEvent::RuntimeUpgraded { spec_version } => {
                self.chain.spec_version = spec_version;
            }
        }
    }

    pub fn avg_latency_ms(&self) -> Option<f64> {
        if self.latency_samples.is_empty() {
            return None;
        }
        let sum: u64 = self.latency_samples.iter().map(|x| *x as u64).sum();
        Some(sum as f64 / self.latency_samples.len() as f64)
    }

    pub fn backfill_ratio(&self) -> Option<f64> {
        let bf = self.backfill?;
        if bf.total == 0 {
            return Some(1.0);
        }
        Some((bf.persisted as f64 / bf.total as f64).clamp(0.0, 1.0))
    }

    /// Blocks/sec over the current sliding window. `None` until at least two
    /// samples exist, or when they are all clustered inside the same tick.
    pub fn backfill_rate(&self) -> Option<f64> {
        if self.backfill_history.len() < 2 {
            return None;
        }
        let (t0, c0) = *self.backfill_history.front()?;
        let (t1, c1) = *self.backfill_history.back()?;
        let dt = t1.duration_since(t0).as_secs_f64();
        if dt <= 0.0 {
            return None;
        }
        let dc = c1.saturating_sub(c0) as f64;
        if dc <= 0.0 {
            return None;
        }
        Some(dc / dt)
    }

    /// Estimated time to drain the remaining backfill at the current rate.
    pub fn backfill_eta(&self) -> Option<Duration> {
        let rate = self.backfill_rate()?;
        let bf = self.backfill?;
        let remaining = bf.total.saturating_sub(bf.persisted);
        if remaining == 0 {
            return Some(Duration::ZERO);
        }
        Some(Duration::from_secs_f64(remaining as f64 / rate))
    }

    pub fn uptime(&self) -> Duration {
        self.started_at.elapsed()
    }

    pub fn blocks_tape(&self) -> &VecDeque<BlockAnim> {
        &self.blocks_tape
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maestro_core::models::BlockHash;

    #[test]
    fn indexer_events_update_state() {
        let mut s = AppState::new(None);
        s.on_indexer(IndexerEvent::Started {
            mode: IndexMode::Backfill,
            start_block: 10,
        });
        assert_eq!(s.mode, Some(IndexMode::Backfill));
        assert_eq!(s.start_block, Some(10));

        s.on_indexer(IndexerEvent::BlockIndexed {
            number: 11,
            hash: BlockHash([0u8; 32]),
            extrinsics: 2,
            events: 5,
            duration_ms: 20,
        });
        assert_eq!(s.last_block.map(|b| b.number), Some(11));
        assert_eq!(s.avg_latency_ms(), Some(20.0));

        s.on_indexer(IndexerEvent::CursorAdvanced { head: 11, tail: 0 });
        assert_eq!(s.cursor.head, 11);

        s.on_indexer(IndexerEvent::LiveModeEntered { from_block: 11 });
        assert_eq!(s.mode, Some(IndexMode::Live));
    }

    #[test]
    fn handler_events_accumulate_per_pallet() {
        let mut s = AppState::new(None);
        s.on_handler(HandlerEvent::EventProcessed {
            pallet: "Balances",
            event_name: "Transfer".into(),
            block: 1,
        });
        s.on_handler(HandlerEvent::Persisted {
            pallet: "Balances",
            table: "transfers",
            count: 3,
            block: 1,
        });
        s.on_handler(HandlerEvent::Error {
            pallet: "Balances",
            block: 1,
            error: "boom".into(),
        });
        let stats = s.handlers.get("Balances").copied().unwrap();
        assert_eq!(stats.events_processed, 1);
        assert_eq!(stats.persisted, 3);
        assert_eq!(stats.errors, 1);
    }

    #[test]
    fn backfill_progress_tracks_persisted_events() {
        let mut s = AppState::new(None);
        s.on_backfill(BackfillEvent::Planned {
            from: 0,
            to: 9,
            total: 10,
        });
        for _ in 0..5 {
            s.on_backfill(BackfillEvent::BlockPersisted { number: 0 });
        }
        assert_eq!(s.backfill_ratio(), Some(0.5));

        s.on_backfill(BackfillEvent::Aborted {
            reason: "fatal".into(),
        });
        assert!(s.backfill.is_none());
        assert!(s.stop_reason.as_deref().unwrap().contains("fatal"));
    }

    #[test]
    fn chain_events_flip_connection_state() {
        let mut s = AppState::new(None);
        s.on_chain(ChainEvent::RpcConnected {
            url: "ws://node".into(),
        });
        assert!(s.chain.connected);
        s.on_chain(ChainEvent::RpcDisconnected {
            reason: "eof".into(),
        });
        assert!(!s.chain.connected);
        s.on_chain(ChainEvent::RuntimeUpgraded { spec_version: 42 });
        assert_eq!(s.chain.spec_version, 42);
    }

    #[test]
    fn log_filter_cycles_through_levels_and_resets_scroll() {
        let mut s = AppState::new(None);
        s.log_scroll = 12;
        assert_eq!(s.log_filter, LogFilter::All);
        s.cycle_log_filter();
        assert_eq!(s.log_filter, LogFilter::Info);
        assert_eq!(s.log_scroll, 0);
        s.cycle_log_filter();
        assert_eq!(s.log_filter, LogFilter::Warn);
        s.cycle_log_filter();
        assert_eq!(s.log_filter, LogFilter::Error);
        s.cycle_log_filter();
        assert_eq!(s.log_filter, LogFilter::All);
    }

    #[test]
    fn log_scroll_helpers_saturate_at_zero() {
        let mut s = AppState::new(None);
        s.scroll_logs_up(5);
        assert_eq!(s.log_scroll, 5);
        s.scroll_logs_down(100);
        assert_eq!(s.log_scroll, 0);
        s.scroll_logs_up(3);
        s.scroll_logs_to_tail();
        assert_eq!(s.log_scroll, 0);
    }

    #[test]
    fn backfill_rate_and_eta_from_sliding_window() {
        let mut s = AppState::new(None);
        let t0 = Instant::now();
        s.on_backfill_at(
            BackfillEvent::Planned {
                from: 0,
                to: 99,
                total: 100,
            },
            t0,
        );
        // 10 samples spaced 200 ms apart span 1.8 s between the first and
        // last entry, so rate = 9 / 1.8 = 5.0 b/s. Remaining = 90 → ETA 18 s.
        for i in 0..10u64 {
            let at = t0 + Duration::from_millis(200 * i);
            s.on_backfill_at(BackfillEvent::BlockPersisted { number: i }, at);
        }
        let rate = s.backfill_rate().expect("rate available");
        assert!((rate - 5.0).abs() < 1e-6, "rate was {rate}");
        let eta = s.backfill_eta().expect("eta available");
        assert_eq!(eta.as_secs(), 18);
    }

    #[test]
    fn backfill_rate_is_none_with_a_single_sample() {
        let mut s = AppState::new(None);
        let t0 = Instant::now();
        s.on_backfill_at(
            BackfillEvent::Planned {
                from: 0,
                to: 9,
                total: 10,
            },
            t0,
        );
        s.on_backfill_at(BackfillEvent::BlockPersisted { number: 0 }, t0);
        assert!(s.backfill_rate().is_none());
        assert!(s.backfill_eta().is_none());
    }

    #[test]
    fn backfill_history_drops_samples_older_than_window() {
        let mut s = AppState::new(None);
        let t0 = Instant::now();
        s.on_backfill_at(
            BackfillEvent::Planned {
                from: 0,
                to: 999,
                total: 1000,
            },
            t0,
        );
        s.on_backfill_at(BackfillEvent::BlockPersisted { number: 0 }, t0);
        // Push a second sample > MAX_AGE later — the first must be evicted.
        let later = t0 + BACKFILL_RATE_MAX_AGE + Duration::from_secs(1);
        s.on_backfill_at(BackfillEvent::BlockPersisted { number: 1 }, later);
        assert_eq!(s.backfill_history.len(), 1);
    }

    #[test]
    fn planned_event_resets_rate_history() {
        let mut s = AppState::new(None);
        let t0 = Instant::now();
        s.on_backfill_at(
            BackfillEvent::Planned {
                from: 0,
                to: 9,
                total: 10,
            },
            t0,
        );
        for i in 0..5u64 {
            s.on_backfill_at(
                BackfillEvent::BlockPersisted { number: i },
                t0 + Duration::from_millis(100 * i),
            );
        }
        assert_eq!(s.backfill_history.len(), 5);
        s.on_backfill_at(
            BackfillEvent::Planned {
                from: 100,
                to: 199,
                total: 100,
            },
            t0 + Duration::from_secs(1),
        );
        assert!(s.backfill_history.is_empty());
    }

    #[test]
    fn avg_latency_is_a_sliding_window() {
        let mut s = AppState::new(None);
        for i in 0..(LATENCY_WINDOW as u32 + 10) {
            s.on_indexer(IndexerEvent::BlockIndexed {
                number: i as u64,
                hash: BlockHash([0u8; 32]),
                extrinsics: 0,
                events: 0,
                duration_ms: i,
            });
        }
        assert_eq!(s.latency_samples.len(), LATENCY_WINDOW);
    }
}
