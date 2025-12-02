//! Core domain layer for Maestro indexer.
//!
//! This crate contains the domain models, port traits (interfaces), and
//! business logic services for the Substrate blockchain indexer. It follows
//! hexagonal architecture principles - this is the innermost layer with
//! no dependencies on infrastructure.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                     maestro (binary)                        │
//! ├─────────────────────────────────────────────────────────────┤
//! │  maestro-graphql  │  maestro-handlers  │  maestro-substrate │
//! │     (API)         │    (bundles)       │     (RPC)          │
//! ├───────────────────┴────────────────────┴────────────────────┤
//! │                    maestro-storage                          │
//! │                     (PostgreSQL)                            │
//! ├─────────────────────────────────────────────────────────────┤
//! │                     maestro-core  ← YOU ARE HERE            │
//! │               (models, ports, services)                     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Modules
//!
//! - [`models`] - Domain models (Block, Event, Extrinsic, etc.)
//! - [`ports`] - Interface traits for adapters to implement
//! - [`services`] - Core business logic (IndexerService)
//! - [`error`] - Domain error types
//! - [`metrics`] - Prometheus metrics definitions
//!
//! # Key Concepts
//!
//! ## Ports
//!
//! Ports define interfaces that external adapters must implement:
//!
//! - [`ports::BlockSource`] - Fetch blocks from a Substrate chain
//! - [`ports::Repositories`] - Persist and query indexed data
//! - [`ports::PalletHandler`] - Process pallet-specific events
//!
//! ## Handler System
//!
//! The indexer uses a handler-based extensibility model. Each pallet
//! that needs custom indexing logic implements [`ports::PalletHandler`].
//! Handlers are registered in a [`ports::HandlerRegistry`] and called
//! for matching events during block processing.
//!
//! ## Indexer Lifecycle
//!
//! 1. Subscribe to finalized blocks from the chain
//! 2. Transform raw blocks into domain models
//! 3. Call registered handlers for each event
//! 4. Persist block data and handler outputs atomically
//! 5. Update cursor for progress tracking

pub mod error;
pub mod metrics;
pub mod models;
pub mod ports;
pub mod services;
