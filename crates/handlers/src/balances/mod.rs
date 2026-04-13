//! Balances pallet handler bundle.
//!
//! This bundle provides indexing support for the Substrate Balances pallet,
//! tracking token transfers between accounts.
//!
//! # Indexed Events
//!
//! - `Balances::Transfer` - Token transfers between accounts
//!
//! # Database Tables
//!
//! - `transfers` - All token transfer records
//!
//! # Usage
//!
//! ```ignore
//! use maestro_handlers::BalancesBundle;
//!
//! let bundle = BalancesBundle::new(pool, event_bus);
//! registry.register(Box::new(bundle));
//! ```

pub mod graphql;
mod handler;
pub mod models;
pub mod storage;

use std::sync::Arc;

use maestro_core::events::EventBus;
use maestro_core::ports::PalletHandler;
use sqlx::PgPool;

use crate::HandlerBundle;

pub use graphql::BalancesQuery;
pub use handler::BalancesHandler;
pub use models::Transfer;
pub use storage::{BalancesStorage, MIGRATIONS, PgBalancesStorage, TransferFilter};

/// Handler bundle for the Balances pallet.
///
/// Tracks token transfers and provides the foundation for balance-related
/// indexing functionality.
pub struct BalancesBundle {
    pool: PgPool,
    bus: EventBus,
}

impl BalancesBundle {
    /// Create a new Balances bundle.
    pub fn new(pool: PgPool, bus: EventBus) -> Self {
        Self { pool, bus }
    }
}

impl HandlerBundle for BalancesBundle {
    fn name(&self) -> &'static str {
        "balances"
    }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        let storage = Arc::new(PgBalancesStorage::new(self.pool.clone()));
        vec![Arc::new(BalancesHandler::new(storage, self.bus.clone()))]
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn priority(&self) -> i32 {
        // High priority - other bundles may depend on balance data
        100
    }

    fn tables_to_purge(&self) -> &'static [&'static str] {
        &["transfers"]
    }
}
