//! Pagination types for list queries.
//!
//! These types implement Relay-style cursor pagination, commonly used
//! with GraphQL but also applicable to other APIs.

/// Opaque cursor for pagination.
///
/// The cursor value is implementation-specific and should be treated
/// as an opaque token by clients.
#[derive(Debug, Clone)]
pub struct Cursor {
    pub value: String,
}

/// Pagination parameters for list queries.
///
/// Supports forward pagination (`first`/`after`) and backward
/// pagination (`last`/`before`).
#[derive(Debug, Clone, Default)]
pub struct Pagination {
    /// Number of items to fetch (forward pagination).
    pub first: Option<i32>,
    /// Cursor to start after (forward pagination).
    pub after: Option<Cursor>,
    /// Number of items to fetch (backward pagination).
    pub last: Option<i32>,
    /// Cursor to end before (backward pagination).
    pub before: Option<Cursor>,
}

/// Paginated result set with edges and page info.
///
/// This is the Relay connection pattern for cursor-based pagination.
#[derive(Debug, Clone)]
pub struct Connection<T> {
    /// List of edges (node + cursor pairs).
    pub edges: Vec<Edge<T>>,
    /// Information about the current page.
    pub page_info: PageInfo,
    /// Total count of items (optional, expensive to compute).
    pub total_count: Option<i64>,
}

/// A single item in a paginated result.
#[derive(Debug, Clone)]
pub struct Edge<T> {
    /// The actual item.
    pub node: T,
    /// Cursor for this item (used for pagination).
    pub cursor: Cursor,
}

/// Information about the current page in a paginated result.
#[derive(Debug, Clone)]
pub struct PageInfo {
    /// Whether there are more items after this page.
    pub has_next_page: bool,
    /// Whether there are items before this page.
    pub has_previous_page: bool,
    /// Cursor of the first item in this page.
    pub start_cursor: Option<Cursor>,
    /// Cursor of the last item in this page.
    pub end_cursor: Option<Cursor>,
}

/// Ordering direction for sorted queries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum OrderDirection {
    /// Ascending order (smallest first).
    #[default]
    Asc,
    /// Descending order (largest first).
    Desc,
}
