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

use crate::error::DomainResult;
use crate::events::HandlerEvent;
use crate::ports::{HandlerOutputs, PalletHandler};

#[async_trait]
impl<H> PalletHandler for H
where
    H: PalletHandlerExt,
{
    fn pallet_name(&self) -> &'static str {
        <H as PalletHandlerExt>::pallet_name(self)
    }

    fn priority(&self) -> i32 {
        <H as PalletHandlerExt>::priority(self)
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();
        if let Some(model) = self.parse(event, block, extrinsic) {
            outputs.add(self.bundle_name(), self.table_name(), &model)?;
            self.event_bus().emit_handler(HandlerEvent::EventProcessed {
                pallet: self.pallet_name(),
                event_name: event.name.clone(),
                block: block.number,
            });
        }
        Ok(outputs)
    }

    async fn on_block_end(
        &self,
        block: &Block,
        outputs: &HandlerOutputs,
    ) -> DomainResult<HandlerOutputs> {
        let models: Vec<H::Model> = outputs.get_typed(self.bundle_name(), self.table_name());

        if models.is_empty() {
            return Ok(HandlerOutputs::new());
        }

        match self.persist(&models).await {
            Ok(()) => {
                self.event_bus().emit_handler(HandlerEvent::Persisted {
                    pallet: self.pallet_name(),
                    table: self.table_name(),
                    count: models.len(),
                    block: block.number,
                });
                Ok(HandlerOutputs::new())
            }
            Err(e) => {
                let msg = format!("{}", e);
                self.event_bus().emit_handler(HandlerEvent::Error {
                    pallet: self.pallet_name(),
                    block: block.number,
                    error: msg,
                });
                Err(e.into())
            }
        }
    }
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
        assert_eq!(PalletHandlerExt::pallet_name(&h), "Dummy");
        assert_eq!(h.bundle_name(), "dummy");
        assert_eq!(h.table_name(), "dummies");
        assert_eq!(PalletHandlerExt::priority(&h), 0);

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

    use crate::events::HandlerEvent;
    use crate::ports::PalletHandler;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingHandler {
        bus: EventBus,
        persisted_calls: AtomicUsize,
        fail: bool,
    }

    #[async_trait]
    impl PalletHandlerExt for CountingHandler {
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
            self.persisted_calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                Err(crate::error::StorageError::QueryError("boom".into()))
            } else {
                Ok(())
            }
        }

        fn event_bus(&self) -> &EventBus {
            &self.bus
        }
    }

    fn mk_block() -> Block {
        Block {
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
        }
    }

    fn mk_event(name: &str) -> RawEvent {
        RawEvent {
            index: 0,
            extrinsic_index: None,
            pallet: "Dummy".to_string(),
            name: name.to_string(),
            data: serde_json::json!({}),
            topics: vec![],
        }
    }

    #[tokio::test]
    async fn blanket_handles_parse_then_persist_happy_path() {
        let bus = EventBus::noop();
        let mut rx = bus.subscribe_handler();

        let handler: Arc<dyn PalletHandler> = Arc::new(CountingHandler {
            bus: bus.clone(),
            persisted_calls: AtomicUsize::new(0),
            fail: false,
        });

        let block = mk_block();
        let outputs = handler
            .handle_event(&mk_event("Ping"), &block, None)
            .await
            .unwrap();

        // Drain and persist.
        let final_outputs = handler.on_block_end(&block, &outputs).await.unwrap();
        assert_eq!(final_outputs.current_size(), 0);

        let first = rx.try_recv().expect("should have an EventProcessed");
        assert!(matches!(
            first,
            HandlerEvent::EventProcessed {
                pallet: "Dummy",
                ..
            }
        ));

        let second = rx.try_recv().expect("should have a Persisted");
        assert!(matches!(
            second,
            HandlerEvent::Persisted {
                pallet: "Dummy",
                table: "dummies",
                count: 1,
                block: 1
            }
        ));
    }

    #[tokio::test]
    async fn blanket_emits_handler_error_on_persist_failure() {
        let bus = EventBus::noop();
        let mut rx = bus.subscribe_handler();

        let handler: Arc<dyn PalletHandler> = Arc::new(CountingHandler {
            bus: bus.clone(),
            persisted_calls: AtomicUsize::new(0),
            fail: true,
        });

        let block = mk_block();
        let outputs = handler
            .handle_event(&mk_event("Ping"), &block, None)
            .await
            .unwrap();
        let res = handler.on_block_end(&block, &outputs).await;
        assert!(res.is_err(), "persist error should surface as Err");

        // Drain the channel: EventProcessed first, then Error.
        let _processed = rx.try_recv().expect("EventProcessed");
        let err = rx.try_recv().expect("Error");
        assert!(matches!(
            err,
            HandlerEvent::Error {
                pallet: "Dummy",
                block: 1,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn blanket_no_parse_no_persist() {
        struct NoneHandler {
            bus: EventBus,
        }

        #[async_trait]
        impl PalletHandlerExt for NoneHandler {
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
                _event: &RawEvent,
                _block: &Block,
                _extrinsic: Option<&RawExtrinsic>,
            ) -> Option<Self::Model> {
                None
            }
            async fn persist(&self, _models: &[Self::Model]) -> crate::error::StorageResult<()> {
                panic!("persist must not be called when parse returns None for every event");
            }
            fn event_bus(&self) -> &EventBus {
                &self.bus
            }
        }

        let bus = EventBus::noop();
        let mut rx = bus.subscribe_handler();
        let handler: Arc<dyn PalletHandler> = Arc::new(NoneHandler { bus: bus.clone() });
        let block = mk_block();
        let outputs = handler
            .handle_event(&mk_event("Ping"), &block, None)
            .await
            .unwrap();
        // No parse happened — no outputs, no events.
        assert_eq!(outputs.current_size(), 0);
        assert!(rx.try_recv().is_err());
        // on_block_end short-circuits on empty outputs — persist() must NOT fire.
        let _ = handler.on_block_end(&block, &outputs).await.unwrap();
    }
}
