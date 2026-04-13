//! Prometheus metrics bridge consumer listening to the event bus.
//!
//! Maps event bus events to Prometheus counters under the `bus_*` namespace.
//! Runs as a background task.

use metrics::counter;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

use super::{BackfillEvent, ChainEvent, EventBus, HandlerEvent, IndexerEvent, StopReason};

pub fn handle_indexer(ev: &IndexerEvent) {
    match ev {
        IndexerEvent::Started { .. } => {
            counter!("bus_indexer_started_total").increment(1);
        }
        IndexerEvent::BlockIndexed { .. } => {
            counter!("bus_blocks_indexed_total").increment(1);
        }
        IndexerEvent::CursorAdvanced { .. } => {
            counter!("bus_cursor_advanced_total").increment(1);
        }
        IndexerEvent::LiveModeEntered { .. } => {
            counter!("bus_live_mode_entered_total").increment(1);
        }
        IndexerEvent::Stopped { reason } => match reason {
            StopReason::ShutdownRequested => {
                counter!("bus_indexer_stopped_total", "reason" => "shutdown").increment(1);
            }
            StopReason::Fatal(_) => {
                counter!("bus_indexer_stopped_total", "reason" => "fatal").increment(1);
            }
        },
    }
}

pub fn handle_backfill(ev: &BackfillEvent) {
    match ev {
        BackfillEvent::Planned { .. } => {
            counter!("bus_backfill_planned_total").increment(1);
        }
        BackfillEvent::BlockFetched { .. } => {
            counter!("bus_backfill_blocks_fetched_total").increment(1);
        }
        BackfillEvent::BlockPersisted { .. } => {
            counter!("bus_backfill_blocks_persisted_total").increment(1);
        }
        BackfillEvent::RangeCompleted { .. } => {
            counter!("bus_backfill_ranges_completed_total").increment(1);
        }
        BackfillEvent::FetchRetried { .. } => {
            counter!("bus_backfill_fetch_retries_total").increment(1);
        }
        BackfillEvent::Aborted { .. } => {
            counter!("bus_backfill_aborted_total").increment(1);
        }
    }
}

pub fn handle_handler(ev: &HandlerEvent) {
    match ev {
        HandlerEvent::EventProcessed { pallet, .. } => {
            counter!("bus_handler_events_total", "pallet" => pallet.to_string()).increment(1);
        }
        HandlerEvent::Persisted { pallet, count, .. } => {
            counter!("bus_handler_persisted_total", "pallet" => pallet.to_string())
                .increment(*count as u64);
        }
        HandlerEvent::Error { pallet, .. } => {
            counter!("bus_handler_errors_total", "pallet" => pallet.to_string()).increment(1);
        }
    }
}

pub fn handle_chain(ev: &ChainEvent) {
    match ev {
        ChainEvent::RpcConnected { .. } => {
            counter!("bus_rpc_connects_total").increment(1);
        }
        ChainEvent::RpcDisconnected { .. } => {
            counter!("bus_rpc_disconnects_total").increment(1);
        }
        ChainEvent::RpcReconnecting { .. } => {
            counter!("bus_rpc_reconnects_total").increment(1);
        }
        ChainEvent::RuntimeUpgraded { .. } => {
            counter!("bus_runtime_upgrades_total").increment(1);
        }
    }
}

/// Spawn the metrics bridge consumer. Mirrors `logger::spawn`.
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
                r = indexer_rx.recv() => dispatch(r, handle_indexer, "indexer"),
                r = backfill_rx.recv() => dispatch(r, handle_backfill, "backfill"),
                r = handler_rx.recv() => dispatch(r, handle_handler, "handler"),
                r = chain_rx.recv() => dispatch(r, handle_chain, "chain"),
            }
        }
    })
}

fn dispatch<E>(r: Result<E, RecvError>, f: fn(&E), channel: &'static str) {
    match r {
        Ok(ev) => f(&ev),
        Err(RecvError::Lagged(n)) => {
            warn!(
                channel,
                dropped = n,
                "metrics_bridge lagged behind event bus"
            );
        }
        Err(RecvError::Closed) => {
            debug!(channel, "event channel closed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{BackfillEvent, ChainEvent, HandlerEvent, IndexerEvent, StopReason};
    use crate::models::BlockHash;
    use crate::services::IndexMode;
    use metrics::{Key, Label};
    use metrics_util::CompositeKey;
    use metrics_util::debugging::{DebuggingRecorder, Snapshotter};

    fn counter_value(
        snap: &[(
            CompositeKey,
            Option<metrics::Unit>,
            Option<metrics::SharedString>,
            metrics_util::debugging::DebugValue,
        )],
        name: &str,
    ) -> u64 {
        for (ck, _unit, _desc, value) in snap {
            if ck.key().name() == name
                && let metrics_util::debugging::DebugValue::Counter(v) = value
            {
                return *v;
            }
        }
        0
    }

    #[test]
    fn metrics_bridge_increments_expected_counters() {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();

        metrics::with_local_recorder(&recorder, || {
            handle_indexer(&IndexerEvent::BlockIndexed {
                number: 1,
                hash: BlockHash([0u8; 32]),
                extrinsics: 0,
                events: 0,
                duration_ms: 5,
            });
            handle_backfill(&BackfillEvent::BlockPersisted { number: 1 });
            handle_backfill(&BackfillEvent::FetchRetried {
                number: 1,
                attempt: 1,
                error: "t".into(),
            });
            handle_backfill(&BackfillEvent::Aborted { reason: "x".into() });
            handle_handler(&HandlerEvent::Persisted {
                pallet: "Balances",
                table: "transfers",
                count: 2,
                block: 1,
            });
            handle_handler(&HandlerEvent::Error {
                pallet: "Balances",
                block: 1,
                error: "e".into(),
            });
            handle_chain(&ChainEvent::RpcDisconnected {
                reason: "eof".into(),
            });
        });

        let snap = snapshotter.snapshot().into_vec();
        assert!(counter_value(&snap, "bus_blocks_indexed_total") >= 1);
        assert!(counter_value(&snap, "bus_backfill_blocks_persisted_total") >= 1);
        assert!(counter_value(&snap, "bus_backfill_fetch_retries_total") >= 1);
        assert!(counter_value(&snap, "bus_backfill_aborted_total") >= 1);
        assert!(counter_value(&snap, "bus_handler_persisted_total") >= 1);
        assert!(counter_value(&snap, "bus_handler_errors_total") >= 1);
        assert!(counter_value(&snap, "bus_rpc_disconnects_total") >= 1);
    }

    // The unused-import silencer keeps the minimal set of imports visible.
    #[allow(dead_code)]
    fn _touch() {
        let _ = (
            Key::from_name("x"),
            Label::new("k", "v"),
            IndexMode::Live,
            StopReason::ShutdownRequested,
        );
        let _ = std::any::type_name::<Snapshotter>();
    }
}
