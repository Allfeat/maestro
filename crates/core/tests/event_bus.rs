//! Integration tests for the EventBus fanout, lag, watch, and noop semantics.

use std::time::Duration;

use maestro_core::events::{
    BackfillEvent, ChainState, CursorState, EventBus, IndexerEvent, StopReason,
};
use maestro_core::models::BlockHash;
use maestro_core::services::IndexMode;
use tokio::sync::broadcast::error::RecvError;
use tokio::time::timeout;

fn block_indexed(number: u64) -> IndexerEvent {
    IndexerEvent::BlockIndexed {
        number,
        hash: BlockHash([0u8; 32]),
        extrinsics: 0,
        events: 0,
        duration_ms: 0,
    }
}

#[tokio::test]
async fn fanout_delivers_to_every_subscriber() {
    let bus = EventBus::new(16);
    let mut a = bus.subscribe_indexer();
    let mut b = bus.subscribe_indexer();

    bus.emit_indexer(block_indexed(1));
    bus.emit_indexer(block_indexed(2));

    let a1 = timeout(Duration::from_millis(100), a.recv())
        .await
        .unwrap()
        .unwrap();
    let a2 = timeout(Duration::from_millis(100), a.recv())
        .await
        .unwrap()
        .unwrap();
    let b1 = timeout(Duration::from_millis(100), b.recv())
        .await
        .unwrap()
        .unwrap();
    let b2 = timeout(Duration::from_millis(100), b.recv())
        .await
        .unwrap()
        .unwrap();

    assert!(matches!(a1, IndexerEvent::BlockIndexed { number: 1, .. }));
    assert!(matches!(a2, IndexerEvent::BlockIndexed { number: 2, .. }));
    assert!(matches!(b1, IndexerEvent::BlockIndexed { number: 1, .. }));
    assert!(matches!(b2, IndexerEvent::BlockIndexed { number: 2, .. }));
}

#[tokio::test]
async fn slow_subscriber_receives_lagged_but_bus_keeps_going() {
    // Capacity of 2. We emit 5 events. The slow subscriber must see Lagged.
    let bus = EventBus::new(2);
    let mut slow = bus.subscribe_indexer();
    let mut fast = bus.subscribe_indexer();

    // Drain fast subscriber in real time so it never lags.
    let fast_task = tokio::spawn(async move {
        let mut count = 0;
        while let Ok(_ev) = fast.recv().await {
            count += 1;
            if count == 5 {
                return count;
            }
        }
        count
    });

    for n in 0..5 {
        bus.emit_indexer(block_indexed(n));
        // Yield so the fast receiver can consume.
        tokio::task::yield_now().await;
    }

    // Slow subscriber never called recv() in between — it must report Lagged.
    let first = slow.recv().await;
    assert!(
        matches!(first, Err(RecvError::Lagged(_))),
        "expected Lagged, got {:?}",
        first
    );

    // Drop the sender side so fast can finish.
    drop(bus);
    let fast_count = timeout(Duration::from_secs(1), fast_task)
        .await
        .unwrap()
        .unwrap();
    assert!(fast_count >= 1, "fast subscriber saw at least one event");
}

#[tokio::test]
async fn watch_cursor_publishes_latest_value() {
    let bus = EventBus::new(16);
    let mut rx = bus.watch_cursor();
    assert_eq!(*rx.borrow_and_update(), CursorState::default());

    bus.update_cursor(CursorState { head: 10, tail: 0 });
    bus.update_cursor(CursorState { head: 20, tail: 0 });
    bus.update_cursor(CursorState { head: 30, tail: 5 });

    // changed() resolves as soon as any update landed.
    rx.changed().await.unwrap();
    let latest = *rx.borrow_and_update();
    assert_eq!(latest, CursorState { head: 30, tail: 5 });
}

#[tokio::test]
async fn noop_bus_emit_with_no_subscribers_does_not_panic() {
    let bus = EventBus::noop();

    // Every emit must be silent even with zero subscribers.
    bus.emit_indexer(IndexerEvent::Stopped {
        reason: StopReason::ShutdownRequested,
    });
    bus.emit_backfill(BackfillEvent::Aborted {
        reason: "noop".into(),
    });
    bus.update_cursor(CursorState { head: 1, tail: 0 });
    bus.update_chain(ChainState {
        connected: true,
        finalized_head: 1,
        spec_version: 1,
    });

    // Any later subscriber works.
    let _ = bus.subscribe_indexer();
}

// Suppress IndexMode unused warning on import in a test harness that does
// not happen to use it — we import it so callers of block_indexed can
// extend this file without re-importing.
#[allow(dead_code)]
fn _mode_touch() {
    let _ = IndexMode::Live;
}
