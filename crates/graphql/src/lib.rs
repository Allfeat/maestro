//! GraphQL API for Maestro indexer.
//!
//! Provides a GraphQL endpoint to query indexed blockchain data.
//!
//! # Building a Schema with Extensions
//!
//! Use `build_schema_with_query` to compose CoreQuery with bundle queries:
//!
//! ```ignore
//! use async_graphql::MergedObject;
//! use maestro_graphql::{build_schema_with_query, CoreQuery};
//! use maestro_handlers::balances::BalancesQuery;
//!
//! #[derive(MergedObject, Default)]
//! struct Query(CoreQuery, BalancesQuery);
//!
//! let schema = build_schema_with_query(Query::default(), repositories)
//!     .data(balances_storage)
//!     .finish();
//! ```

mod schema;
mod server;
mod types;

pub use schema::{
    build_core_schema, build_schema_with_query, schema_builder, CoreQuery, Order, PageInfo,
    MAX_QUERY_COMPLEXITY, MAX_QUERY_DEPTH,
};
pub use server::{serve, serve_with_shutdown, ServerConfig};
pub use types::MaestroSchema;
