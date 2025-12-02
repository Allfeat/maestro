//! Storage layer for Maestro indexer.
//!
//! This crate provides PostgreSQL implementations of the repository traits
//! defined in `maestro-core`. It handles all database interactions including
//! connection pooling, migrations, and CRUD operations.
//!
//! # Architecture
//!
//! The storage layer follows the repository pattern:
//!
//! - [`postgres::Database`] - Connection pool management
//! - [`postgres::PgRepositories`] - Composite repository for all entity types
//! - Individual repositories for blocks, extrinsics, events, and cursor
//!
//! # Usage
//!
//! ```ignore
//! use maestro_storage::{Database, DatabaseConfig, PgRepositories};
//!
//! // Connect to the database
//! let config = DatabaseConfig::for_indexer(&database_url);
//! let db = Database::connect(&config).await?;
//!
//! // Run migrations
//! db.migrate().await?;
//!
//! // Create repositories
//! let repositories = Arc::new(PgRepositories::new(Arc::new(db)));
//! ```

pub mod postgres;

pub use postgres::{Database, DatabaseConfig, PgRepositories, PurgeStats};
