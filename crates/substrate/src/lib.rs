//! Substrate RPC adapter for Maestro indexer.
//!
//! This crate implements the [`BlockSource`] port from `maestro-core`,
//! providing connectivity to Substrate-based blockchains via WebSocket RPC.
//!
//! # Features
//!
//! - Finalized block subscription with automatic reconnection
//! - Dynamic metadata decoding using subxt
//! - SCALE to JSON conversion for events and extrinsics
//! - Compact timestamp extraction from `Timestamp.set` inherent
//!
//! # Usage
//!
//! ```ignore
//! use maestro_substrate::{SubstrateClient, SubstrateClientConfig};
//!
//! let config = SubstrateClientConfig {
//!     ws_url: "ws://localhost:9944".to_string(),
//! };
//!
//! let client = SubstrateClient::connect(config).await?;
//! let genesis = client.genesis_hash().await?;
//! let mut stream = client.subscribe_finalized().await?;
//!
//! while let Some(block) = stream.next().await {
//!     // Process block...
//! }
//! ```
//!
//! # Architecture
//!
//! The client uses subxt's ChainHead backend for efficient block fetching.
//! Block data is decoded into `RawBlock`, `RawExtrinsic`, and `RawEvent`
//! structures defined in `maestro-core`.
//!
//! [`BlockSource`]: maestro_core::ports::BlockSource

mod client;

pub use client::{SubstrateClient, SubstrateClientConfig};
