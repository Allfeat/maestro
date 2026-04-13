//! CLI parsing for the maestro binary.
//!
//! Extracted from `main.rs` to keep the entry point readable as new flags
//! accrete. `TryFrom<Cli> for IndexerConfig` performs the pure-mechanical
//! shape conversion with no side effects.

use clap::Parser;
use maestro_core::ports::BlockMode;
use maestro_core::services::{BackfillConfig, IndexerConfig};

/// Maestro CLI - Allfeat Blockchain Indexer.
#[derive(Parser, Debug)]
#[command(name = "maestro")]
#[command(about = "Maestro - Substrate blockchain indexer by Allfeat")]
#[command(version)]
pub struct Cli {
    /// Substrate node WebSocket URL.
    #[arg(long, env = "WS_URL", default_value = "ws://127.0.0.1:9944")]
    pub ws_url: String,

    /// PostgreSQL database URL.
    #[arg(long, env = "DATABASE_URL", default_value = "postgres://localhost/maestro")]
    pub database_url: String,

    /// GraphQL server port.
    #[arg(long, env = "GRAPHQL_PORT", default_value = "4000")]
    pub graphql_port: u16,

    /// Prometheus metrics port.
    #[arg(long, env = "METRICS_PORT", default_value = "9090")]
    pub metrics_port: u16,

    /// Enable JSON log output.
    #[arg(long, env = "JSON_LOGS", default_value = "false", value_parser = parse_bool)]
    pub json_logs: bool,

    /// Run database migrations and exit.
    #[arg(long, env = "MIGRATE_ONLY", default_value = "false", value_parser = parse_bool)]
    pub migrate_only: bool,

    /// Purge all indexed data from the database and exit.
    #[arg(long, env = "PURGE", default_value = "false", value_parser = parse_bool)]
    pub purge: bool,

    /// Skip confirmation prompts for destructive operations.
    #[arg(long, short = 'y', env = "YES", default_value = "false", value_parser = parse_bool)]
    pub yes: bool,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    pub log_level: String,

    /// Block subscription mode: finalized (safe) or best (fast but may reorg).
    #[arg(long, env = "BLOCK_MODE", default_value = "finalized", value_parser = parse_block_mode)]
    pub block_mode: BlockMode,

    /// Export GraphQL schema (SDL) and exit.
    #[arg(long, env = "EXPORT_SCHEMA", default_value = "false", value_parser = parse_bool)]
    pub export_schema: bool,

    /// Lowest block number to index. Defaults to 0 (genesis).
    /// Cannot be below the chain's earliest V14 block.
    #[arg(long, env = "START_BLOCK", default_value = "0")]
    pub start_block: u64,

    /// Skip backfill entirely and start indexing at the current chain head.
    /// Any existing cursor gap will NOT be filled — use with care.
    #[arg(long, env = "LIVE_ONLY", default_value = "false", value_parser = parse_bool)]
    pub live_only: bool,

    /// Maximum parallel block fetches during backfill.
    #[arg(long, env = "BACKFILL_CONCURRENCY", default_value = "16")]
    pub backfill_concurrency: usize,

    /// Maximum retries per block fetch during backfill before aborting.
    #[arg(long, env = "BACKFILL_MAX_RETRIES", default_value = "5")]
    pub backfill_max_retries: u32,
}

pub fn parse_block_mode(s: &str) -> Result<BlockMode, String> {
    match s.to_lowercase().as_str() {
        "finalized" => Ok(BlockMode::Finalized),
        "best" => Ok(BlockMode::Best),
        _ => Err(format!("Invalid block mode '{}'. Use 'finalized' or 'best'.", s)),
    }
}

pub fn parse_bool(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" => Ok(false),
        _ => Err(format!(
            "Invalid boolean '{}'. Use 'true', 'false', '1', '0', 'yes', 'no'.",
            s
        )),
    }
}

/// Build an `IndexerConfig` from parsed CLI flags. Pure data shape conversion.
impl Cli {
    pub fn to_indexer_config(&self, chain_id: String) -> IndexerConfig {
        IndexerConfig {
            chain_id,
            block_mode: self.block_mode,
            backfill: BackfillConfig {
                start_block: self.start_block,
                live_only: self.live_only,
                concurrency: self.backfill_concurrency,
                max_fetch_retries: self.backfill_max_retries,
            },
            ..Default::default()
        }
    }
}
