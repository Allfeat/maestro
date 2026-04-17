//! Shared GraphQL utilities for handler bundles.
//!
//! Thin façade over `maestro_graphql::validation` so bundle resolvers don't
//! reach across crates themselves. The canonical definitions live in the
//! graphql crate — do not fork them here.

pub use maestro_graphql::validation::{
    DEFAULT_PAGE_SIZE, MAX_CURSOR_LENGTH, MAX_FILTER_STRING_LENGTH, MAX_HASH_LENGTH, MAX_PAGE_SIZE,
    parse_account, parse_hash, validate_cursor, validate_filter_string, validate_pagination_first,
};
