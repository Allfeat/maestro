//! Pallet handler events emitted to the event bus.

/// Events from pallet handlers (via the blanket PalletHandlerExt impl).
#[derive(Debug, Clone)]
pub enum HandlerEvent {
    /// An event was parsed into a domain model.
    EventProcessed {
        pallet: &'static str,
        event_name: String,
        block: u64,
    },
    /// A batch of models was persisted at block end.
    Persisted {
        pallet: &'static str,
        table: &'static str,
        count: usize,
        block: u64,
    },
    /// Parsing or persistence failed for this pallet at this block.
    Error {
        pallet: &'static str,
        block: u64,
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_event_variants_construct_and_clone() {
        let events = [
            HandlerEvent::EventProcessed {
                pallet: "Balances",
                event_name: "Transfer".into(),
                block: 100,
            },
            HandlerEvent::Persisted {
                pallet: "Balances",
                table: "transfers",
                count: 3,
                block: 100,
            },
            HandlerEvent::Error {
                pallet: "Balances",
                block: 100,
                error: "db constraint violation".into(),
            },
        ];
        for e in events {
            let _ = e.clone();
        }
    }
}
