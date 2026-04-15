//! Block timestamp extraction via the `Timestamp::Now` storage item.
//!
//! Reads the canonical runtime-written timestamp at the block's hash using the
//! canonical subxt 0.50 dynamic-storage flow (see `subxt/examples/dynamic.rs`).
//! Returns `None` only when the storage value itself is absent (e.g. pre-Timestamp
//! genesis block). A runtime without the `Timestamp` pallet surfaces as
//! `ChainError::RpcError` from `fetch` — that is an operator mistake, not a
//! per-block soft failure.

use subxt::dynamic;

use maestro_core::error::{ChainError, ChainResult};

use crate::client::SubstrateClientAtBlock;

pub(crate) async fn read_block_timestamp(
    at_block: &SubstrateClientAtBlock,
) -> ChainResult<Option<u64>> {
    let addr = dynamic::storage::<(), u64>("Timestamp", "Now");
    let value = at_block
        .storage()
        .try_fetch(addr, ())
        .await
        .map_err(|e| ChainError::RpcError(format!("fetch Timestamp::Now: {}", e)))?;
    match value {
        Some(v) => Ok(Some(v.decode().map_err(|e| {
            ChainError::RpcError(format!("decode Timestamp::Now: {}", e))
        })?)),
        None => Ok(None),
    }
}
