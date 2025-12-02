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
use maestro_core::models::{AccountId, Block};
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

use super::models::Transfer;
use super::storage::BalancesStorage;

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

        let from = extract_field(data, &["from", "who"], 0, parse_account)
            .or_else(|| {
                warn!(block = block.number, event = event.index, "Failed to parse 'from' in Transfer");
                None
            })?;

        let to = extract_field(data, &["to", "dest"], 1, parse_account)
            .or_else(|| {
                warn!(block = block.number, event = event.index, "Failed to parse 'to' in Transfer");
                None
            })?;

        let amount = extract_field(data, &["amount", "value"], 2, parse_amount)
            .or_else(|| {
                warn!(block = block.number, event = event.index, "Failed to parse 'amount' in Transfer");
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

// =============================================================================
// Event field parsing utilities
// =============================================================================

/// Extract a field from event data, trying multiple key names and falling back to index.
fn extract_field<T>(
    data: &serde_json::Value,
    keys: &[&str],
    index: usize,
    parser: fn(&serde_json::Value) -> Option<T>,
) -> Option<T> {
    keys.iter()
        .find_map(|key| data.get(*key))
        .or_else(|| data.get(index))
        .and_then(parser)
}

/// Parse an account ID from various JSON representations.
fn parse_account(value: &serde_json::Value) -> Option<AccountId> {
    match value {
        // Hex string: "0x1234..."
        serde_json::Value::String(s) => {
            let hex_str = s.strip_prefix("0x").unwrap_or(s);
            let bytes = hex::decode(hex_str).ok()?;
            let arr: [u8; 32] = bytes.try_into().ok()?;
            Some(AccountId(arr))
        }
        // Wrapped object: { "Id": "0x..." }
        serde_json::Value::Object(obj) => {
            obj.get("Id")
                .or_else(|| obj.get("id"))
                .and_then(parse_account)
        }
        // Array: either ["0x..."] or [b0, b1, ..., b31]
        serde_json::Value::Array(arr) => {
            if arr.len() == 1 {
                return parse_account(&arr[0]);
            }
            if arr.len() != 32 {
                return None;
            }
            let mut bytes = [0u8; 32];
            for (i, v) in arr.iter().enumerate() {
                bytes[i] = v.as_u64()? as u8;
            }
            Some(AccountId(bytes))
        }
        _ => None,
    }
}

/// Parse an amount (u128) from JSON.
fn parse_amount(value: &serde_json::Value) -> Option<u128> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().map(u128::from),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Tests des différents formats de compte reçus des nodes Substrate
    // Important car les formats varient selon les versions de metadata

    #[test]
    fn test_parse_account_all_formats() {
        let hex = "0x".to_string() + &"ab".repeat(32);

        // Format direct hex string
        assert!(parse_account(&json!(hex)).is_some());

        // Format sans prefix 0x
        assert!(parse_account(&json!("ab".repeat(32))).is_some());

        // Format wrapped { "Id": "0x..." } (metadata v14+)
        assert!(parse_account(&json!({"Id": hex})).is_some());
        assert!(parse_account(&json!({"id": hex})).is_some());

        // Format array wrapper ["0x..."]
        assert!(parse_account(&json!([hex])).is_some());

        // Format byte array [0, 1, 2, ..., 31]
        let bytes: Vec<u8> = (0..32).collect();
        assert!(parse_account(&json!(bytes)).is_some());
    }

    #[test]
    fn test_parse_account_rejects_invalid() {
        // Mauvaise longueur
        assert!(parse_account(&json!("ab".repeat(16))).is_none());
        // Non-hex
        assert!(parse_account(&json!("not_valid")).is_none());
        // Array mauvaise taille
        assert!(parse_account(&json!([1, 2, 3])).is_none());
    }

    // Test critique: parsing des grands montants (u128)
    #[test]
    fn test_parse_amount_large_values() {
        // Les montants Substrate peuvent dépasser u64
        let large = "340282366920938463463374607431768211455"; // u128::MAX
        assert_eq!(parse_amount(&json!(large)), Some(u128::MAX));

        // Format numérique (limité à u64 en JSON)
        assert_eq!(parse_amount(&json!(u64::MAX)), Some(u64::MAX as u128));
    }

    // Test de extract_field: logique de fallback key -> index
    #[test]
    fn test_extract_field_fallback_chain() {
        let hex = "0x".to_string() + &"aa".repeat(32);

        // Priorité: premier key trouvé
        let data = json!({"from": hex, "who": "0x".to_string() + &"bb".repeat(32)});
        let result = extract_field(&data, &["from", "who"], 0, parse_account);
        assert_eq!(result.unwrap().0, [0xaa; 32]);

        // Fallback sur second key si premier absent
        let data = json!({"who": hex});
        let result = extract_field(&data, &["from", "who"], 0, parse_account);
        assert!(result.is_some());

        // Fallback sur index si aucun key
        let data = json!([hex]);
        let result = extract_field(&data, &["from", "who"], 0, parse_account);
        assert!(result.is_some());
    }
}
