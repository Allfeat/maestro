//! Handler bundles for Maestro indexer.
//!
//! This crate provides a plugin-like system for extending the indexer with
//! custom pallet handlers. Each bundle is self-contained with its own:
//!
//! - Pallet handlers (event/extrinsic processing)
//! - SQL migrations (table definitions)
//! - Models (domain types)
//!
//! # Creating a Custom Bundle
//!
//! ```ignore
//! use maestro_handlers::{HandlerBundle, BundleRegistry};
//!
//! pub struct MyPalletBundle {
//!     // ... dependencies
//! }
//!
//! impl HandlerBundle for MyPalletBundle {
//!     fn name(&self) -> &'static str {
//!         "my_pallet"
//!     }
//!
//!     fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
//!         vec![Arc::new(MyPalletHandler::new(/* ... */))]
//!     }
//!
//!     fn migrations(&self) -> &'static [&'static str] {
//!         &[include_str!("my_pallet/migrations/001_create_tables.sql")]
//!     }
//! }
//! ```
//!
//! # Registering Bundles
//!
//! ```ignore
//! let mut registry = BundleRegistry::new();
//! registry.register(Box::new(BalancesBundle::new(repositories.clone())));
//! registry.register(Box::new(MyPalletBundle::new(/* ... */)));
//!
//! // Run all bundle migrations
//! registry.run_migrations(&db).await?;
//!
//! // Get unified handler registry
//! let handlers = registry.into_handler_registry();
//! ```

pub mod balances;

mod bundle;
mod registry;

pub use bundle::HandlerBundle;
pub use registry::BundleRegistry;

// Re-export balances bundle for convenience
pub use balances::BalancesBundle;
