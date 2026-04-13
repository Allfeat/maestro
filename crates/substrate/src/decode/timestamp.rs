//! Block timestamp extraction via the `Timestamp::Now` storage item.
//!
//! Reads the canonical runtime-written timestamp at the block's hash. Returns
//! `None` if the pallet is absent or the value is missing (e.g. pre-Timestamp
//! genesis block). Adds one storage RPC per block; negligible at 6s/block.

use codec::Decode;
use tracing::warn;

use maestro_core::error::{ChainError, ChainResult};

use crate::client::SubstrateClientAtBlock;

pub(crate) async fn read_block_timestamp(
    at_block: &SubstrateClientAtBlock,
) -> ChainResult<Option<u64>> {
    // Build the Timestamp::Now storage query using the same dynamic-storage
    // shape as `storage.rs::read_storage_map_u64`. `Timestamp::Now` is a
    // plain (non-mapped) storage value, so we pass an empty key vec to
    // `try_fetch`.
    let storage_query = subxt::dynamic::storage::<Vec<subxt::dynamic::Value>, subxt::dynamic::Value>(
        "Timestamp",
        "Now",
    );

    // Constructing the entry can fail if the pallet/item is missing from
    // metadata (e.g. a runtime without a Timestamp pallet). Treat that as
    // "no timestamp available" rather than an error.
    let entry = match at_block.storage().entry(storage_query) {
        Ok(entry) => entry,
        Err(e) => {
            warn!(
                block_hash = ?at_block.block_hash(),
                error = %e,
                "Timestamp::Now entry construction failed — treating as missing. \
                 Usually this means the Timestamp pallet is absent from metadata."
            );
            return Ok(None);
        }
    };

    let raw_bytes: Vec<u8> = match entry.try_fetch(vec![]).await {
        Ok(Some(value)) => value.bytes().to_vec(),
        Ok(None) => return Ok(None),
        Err(e) => {
            return Err(ChainError::RpcError(format!("fetch Timestamp::Now: {}", e)));
        }
    };

    // Timestamp::Now is `Moment = u64` (SCALE-encoded as 8 little-endian bytes).
    let ts = u64::decode(&mut &raw_bytes[..])
        .map_err(|e| ChainError::RpcError(format!("decode Timestamp::Now: {}", e)))?;

    Ok(Some(ts))
}

#[cfg(test)]
mod tests {
    #[test]
    fn dynamic_storage_query_for_timestamp_now_compiles() {
        // Reachability check: the subxt dynamic storage builder signature used by
        // read_block_timestamp must compile for the Timestamp::Now entry. Catches
        // silent regressions where the builder shape drifts on subxt upgrade.
        let _query = subxt::dynamic::storage::<Vec<subxt::dynamic::Value>, subxt::dynamic::Value>(
            "Timestamp",
            "Now",
        );
    }
}
