//! Substrate RPC client with dynamic metadata decoding.

use std::sync::Arc;

use async_trait::async_trait;
use futures::StreamExt;
use subxt::backend::chain_head::{ChainHeadBackend, ChainHeadBackendBuilder};
use subxt::backend::rpc::RpcClient;
use subxt::blocks::Block;
use subxt::{OnlineClient, PolkadotConfig};
use tracing::{debug, instrument, trace, warn};

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::metrics::record_decode_error;
use maestro_core::models::BlockHash;
use maestro_core::ports::{
    BlockSource, FinalizedBlockStream, FinalizedHead, RawBlock, RawEvent, RawExtrinsic,
};

/// Configuration for the Substrate client.
#[derive(Debug, Clone)]
pub struct SubstrateClientConfig {
    /// WebSocket URL (e.g., "ws://localhost:9944").
    pub ws_url: String,
}

pub type SubstrateBlock = Block<PolkadotConfig, OnlineClient<PolkadotConfig>>;

impl Default for SubstrateClientConfig {
    fn default() -> Self {
        Self {
            ws_url: "ws://127.0.0.1:9944".to_string(),
        }
    }
}

/// Substrate client adapter implementing the BlockSource port.
pub struct SubstrateClient {
    client: OnlineClient<PolkadotConfig>,
}

impl SubstrateClient {
    /// Connect to a Substrate node.
    #[instrument(skip_all, fields(url = %config.ws_url))]
    pub async fn connect(config: SubstrateClientConfig) -> ChainResult<Self> {
        debug!("Connecting to node");

        let rpc_client = RpcClient::from_url(&config.ws_url)
            .await
            .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?;
        let backend: ChainHeadBackend<PolkadotConfig> =
            ChainHeadBackendBuilder::default().build_with_background_driver(rpc_client.clone());
        let client = OnlineClient::<PolkadotConfig>::from_backend(Arc::new(backend))
            .await
            .map_err(|e| ChainError::ConnectionFailed(e.to_string()))?;

        debug!("Connected successfully");

        Ok(Self { client })
    }
}

#[async_trait]
impl BlockSource for SubstrateClient {
    async fn genesis_hash(&self) -> ChainResult<BlockHash> {
        let hash = self.client.genesis_hash();
        Ok(BlockHash(hash.0))
    }

    async fn finalized_head(&self) -> ChainResult<FinalizedHead> {
        let head = self
            .client
            .blocks()
            .at_latest()
            .await
            .map_err(|e| ChainError::RpcError(e.to_string()))?;

        Ok(FinalizedHead {
            number: head.number() as u64,
            hash: head.hash().into(),
        })
    }

    async fn subscribe_finalized(&self) -> ChainResult<FinalizedBlockStream> {
        let subscription = self
            .client
            .blocks()
            .subscribe_finalized()
            .await
            .map_err(|e| ChainError::SubscriptionError(e.to_string()))?;

        let stream = subscription.then(|result| async move {
            match result {
                Ok(block) => {
                    let extrinsics = decode_extrinsics(&block).await?;
                    let events = decode_events(&block).await?;
                    let timestamp = get_block_timestamp(&block).await?;

                    Ok(RawBlock {
                        number: block.number() as u64,
                        hash: block.hash().into(),
                        parent_hash: block.header().parent_hash.into(),
                        state_root: block.header().state_root.into(),
                        extrinsics_root: block.header().extrinsics_root.into(),
                        extrinsics,
                        events,
                        timestamp,
                    })
                }
                Err(e) => Err(ChainError::SubscriptionError(e.to_string())),
            }
        });

        Ok(Box::pin(stream))
    }

    async fn runtime_version(&self) -> ChainResult<u32> {
        let version = self.client.runtime_version();
        Ok(version.spec_version)
    }
}

// =============================================================================
// Block decoding helpers
// =============================================================================

/// Decode events from a block.
async fn decode_events(block: &SubstrateBlock) -> ChainResult<Vec<RawEvent>> {
    let events = block
        .events()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    let mut raw_events = Vec::new();

    for (index, event) in events.iter().enumerate() {
        match event {
            Ok(ev) => {
                let pallet = ev.pallet_name().to_string();
                let name = ev.variant_name().to_string();

                let data = ev
                    .field_values()
                    .map(|composite| composite_to_json(&composite))
                    .unwrap_or(serde_json::Value::Null);

                let extrinsic_index = match ev.phase() {
                    subxt::events::Phase::ApplyExtrinsic(idx) => Some(idx),
                    _ => None,
                };

                raw_events.push(RawEvent {
                    index: index as u32,
                    extrinsic_index,
                    pallet,
                    name,
                    data,
                    topics: Vec::new(),
                });
            }
            Err(e) => {
                trace!(index, error = ?e, "Failed to decode event");
                record_decode_error("event", "unknown");
            }
        }
    }

    Ok(raw_events)
}

