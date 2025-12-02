//! GraphQL type definitions.

use async_graphql::{EmptyMutation, EmptySubscription, Schema};

use crate::schema::CoreQuery;

/// The core GraphQL schema type (without bundle extensions).
pub type MaestroSchema = Schema<CoreQuery, EmptyMutation, EmptySubscription>;
