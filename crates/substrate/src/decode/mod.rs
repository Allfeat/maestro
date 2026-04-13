//! Block decoding: raw SCALE → domain `RawBlock` / `RawEvent` / `RawExtrinsic`.

mod events;
mod extrinsics;
mod timestamp;

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::ports::RawBlock;

use crate::client::SubstrateBlock;

pub(crate) async fn decode_raw_block(block: &SubstrateBlock) -> ChainResult<RawBlock> {
    // In subxt 0.50, the stream yields a lightweight `Block<T>` that does not
    // expose events/extrinsics directly; we resolve it to a `ClientAtBlock` to
    // access them.
    let at_block = block
        .at()
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

    let extrinsics = extrinsics::decode_extrinsics(&at_block, &events).await?;
    let raw_events = events::decode_events(&events);
    let timestamp = timestamp::read_block_timestamp(&at_block).await?;

    Ok(RawBlock {
        number: block.number(),
        hash: block.hash().into(),
        parent_hash: block.header().parent_hash.into(),
        state_root: block.header().state_root.into(),
        extrinsics_root: block.header().extrinsics_root.into(),
        extrinsics,
        events: raw_events,
        timestamp,
    })
}
