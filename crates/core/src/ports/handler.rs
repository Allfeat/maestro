//! Port trait for pallet-specific event handlers.
//!
//! This is the main extensibility point for the indexer. Each pallet
//! that needs custom indexing logic implements this trait.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

use crate::error::{DomainError, DomainResult};
use crate::models::Block;
use crate::ports::block_source::{RawBlock, RawEvent, RawExtrinsic};

/// Context passed to handlers during block processing.
pub struct HandlerContext {
    /// Current block being processed.
    pub block: Block,
    /// Raw block data for additional processing.
    pub raw_block: RawBlock,
    /// Accumulated outputs from all handlers.
    pub outputs: HandlerOutputs,
}

/// Default maximum size for handler outputs (50 MB).
pub const DEFAULT_HANDLER_OUTPUTS_MAX_SIZE: usize = 50 * 1024 * 1024;

/// Accumulated outputs from pallet handlers.
#[derive(Debug)]
pub struct HandlerOutputs {
    /// Generic key-value storage for handler outputs.
    /// Key format: "pallet_name:entity_type" (e.g., "balances:transfers")
    pub data: HashMap<String, Vec<serde_json::Value>>,
    /// Approximate current size in bytes.
    current_size: usize,
    /// Maximum allowed size in bytes.
    max_size: usize,
}

impl Default for HandlerOutputs {
    fn default() -> Self {
        Self {
            data: HashMap::new(),
            current_size: 0,
            max_size: DEFAULT_HANDLER_OUTPUTS_MAX_SIZE,
        }
    }
}

impl HandlerOutputs {
    pub fn new() -> Self {
        Self::default()
    }

