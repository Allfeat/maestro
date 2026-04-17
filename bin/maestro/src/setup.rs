//! Connect to DB + Substrate, register handler bundles, build the GraphQL
//! schema, and wire up the `IndexerService`. Returns `None` when a CLI flag
//! asks for an early exit (`--migrate-only`, `--purge`).

use std::sync::Arc;

use anyhow::{Context, Result};
use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Schema};
use tracing::{debug, info};

use maestro_core::ports::{BlockSource, StorageReader};
use maestro_core::services::IndexerService;
use maestro_graphql::{CoreQuery, MAX_QUERY_COMPLEXITY, MAX_QUERY_DEPTH};
use maestro_handlers::ats::{AtsQuery, AtsStorage, PgAtsStorage};
use maestro_handlers::balances::{BalancesQuery, BalancesStorage, PgBalancesStorage};
use maestro_handlers::midds::{MiddsQuery, MiddsStorage, PgMiddsStorage};
use maestro_handlers::{AtsBundle, BalancesBundle, BundleRegistry, MiddsBundle};
use maestro_storage::{Database, DatabaseConfig, PgRepositories};
use maestro_substrate::{SubstrateClient, SubstrateClientConfig};

use crate::bootstrap::{Bootstrap, mask_password};
use crate::purge::handle_purge;

#[derive(MergedObject, Default)]
pub struct MaestroQuery(CoreQuery, BalancesQuery, AtsQuery, MiddsQuery);

pub type MaestroSchema = Schema<MaestroQuery, EmptyMutation, EmptySubscription>;

/// Concrete indexer specialized on the Postgres + Substrate adapters.
pub type PgSubstrateIndexer = IndexerService<SubstrateClient, PgRepositories>;

/// Everything `run_services` needs to start the GraphQL + indexer tasks.
pub struct Services {
    pub indexer: PgSubstrateIndexer,
    pub schema: MaestroSchema,
    pub graphql_port: u16,
    pub db: Arc<Database>,
    pub graphql_db: Arc<Database>,
}

/// Prepare all long-running services. `Ok(None)` means the CLI flag handled
/// its work synchronously (migrations applied, database purged) and main can
/// return cleanly.
pub async fn setup_services(bootstrap: &Bootstrap) -> Result<Option<Services>> {
    let cli = bootstrap.cli.clone();

    info!("🚀 Starting Maestro Indexer");
    debug!(ws_url = %cli.ws_url, "Substrate endpoint");
    debug!(database_url = %mask_password(&cli.database_url), "Database endpoint");

    // ─────────────────────────────────────────────────────────────────────
    // 🗄️ DATABASE
    // ─────────────────────────────────────────────────────────────────────
    let indexer_db_config = DatabaseConfig::for_indexer(&cli.database_url);
    let graphql_db_config = DatabaseConfig::for_graphql(&cli.database_url);

    info!("🗄️  Connecting to database...");
    let db = Database::connect(&indexer_db_config)
        .await
        .context("Failed to connect to database")?;

    db.migrate().await.context("Failed to run migrations")?;
    info!("🗄️  Database ready (migrations applied)");

    // ─────────────────────────────────────────────────────────────────────
    // 📡 SUBSTRATE CONNECTION (needed before bundle registration for MIDDS)
    // ─────────────────────────────────────────────────────────────────────
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

    // ─────────────────────────────────────────────────────────────────────
    // 📦 HANDLER BUNDLES
    // ─────────────────────────────────────────────────────────────────────
    let storage_reader: Arc<dyn StorageReader> = substrate_client.clone();
    let mut bundle_registry = BundleRegistry::new();
    bundle_registry.register(Box::new(BalancesBundle::new(
        db.pool().clone(),
        bootstrap.event_bus.clone(),
    )));
    bundle_registry.register(Box::new(AtsBundle::new(db.pool().clone())));
    bundle_registry.register(Box::new(MiddsBundle::new(
        db.pool().clone(),
        storage_reader,
    )));

    bundle_registry
        .run_migrations(db.pool())
        .await
        .context("Failed to run bundle migrations")?;

    // Early-exit CLI flags that don't need GraphQL or the indexer.
    if cli.migrate_only {
        info!("🛑 --migrate-only flag set, exiting");
        return Ok(None);
    }
    if cli.purge {
        handle_purge(&db, &bundle_registry, cli.yes).await?;
        return Ok(None);
    }

    // ─────────────────────────────────────────────────────────────────────
    // 🧩 REPOSITORIES + INDEXER
    // ─────────────────────────────────────────────────────────────────────
    let graphql_db = Database::connect(&graphql_db_config)
        .await
        .context("Failed to create GraphQL database pool")?;

    let db = Arc::new(db);
    let graphql_db = Arc::new(graphql_db);

    let indexer_repositories = Arc::new(PgRepositories::new(db.clone()));
    let graphql_repositories = Arc::new(PgRepositories::new(graphql_db.clone()));

    let handlers = Arc::new(bundle_registry.into_handler_registry());
    let indexer_config = cli.to_indexer_config(hex::encode(genesis_hash.0));
    let indexer = IndexerService::new(
        indexer_config,
        substrate_client.clone(),
        indexer_repositories.clone(),
        handlers,
        bootstrap.event_bus.clone(),
    );

    // ─────────────────────────────────────────────────────────────────────
    // 🌐 GRAPHQL SCHEMA (shared across requests)
    // ─────────────────────────────────────────────────────────────────────
    let graphql_balances_storage: Arc<dyn BalancesStorage> =
        Arc::new(PgBalancesStorage::new(graphql_db.pool().clone()));
    let graphql_ats_storage: Arc<dyn AtsStorage> =
        Arc::new(PgAtsStorage::new(graphql_db.pool().clone()));
    let graphql_midds_storage: Arc<dyn MiddsStorage> =
        Arc::new(PgMiddsStorage::new(graphql_db.pool().clone()));
    let repos: Arc<dyn maestro_core::ports::Repositories> = graphql_repositories;

    let schema = Schema::build(MaestroQuery::default(), EmptyMutation, EmptySubscription)
        .data(repos)
        .data(graphql_balances_storage)
        .data(graphql_ats_storage)
        .data(graphql_midds_storage)
        .limit_depth(MAX_QUERY_DEPTH)
        .limit_complexity(MAX_QUERY_COMPLEXITY)
        .finish();

    Ok(Some(Services {
        indexer,
        schema,
        graphql_port: cli.graphql_port,
        db,
        graphql_db,
    }))
}

/// Build and print the merged GraphQL SDL — used by `--export-schema`. Does
/// not need DB or Substrate access because the SDL only depends on types.
pub fn print_schema_sdl() {
    let schema = Schema::build(MaestroQuery::default(), EmptyMutation, EmptySubscription)
        .limit_depth(MAX_QUERY_DEPTH)
        .limit_complexity(MAX_QUERY_COMPLEXITY)
        .finish();
    println!("{}", schema.sdl());
}
