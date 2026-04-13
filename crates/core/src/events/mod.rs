//! In-process event bus.
//!
//! Decouples services (indexer, backfill, handlers) from observability
//! consumers (logger, metrics, future TUI). Uses tokio broadcast channels
//! for discrete events and watch channels for latest-value state.

pub mod backfill;
pub mod chain;
pub mod handler;
pub mod indexer;
pub mod logger;
pub mod metrics_bridge;

pub use backfill::BackfillEvent;
pub use chain::ChainEvent;
pub use handler::HandlerEvent;
pub use indexer::{IndexerEvent, StopReason};

use tokio::sync::{broadcast, watch};

/// Default broadcast channel capacity. Sized for a 60 Hz TUI consumer.
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// Latest-known indexer cursor state — published via a `watch` channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CursorState {
    /// Highest indexed block.
    pub head: u64,
    /// Lowest indexed block (backfill floor).
    pub tail: u64,
}

/// Latest-known chain connection state — published via a `watch` channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChainState {
    /// Whether the RPC connection is currently up.
    pub connected: bool,
    /// Latest finalized head reported by the node.
    pub finalized_head: u64,
    /// Chain runtime spec version.
    pub spec_version: u32,
}

/// In-process event bus. Cheap to clone — all channels are `Arc`-backed.
#[derive(Clone)]
pub struct EventBus {
    indexer: broadcast::Sender<IndexerEvent>,
    backfill: broadcast::Sender<BackfillEvent>,
    handler: broadcast::Sender<HandlerEvent>,
    chain: broadcast::Sender<ChainEvent>,
    cursor_state: watch::Sender<CursorState>,
    chain_state: watch::Sender<ChainState>,
}

impl EventBus {
    /// Construct a new bus with the given broadcast capacity per channel.
    pub fn new(capacity: usize) -> Self {
        let (indexer, _) = broadcast::channel(capacity);
        let (backfill, _) = broadcast::channel(capacity);
        let (handler, _) = broadcast::channel(capacity);
        let (chain, _) = broadcast::channel(capacity);
        let (cursor_state, _) = watch::channel(CursorState::default());
        let (chain_state, _) = watch::channel(ChainState::default());
        Self {
            indexer,
            backfill,
            handler,
            chain,
            cursor_state,
            chain_state,
        }
    }

    /// A bus with default capacity for tests and throwaway callers.
    /// Emits against `noop()` succeed silently even with zero subscribers
    /// (see the `emit_*` methods — they discard send errors).
    pub fn noop() -> Self {
        Self::new(DEFAULT_BUS_CAPACITY)
    }

    pub fn emit_indexer(&self, ev: IndexerEvent) {
        let _ = self.indexer.send(ev);
    }
    pub fn emit_backfill(&self, ev: BackfillEvent) {
        let _ = self.backfill.send(ev);
    }
    pub fn emit_handler(&self, ev: HandlerEvent) {
        let _ = self.handler.send(ev);
    }
    pub fn emit_chain(&self, ev: ChainEvent) {
        let _ = self.chain.send(ev);
    }

    pub fn update_cursor(&self, s: CursorState) {
        let _ = self.cursor_state.send(s);
    }
    pub fn update_chain(&self, s: ChainState) {
        let _ = self.chain_state.send(s);
    }

    pub fn subscribe_indexer(&self) -> broadcast::Receiver<IndexerEvent> {
        self.indexer.subscribe()
    }
    pub fn subscribe_backfill(&self) -> broadcast::Receiver<BackfillEvent> {
        self.backfill.subscribe()
    }
    pub fn subscribe_handler(&self) -> broadcast::Receiver<HandlerEvent> {
        self.handler.subscribe()
    }
    pub fn subscribe_chain(&self) -> broadcast::Receiver<ChainEvent> {
        self.chain.subscribe()
    }
    pub fn watch_cursor(&self) -> watch::Receiver<CursorState> {
        self.cursor_state.subscribe()
    }
    pub fn watch_chain(&self) -> watch::Receiver<ChainState> {
        self.chain_state.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new(DEFAULT_BUS_CAPACITY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_bus_new_and_noop_construct() {
        let bus = EventBus::new(1024);
        let _ = bus.subscribe_indexer();
        let _ = bus.subscribe_backfill();
        let _ = bus.subscribe_handler();
        let _ = bus.subscribe_chain();
        let _ = bus.watch_cursor();
        let _ = bus.watch_chain();

        let _noop = EventBus::noop();
    }

    #[test]
    fn cursor_state_defaults_reasonable() {
        let s = CursorState::default();
        assert_eq!(s.head, 0);
        assert_eq!(s.tail, 0);
    }

    #[test]
    fn chain_state_defaults_reasonable() {
        let s = ChainState::default();
        assert!(!s.connected);
        assert_eq!(s.finalized_head, 0);
    }
}
