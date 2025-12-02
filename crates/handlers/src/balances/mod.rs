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
//! let bundle = BalancesBundle::new(pool);
//! registry.register(Box::new(bundle));
//! ```

mod handler;
pub mod graphql;
pub mod models;
pub mod storage;

use std::sync::Arc;

use maestro_core::ports::PalletHandler;
use sqlx::PgPool;

use crate::HandlerBundle;

pub use graphql::BalancesQuery;
pub use handler::BalancesHandler;
pub use models::Transfer;
pub use storage::{BalancesStorage, PgBalancesStorage, TransferFilter, MIGRATIONS};

/// Handler bundle for the Balances pallet.
///
/// Tracks token transfers and provides the foundation for balance-related
/// indexing functionality.
pub struct BalancesBundle {
    pool: PgPool,
}

impl BalancesBundle {
    /// Create a new Balances bundle.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl HandlerBundle for BalancesBundle {
    fn name(&self) -> &'static str {
        "balances"
    }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        let storage = Arc::new(PgBalancesStorage::new(self.pool.clone()));
        vec![Arc::new(BalancesHandler::new(storage))]
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
