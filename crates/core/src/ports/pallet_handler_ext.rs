//! Extension trait providing shared boilerplate for pallet handlers.
//!
//! See the design doc at `docs/superpowers/specs/2026-04-13-codebase-cleanup-refactor-design.md`
//! Section "Component 2 — PalletHandlerExt trait" for the high-level rationale.

use async_trait::async_trait;

use crate::error::StorageResult;
use crate::events::EventBus;
use crate::models::Block;
use crate::ports::{RawEvent, RawExtrinsic};

/// Extension trait that absorbs the `handle_event → outputs → on_block_end
/// → persist` boilerplate shared by every pallet handler.
///
/// Implementing this trait gives you a `PalletHandler` impl for free via
/// the blanket impl in this module. A handler that needs a genuinely
/// different pipeline (e.g. multi-model, cross-pallet dependencies) can
/// still implement `PalletHandler` directly — the blanket only covers
/// types that opt in to `PalletHandlerExt`.
#[async_trait]
pub trait PalletHandlerExt: Send + Sync + 'static {
    /// Domain model this handler extracts from events.
    type Model: Clone + Send + Sync + serde::Serialize + serde::de::DeserializeOwned + 'static;

    /// Pallet name, as it appears in the chain metadata (e.g. "Balances").
    fn pallet_name(&self) -> &'static str;

    /// Bundle name (for logging and event labels).
    fn bundle_name(&self) -> &'static str;

    /// SQL table name (for logging and event labels).
    fn table_name(&self) -> &'static str;

    /// Handler priority — higher runs first. Default 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Convert a raw event into a domain model, or return None to skip it.
    /// This is the only place a handler writes pallet-specific parse logic.
    fn parse(
        &self,
        event: &RawEvent,
        block: &Block,
        extrinsic: Option<&RawExtrinsic>,
    ) -> Option<Self::Model>;

    /// Persist a batch of parsed models. Called once per block by the
    /// blanket impl of `PalletHandler::on_block_end`.
    async fn persist(&self, models: &[Self::Model]) -> StorageResult<()>;

    /// Reference to the event bus — required, not optional. Tests use
    /// `EventBus::noop()` to satisfy this without setting up subscribers.
    fn event_bus(&self) -> &EventBus;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventBus;
    use crate::models::{Block, BlockHash};
    use crate::ports::{RawEvent, RawExtrinsic};
    use async_trait::async_trait;
    use chrono::Utc;
    use serde::{Deserialize, Serialize};
    use std::sync::Arc;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
    struct Dummy {
        id: String,
    }

    struct DummyHandler {
        bus: EventBus,
    }

    #[async_trait]
    impl PalletHandlerExt for DummyHandler {
        type Model = Dummy;
        fn pallet_name(&self) -> &'static str {
            "Dummy"
        }
        fn bundle_name(&self) -> &'static str {
            "dummy"
        }
        fn table_name(&self) -> &'static str {
            "dummies"
        }

        fn parse(
            &self,
            event: &RawEvent,
            _block: &Block,
            _extrinsic: Option<&RawExtrinsic>,
        ) -> Option<Self::Model> {
            Some(Dummy {
                id: event.name.clone(),
            })
        }

        async fn persist(&self, _models: &[Self::Model]) -> crate::error::StorageResult<()> {
            Ok(())
        }

        fn event_bus(&self) -> &EventBus {
            &self.bus
        }
    }

    // This test proves the trait is well-formed by constructing a type
    // that implements it. The blanket impl lands in Task 12; this test
    // only exercises the trait's own shape.
    #[test]
    fn dummy_handler_implements_pallet_handler_ext() {
        let h = DummyHandler {
            bus: EventBus::noop(),
        };
        assert_eq!(h.pallet_name(), "Dummy");
        assert_eq!(h.bundle_name(), "dummy");
        assert_eq!(h.table_name(), "dummies");
        assert_eq!(h.priority(), 0);

        let block = Block {
            number: 1,
            hash: BlockHash([0u8; 32]),
            parent_hash: BlockHash([0u8; 32]),
            state_root: BlockHash([0u8; 32]),
            extrinsics_root: BlockHash([0u8; 32]),
            author: None,
            timestamp: Some(Utc::now()),
            extrinsic_count: 0,
            event_count: 0,
            indexed_at: Utc::now(),
        };
        let ev = RawEvent {
            index: 0,
            extrinsic_index: None,
            pallet: "Dummy".to_string(),
            name: "Ping".to_string(),
            data: serde_json::json!({}),
            topics: vec![],
        };

        let parsed = h.parse(&ev, &block, None).unwrap();
        assert_eq!(parsed.id, "Ping");

        let _arc: Arc<dyn std::any::Any + Send + Sync> = Arc::new(h.bus.clone());
    }
}
