//! Backfill service events emitted to the event bus.

/// Events from the historical backfill loop.
#[derive(Debug, Clone)]
pub enum BackfillEvent {
    /// Backfill plan computed — about to start fetching.
    Planned { from: u64, to: u64, total: u64 },
    /// Block fetched from RPC (not yet persisted).
    BlockFetched { number: u64 },
    /// Block persisted to storage.
    BlockPersisted { number: u64 },
    /// A contiguous range was fully backfilled.
    RangeCompleted { from: u64, to: u64 },
    /// A single-block fetch failed and is being retried.
    FetchRetried {
        number: u64,
        attempt: u32,
        error: String,
    },
    /// The backfill run was aborted (retry budget exhausted or fatal error).
    Aborted { reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backfill_event_variants_construct_and_clone() {
        let events = [
            BackfillEvent::Planned { from: 0, to: 1000, total: 1001 },
            BackfillEvent::BlockFetched { number: 42 },
            BackfillEvent::BlockPersisted { number: 42 },
            BackfillEvent::RangeCompleted { from: 0, to: 1000 },
            BackfillEvent::FetchRetried {
                number: 42,
                attempt: 2,
                error: "rpc timeout".into(),
            },
            BackfillEvent::Aborted {
                reason: "max retries exhausted".into(),
            },
        ];
        for e in events {
            let _ = e.clone();
        }
    }
}
