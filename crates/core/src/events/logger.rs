//! Tracing logger consumer listening to the event bus.
//!
//! Turns events into structured `tracing` emissions. Runs as a background task.

use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

use super::{
    BackfillEvent, ChainEvent, ChainState, CursorState, EventBus, HandlerEvent, IndexerEvent,
    StopReason,
};

/// Render an `IndexerEvent` as a structured tracing entry.
pub fn log_indexer(ev: &IndexerEvent) {
    match ev {
        IndexerEvent::Started { mode, start_block } => {
            info!(mode = mode.as_label(), start_block, "indexer started");
        }
        IndexerEvent::BlockIndexed {
            number,
            extrinsics,
            events,
            duration_ms,
            ..
        } => {
            debug!(
                number,
                extrinsics, events, duration_ms, "block indexed"
            );
        }
        IndexerEvent::CursorAdvanced { head, tail } => {
            debug!(head, tail, "cursor advanced");
        }
        IndexerEvent::LiveModeEntered { from_block } => {
            info!(from_block, "entered live mode");
        }
        IndexerEvent::Stopped { reason } => match reason {
            StopReason::ShutdownRequested => info!("indexer stopped (shutdown requested)"),
            StopReason::Fatal(err) => error!(error = %err, "indexer stopped (fatal)"),
        },
    }
}

/// Render a `BackfillEvent` as a structured tracing entry.
pub fn log_backfill(ev: &BackfillEvent) {
    match ev {
        BackfillEvent::Planned { from, to, total } => {
            info!(from, to, total, "backfill planned");
        }
        BackfillEvent::BlockFetched { number } => {
            debug!(number, "backfill block fetched");
        }
        BackfillEvent::BlockPersisted { number } => {
            debug!(number, "backfill block persisted");
        }
        BackfillEvent::RangeCompleted { from, to } => {
            info!(from, to, "backfill range completed");
        }
        BackfillEvent::FetchRetried {
            number,
            attempt,
            error,
        } => {
            warn!(number, attempt, error = %error, "backfill fetch retried");
        }
        BackfillEvent::Aborted { reason } => {
            error!(reason = %reason, "backfill aborted");
        }
    }
}

/// Render a `HandlerEvent` as a structured tracing entry.
pub fn log_handler(ev: &HandlerEvent) {
    match ev {
        HandlerEvent::EventProcessed {
            pallet,
            event_name,
            block,
        } => {
            debug!(pallet, event = %event_name, block, "handler processed event");
        }
        HandlerEvent::Persisted {
            pallet,
            table,
            count,
            block,
        } => {
            debug!(pallet, table, count, block, "handler persisted");
        }
        HandlerEvent::Error {
            pallet,
            block,
            error,
        } => {
            warn!(pallet, block, error = %error, "handler error");
        }
    }
}

/// Render a `ChainEvent` as a structured tracing entry.
pub fn log_chain(ev: &ChainEvent) {
    match ev {
        ChainEvent::RpcConnected { url } => info!(url = %url, "rpc connected"),
        ChainEvent::RpcDisconnected { reason } => warn!(reason = %reason, "rpc disconnected"),
        ChainEvent::RpcReconnecting { attempt } => info!(attempt, "rpc reconnecting"),
        ChainEvent::RuntimeUpgraded { spec_version } => info!(spec_version, "runtime upgraded"),
    }
}

/// Render a `CursorState` update as a structured tracing entry.
pub fn log_cursor(state: &CursorState) {
    debug!(head = state.head, tail = state.tail, "cursor state");
}

/// Render a `ChainState` update as a structured tracing entry.
pub fn log_chain_state(state: &ChainState) {
    debug!(
        connected = state.connected,
        finalized_head = state.finalized_head,
        spec_version = state.spec_version,
        "chain state"
    );
}

