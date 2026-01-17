//! Handler for the Balances pallet.
//!
//! This handler processes events from the Substrate Balances pallet and extracts
//! transfer information for indexing.
//!
//! # Supported Events
//!
//! - `Transfer`: Token transfer between accounts

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use maestro_core::error::DomainResult;
use maestro_core::models::Block;
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

use super::models::Transfer;
use super::storage::BalancesStorage;
use crate::utils::{extract_field, parse_account, parse_amount};

// =============================================================================
// Handler
// =============================================================================

/// Handler for the Balances pallet.
///
/// Extracts transfer events and persists them using its own storage.
pub struct BalancesHandler {
    storage: Arc<dyn BalancesStorage>,
}

impl BalancesHandler {
    pub fn new(storage: Arc<dyn BalancesStorage>) -> Self {
        Self { storage }
    }

    /// Process a Transfer event into a domain model.
    fn process_transfer(&self, event: &RawEvent, block: &Block) -> Option<Transfer> {
        let data = &event.data;

        let from = extract_field(data, &["from", "who"], 0, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'from' in Transfer"
            );
            None
        })?;

        let to = extract_field(data, &["to", "dest"], 1, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'to' in Transfer"
            );
            None
        })?;

        let amount = extract_field(data, &["amount", "value"], 2, parse_amount).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'amount' in Transfer"
            );
            None
        })?;

        Some(Transfer {
            id: format!("{}-{}", block.number, event.index),
            block_number: block.number,
            block_hash: block.hash.clone(),
            event_index: event.index,
            extrinsic_index: event.extrinsic_index,
            from,
            to,
            amount,
            success: true,
            timestamp: block.timestamp,
        })
    }
}

#[async_trait]
impl PalletHandler for BalancesHandler {
    fn pallet_name(&self) -> &'static str {
        "Balances"
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        if event.name == "Transfer"
            && let Some(transfer) = self.process_transfer(event, block)
        {
            outputs.add("balances", "transfers", &transfer)?;
        }

        Ok(outputs)
    }

    async fn on_block_end(
        &self,
        block: &Block,
        outputs: &HandlerOutputs,
    ) -> DomainResult<HandlerOutputs> {
        let transfers: Vec<Transfer> = outputs.get_typed("balances", "transfers");

        if !transfers.is_empty() {
            debug!(
                block = block.number,
                count = transfers.len(),
                "Persisting transfers"
            );

            if let Err(e) = self.storage.insert_transfers(&transfers).await {
                warn!(block = block.number, error = ?e, "Failed to persist transfers");
                return Err(e.into());
            }
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maestro_core::error::StorageResult;
    use maestro_core::models::BlockHash;
    use maestro_core::ports::{Connection, OrderDirection, PageInfo, Pagination};
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
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000000000000"
            }),
        );

        let transfer = handler.process_transfer(&event, &block);
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
        let handler = BalancesHandler::new(Arc::new(MockStorage));
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

        let transfer = handler.process_transfer(&event, &block);
        assert!(transfer.is_some());

        let t = transfer.unwrap();
        assert_eq!(t.from.0, [0x11; 32]);
        assert_eq!(t.to.0, [0x22; 32]);
        assert_eq!(t.amount, 5000);
    }

    #[test]
    fn test_process_transfer_missing_from() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.process_transfer(&event, &block);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_missing_to() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.process_transfer(&event, &block);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_missing_amount() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(32),
                "to": "0x".to_string() + &"cd".repeat(32)
            }),
        );

        let transfer = handler.process_transfer(&event, &block);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_process_transfer_invalid_from_length() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "Transfer",
            json!({
                "from": "0x".to_string() + &"ab".repeat(16), // Only 16 bytes
                "to": "0x".to_string() + &"cd".repeat(32),
                "amount": "1000"
            }),
        );

        let transfer = handler.process_transfer(&event, &block);
        assert!(transfer.is_none());
    }

    #[test]
    fn test_pallet_name() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        assert_eq!(handler.pallet_name(), "Balances");
    }

    #[test]
    fn test_priority() {
        let handler = BalancesHandler::new(Arc::new(MockStorage));
        assert_eq!(handler.priority(), 10);
    }
}
