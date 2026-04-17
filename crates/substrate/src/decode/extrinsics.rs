//! Decode block extrinsics into `RawExtrinsic`s.

use tracing::trace;

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::metrics::record_decode_error;
use maestro_core::ports::RawExtrinsic;
use subxt::PolkadotConfig;
use subxt::dynamic::Value;

use crate::client::SubstrateClientAtBlock;
use crate::decode::events::build_extrinsic_results;
use crate::scale_json::value_to_json;

pub(crate) async fn decode_extrinsics(
    block: &SubstrateClientAtBlock,
    events: &subxt::events::Events<PolkadotConfig>,
) -> ChainResult<Vec<RawExtrinsic>> {
    let extrinsics = block
        .extrinsics()
        .fetch()
        .await
        .map_err(|e| ChainError::rpc_with_source(e.to_string(), e))?;

    let results = build_extrinsic_results(events);
    let mut raw_extrinsics = Vec::new();

    for (index, ext_result) in extrinsics.iter().enumerate() {
        let ext = match ext_result {
            Ok(ext) => ext,
            Err(e) => {
                trace!(index, error = ?e, "Failed to decode extrinsic");
                continue;
            }
        };

        let pallet = ext.pallet_name().to_string();
        let call = ext.call_name().to_string();

        let (success, error) = results
            .get(&(index as u32))
            .cloned()
            .unwrap_or((true, None));

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
            .decode_call_data_fields_unchecked_as::<Value>()
            .map(|value| value_to_json(&value))
            .unwrap_or_else(|e| {
                trace!(
                    index,
                    pallet = %pallet,
                    call = %call,
                    error = ?e,
                    "Failed to decode extrinsic call data"
                );
                record_decode_error("extrinsic", &pallet);
                serde_json::Value::Null
            });

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
