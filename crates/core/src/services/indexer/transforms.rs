//! Pure transformations: RawBlock → domain models. No service state, no
//! side effects — kept out of `IndexerService` so the allocation-heavy
//! work is easy to test in isolation.

use crate::models::{Block, BlockHash, Event, Extrinsic, ExtrinsicStatus};
use crate::ports::RawBlock;

/// Build the canonical [`Block`] from its decoded RPC counterpart.
pub fn transform_block(raw: &RawBlock) -> Block {
    Block {
        number: raw.number,
        hash: BlockHash(raw.hash),
        parent_hash: BlockHash(raw.parent_hash),
        state_root: BlockHash(raw.state_root),
        extrinsics_root: BlockHash(raw.extrinsics_root),
        author: None,
        timestamp: raw.timestamp.map(|ts| {
            chrono::DateTime::from_timestamp_millis(ts as i64).unwrap_or_else(chrono::Utc::now)
        }),
        extrinsic_count: raw.extrinsics.len() as u32,
        event_count: raw.events.len() as u32,
        indexed_at: chrono::Utc::now(),
    }
}

/// Project the raw extrinsic vector into domain [`Extrinsic`]s for a block.
pub fn transform_extrinsics(raw: &RawBlock, block: &Block) -> Vec<Extrinsic> {
    raw.extrinsics
        .iter()
        .map(|ext| Extrinsic {
            id: format!("{}-{}", block.number, ext.index),
            block_number: block.number,
            block_hash: block.hash,
            index: ext.index,
            pallet: ext.pallet.clone(),
            call: ext.call.clone(),
            signer: ext.signer.map(crate::models::AccountId),
            status: if ext.success {
                ExtrinsicStatus::Success
            } else {
                ExtrinsicStatus::Failed
            },
            error: ext.error.clone(),
            args: ext.args.clone(),
            raw: hex::encode(&ext.bytes),
            tip: ext.tip,
            nonce: ext.nonce,
        })
        .collect()
}

/// Project the raw event vector into domain [`Event`]s for a block.
pub fn transform_events(raw: &RawBlock, block: &Block) -> Vec<Event> {
    raw.events
        .iter()
        .map(|evt| Event {
            id: format!("{}-{}", block.number, evt.index),
            block_number: block.number,
            block_hash: block.hash,
            index: evt.index,
            extrinsic_index: evt.extrinsic_index,
            pallet: evt.pallet.clone(),
            name: evt.name.clone(),
            data: evt.data.clone(),
            topics: evt.topics.iter().map(hex::encode).collect(),
        })
        .collect()
}
