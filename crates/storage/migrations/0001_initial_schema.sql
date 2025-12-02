-- Initial schema for Substrate indexer (frame_system only)
-- Migration: 0001_initial_schema

-- Enable necessary extensions
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";

-- ============================================================================
-- Core tables (frame_system)
-- ============================================================================

-- Blocks table
CREATE TABLE blocks (
    number BIGINT PRIMARY KEY,
    hash BYTEA NOT NULL UNIQUE,
    parent_hash BYTEA NOT NULL,
    state_root BYTEA NOT NULL,
    extrinsics_root BYTEA NOT NULL,
    author BYTEA,
    timestamp TIMESTAMPTZ,
    extrinsic_count INTEGER NOT NULL DEFAULT 0,
    event_count INTEGER NOT NULL DEFAULT 0,
    indexed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_blocks_hash ON blocks(hash);
CREATE INDEX idx_blocks_timestamp ON blocks(timestamp);
CREATE INDEX idx_blocks_author ON blocks(author) WHERE author IS NOT NULL;

-- Extrinsics table
CREATE TABLE extrinsics (
    id TEXT PRIMARY KEY,
    block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    block_hash BYTEA NOT NULL,
    index INTEGER NOT NULL,
    pallet TEXT NOT NULL,
    call TEXT NOT NULL,
    signer BYTEA,
    status TEXT NOT NULL CHECK (status IN ('success', 'failed')),
    error TEXT,
    args JSONB NOT NULL DEFAULT '{}',
    raw TEXT NOT NULL,
    tip NUMERIC(39, 0),
    nonce INTEGER,

    UNIQUE(block_number, index)
);

CREATE INDEX idx_extrinsics_block ON extrinsics(block_number);
CREATE INDEX idx_extrinsics_pallet_call ON extrinsics(pallet, call);
CREATE INDEX idx_extrinsics_signer ON extrinsics(signer) WHERE signer IS NOT NULL;
CREATE INDEX idx_extrinsics_status ON extrinsics(status);

-- Events table
CREATE TABLE events (
    id TEXT PRIMARY KEY,
    block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    block_hash BYTEA NOT NULL,
    index INTEGER NOT NULL,
    extrinsic_index INTEGER,
    pallet TEXT NOT NULL,
    name TEXT NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    topics TEXT[] NOT NULL DEFAULT '{}',

    UNIQUE(block_number, index)
);

CREATE INDEX idx_events_block ON events(block_number);
CREATE INDEX idx_events_extrinsic ON events(block_number, extrinsic_index) WHERE extrinsic_index IS NOT NULL;
CREATE INDEX idx_events_pallet_name ON events(pallet, name);

-- Indexer cursor (tracks indexing progress)
CREATE TABLE indexer_cursor (
    chain_id TEXT PRIMARY KEY,
    last_indexed_block BIGINT NOT NULL,
    last_indexed_hash BYTEA NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- ============================================================================
-- Bundle migration tracking
-- ============================================================================

CREATE TABLE bundle_migrations (
    bundle_name TEXT NOT NULL,
    migration_index INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    executed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (bundle_name, migration_index)
);