/// Decode extrinsics from a block.
async fn decode_extrinsics(block: &SubstrateBlock) -> ChainResult<Vec<RawExtrinsic>> {
    let extrinsics = block
        .extrinsics()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    let events = block
        .events()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    let mut raw_extrinsics = Vec::new();

    for (index, ext) in extrinsics.iter().enumerate() {
        let pallet = ext
            .pallet_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        let call = ext
            .variant_name()
            .map(|s| s.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let (success, error) = get_extrinsic_result(&events, index as u32);

        let signer = ext.address_bytes().and_then(|bytes| {
            if bytes.len() == 32 {
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&bytes[..32]);
                Some(arr)
            } else {
                trace!(index, len = bytes.len(), "Invalid signer address length");
                None
            }
        });

        let args = ext
            .field_values()
            .map(|composite| composite_to_json(&composite))
            .unwrap_or(serde_json::Value::Null);

        raw_extrinsics.push(RawExtrinsic {
            index: index as u32,
            bytes: ext.bytes().to_vec(),
            pallet,
            call,
            signer,
            args,
            success,
            error,
            tip: None,
            nonce: None,
        });
    }

    Ok(raw_extrinsics)
}

/// Get extrinsic result from events.
fn get_extrinsic_result(
    events: &subxt::events::Events<PolkadotConfig>,
    ext_index: u32,
) -> (bool, Option<String>) {
    for ev in events.iter().flatten() {
        if let subxt::events::Phase::ApplyExtrinsic(idx) = ev.phase()
            && idx == ext_index
        {
            let pallet = ev.pallet_name();
            let name = ev.variant_name();

            if pallet == "System" {
                if name == "ExtrinsicSuccess" {
                    return (true, None);
                } else if name == "ExtrinsicFailed" {
                    let error_info = ev
                        .field_values()
                        .map(|v| format!("{:?}", v))
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    return (false, Some(error_info));
                }
            }
        }
    }
    (true, None)
}

/// Get timestamp from Timestamp.set inherent.
async fn get_block_timestamp(block: &SubstrateBlock) -> ChainResult<Option<u64>> {
    let extrinsics = block
        .extrinsics()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    for ext in extrinsics.iter() {
        let pallet = match ext.pallet_name() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let call = match ext.variant_name() {
            Ok(c) => c,
            Err(_) => continue,
        };

        if pallet == "Timestamp" && call == "set" {
            if let Ok(values) = ext.field_values() {
                let value_str = format!("{:?}", values);
                if let Some(ts) = parse_timestamp_from_debug(&value_str) {
                    return Ok(Some(ts));
                }

                warn!(
                    block = block.number(),
                    "Could not parse timestamp from Timestamp.set: {:?}", values
                );
            }

            let bytes = ext.bytes();
            if bytes.len() >= 5
                && let Some(ts) = try_decode_compact_u64(&bytes[2..])
            {
                return Ok(Some(ts));
            }
        }
    }

    Ok(None)
}

/// Try to decode a Compact<u64> from bytes.
fn try_decode_compact_u64(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let first = bytes[0];
    let mode = first & 0b11;

    match mode {
        0b00 => Some((first >> 2) as u64),
        0b01 => {
            if bytes.len() < 2 {
                return None;
            }
            let value = u16::from_le_bytes([bytes[0], bytes[1]]) >> 2;
            Some(value as u64)
        }
        0b10 => {
            if bytes.len() < 4 {
                return None;
            }
            let value = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) >> 2;
            Some(value as u64)
        }
        0b11 => {
            let num_bytes = ((first >> 2) + 4) as usize;
            if bytes.len() < 1 + num_bytes || num_bytes > 8 {
                return None;
            }
            let mut value_bytes = [0u8; 8];
            value_bytes[..num_bytes].copy_from_slice(&bytes[1..1 + num_bytes]);
            Some(u64::from_le_bytes(value_bytes))
        }
        _ => None,
    }
}

/// Parse timestamp from debug string.
fn parse_timestamp_from_debug(s: &str) -> Option<u64> {
    const MIN_TIMESTAMP_MS: u64 = 1_577_836_800_000;
    const MAX_TIMESTAMP_MS: u64 = 2_524_608_000_000;

    for part in s.split(|c: char| !c.is_ascii_digit()) {
        if let Ok(num) = part.parse::<u64>()
            && (MIN_TIMESTAMP_MS..=MAX_TIMESTAMP_MS).contains(&num)
        {
            return Some(num);
        }
    }
    None
}

// =============================================================================
// SCALE Value to JSON conversion
// =============================================================================

use subxt::ext::scale_value::{Composite, Value, ValueDef};

