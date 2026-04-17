//! Spawn the GraphQL + indexer tasks and coordinate a graceful shutdown.
//!
//! Timeouts mirror historical behaviour (30s indexer, 10s graphql, 5s
//! passive consumers, 2s TUI). All handles are awaited so their exit is
//! logged even if they already finished before the shutdown broadcast.

use std::time::Duration;

use anyhow::Result;
use tokio::sync::watch;
use tracing::{Instrument, debug, error, info, info_span, warn};

use maestro_core::error::IndexerError;
use maestro_graphql::{ServerConfig, serve_with_shutdown};

use crate::bootstrap::{Bootstrap, shutdown_signal, wait_for_shutdown};
use crate::setup::Services;

/// Drive the long-running services until shutdown is requested, then tear
/// them down with bounded timeouts.
pub async fn run_services(bootstrap: Bootstrap, services: Services) -> Result<()> {
    let Bootstrap {
        shutdown_tx,
        shutdown_rx,
        logger_handle,
        metrics_bridge_handle,
        tui_handle,
        metrics_enabled,
        cli,
        ..
    } = bootstrap;

    let Services {
        indexer,
        schema,
        graphql_port,
        db,
        graphql_db,
    } = services;

    // ─────────────────────────────────────────────────────────────────────
    // ⚡ SERVICES START
    // ─────────────────────────────────────────────────────────────────────
    let graphql_config = ServerConfig {
        host: "0.0.0.0".to_string(),
        port: graphql_port,
        enable_playground: true,
    };

    let graphql_handle = spawn_graphql(schema, graphql_config, shutdown_tx.subscribe());
    let indexer_handle = spawn_indexer(indexer, shutdown_rx, shutdown_tx.clone());

    // ─────────────────────────────────────────────────────────────────────
    // ✅ READY
    // ─────────────────────────────────────────────────────────────────────
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

    // The TUI captures Ctrl+C itself (raw mode swallows SIGINT) and signals
    // quit via `shutdown_tx`. Race the OS signal against the watch so main
    // unparks either way.
    let mut main_shutdown_rx = shutdown_tx.subscribe();
    tokio::select! {
        _ = shutdown_signal() => {}
        _ = wait_for_shutdown(&mut main_shutdown_rx) => {}
    }

    // ─────────────────────────────────────────────────────────────────────
    // 🛑 SHUTDOWN
    // ─────────────────────────────────────────────────────────────────────
    info!("🛑 Shutting down...");
    let _ = shutdown_tx.send(true);

    await_with_timeout("Indexer", indexer_handle, Duration::from_secs(30)).await;
    await_with_timeout("GraphQL", graphql_handle, Duration::from_secs(10)).await;
    await_with_timeout("Event logger", logger_handle, Duration::from_secs(5)).await;
    await_with_timeout(
        "Metrics bridge",
        metrics_bridge_handle,
        Duration::from_secs(5),
    )
    .await;
    if let Some(handle) = tui_handle {
        await_with_timeout("TUI", handle, Duration::from_secs(2)).await;
    }

    db.close().await;
    graphql_db.close().await;

    info!("🛑 Shutdown complete");
    Ok(())
}

fn spawn_graphql(
    schema: crate::setup::MaestroSchema,
    config: ServerConfig,
    mut shutdown_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(
        async move {
            let shutdown_signal = async move {
                while !*shutdown_rx.borrow() {
                    if shutdown_rx.changed().await.is_err() {
                        break;
                    }
                }
            };

            if let Err(e) = serve_with_shutdown(schema, config, shutdown_signal).await {
                error!(error = %e, "❌ Server error");
            }
            debug!("Server stopped");
        }
        .instrument(info_span!("graphql")),
    )
}

fn spawn_indexer(
    indexer: crate::setup::PgSubstrateIndexer,
    shutdown_rx: watch::Receiver<bool>,
    shutdown_tx: watch::Sender<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(
        async move {
            if let Err(e) = indexer.run(shutdown_rx).await {
                match &e {
                    IndexerError::ShutdownRequested => {}
                    IndexerError::ChainMismatch { .. } => {
                        // Fatal — trigger shutdown so the GraphQL task exits too.
                        let _ = shutdown_tx.send(true);
                    }
                    _ => error!(error = ?e, "❌ Indexer error"),
                }
            }
        }
        .instrument(info_span!("indexer")),
    )
}

async fn await_with_timeout<T>(label: &str, handle: tokio::task::JoinHandle<T>, timeout: Duration) {
    match tokio::time::timeout(timeout, handle).await {
        Ok(_) => debug!("{label} stopped"),
        Err(_) => warn!("⚠️  {label} shutdown timed out"),
    }
}
