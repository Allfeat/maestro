//! GraphQL API for Maestro indexer.
//!
//! Provides a GraphQL endpoint to query indexed blockchain data.
//!
//! # Building a Schema with Extensions
//!
//! Compose `CoreQuery` with bundle queries using `async_graphql::MergedObject`,
//! then build the schema directly:
//!
//! ```ignore
//! use async_graphql::{EmptyMutation, EmptySubscription, MergedObject, Schema};
//! use maestro_graphql::{CoreQuery, MAX_QUERY_COMPLEXITY, MAX_QUERY_DEPTH};
//! use maestro_handlers::balances::BalancesQuery;
//!
//! #[derive(MergedObject, Default)]
//! struct Query(CoreQuery, BalancesQuery);
//!
//! let schema = Schema::build(Query::default(), EmptyMutation, EmptySubscription)
//!     .data(repositories)
//!     .limit_depth(MAX_QUERY_DEPTH)
//!     .limit_complexity(MAX_QUERY_COMPLEXITY)
//!     .finish();
//! ```

mod schema;
mod server;
pub mod validation;

pub use schema::{
    CoreQuery, MAX_QUERY_COMPLEXITY, MAX_QUERY_DEPTH, Order, PageInfo, convert_order,
};
pub use server::{ServerConfig, serve_with_shutdown};
