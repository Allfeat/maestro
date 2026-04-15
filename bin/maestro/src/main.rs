//! Maestro - Substrate blockchain indexer.
//!
//! # Usage
//!
//! ```bash
//! # Start with default config
//! maestro
//!
//! # Start with environment overrides
//! DATABASE_URL=postgres://localhost/maestro WS_URL=ws://localhost:9944 maestro
//! ```

use std::io::{self, Write};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::signal;
use tokio::sync::watch;
use tracing::{Instrument, debug, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Schema};
use maestro_core::error::IndexerError;
use maestro_core::metrics::init_metrics;
use maestro_core::ports::{BlockSource, StorageReader};
use maestro_core::services::IndexerService;
use maestro_graphql::{CoreQuery, ServerConfig, serve_with_shutdown};
use maestro_handlers::ats::{AtsQuery, AtsStorage, PgAtsStorage};
use maestro_handlers::balances::{BalancesQuery, BalancesStorage, PgBalancesStorage};
use maestro_handlers::midds::{MiddsQuery, MiddsStorage, PgMiddsStorage};
use maestro_handlers::{AtsBundle, BalancesBundle, BundleRegistry, MiddsBundle};
use maestro_storage::{Database, DatabaseConfig, PgRepositories};
use maestro_substrate::{SubstrateClient, SubstrateClientConfig};

mod cli;
mod tui;

