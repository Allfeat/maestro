//! Indexer service events emitted to the event bus.

use crate::models::BlockHash;
use crate::services::IndexMode;

/// Why the indexer stopped. Attached to `IndexerEvent::Stopped`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    /// Graceful shutdown requested via signal or shutdown channel.
    ShutdownRequested,
    /// Fatal error — indexer cannot continue.
    Fatal(String),
}

/// High-level events from the indexer service.
#[derive(Debug, Clone)]
pub enum IndexerEvent {
    /// Indexer started. Emitted once per run.
    Started {
        mode: IndexMode,
        start_block: u64,
    },
    /// A block was fully indexed (extrinsics + events + handlers persisted).
    BlockIndexed {
        number: u64,
        hash: BlockHash,
        extrinsics: u32,
        events: u32,
        duration_ms: u32,
    },
    /// Indexer cursor advanced. `head` is the latest indexed block,
    /// `tail` is the lowest indexed block (for backfill progress).
    CursorAdvanced { head: u64, tail: u64 },
    /// Transition from backfill to live mode at the given block.
    LiveModeEntered { from_block: u64 },
    /// Indexer is stopping.
    Stopped { reason: StopReason },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::BlockHash;
    use crate::services::IndexMode;

    #[test]
    fn indexer_event_variants_construct_and_clone() {
        let e1 = IndexerEvent::Started {
            mode: IndexMode::Live,
            start_block: 0,
        };
        let e2 = IndexerEvent::BlockIndexed {
            number: 42,
            hash: BlockHash([0u8; 32]),
            extrinsics: 3,
            events: 7,
            duration_ms: 12,
        };
        let e3 = IndexerEvent::CursorAdvanced { head: 42, tail: 0 };
        let e4 = IndexerEvent::LiveModeEntered { from_block: 100 };
        let e5 = IndexerEvent::Stopped {
            reason: StopReason::ShutdownRequested,
        };

        let _ = (e1.clone(), e2.clone(), e3.clone(), e4.clone(), e5.clone());
    }
}
