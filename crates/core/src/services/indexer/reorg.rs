//! Parent-hash reorg detection used by the live loop.
//!
//! When the incoming block's `parent_hash` doesn't match what we stored at
//! `block - 1`, we delete the conflicting suffix and let the normal insert
//! path re-run on the correct chain.

use tracing::{info, instrument, warn};

use crate::error::IndexerResult;
use crate::metrics::{record_blocks_deleted, record_reorg_detected};
use crate::models::BlockHash;
use crate::ports::{BlockSource, RawBlock, Repositories};

use super::IndexerService;

impl<S: BlockSource + 'static, R: Repositories> IndexerService<S, R> {
    /// Check for chain reorganization by comparing parent hash. Returns
    /// `Ok(true)` when a reorg was detected and cleaned up.
    #[instrument(skip(self, raw_block), fields(block = raw_block.number))]
    pub(super) async fn check_and_handle_reorg(&self, raw_block: &RawBlock) -> IndexerResult<bool> {
        let block_number = raw_block.number;

        if block_number == 0 {
            return Ok(false);
        }

        let stored_prev = self
            .repositories
            .blocks()
            .get_block(block_number - 1)
            .await?;

        match stored_prev {
            Some(prev_block) => {
                let expected_parent = BlockHash(raw_block.parent_hash);
                if prev_block.hash != expected_parent {
                    warn!(
                        block = block_number,
                        expected = %hex::encode(&expected_parent.0[..8]),
                        stored = %hex::encode(&prev_block.hash.0[..8]),
                        "🔄 Reorg detected! Parent hash mismatch"
                    );

                    record_reorg_detected(block_number);

                    let deleted = self
                        .repositories
                        .delete_from_block_atomic(block_number - 1, &self.config.chain_id)
                        .await?;

                    info!(block = block_number, deleted = deleted, "🔄 Reorg handled");
                    record_blocks_deleted(deleted);

                    return Ok(true);
                }
            }
            None => {
                let latest = self.repositories.blocks().latest_block_number().await?;
                if let Some(latest_num) = latest
                    && block_number > latest_num + 1
                {
                    warn!(
                        block = block_number,
                        latest = latest_num,
                        gap = block_number - latest_num - 1,
                        "⚠️  Gap detected in block sequence"
                    );
                }
            }
        }

        Ok(false)
    }
}
