//! In-process event bus.
//!
//! Decouples services (indexer, backfill, handlers) from observability
//! consumers (logger, metrics, future TUI). Uses tokio broadcast channels
//! for discrete events and watch channels for latest-value state.

pub mod backfill;
pub mod chain;
pub mod handler;
pub mod indexer;
pub mod logger;
pub mod metrics_bridge;
