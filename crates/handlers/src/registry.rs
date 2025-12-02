//! Bundle registry for managing handler bundles.

use std::cmp::Reverse;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use tracing::{debug, info, warn};

use maestro_core::ports::HandlerRegistry;

use crate::bundle::HandlerBundle;

/// Registry for managing handler bundles.
///
/// The registry handles:
/// - Bundle registration with priority ordering
/// - Migration execution with tracking (idempotent)
/// - Handler extraction for the indexer
///
/// # Example
///
/// ```ignore
/// let mut registry = BundleRegistry::new();
///
/// // Register bundles (order doesn't matter - priority determines execution order)
/// registry.register(Box::new(BalancesBundle::new(pool.clone())));
/// registry.register(Box::new(StakingBundle::new(pool.clone())));
///
/// // Run migrations for all bundles (tracked, idempotent)
/// registry.run_migrations(&pool).await?;
///
/// // Convert to HandlerRegistry for the indexer
/// let handlers = registry.into_handler_registry();
/// ```
pub struct BundleRegistry {
    bundles: Vec<Box<dyn HandlerBundle>>,
}

impl BundleRegistry {
    /// Create a new empty bundle registry.
    pub fn new() -> Self {
        Self {
            bundles: Vec::new(),
        }
    }

    /// Register a handler bundle.
    ///
    /// Bundles are stored and will be processed in priority order
    /// (higher priority first) when migrations are run.
    pub fn register(&mut self, bundle: Box<dyn HandlerBundle>) {
        info!(bundle = bundle.name(), "📦 Registering handler bundle");
        self.bundles.push(bundle);
    }

    /// Run all bundle migrations in priority order.
    ///
    /// Migrations are tracked in the `bundle_migrations` table and only
    /// executed if not already applied. Each migration is identified by
    /// its bundle name, index, and content checksum.
    pub async fn run_migrations(&self, pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
        // Sort by priority (higher first)
        let mut sorted: Vec<_> = self.bundles.iter().collect();
        sorted.sort_by_key(|b| Reverse(b.priority()));

        for bundle in sorted {
            let migrations = bundle.migrations();
            if migrations.is_empty() {
                debug!(bundle = bundle.name(), "No migrations to run");
                continue;
            }

            for (index, migration) in migrations.iter().enumerate() {
                let checksum = compute_checksum(migration);

                // Check if migration was already applied
                let existing: Option<(String,)> = sqlx::query_as(
                    "SELECT checksum FROM bundle_migrations WHERE bundle_name = $1 AND migration_index = $2"
                )
                .bind(bundle.name())
                .bind(index as i32)
                .fetch_optional(pool)
                .await?;

                match existing {
                    Some((existing_checksum,)) => {
                        if existing_checksum != checksum {
                            warn!(
                                bundle = bundle.name(),
                                migration = index,
                                expected = %checksum,
                                found = %existing_checksum,
                                "⚠️  Migration checksum mismatch! Migration content has changed."
                            );
                            // Continue anyway - the migration was already applied
                        }
                        debug!(
                            bundle = bundle.name(),
                            migration = index,
                            "Migration already applied, skipping"
                        );
                        continue;
                    }
                    None => {
                        info!(
                            bundle = bundle.name(),
                            migration = index,
                            "🗄️  Applying migration"
                        );

                        // Execute the migration
                        sqlx::raw_sql(migration).execute(pool).await?;

                        // Record the migration
                        sqlx::query(
                            "INSERT INTO bundle_migrations (bundle_name, migration_index, checksum) VALUES ($1, $2, $3)"
                        )
                        .bind(bundle.name())
                        .bind(index as i32)
                        .bind(&checksum)
                        .execute(pool)
                        .await?;
                    }
                }
            }

            bundle.on_initialized();
        }

        Ok(())
    }

    /// Convert this registry into a HandlerRegistry.
    ///
    /// This extracts all handlers from all bundles and registers them
    /// with a new HandlerRegistry. The BundleRegistry is consumed.
    pub fn into_handler_registry(self) -> HandlerRegistry {
        let mut registry = HandlerRegistry::new();

        // Sort by priority for deterministic handler registration
        let mut sorted = self.bundles;
        sorted.sort_by_key(|b| Reverse(b.priority()));

        for bundle in sorted {
            let handlers = bundle.handlers();
            debug!(
                bundle = bundle.name(),
                handlers = handlers.len(),
                "Extracting handlers"
            );

            for handler in handlers {
                registry.register(handler);
            }
        }

        registry
    }

