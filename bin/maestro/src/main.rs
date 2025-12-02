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
use tracing_subscriber::{EnvFilter, fmt};

use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Schema};
use maestro_core::error::IndexerError;
use maestro_core::metrics::init_metrics;
use maestro_core::ports::{BlockMode, BlockSource};
use maestro_core::services::{IndexerConfig, IndexerService};
use maestro_graphql::{CoreQuery, ServerConfig, serve_with_shutdown};
use maestro_handlers::balances::{BalancesQuery, BalancesStorage, PgBalancesStorage};
use maestro_handlers::{BalancesBundle, BundleRegistry};
use maestro_storage::{Database, DatabaseConfig, PgRepositories};
use maestro_substrate::{SubstrateClient, SubstrateClientConfig};

/// Maestro CLI - Allfeat Blockchain Indexer.
#[derive(Parser, Debug)]
#[command(name = "maestro")]
#[command(about = "Maestro - Substrate blockchain indexer by Allfeat")]
#[command(version)]
struct Cli {
    /// Substrate node WebSocket URL.
    #[arg(long, env = "WS_URL", default_value = "ws://127.0.0.1:9944")]
    ws_url: String,

    /// PostgreSQL database URL.
    #[arg(
        long,
        env = "DATABASE_URL",
        default_value = "postgres://localhost/maestro"
    )]
    database_url: String,

    /// GraphQL server port.
    #[arg(long, env = "GRAPHQL_PORT", default_value = "4000")]
    graphql_port: u16,

    /// Prometheus metrics port.
    #[arg(long, env = "METRICS_PORT", default_value = "9090")]
    metrics_port: u16,

    /// Enable JSON log output.
    #[arg(long, env = "JSON_LOGS")]
    json_logs: bool,

    /// Run database migrations and exit.
    #[arg(long)]
    migrate_only: bool,

    /// Purge all indexed data from the database and exit.
    ///
    /// This will delete all blocks, extrinsics, events, transfers, and reset
    /// the indexer cursor. Schema/migrations are preserved.
    #[arg(long)]
    purge: bool,

    /// Skip confirmation prompt for destructive operations (like --purge).
    #[arg(long, short = 'y')]
    yes: bool,

    /// Log level (trace, debug, info, warn, error).
    #[arg(long, env = "LOG_LEVEL", default_value = "info")]
    log_level: String,

    /// Block subscription mode: finalized (safe) or best (fast but may reorg).
    #[arg(long, env = "BLOCK_MODE", default_value = "finalized", value_parser = parse_block_mode)]
    block_mode: BlockMode,
}

/// Parse block mode from string.
fn parse_block_mode(s: &str) -> Result<BlockMode, String> {
    match s.to_lowercase().as_str() {
        "finalized" => Ok(BlockMode::Finalized),
        "best" => Ok(BlockMode::Best),
        _ => Err(format!(
            "Invalid block mode '{}'. Use 'finalized' or 'best'.",
            s
        )),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    let cli = Cli::parse();
    init_tracing(&cli.log_level, cli.json_logs);

    // Prometheus metrics exporter (optional - failures don't crash the app)
    let metrics_enabled = match format!("0.0.0.0:{}", cli.metrics_port).parse::<std::net::SocketAddr>() {
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
                    warn!("⚠️  Failed to start metrics exporter: {}. Continuing without metrics.", e);
                    false
                }
            }
        }
        Err(e) => {
            warn!("⚠️  Invalid metrics address: {}. Continuing without metrics.", e);
            false
        }
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
    // 📦 HANDLER BUNDLES (register early for migrations and purge)
    // ─────────────────────────────────────────────────────────────────────────
    let mut bundle_registry = BundleRegistry::new();
    bundle_registry.register(Box::new(BalancesBundle::new(db.pool().clone())));

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

    // ─────────────────────────────────────────────────────────────────────────
    // 📡 SUBSTRATE CONNECTION
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

    // Convert to handler registry for the indexer
    let handlers = Arc::new(bundle_registry.into_handler_registry());

    let indexer_config = IndexerConfig {
        chain_id: hex::encode(genesis_hash.0),
        block_mode: cli.block_mode,
        ..Default::default()
    };

    let indexer = IndexerService::new(
        indexer_config,
        substrate_client.clone(),
        indexer_repositories.clone(),
        handlers,
    );

    // ─────────────────────────────────────────────────────────────────────────
    // ⚡ SERVICES START
    // ─────────────────────────────────────────────────────────────────────────
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut graphql_shutdown_rx = shutdown_tx.subscribe();

    let graphql_config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: cli.graphql_port,
        enable_playground: true,
    };

    // Create GraphQL balances storage (for read queries)
    let graphql_balances_storage: Arc<dyn BalancesStorage> =
        Arc::new(PgBalancesStorage::new(graphql_db.pool().clone()));

    // Compose the GraphQL schema from core + bundle queries
    // Includes DoS protection: depth limit (10), complexity limit (500)
    #[derive(MergedObject, Default)]
    struct Query(CoreQuery, BalancesQuery);

    let repos: Arc<dyn maestro_core::ports::Repositories> = graphql_repositories;
    let schema = Schema::build(Query::default(), EmptyMutation, EmptySubscription)
        .data(repos)
        .data(graphql_balances_storage)
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

    shutdown_signal().await;

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

    db.close().await;
    graphql_db.close().await;

    info!("🛑 Shutdown complete");
    Ok(())
}

/// Initialize tracing subscriber.
fn init_tracing(level: &str, json: bool) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

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
