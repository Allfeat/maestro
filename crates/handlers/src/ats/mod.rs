//! ATS (Allfeat Timestamp) pallet handler bundle.
//!
//! This bundle provides indexing support for the Allfeat ATS pallet,
//! tracking timestamped creative works, their versions, and ownership transfers.
//!
//! # Indexed Events
//!
//! - `Ats::ATSRegistered` - New ATS work registrations
//! - `Ats::ATSUpdated` - New versions of existing ATS works
//! - `Ats::ATSClaimed` - Ownership transfers via ZKP verification
//! - `Ats::VerificationKeyUpdated` - ZKP verification key updates
//!
//! # Database Tables
//!
//! - `ats_works` - Main ATS registry with current ownership
//! - `ats_versions` - Version history with hash commitments
//! - `ats_ownership_transfers` - Ownership claim history
//! - `ats_verification_keys` - ZKP verification key history
//!
//! # Usage
//!
//! ```ignore
//! use maestro_handlers::AtsBundle;
//!
//! let bundle = AtsBundle::new(pool);
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

pub use graphql::AtsQuery;
pub use handler::AtsHandler;
pub use models::{AtsOwnershipTransfer, AtsVerificationKeyUpdate, AtsVersion, AtsWork};
pub use storage::{
    AtsStorage, AtsTransferFilter, AtsWorkFilter, PgAtsStorage, MIGRATIONS,
};

/// Handler bundle for the ATS (Allfeat Timestamp) pallet.
///
/// Tracks timestamped creative works, their versions, ownership transfers,
/// and ZKP verification key updates.
pub struct AtsBundle {
    pool: PgPool,
}

impl AtsBundle {
    /// Create a new ATS bundle.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl HandlerBundle for AtsBundle {
    fn name(&self) -> &'static str {
        "ats"
    }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        let storage = Arc::new(PgAtsStorage::new(self.pool.clone()));
        vec![Arc::new(AtsHandler::new(storage))]
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn priority(&self) -> i32 {
        // Standard priority for application-specific pallets
        50
    }

    fn tables_to_purge(&self) -> &'static [&'static str] {
        // Order matters: children tables before parents (due to foreign keys)
        &[
            "ats_verification_keys",
            "ats_ownership_transfers",
            "ats_versions",
            "ats_works",
        ]
    }
}