/// Spawn the logger consumer. Runs until the shutdown channel fires or
/// every upstream sender is dropped. Returns the join handle so the
/// caller can await clean termination.
pub fn spawn(bus: EventBus, mut shutdown: watch::Receiver<bool>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut indexer_rx = bus.subscribe_indexer();
        let mut backfill_rx = bus.subscribe_backfill();
        let mut handler_rx = bus.subscribe_handler();
        let mut chain_rx = bus.subscribe_chain();

        loop {
            tokio::select! {
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
                r = indexer_rx.recv() => handle(r, log_indexer, "indexer"),
                r = backfill_rx.recv() => handle(r, log_backfill, "backfill"),
                r = handler_rx.recv() => handle(r, log_handler, "handler"),
                r = chain_rx.recv() => handle(r, log_chain, "chain"),
            }
        }
    })
}

fn handle<E>(
    r: Result<E, RecvError>,
    log: fn(&E),
    channel: &'static str,
) {
    match r {
        Ok(ev) => log(&ev),
        Err(RecvError::Lagged(n)) => {
            warn!(channel, dropped = n, "logger lagged behind event bus");
        }
        Err(RecvError::Closed) => {
            debug!(channel, "event channel closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{
        BackfillEvent, ChainEvent, ChainState, CursorState, HandlerEvent, IndexerEvent, StopReason,
    };
    use crate::models::BlockHash;
    use crate::services::IndexMode;

    /// Smoke test: every event variant maps to a tracing emission without
    /// panicking. We don't assert on the actual log contents — that would
    /// be brittle and adds no value over a plain compile check.
    #[test]
    fn log_functions_accept_every_variant() {
        log_indexer(&IndexerEvent::Started {
            mode: IndexMode::Live,
            start_block: 0,
        });
        log_indexer(&IndexerEvent::BlockIndexed {
            number: 1,
            hash: BlockHash([0u8; 32]),
            extrinsics: 0,
            events: 0,
            duration_ms: 1,
        });
        log_indexer(&IndexerEvent::CursorAdvanced { head: 10, tail: 0 });
        log_indexer(&IndexerEvent::LiveModeEntered { from_block: 100 });
        log_indexer(&IndexerEvent::Stopped {
            reason: StopReason::ShutdownRequested,
        });
        log_indexer(&IndexerEvent::Stopped {
            reason: StopReason::Fatal("db down".into()),
        });

        log_backfill(&BackfillEvent::Planned {
            from: 0,
            to: 100,
            total: 101,
        });
        log_backfill(&BackfillEvent::BlockFetched { number: 5 });
        log_backfill(&BackfillEvent::BlockPersisted { number: 5 });
        log_backfill(&BackfillEvent::RangeCompleted { from: 0, to: 100 });
        log_backfill(&BackfillEvent::FetchRetried {
            number: 42,
            attempt: 3,
            error: "timeout".into(),
        });
        log_backfill(&BackfillEvent::Aborted {
            reason: "budget".into(),
        });

        log_handler(&HandlerEvent::EventProcessed {
            pallet: "Balances",
            event_name: "Transfer".into(),
            block: 1,
        });
        log_handler(&HandlerEvent::Persisted {
            pallet: "Balances",
            table: "transfers",
            count: 3,
            block: 1,
        });
        log_handler(&HandlerEvent::Error {
            pallet: "Balances",
            block: 1,
            error: "db".into(),
        });

        log_chain(&ChainEvent::RpcConnected {
            url: "ws://node".into(),
        });
        log_chain(&ChainEvent::RpcDisconnected {
            reason: "EOF".into(),
        });
        log_chain(&ChainEvent::RpcReconnecting { attempt: 2 });
        log_chain(&ChainEvent::RuntimeUpgraded { spec_version: 42 });

        log_cursor(&CursorState { head: 99, tail: 0 });
        log_chain_state(&ChainState {
            connected: true,
            finalized_head: 100,
            spec_version: 1,
        });
    }
}
