-- 0002: Add first_indexed_block column to indexer_cursor.
-- Phase 2 cursor model: contiguous [first_indexed_block, last_indexed_block].
-- Pre-Phase-2 deployments always indexed from genesis (no start-block CLI),
-- so backfilling first_indexed_block = 0 is provably correct for every row.

ALTER TABLE indexer_cursor
    ADD COLUMN first_indexed_block BIGINT NOT NULL DEFAULT 0;

UPDATE indexer_cursor SET first_indexed_block = 0 WHERE first_indexed_block IS NULL;

ALTER TABLE indexer_cursor ALTER COLUMN first_indexed_block DROP DEFAULT;
