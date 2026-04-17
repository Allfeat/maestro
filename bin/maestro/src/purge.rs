//! `--purge` command: delete all indexed data after a confirmation prompt.
//!
//! Core tables are cleared by [`Database::purge`], bundle-specific tables
//! via [`BundleRegistry::purge_tables`]. Runs bundle purge first in case a
//! bundle table has FKs into core data.

use std::io::{self, Write};

use anyhow::{Context, Result};
use tracing::{info, warn};

use maestro_handlers::BundleRegistry;
use maestro_storage::Database;

/// Handle the `--purge` CLI flag. `skip_confirmation` corresponds to `--yes`.
pub async fn handle_purge(
    db: &Database,
    bundle_registry: &BundleRegistry,
    skip_confirmation: bool,
) -> Result<()> {
    let bundle_tables = bundle_registry.tables_to_purge();

    warn!("⚠️  PURGE MODE: This will delete ALL indexed data!");
    warn!("   - All blocks, extrinsics, events");
    if !bundle_tables.is_empty() {
        warn!("   - Bundle tables: {}", bundle_tables.join(", "));
    }
    warn!("   - The indexer cursor will be reset");
    warn!("   - Schema and migrations will be preserved");

    if !skip_confirmation {
        print!("\n🔴 Are you sure you want to purge all data? [y/N] ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        if !input.trim().eq_ignore_ascii_case("y") {
            info!("❌ Purge cancelled");
            return Ok(());
        }
    }

    info!("🗑️  Purging database...");

    let bundle_tables_purged = bundle_registry
        .purge_tables(db.pool())
        .await
        .context("Failed to purge bundle tables")?;

    if bundle_tables_purged > 0 {
        info!("   🧹 Purged {} bundle table(s)", bundle_tables_purged);
    }

    let stats = db.purge().await.context("Failed to purge database")?;

    info!("✅ Database purged successfully");
    info!("   📦 Blocks removed: {}", stats.blocks_removed);
    info!("   📝 Extrinsics removed: {}", stats.extrinsics_removed);
    info!("   📣 Events removed: {}", stats.events_removed);
    info!("   The indexer will start from block 0 on next run");

    Ok(())
}
