//! Chain RPC transport events emitted to the event bus.

/// Events from the Substrate RPC adapter.
#[derive(Debug, Clone)]
pub enum ChainEvent {
    /// RPC websocket connected.
    RpcConnected { url: String },
    /// RPC websocket disconnected.
    RpcDisconnected { reason: String },
    /// RPC reconnection in progress.
    RpcReconnecting { attempt: u32 },
    /// Runtime spec upgrade observed on chain.
    RuntimeUpgraded { spec_version: u32 },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_event_variants_construct_and_clone() {
        let events = [
            ChainEvent::RpcConnected {
                url: "ws://127.0.0.1:9944".into(),
            },
            ChainEvent::RpcDisconnected {
                reason: "timeout".into(),
            },
            ChainEvent::RpcReconnecting { attempt: 3 },
            ChainEvent::RuntimeUpgraded { spec_version: 5 },
        ];
        for e in events {
            let _ = e.clone();
        }
    }
}
