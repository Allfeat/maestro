//! Handler bundle trait definition.

use std::sync::Arc;

use maestro_core::ports::PalletHandler;

/// A self-contained bundle of handlers for one or more pallets.
///
/// Bundles provide a plugin-like architecture where each bundle can:
/// - Define its own database schema via migrations
/// - Register one or more pallet handlers
/// - Be independently developed and tested
///
/// # Example
///
/// ```ignore
/// pub struct MyBundle { /* ... */ }
///
/// impl HandlerBundle for MyBundle {
///     fn name(&self) -> &'static str { "my_bundle" }
///
///     fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
///         vec![Arc::new(MyHandler::new())]
///     }
///
///     fn migrations(&self) -> &'static [&'static str] {
///         &[include_str!("migrations/001_init.sql")]
///     }
/// }
/// ```
pub trait HandlerBundle: Send + Sync {
    /// Unique name identifying this bundle.
    ///
    /// Used for logging and migration tracking.
    fn name(&self) -> &'static str;

    /// Returns all pallet handlers provided by this bundle.
    ///
    /// These handlers will be registered with the indexer's HandlerRegistry.
    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>>;

    /// SQL migration statements for this bundle's schema.
    ///
    /// Migrations are executed in order when the bundle is registered.
    /// Each string should be a complete SQL statement or set of statements.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn migrations(&self) -> &'static [&'static str] {
    ///     &[
    ///         include_str!("migrations/001_create_tables.sql"),
    ///         include_str!("migrations/002_add_indexes.sql"),
    ///     ]
    /// }
    /// ```
    fn migrations(&self) -> &'static [&'static str] {
        &[]
    }

    /// Priority for bundle initialization (higher = earlier).
    ///
    /// Bundles with dependencies on other bundles should use lower priority.
    /// Default is 0.
    fn priority(&self) -> i32 {
        0
    }

    /// Called after all migrations have been run.
    ///
    /// Override this for any post-migration initialization.
    fn on_initialized(&self) {}

    /// Tables owned by this bundle that should be truncated during purge.
    ///
    /// Return the table names that this bundle creates and manages.
    /// These tables will be explicitly truncated when running `--purge`.
    ///
    /// Note: Tables with `ON DELETE CASCADE` referencing `blocks` will be
    /// automatically cleaned up, but it's still recommended to list them
    /// here for explicit documentation and safety.
    ///
    /// # Example
    ///
    /// ```ignore
    /// fn tables_to_purge(&self) -> &'static [&'static str] {
    ///     &["transfers", "rewards"]
    /// }
    /// ```
    fn tables_to_purge(&self) -> &'static [&'static str] {
        &[]
    }
}