use cli::Cli;
use tui::{LogBuffer, TuiLogLayer};

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();

    // TUI is only compatible with long-running indexing mode. Disable it for
    // one-shot commands and when the user asked for JSON logs (pipe friendly).
    let tui_active =
        cli.tui && !cli.json_logs && !cli.migrate_only && !cli.purge && !cli.export_schema;

    let log_buffer = init_tracing(&cli.log_level, cli.json_logs, tui_active);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    // ─────────────────────────────────────────────────────────────────────────
    // 📄 SCHEMA EXPORT (early exit, no DB needed)
    // ─────────────────────────────────────────────────────────────────────────
    if cli.export_schema {
        #[derive(MergedObject, Default)]
        struct Query(CoreQuery, BalancesQuery, AtsQuery);

        let schema = Schema::build(Query::default(), EmptyMutation, EmptySubscription)
            .limit_depth(maestro_graphql::MAX_QUERY_DEPTH)
            .limit_complexity(maestro_graphql::MAX_QUERY_COMPLEXITY)
            .finish();

        println!("{}", schema.sdl());
        return Ok(());
    }

    // Prometheus metrics exporter (optional - failures don't crash the app)
    let metrics_enabled =
        match format!("0.0.0.0:{}", cli.metrics_port).parse::<std::net::SocketAddr>() {
            Ok(metrics_addr) => {
                match PrometheusBuilder::new()
                    .with_http_listener(metrics_addr)
                    .install()
                {
                    Ok(()) => {
                        init_metrics();
                        true
                    }
                    Err(e) => {
                        warn!(
                            "⚠️  Failed to start metrics exporter: {}. Continuing without metrics.",
                            e
                        );
                        false
                    }
                }
            }
            Err(e) => {
                warn!(
                    "⚠️  Invalid metrics address: {}. Continuing without metrics.",
                    e
                );
                false
            }
        };

    // Event bus — in-process, shared by future event producers and consumers.
    // Phase 0 wires the bus and two passive consumers (logger, metrics_bridge).
    // No service emits events yet — that starts in Phase 1.
    let event_bus = maestro_core::events::EventBus::new(maestro_core::events::DEFAULT_BUS_CAPACITY);
    let logger_handle =
        maestro_core::events::logger::spawn(event_bus.clone(), shutdown_tx.subscribe());
    let metrics_bridge_handle =
        maestro_core::events::metrics_bridge::spawn(event_bus.clone(), shutdown_tx.subscribe());

    // TUI — 4th event consumer, only active when explicitly requested.
    let tui_handle = if tui_active {
        let bus = event_bus.clone();
        let tx = shutdown_tx.clone();
        let rx = shutdown_tx.subscribe();
        let logs = log_buffer.clone();
        Some(tokio::spawn(async move {
            if let Err(e) = tui::run(bus, tx, rx, logs).await {
                eprintln!("tui error: {e:#}");
            }
        }))
    } else {
        None
    };

    // ─────────────────────────────────────────────────────────────────────────
    // 🚀 STARTUP
    // ─────────────────────────────────────────────────────────────────────────
    info!("🚀 Starting Maestro Indexer");
    debug!(ws_url = %cli.ws_url, "Substrate endpoint");
    debug!(database_url = %mask_password(&cli.database_url), "Database endpoint");

    // ─────────────────────────────────────────────────────────────────────────
    // 🗄️ DATABASE
    // ─────────────────────────────────────────────────────────────────────────
    let indexer_db_config = DatabaseConfig::for_indexer(&cli.database_url);
    let graphql_db_config = DatabaseConfig::for_graphql(&cli.database_url);

    info!("🗄️  Connecting to database...");
    let db = Database::connect(&indexer_db_config)
        .await
        .context("Failed to connect to database")?;

    db.migrate().await.context("Failed to run migrations")?;
    info!("🗄️  Database ready (migrations applied)");

    // ─────────────────────────────────────────────────────────────────────────
    // 📡 SUBSTRATE CONNECTION (needed before bundle registration for MIDDS)
    // ─────────────────────────────────────────────────────────────────────────
    info!("📡 Connecting to Substrate node...");
    let substrate_config = SubstrateClientConfig {
        ws_url: cli.ws_url.clone(),
    };

    let substrate_client = SubstrateClient::connect(substrate_config)
        .await
        .context("Failed to connect to Substrate node")?;

    let substrate_client = Arc::new(substrate_client);

    let genesis_hash = substrate_client.genesis_hash().await?;
    let runtime_version = substrate_client.runtime_version().await?;
    let finalized = substrate_client.finalized_head().await?;

    info!(
        genesis = %hex::encode(&genesis_hash.0[..8]),
        runtime = runtime_version,
        head = finalized.number,
        "🔗 Chain connected"
    );

    // ─────────────────────────────────────────────────────────────────────────
    // 📦 HANDLER BUNDLES
    // ─────────────────────────────────────────────────────────────────────────
    let storage_reader: Arc<dyn StorageReader> = substrate_client.clone();
    let mut bundle_registry = BundleRegistry::new();
    bundle_registry.register(Box::new(BalancesBundle::new(
        db.pool().clone(),
        event_bus.clone(),
    )));
    bundle_registry.register(Box::new(AtsBundle::new(db.pool().clone())));
    bundle_registry.register(Box::new(MiddsBundle::new(
        db.pool().clone(),
        storage_reader,
    )));

    // Run bundle-specific migrations
    bundle_registry
        .run_migrations(db.pool())
        .await
        .context("Failed to run bundle migrations")?;

    if cli.migrate_only {
        info!("🛑 --migrate-only flag set, exiting");
        return Ok(());
    }

    if cli.purge {
        return handle_purge(&db, &bundle_registry, cli.yes).await;
    }

    let graphql_db = Database::connect(&graphql_db_config)
        .await
        .context("Failed to create GraphQL database pool")?;

    let db = Arc::new(db);
    let graphql_db = Arc::new(graphql_db);

    let indexer_repositories = Arc::new(PgRepositories::new(db.clone()));
    let graphql_repositories = Arc::new(PgRepositories::new(graphql_db.clone()));

    // Convert to handler registry for the indexer
    let handlers = Arc::new(bundle_registry.into_handler_registry());

    let indexer_config = cli.to_indexer_config(hex::encode(genesis_hash.0));

    let indexer = IndexerService::new(
        indexer_config,
        substrate_client.clone(),
        indexer_repositories.clone(),
        handlers,
        event_bus.clone(),
    );

    // ─────────────────────────────────────────────────────────────────────────
    // ⚡ SERVICES START
    // ─────────────────────────────────────────────────────────────────────────
    let mut graphql_shutdown_rx = shutdown_tx.subscribe();

    let graphql_config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: cli.graphql_port,
        enable_playground: true,
    };

    // Create GraphQL storage instances (for read queries)
    let graphql_balances_storage: Arc<dyn BalancesStorage> =
        Arc::new(PgBalancesStorage::new(graphql_db.pool().clone()));
    let graphql_ats_storage: Arc<dyn AtsStorage> =
        Arc::new(PgAtsStorage::new(graphql_db.pool().clone()));
    let graphql_midds_storage: Arc<dyn MiddsStorage> =
        Arc::new(PgMiddsStorage::new(graphql_db.pool().clone()));

    // Compose the GraphQL schema from core + bundle queries
    // Includes DoS protection: depth limit (15), complexity limit (500)
    #[derive(MergedObject, Default)]
    struct Query(CoreQuery, BalancesQuery, AtsQuery, MiddsQuery);

    let repos: Arc<dyn maestro_core::ports::Repositories> = graphql_repositories;
    let schema = Schema::build(Query::default(), EmptyMutation, EmptySubscription)
        .data(repos)
        .data(graphql_balances_storage)
        .data(graphql_ats_storage)
        .data(graphql_midds_storage)
        .limit_depth(maestro_graphql::MAX_QUERY_DEPTH)
        .limit_complexity(maestro_graphql::MAX_QUERY_COMPLEXITY)
        .finish();
    let graphql_port = cli.graphql_port;
    let graphql_handle = tokio::spawn(
        async move {
            let shutdown_signal = async move {
                while !*graphql_shutdown_rx.borrow() {
                    if graphql_shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            };

            if let Err(e) = serve_with_shutdown(schema, graphql_config, shutdown_signal).await {
                error!(error = %e, "❌ Server error");
            }
            debug!("Server stopped");
        }
        .instrument(info_span!("graphql")),
    );

    let indexer_shutdown_tx = shutdown_tx.clone();
    let indexer_handle = tokio::spawn(
        async move {
            if let Err(e) = indexer.run(shutdown_rx).await {
                match &e {
                    IndexerError::ShutdownRequested => {}
                    IndexerError::ChainMismatch { .. } => {
                        // Chain mismatch is fatal - trigger shutdown
                        let _ = indexer_shutdown_tx.send(true);
                    }
                    _ => error!(error = ?e, "❌ Indexer error"),
                }
            }
        }
        .instrument(info_span!("indexer")),
    );

    // ─────────────────────────────────────────────────────────────────────────
    // ✅ READY
    // ─────────────────────────────────────────────────────────────────────────
    info!("✅ Maestro ready");
    info!("   ⚡ GraphQL:  http://localhost:{}/graphql", graphql_port);
    if metrics_enabled {
        info!(
            "   📊 Metrics:  http://localhost:{}/metrics",
            cli.metrics_port
        );
    } else {
        info!("   📊 Metrics:  disabled");
    }
    info!("   Press Ctrl+C to stop");

    // Also react to the shutdown watch: when the TUI task is active it
    // captures Ctrl+C itself (raw mode swallows SIGINT) and signals quit via
    // `shutdown_tx`. Without this branch, main would stay parked on the OS
    // signal handler even though the TUI has already requested shutdown.
    let mut main_shutdown_rx = shutdown_tx.subscribe();
    tokio::select! {
        _ = shutdown_signal() => {}
        _ = wait_for_shutdown(&mut main_shutdown_rx) => {}
    }

    // ─────────────────────────────────────────────────────────────────────────
    // 🛑 SHUTDOWN
    // ─────────────────────────────────────────────────────────────────────────
    info!("🛑 Shutting down...");
    let _ = shutdown_tx.send(true);

    match tokio::time::timeout(std::time::Duration::from_secs(30), indexer_handle).await {
        Ok(_) => debug!("Indexer stopped"),
        Err(_) => warn!("⚠️  Indexer shutdown timed out"),
    }

    match tokio::time::timeout(std::time::Duration::from_secs(10), graphql_handle).await {
        Ok(_) => debug!("GraphQL stopped"),
        Err(_) => warn!("⚠️  GraphQL shutdown timed out"),
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), logger_handle).await {
        Ok(_) => debug!("Event logger stopped"),
        Err(_) => warn!("⚠️  Event logger shutdown timed out"),
    }

    match tokio::time::timeout(std::time::Duration::from_secs(5), metrics_bridge_handle).await {
        Ok(_) => debug!("Metrics bridge stopped"),
        Err(_) => warn!("⚠️  Metrics bridge shutdown timed out"),
    }

    if let Some(handle) = tui_handle {
        match tokio::time::timeout(std::time::Duration::from_secs(2), handle).await {
            Ok(_) => debug!("TUI stopped"),
            Err(_) => warn!("⚠️  TUI shutdown timed out"),
        }
    }

    db.close().await;
    graphql_db.close().await;

    info!("🛑 Shutdown complete");
    Ok(())
}

