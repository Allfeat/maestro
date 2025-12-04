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

        #[allow(clippy::single_match)]
        match event.name.as_str() {
            "Transfer" => {
                if let Some(transfer) = self.process_transfer(event, block) {
                    outputs.add("balances", "transfers", &transfer)?;
                }
            }
            _ => {}
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
