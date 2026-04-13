//! Block decoding: raw SCALE → domain `RawBlock` / `RawEvent` / `RawExtrinsic`.

mod events;
mod extrinsics;
mod timestamp;

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::ports::RawBlock;

use crate::client::{SubstrateBlock, SubstrateClientAtBlock};

pub(crate) async fn decode_raw_block(block: &SubstrateBlock) -> ChainResult<RawBlock> {
    // In subxt 0.50, the stream yields a lightweight `Block<T>` that does not
    // expose events/extrinsics directly; we resolve it to a `ClientAtBlock` to
    // access them.
    let at_block = block
        .at()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    decode_raw_block_at(&at_block).await
}

/// Decode a `RawBlock` directly from a resolved `ClientAtBlock`.
///
/// This is the meaty decoder shared by the subscription path (via
/// `decode_raw_block`) and the historic-sync path (via
/// `SubstrateClient::fetch_block_at`, which obtains its `ClientAtBlock`
/// from `OnlineClient::at_block(number)` rather than from a stream item).
pub(crate) async fn decode_raw_block_at(
    at_block: &SubstrateClientAtBlock,
) -> ChainResult<RawBlock> {
    let header = at_block
        .block_header()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    // Fetch events once here and share the handle between the two decoders.
    // Extrinsics need them for per-extrinsic success/error lookup; events
    // need them to project into `RawEvent`s. Fetching twice was pure waste.
    let events = at_block
        .events()
        .fetch()
        .await
        .map_err(|e| ChainError::RpcError(e.to_string()))?;

    let extrinsics = extrinsics::decode_extrinsics(at_block, &events).await?;
    let raw_events = events::decode_events(&events);
    let timestamp = timestamp::read_block_timestamp(at_block).await?;

    Ok(RawBlock {
        number: at_block.block_number(),
        hash: at_block.block_hash().into(),
        parent_hash: header.parent_hash.into(),
        state_root: header.state_root.into(),
        extrinsics_root: header.extrinsics_root.into(),
        extrinsics,
        events: raw_events,
        timestamp,
    })
}