/// Convert a Composite to a JSON value.
fn composite_to_json<T>(composite: &Composite<T>) -> serde_json::Value {
    match composite {
        Composite::Unnamed(values) => {
            // Check if this looks like a byte array (e.g., AccountId, Hash)
            if let Some(hex_str) = try_as_byte_array(values) {
                return serde_json::Value::String(hex_str);
            }
            // Unwrap single-element tuples (common for newtype wrappers like AccountId)
            if values.len() == 1 {
                return value_to_json(&values[0]);
            }
            serde_json::Value::Array(values.iter().map(value_to_json).collect())
        }
        Composite::Named(fields) => {
            let obj: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .map(|(name, v)| (name.clone(), value_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
    }
}

/// Try to interpret an unnamed composite as a byte array (AccountId, Hash, etc).
/// Returns a hex string if the composite looks like a byte array, None otherwise.
fn try_as_byte_array<T>(values: &[Value<T>]) -> Option<String> {
    // Common byte array sizes: 32 (AccountId, Hash), 20 (EthAddress), etc.
    let len = values.len();
    if len != 32 && len != 20 && len != 64 {
        return None;
    }

    let mut bytes = Vec::with_capacity(len);
    for value in values {
        if let ValueDef::Primitive(subxt::ext::scale_value::Primitive::U128(n)) = &value.value {
            if *n <= 255 {
                bytes.push(*n as u8);
            } else {
                return None;
            }
        } else {
            return None;
        }
    }

    Some(format!("0x{}", hex::encode(bytes)))
}

/// Convert a Value to a JSON value.
fn value_to_json<T>(value: &Value<T>) -> serde_json::Value {
    value_def_to_json(&value.value)
}

/// Convert a ValueDef to a JSON value.
fn value_def_to_json<T>(value: &ValueDef<T>) -> serde_json::Value {
    match value {
        ValueDef::Composite(composite) => composite_to_json(composite),
        ValueDef::Variant(variant) => {
            let variant_name = &variant.name;
            let inner = composite_to_json(&variant.values);

            // For Option variants, simplify
            if variant_name == "Some" {
                // Extract the single value from Some
                if let serde_json::Value::Array(arr) = &inner
                    && arr.len() == 1
                {
                    return arr[0].clone();
                }
                return inner;
            } else if variant_name == "None" {
                return serde_json::Value::Null;
            }

            // For Id variant (common in AccountId), unwrap it
            if variant_name == "Id" {
                if let serde_json::Value::Array(arr) = &inner
                    && arr.len() == 1
                {
                    return arr[0].clone();
                }
                return inner;
            }

            // For other variants, wrap in object
            let mut map = serde_json::Map::new();
            map.insert(variant_name.clone(), inner);
            serde_json::Value::Object(map)
        }
        ValueDef::Primitive(primitive) => primitive_to_json(primitive),
        ValueDef::BitSequence(bits) => serde_json::Value::String(format!("{:?}", bits)),
    }
}

/// Convert a Primitive to a JSON value.
fn primitive_to_json(primitive: &subxt::ext::scale_value::Primitive) -> serde_json::Value {
    use subxt::ext::scale_value::Primitive;
    match primitive {
        Primitive::Bool(b) => serde_json::Value::Bool(*b),
        Primitive::Char(c) => serde_json::Value::String(c.to_string()),
        Primitive::String(s) => serde_json::Value::String(s.clone()),
        Primitive::U128(n) => serde_json::Value::String(n.to_string()),
        Primitive::I128(n) => serde_json::Value::String(n.to_string()),
        Primitive::U256(n) => serde_json::Value::String(format!("{:?}", n)),
        Primitive::I256(n) => serde_json::Value::String(format!("{:?}", n)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_decode_compact_u64_single_byte() {
        assert_eq!(try_decode_compact_u64(&[252]), Some(63));
        assert_eq!(try_decode_compact_u64(&[0]), Some(0));
        assert_eq!(try_decode_compact_u64(&[4]), Some(1));
    }

    #[test]
    fn test_try_decode_compact_u64_two_byte() {
        assert_eq!(try_decode_compact_u64(&[0xFD, 0xFF]), Some(16383));
    }

    #[test]
    fn test_try_decode_compact_u64_four_byte() {
        let encoded = (1_000_000_000u32 << 2 | 0b10).to_le_bytes();
        assert_eq!(try_decode_compact_u64(&encoded), Some(1_000_000_000));
    }

    #[test]
    fn test_try_decode_compact_u64_big_integer() {
        let timestamp: u64 = 1_733_097_600_000;
        let mut bytes = vec![0b00001011];
        bytes.extend_from_slice(&timestamp.to_le_bytes()[..6]);
        assert_eq!(try_decode_compact_u64(&bytes), Some(timestamp));
    }

    #[test]
    fn test_parse_timestamp_from_debug() {
        assert_eq!(
            parse_timestamp_from_debug("Compact(1733097600000)"),
            Some(1733097600000)
        );
        assert_eq!(
            parse_timestamp_from_debug("{now: 1700000000000}"),
            Some(1700000000000)
        );
        assert_eq!(parse_timestamp_from_debug("Compact(1500000000000)"), None);
        assert_eq!(parse_timestamp_from_debug("Compact(3000000000000)"), None);
        assert_eq!(parse_timestamp_from_debug("no timestamp here"), None);
    }
}
