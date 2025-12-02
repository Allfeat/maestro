//! PostgreSQL database connection and configuration.

use sqlx::postgres::{PgPool, PgPoolOptions};
use std::time::Duration;
use tracing::{debug, instrument};

use maestro_core::error::{StorageError, StorageResult};

/// Database configuration.
#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    /// PostgreSQL connection URL.
    pub url: String,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of connections to maintain.
    pub min_connections: u32,
    /// Connection acquisition timeout.
    pub acquire_timeout: Duration,
    /// Idle connection timeout.
    pub idle_timeout: Duration,
    /// Maximum connection lifetime.
    pub max_lifetime: Duration,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://localhost/maestro".to_string(),
            max_connections: 20,
            min_connections: 5,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(1800),
        }
    }
}

impl DatabaseConfig {
    /// Create config from environment variable.
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://localhost/maestro".to_string()),
            ..Default::default()
        }
    }

    /// Create a configuration optimized for the indexer.
    pub fn for_indexer(url: &str) -> Self {
        Self {
            url: url.to_string(),
            max_connections: 10,
            min_connections: 3,
            acquire_timeout: Duration::from_secs(30),
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(1800),
        }
    }

    /// Create a configuration optimized for GraphQL queries.
    pub fn for_graphql(url: &str) -> Self {
        Self {
            url: url.to_string(),
            max_connections: 15,
            min_connections: 2,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(300),
            max_lifetime: Duration::from_secs(900),
        }
    }
}

/// Database connection pool wrapper.
#[derive(Clone)]
pub struct Database {
    pool: PgPool,
}

impl Database {
    /// Connect to the database with the given configuration.
    #[instrument(skip_all)]
    pub async fn connect(config: &DatabaseConfig) -> StorageResult<Self> {
        debug!(
            max_conn = config.max_connections,
            min_conn = config.min_connections,
            "Creating connection pool"
        );

        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(Some(config.idle_timeout))
            .max_lifetime(Some(config.max_lifetime))
            .connect(&config.url)
            .await
            .map_err(|e| StorageError::ConnectionError(e.to_string()))?;

        debug!("Connection pool created");

        Ok(Self { pool })
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Run database migrations.
    #[instrument(skip(self))]
    pub async fn migrate(&self) -> StorageResult<()> {
        debug!("Running migrations");

        sqlx::migrate!("./migrations")
            .run(&self.pool)
            .await
            .map_err(|e| StorageError::MigrationError(e.to_string()))?;

        debug!("Migrations completed");

        Ok(())
    }

    /// Check if the database connection is healthy.
    pub async fn is_healthy(&self) -> bool {
        sqlx::query("SELECT 1").fetch_one(&self.pool).await.is_ok()
    }

    /// Close the connection pool.
    pub async fn close(&self) {
        self.pool.close().await;
    }

    /// Purge all indexed data from the database.
    ///
    /// This operation:
    /// - Truncates all data tables (blocks, extrinsics, events, and bundle-specific tables)
    /// - Resets the indexer cursor
    /// - Preserves the schema and migrations tracking
    ///
    /// Use this to restart indexing from block 0 without dropping the database.
    #[instrument(skip(self))]
    pub async fn purge(&self) -> StorageResult<PurgeStats> {
        debug!("Starting database purge");

        // Count rows before purge for reporting
        let block_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM blocks")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let extrinsic_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM extrinsics")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let event_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM events")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        // TRUNCATE CASCADE will handle foreign key relationships
        // This removes data from: blocks, extrinsics, events, and any bundle tables
        // that reference blocks (like transfers)
        sqlx::query("TRUNCATE blocks CASCADE")
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        debug!("Truncated blocks (cascade to extrinsics, events, transfers)");

        // Clear the indexer cursor
        sqlx::query("TRUNCATE indexer_cursor")
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        debug!("Truncated indexer cursor");

        // Note: We keep bundle_migrations intact so migrations don't re-run
        // The schema is already in place, we just clear the data

        debug!("Database purge completed");

        Ok(PurgeStats {
            blocks_removed: block_count.0 as u64,
            extrinsics_removed: extrinsic_count.0 as u64,
            events_removed: event_count.0 as u64,
        })
    }
}

/// Statistics from a database purge operation.
#[derive(Debug, Clone)]
pub struct PurgeStats {
    /// Number of blocks removed.
    pub blocks_removed: u64,
    /// Number of extrinsics removed.
    pub extrinsics_removed: u64,
    /// Number of events removed.
    pub events_removed: u64,
}