/// Initialize tracing subscriber.
///
/// In TUI mode, stdout belongs to ratatui so the normal `fmt` layer cannot
/// run. Instead we install a custom [`TuiLogLayer`] that pushes every event
/// into a shared ring buffer, then hand that buffer to the TUI task so its
/// logs panel can render them. Returns the buffer only when the TUI is
/// active — all other modes keep the classic stdout/JSON output.
fn init_tracing(level: &str, json: bool, tui_active: bool) -> Option<LogBuffer> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    if tui_active {
        let buffer = LogBuffer::default();
        tracing_subscriber::registry()
            .with(filter)
            .with(TuiLogLayer::new(buffer.clone()))
            .init();
        return Some(buffer);
    }

    if json {
        fmt().with_env_filter(filter).json().init();
    } else {
        fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_file(false)
            .with_line_number(false)
            .init();
    }
    None
}

/// Mask password in database URL for logging.
fn mask_password(url_str: &str) -> String {
    match url::Url::parse(url_str) {
        Ok(mut url) => {
            if url.password().is_some() {
                let _ = url.set_password(Some("****"));
            }
            url.to_string()
        }
        Err(_) => url_str.to_string(),
    }
}

/// Await until `rx` observes `true`. Used in `main` to cooperate with the
/// TUI, which forwards its quit-key to the shared shutdown channel.
async fn wait_for_shutdown(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
}

/// Wait for shutdown signal (Ctrl+C or SIGTERM).
async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

/// Handle the --purge command.
async fn handle_purge(
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

    // First purge bundle tables explicitly (before core tables, in case of dependencies)
    let bundle_tables_purged = bundle_registry
        .purge_tables(db.pool())
        .await
        .context("Failed to purge bundle tables")?;

    if bundle_tables_purged > 0 {
        info!("   🧹 Purged {} bundle table(s)", bundle_tables_purged);
    }

    // Then purge core tables
    let stats = db.purge().await.context("Failed to purge database")?;

    info!("✅ Database purged successfully");
    info!("   📦 Blocks removed: {}", stats.blocks_removed);
    info!("   📝 Extrinsics removed: {}", stats.extrinsics_removed);
    info!("   📣 Events removed: {}", stats.events_removed);
    info!("   The indexer will start from block 0 on next run");

    Ok(())
}
