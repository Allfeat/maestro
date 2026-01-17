//! MIDDS (Musical Industry Digital Distribution Standard) handler bundle.
//!
//! This bundle provides indexing support for the Allfeat MIDDS pallets,
//! tracking musical works, recordings, and releases.
//!
//! # Indexed Events
//!
//! - `MusicalWorks::MIDDSRegistered` - New musical work registrations
//! - `MusicalWorks::MIDDSUnregistered` - Musical work removals
//! - `Recordings::MIDDSRegistered` - New recording registrations
//! - `Recordings::MIDDSUnregistered` - Recording removals
//! - `Releases::MIDDSRegistered` - New release registrations
//! - `Releases::MIDDSUnregistered` - Release removals
//!
//! # Database Tables
//!
//! - `midds_musical_works` - Musical compositions (ISWC)
//! - `midds_recordings` - Audio recordings (ISRC)
//! - `midds_releases` - Albums/singles (EAN/UPC)
//!
//! # Usage
//!
//! ```ignore
//! use maestro_handlers::MiddsBundle;
//!
//! let bundle = MiddsBundle::new(pool, storage_reader);
//! registry.register(Box::new(bundle));
//! ```

mod handler;
pub mod graphql;
pub mod models;
pub mod storage;

use std::sync::Arc;

use maestro_core::ports::{PalletHandler, StorageReader};
use sqlx::PgPool;

use crate::HandlerBundle;

pub use graphql::MiddsQuery;
pub use handler::{MusicalWorksHandler, RecordingsHandler, ReleasesHandler};
pub use models::{Creator, Date, MusicalWork, PartyId, Recording, Release};
pub use storage::{
    MiddsStorage, MusicalWorkFilter, PgMiddsStorage, RecordingFilter, ReleaseFilter, MIGRATIONS,
};

/// Handler bundle for the MIDDS pallets.
///
/// Tracks musical works, recordings, and releases registered on-chain.
pub struct MiddsBundle {
    pool: PgPool,
    chain_reader: Arc<dyn StorageReader>,
}

impl MiddsBundle {
    /// Create a new MIDDS bundle.
    pub fn new(pool: PgPool, chain_reader: Arc<dyn StorageReader>) -> Self {
        Self { pool, chain_reader }
    }
}

impl HandlerBundle for MiddsBundle {
    fn name(&self) -> &'static str {
        "midds"
    }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        let storage: Arc<dyn MiddsStorage> = Arc::new(PgMiddsStorage::new(self.pool.clone()));
        vec![
            Arc::new(MusicalWorksHandler::new(
                storage.clone(),
                self.chain_reader.clone(),
            )),
            Arc::new(RecordingsHandler::new(
                storage.clone(),
                self.chain_reader.clone(),
            )),
            Arc::new(ReleasesHandler::new(storage, self.chain_reader.clone())),
        ]
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn priority(&self) -> i32 {
        // Same priority as ATS
        50
    }

    fn tables_to_purge(&self) -> &'static [&'static str] {
        // Order matters: dependent tables first
        &[
            "midds_releases",
            "midds_recordings",
            "midds_musical_works",
        ]
    }
}