    /// Create with a custom size limit.
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            data: HashMap::new(),
            current_size: 0,
            max_size,
        }
    }

    /// Add an output entity.
    /// Returns an error if serialization fails or if the size limit would be exceeded.
    pub fn add<T: serde::Serialize>(
        &mut self,
        pallet: &str,
        entity_type: &str,
        value: T,
    ) -> DomainResult<()> {
        let json = serde_json::to_value(value).map_err(|e| {
            DomainError::DecodingError(format!(
                "Failed to serialize handler output for {}:{}: {}",
                pallet, entity_type, e
            ))
        })?;

        // Estimate the size of the JSON value
        let value_size = estimate_json_size(&json);

        // Check if adding this would exceed the limit
        if self.current_size + value_size > self.max_size {
            return Err(DomainError::ValidationError(format!(
                "Handler outputs size limit exceeded: {} + {} > {} bytes",
                self.current_size, value_size, self.max_size
            )));
        }

        let key = format!("{}:{}", pallet, entity_type);
        self.data.entry(key).or_default().push(json);
        self.current_size += value_size;

        Ok(())
    }

    /// Get the current approximate size in bytes.
    pub fn current_size(&self) -> usize {
        self.current_size
    }

    /// Get outputs for a specific pallet and entity type.
    pub fn get(&self, pallet: &str, entity_type: &str) -> Option<&Vec<serde_json::Value>> {
        let key = format!("{}:{}", pallet, entity_type);
        self.data.get(&key)
    }

    /// Get typed outputs.
    pub fn get_typed<T: serde::de::DeserializeOwned>(
        &self,
        pallet: &str,
        entity_type: &str,
    ) -> Vec<T> {
        self.get(pallet, entity_type)
            .map(|values| {
                values
                    .iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Merge another outputs into this one.
    /// Note: This does not check size limits to avoid failing mid-block processing.
    /// The size is still tracked for monitoring purposes.
    pub fn merge(&mut self, other: HandlerOutputs) {
        for (key, values) in other.data {
            self.data.entry(key).or_default().extend(values);
        }
        self.current_size += other.current_size;
    }
}

/// Estimate the size of a JSON value in bytes.
/// This is an approximation for memory tracking purposes.
fn estimate_json_size(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null => 4,
        serde_json::Value::Bool(_) => 5,
        serde_json::Value::Number(n) => n.to_string().len(),
        serde_json::Value::String(s) => s.len() + 2, // quotes
        serde_json::Value::Array(arr) => {
            2 + arr.iter().map(estimate_json_size).sum::<usize>() + arr.len().saturating_sub(1) // commas
        }
        serde_json::Value::Object(obj) => {
            2 + obj
                .iter()
                .map(|(k, v)| k.len() + 3 + estimate_json_size(v)) // key + quotes + colon
                .sum::<usize>()
                + obj.len().saturating_sub(1) // commas
        }
    }
}

/// Trait for pallet-specific event handlers.
///
/// Implement this trait to add custom indexing logic for a pallet.
/// The handler will be called for each event that matches the pallet name.
#[async_trait]
pub trait PalletHandler: Send + Sync {
    /// Pallet name this handler processes (e.g., "Balances", "ATS").
    fn pallet_name(&self) -> &'static str;

    /// Process an event from this pallet.
    ///
    /// Returns extracted entities to be persisted.
    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs>;

    /// Process an extrinsic call from this pallet (optional).
    ///
    /// Override this if you need to extract data from calls themselves,
    /// not just events.
    async fn handle_extrinsic(
        &self,
        _extrinsic: &RawExtrinsic,
        _block: &Block,
    ) -> DomainResult<HandlerOutputs> {
        Ok(HandlerOutputs::new())
    }

    /// Called at the start of processing a block (optional).
    ///
    /// Useful for initializing per-block state.
    async fn on_block_start(&self, _block: &Block) -> DomainResult<()> {
        Ok(())
    }

    /// Called at the end of processing a block (optional).
    ///
    /// Useful for finalizing per-block aggregations.
    async fn on_block_end(
        &self,
        _block: &Block,
        _outputs: &HandlerOutputs,
    ) -> DomainResult<HandlerOutputs> {
        Ok(HandlerOutputs::new())
    }

    /// Priority for handler execution (higher = earlier).
    /// Default is 0. System handlers should use negative values.
    fn priority(&self) -> i32 {
        0
    }
}

/// Registry for pallet handlers.
pub struct HandlerRegistry {
    handlers: HashMap<String, Arc<dyn PalletHandler>>,
    ordered_handlers: Vec<Arc<dyn PalletHandler>>,
}

impl HandlerRegistry {
    pub fn new() -> Self {
        Self {
            handlers: HashMap::new(),
            ordered_handlers: Vec::new(),
        }
    }

    /// Register a handler for a pallet.
    pub fn register(&mut self, handler: Arc<dyn PalletHandler>) {
        let pallet = handler.pallet_name().to_string();
        self.handlers.insert(pallet, handler.clone());
        self.ordered_handlers.push(handler);
        // Sort by priority (descending)
        self.ordered_handlers
            .sort_by_key(|b| std::cmp::Reverse(b.priority()));
    }

    /// Get handler for a specific pallet.
    pub fn get(&self, pallet: &str) -> Option<&Arc<dyn PalletHandler>> {
        self.handlers.get(pallet)
    }

    /// Get all handlers in priority order.
    pub fn all(&self) -> &[Arc<dyn PalletHandler>] {
        &self.ordered_handlers
    }

    /// Check if a pallet has a registered handler.
    pub fn has_handler(&self, pallet: &str) -> bool {
        self.handlers.contains_key(pallet)
    }

    /// List all registered pallet names.
    pub fn registered_pallets(&self) -> Vec<&str> {
        self.handlers.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for HandlerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Test critique: sérialisation/désérialisation roundtrip des outputs
    #[test]
    fn test_handler_outputs_typed_roundtrip() {
        #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
        struct Transfer { from: String, to: String, amount: u64 }

        let mut outputs = HandlerOutputs::new();
        let transfer = Transfer { from: "alice".into(), to: "bob".into(), amount: 100 };

        outputs.add("balances", "transfers", &transfer).unwrap();

        let retrieved: Vec<Transfer> = outputs.get_typed("balances", "transfers");
        assert_eq!(retrieved[0], transfer);
    }

    // Test critique: protection contre les DoS via outputs trop volumineux
    #[test]
    fn test_handler_outputs_size_limit_enforced() {
        let mut outputs = HandlerOutputs::with_max_size(100);

        // Petit ajout OK
        assert!(outputs.add("test", "data", "small").is_ok());

        // Gros ajout bloqué
        let large = "x".repeat(200);
        let result = outputs.add("test", "data", &large);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("size limit"));
    }

    // Test critique: merge préserve les données de plusieurs handlers
    #[test]
    fn test_handler_outputs_merge_preserves_all() {
        let mut outputs1 = HandlerOutputs::new();
        outputs1.add("pallet1", "type1", "v1").unwrap();

        let mut outputs2 = HandlerOutputs::new();
        outputs2.add("pallet1", "type1", "v2").unwrap();
        outputs2.add("pallet2", "type2", "v3").unwrap();

        outputs1.merge(outputs2);

        // Même clé: valeurs fusionnées
        let type1: Vec<String> = outputs1.get_typed("pallet1", "type1");
        assert_eq!(type1.len(), 2);

        // Nouvelle clé: ajoutée
        let type2: Vec<String> = outputs1.get_typed("pallet2", "type2");
        assert_eq!(type2.len(), 1);
    }

    // Test critique: les handlers sont triés par priorité (décroissante)
    #[test]
    fn test_handler_registry_priority_order() {
        struct MockHandler(&'static str, i32);

        #[async_trait]
        impl PalletHandler for MockHandler {
            fn pallet_name(&self) -> &'static str { self.0 }
            fn priority(&self) -> i32 { self.1 }
            async fn handle_event(&self, _: &RawEvent, _: &Block, _: Option<&RawExtrinsic>) -> DomainResult<HandlerOutputs> {
                Ok(HandlerOutputs::new())
            }
        }

        let mut registry = HandlerRegistry::new();
        registry.register(Arc::new(MockHandler("Low", -10)));
        registry.register(Arc::new(MockHandler("High", 100)));
        registry.register(Arc::new(MockHandler("Medium", 50)));

        let all = registry.all();
        // Ordre décroissant par priorité
        assert_eq!(all[0].pallet_name(), "High");
        assert_eq!(all[1].pallet_name(), "Medium");
        assert_eq!(all[2].pallet_name(), "Low");
    }
}
