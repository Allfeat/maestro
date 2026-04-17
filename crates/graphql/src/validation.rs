//! Shared validation helpers for GraphQL resolvers.
//!
//! Centralized here so core `CoreQuery` and handler bundles (balances, ats,
//! midds) apply identical bounds. Any new resolver that takes a cursor,
//! account, or filter string must call these helpers — don't reinvent.

use async_graphql::{Error, Result};

use maestro_core::models::AccountId;

/// Maximum length for hash strings (64 hex chars + "0x" prefix).
pub const MAX_HASH_LENGTH: usize = 66;
/// Maximum length for string filter parameters.
pub const MAX_FILTER_STRING_LENGTH: usize = 128;
/// Maximum length for an opaque pagination cursor.
///
/// Cursors are opaque tokens produced by the server — legitimate ones fit in
/// ~64 bytes. The 512 cap is slack for future encodings while still rejecting
/// trivially oversized payloads (DoS guard).
pub const MAX_CURSOR_LENGTH: usize = 512;
/// Maximum page size for pagination.
pub const MAX_PAGE_SIZE: i32 = 100;
/// Default page size for pagination.
pub const DEFAULT_PAGE_SIZE: i32 = 20;

/// Parse and validate a hex hash string (with or without `0x` prefix).
pub fn parse_hash(s: &str) -> Result<[u8; 32]> {
    if s.len() > MAX_HASH_LENGTH {
        return Err(Error::new(format!(
            "Hash too long: maximum {} characters allowed",
            MAX_HASH_LENGTH
        )));
    }

    let s = s.strip_prefix("0x").unwrap_or(s);

    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::new(
            "Invalid hash: must contain only hexadecimal characters",
        ));
    }

    let bytes = hex::decode(s).map_err(|e| Error::new(format!("Invalid hash: {}", e)))?;

    bytes
        .try_into()
        .map_err(|_| Error::new("Hash must be exactly 32 bytes (64 hex characters)"))
}

/// Parse and validate an account address (same shape as a 32-byte hash).
pub fn parse_account(s: &str) -> Result<AccountId> {
    let bytes = parse_hash(s)?;
    Ok(AccountId(bytes))
}

/// Validate a non-empty, bounded-length string filter.
pub fn validate_filter_string(s: &Option<String>, field_name: &str) -> Result<()> {
    if let Some(value) = s {
        if value.len() > MAX_FILTER_STRING_LENGTH {
            return Err(Error::new(format!(
                "{} too long: maximum {} characters allowed",
                field_name, MAX_FILTER_STRING_LENGTH
            )));
        }
        if value.is_empty() {
            return Err(Error::new(format!("{} cannot be empty", field_name)));
        }
    }
    Ok(())
}

/// Clamp `first` into `[1, MAX_PAGE_SIZE]`, defaulting to `DEFAULT_PAGE_SIZE`.
pub fn validate_pagination_first(first: Option<i32>) -> i32 {
    first.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

/// Reject cursors above `MAX_CURSOR_LENGTH`. Empty strings and `None` pass —
/// the storage layer rejects malformed cursor payloads itself.
pub fn validate_cursor(cursor: &Option<String>, field_name: &str) -> Result<()> {
    if let Some(value) = cursor
        && value.len() > MAX_CURSOR_LENGTH
    {
        return Err(Error::new(format!(
            "{} cursor too long: maximum {} characters allowed",
            field_name, MAX_CURSOR_LENGTH
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hash_rejects_invalid_input() {
        assert!(parse_hash(&"ab".repeat(100)).is_err());
        assert!(parse_hash("0x<script>alert(1)</script>").is_err());
        assert!(parse_hash(&"ab".repeat(16)).is_err());
    }

    #[test]
    fn parse_hash_accepts_both_formats() {
        let with_prefix = parse_hash(&("0x".to_string() + &"ab".repeat(32)));
        let without_prefix = parse_hash(&"ab".repeat(32));
        assert!(with_prefix.is_ok());
        assert!(without_prefix.is_ok());
        assert_eq!(with_prefix.unwrap(), without_prefix.unwrap());
    }

    #[test]
    fn validate_filter_string_boundaries() {
        assert!(validate_filter_string(&Some("".into()), "x").is_err());
        assert!(validate_filter_string(&Some("x".repeat(200)), "x").is_err());
        assert!(validate_filter_string(&None, "x").is_ok());
    }

    #[test]
    fn pagination_first_clamping() {
        assert_eq!(validate_pagination_first(Some(-100)), 1);
        assert_eq!(validate_pagination_first(Some(0)), 1);
        assert_eq!(validate_pagination_first(Some(10000)), MAX_PAGE_SIZE);
        assert_eq!(validate_pagination_first(Some(50)), 50);
        assert_eq!(validate_pagination_first(None), DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn cursor_respects_upper_bound() {
        assert!(validate_cursor(&None, "after").is_ok());
        assert!(validate_cursor(&Some(String::new()), "after").is_ok());
        assert!(validate_cursor(&Some("x".repeat(MAX_CURSOR_LENGTH)), "after").is_ok());
        let err = validate_cursor(&Some("x".repeat(MAX_CURSOR_LENGTH + 1)), "after").unwrap_err();
        assert!(err.message.contains("too long"));
        assert!(err.message.contains("after"));
    }

    #[test]
    fn parse_account_valid() {
        let hex = "0x".to_string() + &"ef".repeat(32);
        let result = parse_account(&hex);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, [0xef; 32]);
    }
}
