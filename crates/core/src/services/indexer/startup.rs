//! Startup checks: V14 floor, chain-id match, cursor consistency.
//!
//! All methods here live on `IndexerService` via a second `impl` block. They
//! run once, in order, before the live/backfill loop kicks off. A single
//! failure aborts the service — callers treat these errors as fatal.

use tracing::{debug, error, info, instrument, warn};

use crate::error::{IndexerError, IndexerResult};
use crate::events::CursorState;
use crate::metrics::record_blocks_deleted;
use crate::models::IndexerCursor;
use crate::ports::{BlockSource, Repositories};

use super::IndexerService;

/// Pure V14 metadata floor check. Returns `PreV14BlockRequested` if the
/// requested `start_block` is below the chain's earliest V14 block.
pub(crate) fn check_v14_floor(start_block: u64, earliest_v14: u64) -> IndexerResult<()> {
    if start_block < earliest_v14 {
        return Err(IndexerError::PreV14BlockRequested {
            requested: start_block,
            earliest_v14,
        });
    }
    Ok(())
}

impl<S: BlockSource + 'static, R: Repositories> IndexerService<S, R> {
    /// Initialize the in-memory cursor mirror from the persisted cursor
    /// (if any) and publish an initial `CursorState` on the watch channel so
    /// consumers like the TUI render a meaningful head/tail on first frame.
    pub(super) async fn seed_cursor_mirror(&self, existing: Option<&IndexerCursor>) {
        let state = existing.map(|c| CursorState {
            head: c.last_indexed_block,
            tail: c.first_indexed_block,
        });
        if let Some(s) = state {
            self.event_bus.update_cursor(s);
        }
        *self.cursor_mirror.lock().await = state;
    }

    pub(super) fn enforce_v14_floor(&self, earliest_v14: u64) -> IndexerResult<()> {
        check_v14_floor(self.config.backfill.start_block, earliest_v14)
    }

    pub(super) async fn warn_if_cursor_gap(&self, cursor: &Option<IndexerCursor>) {
        let Some(c) = cursor else { return };

        // Tip-side gap: blocks between cursor top and current chain head.
        if let Ok(head) = self.block_source.finalized_head().await {
            let gap = head.number.saturating_sub(c.last_indexed_block);
            if gap > 1 {
                warn!(
                    cursor_top = c.last_indexed_block,
                    tip = head.number,
                    gap,
                    "⚠️  --live-only with a cursor gap; {gap} blocks will never be filled"
                );
            }
        }

        // Floor-side gap: blocks below cursor floor that --start-block wanted.
        let start_block = self.config.backfill.start_block;
        if c.first_indexed_block > start_block {
            let gap = c.first_indexed_block - start_block;
            warn!(
                start_block,
                cursor_floor = c.first_indexed_block,
                gap,
                "⚠️  --live-only with a floor-side cursor gap; {gap} blocks below {} will never be filled",
                c.first_indexed_block
            );
        }
    }

    /// Verify the connected chain matches any existing indexed data.
    /// Returns error if database contains data from a different chain.
    pub(super) async fn verify_chain_id(&self) -> IndexerResult<()> {
        let existing_cursor = self.repositories.cursor().get_any_cursor().await?;

        if let Some(cursor) = existing_cursor {
            if cursor.chain_id != self.config.chain_id {
                let connected_short = &self.config.chain_id[..16.min(self.config.chain_id.len())];
                let expected_short = &cursor.chain_id[..16.min(cursor.chain_id.len())];

                error!(
                    connected = connected_short,
                    expected = expected_short,
                    "❌ Chain mismatch! Database contains data from a different chain"
                );
                error!(
                    "   Manual action required: either connect to the correct chain or clear the database"
                );

                return Err(IndexerError::ChainMismatch {
                    connected: self.config.chain_id.clone(),
                    expected: cursor.chain_id,
                });
            }
            debug!("Chain ID verified");
        }

        Ok(())
    }

    /// Verify consistency between stored cursor and chain state on reconnection.
    #[instrument(skip(self))]
    pub(super) async fn verify_consistency_on_startup(
        &self,
    ) -> IndexerResult<Option<IndexerCursor>> {
        let cursor = self
            .repositories
            .cursor()
            .get_cursor(&self.config.chain_id)
            .await?;

        let Some(cursor) = cursor else {
            debug!("No cursor found, starting fresh");
            return Ok(None);
        };

        debug!(
            block = cursor.last_indexed_block,
            "Verifying cursor consistency"
        );

        let stored_block = self
            .repositories
            .blocks()
            .get_block(cursor.last_indexed_block)
            .await?;

        match stored_block {
            Some(block) if block.hash == cursor.last_indexed_hash => {
                debug!(
                    block = cursor.last_indexed_block,
                    "Cursor verified, resuming"
                );
                Ok(Some(cursor))
            }
            Some(_) => {
                warn!(
                    block = cursor.last_indexed_block,
                    "⚠️  Cursor hash mismatch, cleaning up"
                );
                let deleted = self
                    .repositories
                    .delete_from_block_atomic(cursor.last_indexed_block, &self.config.chain_id)
                    .await?;
                info!(deleted, "🗑️  Cleaned inconsistent data");
                record_blocks_deleted(deleted);
                Ok(None)
            }
            None => {
                warn!(
                    block = cursor.last_indexed_block,
                    "⚠️  Cursor points to missing block"
                );
                let deleted = self
                    .repositories
                    .delete_from_block_atomic(0, &self.config.chain_id)
                    .await?;
                info!(deleted, "🗑️  Cleaned inconsistent data");
                record_blocks_deleted(deleted);
                Ok(None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_at_or_above_floor_passes() {
        assert!(check_v14_floor(0, 0).is_ok());
        assert!(check_v14_floor(100, 100).is_ok());
        assert!(check_v14_floor(200, 100).is_ok());
    }

    #[test]
    fn start_below_floor_errors_with_both_numbers() {
        let err = check_v14_floor(50, 473291).unwrap_err();
        match err {
            IndexerError::PreV14BlockRequested {
                requested,
                earliest_v14,
            } => {
                assert_eq!(requested, 50);
                assert_eq!(earliest_v14, 473291);
            }
            other => panic!("expected PreV14BlockRequested, got {other:?}"),
        }
    }
}
