//! Metrics definitions for the indexer.
//!
//! This module defines all metrics used throughout the indexer.
//! Metrics are collected using the `metrics` crate and can be exported
//! to Prometheus via `metrics-exporter-prometheus`.

use metrics::{counter, describe_counter, describe_histogram, histogram};
use std::time::Instant;

/// Initialize all metric descriptions.
/// Call this once at startup before any metrics are recorded.
pub fn init_metrics() {
    describe_counter!(
        "decode_errors_total",
        "Total number of decode errors during block processing"
    );
    describe_counter!(
        "blocks_indexed_total",
        "Total number of blocks successfully indexed"
    );
    describe_histogram!(
        "block_processing_duration_seconds",
        "Time taken to process a block in seconds"
    );
    describe_counter!(
        "handler_errors_total",
        "Total number of handler errors during event/extrinsic processing"
    );
    describe_counter!(
        "reorgs_detected_total",
        "Total number of chain reorganizations detected"
    );
    describe_counter!(
        "blocks_deleted_total",
        "Total number of blocks deleted due to reorg"
    );
}

/// Record a decode error.
///
/// # Arguments
/// * `error_type` - The type of error ("event" or "extrinsic")
/// * `pallet` - The pallet name (if known)
pub fn record_decode_error(error_type: &str, pallet: &str) {
    counter!("decode_errors_total", "type" => error_type.to_string(), "pallet" => pallet.to_string())
        .increment(1);
}

/// Record a successfully indexed block.
pub fn record_block_indexed() {
    counter!("blocks_indexed_total").increment(1);
}

/// Record block processing duration.
pub fn record_block_processing_duration(duration_secs: f64) {
    histogram!("block_processing_duration_seconds").record(duration_secs);
}

/// Record a handler error.
///
/// # Arguments
/// * `handler_type` - The type of handler ("event" or "extrinsic")
/// * `pallet` - The pallet name
pub fn record_handler_error(handler_type: &str, pallet: &str) {
    counter!("handler_errors_total", "type" => handler_type.to_string(), "pallet" => pallet.to_string())
        .increment(1);
}

/// Record a chain reorganization detection.
///
/// # Arguments
/// * `at_block` - The block number where the reorg was detected
pub fn record_reorg_detected(at_block: u64) {
    counter!("reorgs_detected_total", "at_block" => at_block.to_string()).increment(1);
}

/// Record the number of blocks deleted due to reorg.
///
/// # Arguments
/// * `count` - Number of blocks deleted
pub fn record_blocks_deleted(count: u64) {
    counter!("blocks_deleted_total").increment(count);
}

/// A timer that automatically records duration when dropped.
pub struct ProcessingTimer {
    start: Instant,
}

impl ProcessingTimer {
    /// Start a new processing timer.
    pub fn new() -> Self {
        Self {
            start: Instant::now(),
        }
    }
}

impl Default for ProcessingTimer {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for ProcessingTimer {
    fn drop(&mut self) {
        let duration = self.start.elapsed().as_secs_f64();
        record_block_processing_duration(duration);
    }
}
