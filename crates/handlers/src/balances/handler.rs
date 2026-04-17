//! Handler for the Substrate Balances pallet — indexes `Transfer` events.
use std::sync::Arc;

use async_trait::async_trait;
use tracing::warn;

use super::models::Transfer;
use super::storage::BalancesStorage;
use crate::utils::{extract_field, parse_account, parse_amount};
use maestro_core::error::StorageResult;
use maestro_core::events::EventBus;
use maestro_core::models::Block;
use maestro_core::ports::{PalletHandlerExt, RawEvent, RawExtrinsic};

/// Parses `Balances::Transfer` events into [`Transfer`] models; orchestration
/// and `HandlerEvent` emission come from `impl PalletHandler for H: PalletHandlerExt`.
pub struct BalancesHandler {
    storage: Arc<dyn BalancesStorage>,
    bus: EventBus,
}

impl BalancesHandler {
    pub fn new(storage: Arc<dyn BalancesStorage>, bus: EventBus) -> Self {
        Self { storage, bus }
    }
}

#[async_trait]
impl PalletHandlerExt for BalancesHandler {
    type Model = Transfer;

    fn pallet_name(&self) -> &'static str {
        "Balances"
    }

    fn bundle_name(&self) -> &'static str {
        "balances"
    }

    fn table_name(&self) -> &'static str {
        "transfers"
    }

    fn priority(&self) -> i32 {
        10
    }

    fn parse(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> Option<Self::Model> {
        if event.name != "Transfer" {
            return None;
        }
        let data = &event.data;
        let warn_missing = |field: &str| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse '{}' in Transfer",
                field,
            );
        };
        let from = extract_field(data, &["from", "who"], 0, parse_account).or_else(|| {
            warn_missing("from");
            None
        })?;
        let to = extract_field(data, &["to", "dest"], 1, parse_account).or_else(|| {
            warn_missing("to");
            None
        })?;
        let amount = extract_field(data, &["amount", "value"], 2, parse_amount).or_else(|| {
            warn_missing("amount");
            None
        })?;
        Some(Transfer {
            id: format!("{}-{}", block.number, event.index),
            block_number: block.number,
            block_hash: block.hash,
            event_index: event.index,
            extrinsic_index: event.extrinsic_index,
            from,
            to,
            amount,
            success: true,
            timestamp: block.timestamp,
        })
    }

    async fn persist(&self, models: &[Self::Model]) -> StorageResult<()> {
        self.storage.insert_transfers(models).await
    }

    fn event_bus(&self) -> &EventBus {
        &self.bus
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maestro_core::error::{StorageError, StorageResult};
    use maestro_core::events::{EventBus, HandlerEvent};
    use maestro_core::models::BlockHash;
    use maestro_core::ports::{
        Connection, OrderDirection, PageInfo, Pagination, PalletHandler, PalletHandlerExt,
    };
    use serde_json::json;

    use super::super::storage::TransferFilter;

    /// Mock storage for testing (doesn't persist anything).
    struct MockStorage;

    #[async_trait]
    impl BalancesStorage for MockStorage {
        async fn insert_transfers(&self, _transfers: &[Transfer]) -> StorageResult<()> {
            Ok(())
        }

        async fn get_transfer(&self, _id: &str) -> StorageResult<Option<Transfer>> {
            Ok(None)
        }

        async fn list_transfers_for_block(
            &self,
            _block_number: u64,
        ) -> StorageResult<Vec<Transfer>> {
            Ok(vec![])
        }

        async fn list_transfers(
            &self,
            _filter: TransferFilter,
            _pagination: Pagination,
            _order: OrderDirection,
        ) -> StorageResult<Connection<Transfer>> {
            Ok(Connection {
                edges: vec![],
                page_info: PageInfo {
                    has_next_page: false,
                    has_previous_page: false,
                    start_cursor: None,
                    end_cursor: None,
                },
                total_count: Some(0),
            })
        }

        async fn delete_transfers_from(&self, _from_block: u64) -> StorageResult<u64> {
            Ok(0)
        }
    }

    /// Mock storage that always fails, to exercise the Error-event path.
    struct FailingStorage;

    #[async_trait]
    impl BalancesStorage for FailingStorage {
        async fn insert_transfers(&self, _transfers: &[Transfer]) -> StorageResult<()> {
            Err(StorageError::query("failing-storage test boom"))
        }

        async fn get_transfer(&self, _id: &str) -> StorageResult<Option<Transfer>> {
            Ok(None)
        }

        async fn list_transfers_for_block(
            &self,
            _block_number: u64,
        ) -> StorageResult<Vec<Transfer>> {
            Ok(vec![])
        }

        async fn list_transfers(
            &self,
            _filter: TransferFilter,
            _pagination: Pagination,
            _order: OrderDirection,
        ) -> StorageResult<Connection<Transfer>> {
            Ok(Connection {
                edges: vec![],
                page_info: PageInfo {
                    has_next_page: false,
                    has_previous_page: false,
                    start_cursor: None,
                    end_cursor: None,
                },
                total_count: Some(0),
            })
        }

        async fn delete_transfers_from(&self, _from_block: u64) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn valid_transfer_event() -> RawEvent {
        mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000000000000"
            }),
        )
    }

    fn mock_block(number: u64) -> Block {
        Block {
            number,
            hash: BlockHash([0xaa; 32]),
            parent_hash: BlockHash([0xbb; 32]),
            state_root: BlockHash([0xcc; 32]),
            extrinsics_root: BlockHash([0xdd; 32]),
            author: None,
            timestamp: Some(Utc::now()),
            extrinsic_count: 0,
            event_count: 0,
            indexed_at: Utc::now(),
        }
    }

    fn mock_event(name: &str, data: serde_json::Value) -> RawEvent {
        RawEvent {
            index: 0,
            extrinsic_index: Some(0),
            pallet: "Balances".to_string(),
            name: name.to_string(),
            data,
            topics: vec![],
        }
    }

    #[test]
    fn test_process_transfer_valid() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000000000000"
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_some());

        let t = transfer.unwrap();
        assert_eq!(t.from.0, [0xab; 32]);
        assert_eq!(t.to.0, [0xcd; 32]);
        assert_eq!(t.amount, 1_000_000_000_000u128);
        assert_eq!(t.block_number, 100);
        assert_eq!(t.id, "100-0");
    }

    #[test]
    fn test_process_transfer_alternate_field_names() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(50);

        // Uses "who" and "dest" instead of "from" and "to"
        let event = mock_event(
            "Transfer",
            json!({
                "who": "0x".to_string() + &"11".repeat(32),
                "dest": "0x".to_string() + &"22".repeat(32),
                "value": "5000"
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_some());

        let t = transfer.unwrap();
        assert_eq!(t.from.0, [0x11; 32]);
        assert_eq!(t.to.0, [0x22; 32]);
        assert_eq!(t.amount, 5000);
    }

    #[test]
    fn test_process_transfer_missing_from() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_missing_to() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_missing_amount() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "to": "0x".to_string() + &"cd".repeat(32)
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_invalid_from_length() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(16), // Only 16 bytes
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.parse(&event, &block, None);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_pallet_name() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        assert_eq!(PalletHandlerExt::pallet_name(&handler), "Balances");
    }

    #[test]
    fn test_priority() {
        let handler = BalancesHandler::new(Arc::new(MockStorage), EventBus::noop());
        assert_eq!(PalletHandlerExt::priority(&handler), 10);
    }

    #[tokio::test]
    async fn emits_persisted_event_on_successful_persist() {
        let bus = EventBus::noop();
        let mut rx = bus.subscribe_handler();

        let handler: Arc<dyn PalletHandler> =
            Arc::new(BalancesHandler::new(Arc::new(MockStorage), bus.clone()));

        let block = mock_block(100);
        let event = valid_transfer_event();

        let outputs = handler
            .handle_event(&event, &block, None)
            .await
            .expect("handle_event should succeed");
        let final_outputs = handler
            .on_block_end(&block, &outputs)
            .await
            .expect("on_block_end should succeed");
        assert_eq!(final_outputs.current_size(), 0);

        // Drain the channel: EventProcessed first, then Persisted.
        let first = rx.try_recv().expect("expected EventProcessed");
        assert!(matches!(
            first,
            HandlerEvent::EventProcessed {
                pallet: "Balances",
                ..
            }
        ));

        let second = rx.try_recv().expect("expected Persisted");
        assert!(matches!(
            second,
            HandlerEvent::Persisted {
                pallet: "Balances",
                table: "transfers",
                count: 1,
                block: 100,
            }
        ));
    }

    #[tokio::test]
    async fn emits_error_event_on_persist_failure() {
        let bus = EventBus::noop();
        let mut rx = bus.subscribe_handler();

        let handler: Arc<dyn PalletHandler> =
            Arc::new(BalancesHandler::new(Arc::new(FailingStorage), bus.clone()));

        let block = mock_block(100);
        let event = valid_transfer_event();

        let outputs = handler
            .handle_event(&event, &block, None)
            .await
            .expect("handle_event should succeed");
        let res = handler.on_block_end(&block, &outputs).await;
        assert!(res.is_err(), "persist error should surface as Err");

        // Drain the channel: EventProcessed first, then Error.
        let _processed = rx.try_recv().expect("expected EventProcessed");
        let err = rx.try_recv().expect("expected Error");
        assert!(matches!(
            err,
            HandlerEvent::Error {
                pallet: "Balances",
                block: 100,
                ..
            }
        ));
    }
}
