//! Process-level bootstrap: CLI/env → tracing → passive event consumers.
//!
//! Everything that happens before we touch external state (DB, Substrate)
//! lives here. Owning the handles to passive consumers + the TUI here keeps
//! their lifetime tied to the process, not to the GraphQL/indexer tasks.

use std::sync::Arc;

use metrics_exporter_prometheus::PrometheusBuilder;
use tokio::signal;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::warn;
use tracing_subscriber::{EnvFilter, fmt, prelude::*};

use maestro_core::events::EventBus;
use maestro_core::metrics::init_metrics;

use crate::cli::Cli;
use crate::tui::{self, LogBuffer, TuiLogLayer};

/// Process-wide context produced by [`Bootstrap::init`]. Owns the handles to
/// background consumers so teardown can await them together.
pub struct Bootstrap {
    pub cli: Arc<Cli>,
    pub metrics_enabled: bool,
    pub event_bus: EventBus,
    pub shutdown_tx: watch::Sender<bool>,
    pub shutdown_rx: watch::Receiver<bool>,
    pub logger_handle: JoinHandle<()>,
    pub metrics_bridge_handle: JoinHandle<()>,
    pub tui_handle: Option<JoinHandle<()>>,
}

impl Bootstrap {
    /// Spin up tracing, the shutdown channel, metrics, the event bus and its
    /// passive consumers, plus the optional TUI. Not async: we're inside
    /// `#[tokio::main]` already.
    pub fn init(cli: Cli) -> Self {
        let tui_active =
            cli.tui && !cli.json_logs && !cli.migrate_only && !cli.purge && !cli.export_schema;

        let log_buffer = init_tracing(&cli.log_level, cli.json_logs, tui_active);
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        let metrics_enabled = install_metrics_exporter(cli.metrics_port);

        // Event bus with two always-on passive consumers (logger, metrics).
        let event_bus = EventBus::new(maestro_core::events::DEFAULT_BUS_CAPACITY);
        let logger_handle =
            maestro_core::events::logger::spawn(event_bus.clone(), shutdown_tx.subscribe());
        let metrics_bridge_handle =
            maestro_core::events::metrics_bridge::spawn(event_bus.clone(), shutdown_tx.subscribe());

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

        Self {
            cli: Arc::new(cli),
            metrics_enabled,
            event_bus,
            shutdown_tx,
            shutdown_rx,
            logger_handle,
            metrics_bridge_handle,
            tui_handle,
        }
    }
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

/// Install the Prometheus exporter on `0.0.0.0:{port}`. Failures are logged
/// and swallowed so metrics stay optional — returns whether it bound.
fn install_metrics_exporter(port: u16) -> bool {
    let addr = match format!("0.0.0.0:{port}").parse::<std::net::SocketAddr>() {
        Ok(a) => a,
        Err(e) => {
            warn!("⚠️  Invalid metrics address: {e}. Continuing without metrics.");
            return false;
        }
    };

    match PrometheusBuilder::new().with_http_listener(addr).install() {
        Ok(()) => {
            init_metrics();
            true
        }
        Err(e) => {
            warn!("⚠️  Failed to start metrics exporter: {e}. Continuing without metrics.");
            false
        }
    }
}

/// Mask password in database URL for logging.
pub fn mask_password(url_str: &str) -> String {
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

/// Await until `rx` observes `true`. Used to cooperate with the TUI, which
/// forwards its quit-key through the shared shutdown channel.
pub async fn wait_for_shutdown(rx: &mut watch::Receiver<bool>) {
    if *rx.borrow() {
        return;
    }
    while rx.changed().await.is_ok() {
        if *rx.borrow() {
            return;
        }
    }
}

/// Wait for an OS shutdown signal (Ctrl+C or SIGTERM).
pub async fn shutdown_signal() {
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