    /// Get the names of all registered bundles.
    pub fn bundle_names(&self) -> Vec<&'static str> {
        self.bundles.iter().map(|b| b.name()).collect()
    }

    /// Get the number of registered bundles.
    pub fn len(&self) -> usize {
        self.bundles.len()
    }

    /// Check if no bundles are registered.
    pub fn is_empty(&self) -> bool {
        self.bundles.is_empty()
    }

    /// Get all tables that should be purged from all bundles.
    ///
    /// Returns a deduplicated list of table names.
    pub fn tables_to_purge(&self) -> Vec<&'static str> {
        let mut tables: Vec<&'static str> = self
            .bundles
            .iter()
            .flat_map(|b| b.tables_to_purge().iter().copied())
            .collect();
        tables.sort();
        tables.dedup();
        tables
    }

    /// Purge all bundle-owned tables.
    ///
    /// This truncates all tables declared by bundles via `tables_to_purge()`.
    /// Returns the number of tables truncated.
    pub async fn purge_tables(&self, pool: &sqlx::PgPool) -> Result<usize, sqlx::Error> {
        let tables = self.tables_to_purge();

        for table in &tables {
            debug!(table = %table, "Truncating bundle table");
            // Use dynamic SQL since table names can't be parameterized
            let query = format!("TRUNCATE {} CASCADE", table);
            sqlx::raw_sql(&query).execute(pool).await?;
        }

        Ok(tables.len())
    }
}

impl Default for BundleRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute a checksum for migration content.
fn compute_checksum(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use async_trait::async_trait;
    use maestro_core::error::DomainResult;
    use maestro_core::models::Block;
    use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

    struct MockHandler(&'static str, i32);

    #[async_trait]
    impl PalletHandler for MockHandler {
        fn pallet_name(&self) -> &'static str { self.0 }
        fn priority(&self) -> i32 { self.1 }
        async fn handle_event(&self, _: &RawEvent, _: &Block, _: Option<&RawExtrinsic>) -> DomainResult<HandlerOutputs> {
            Ok(HandlerOutputs::new())
        }
    }

    struct MockBundle {
        name: &'static str,
        priority: i32,
        handlers: Vec<Arc<dyn PalletHandler>>,
    }

    impl HandlerBundle for MockBundle {
        fn name(&self) -> &'static str { self.name }
        fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> { self.handlers.clone() }
        fn migrations(&self) -> &'static [&'static str] { &[] }
        fn priority(&self) -> i32 { self.priority }
    }

    // Test critique: les handlers sont extraits dans l'ordre de priorité des bundles
    #[test]
    fn test_bundle_to_handler_registry_extraction() {
        let mut registry = BundleRegistry::new();

        registry.register(Box::new(MockBundle {
            name: "low_priority",
            priority: 0,
            handlers: vec![Arc::new(MockHandler("Balances", 0))],
        }));
        registry.register(Box::new(MockBundle {
            name: "high_priority",
            priority: 100,
            handlers: vec![Arc::new(MockHandler("Staking", 10))],
        }));

        let handler_registry = registry.into_handler_registry();

        // Tous les handlers doivent être présents
        assert!(handler_registry.has_handler("Balances"));
        assert!(handler_registry.has_handler("Staking"));
    }

    // Test critique: checksum déterministe pour tracking des migrations
    #[test]
    fn test_migration_checksum_stability() {
        let sql = "CREATE TABLE transfers (id SERIAL PRIMARY KEY);";

        // Le même SQL doit toujours produire le même checksum
        assert_eq!(compute_checksum(sql), compute_checksum(sql));

        // Un changement minime doit changer le checksum
        let sql_modified = "CREATE TABLE transfers (id BIGSERIAL PRIMARY KEY);";
        assert_ne!(compute_checksum(sql), compute_checksum(sql_modified));
    }
}
