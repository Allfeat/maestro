//! Shared GraphQL utilities for handler bundles.
//!
//! This module provides common parsing and validation functions used across
//! multiple bundle GraphQL implementations.

use async_graphql::Result;
use maestro_core::models::AccountId;

/// Maximum length for hash strings (64 hex chars + "0x" prefix).
pub const MAX_HASH_LENGTH: usize = 66;
/// Maximum page size for pagination.
pub const MAX_PAGE_SIZE: i32 = 100;
/// Default page size for pagination.
pub const DEFAULT_PAGE_SIZE: i32 = 20;

/// Parse and validate a hash string.
///
/// Accepts hex strings with or without "0x" prefix.
/// Returns a 32-byte array on success.
pub fn parse_hash(s: &str) -> Result<[u8; 32]> {
    if s.len() > MAX_HASH_LENGTH {
        return Err(async_graphql::Error::new(format!(
            "Hash too long: maximum {} characters allowed",
            MAX_HASH_LENGTH
        )));
    }

    let s = s.strip_prefix("0x").unwrap_or(s);

    if !s.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(async_graphql::Error::new(
            "Invalid hash: must contain only hexadecimal characters",
        ));
    }

    let bytes =
        hex::decode(s).map_err(|e| async_graphql::Error::new(format!("Invalid hash: {}", e)))?;

    bytes
        .try_into()
        .map_err(|_| async_graphql::Error::new("Hash must be exactly 32 bytes (64 hex characters)"))
}

/// Parse and validate an account address.
///
/// Wraps `parse_hash` and returns an `AccountId`.
pub fn parse_account(s: &str) -> Result<AccountId> {
    let bytes = parse_hash(s)?;
    Ok(AccountId(bytes))
}

/// Validate and normalize pagination first parameter.
///
/// Clamps the value between 1 and `MAX_PAGE_SIZE`, defaulting to `DEFAULT_PAGE_SIZE`.
pub fn validate_pagination_first(first: Option<i32>) -> i32 {
    first.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hash_valid_with_prefix() {
        let hex = "0x".to_string() + &"ab".repeat(32);
        let result = parse_hash(&hex);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0xab; 32]);
    }

    #[test]
    fn test_parse_hash_valid_without_prefix() {
        let hex = "cd".repeat(32);
        let result = parse_hash(&hex);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [0xcd; 32]);
    }

    #[test]
    fn test_parse_hash_too_long() {
        let hex = "0x".to_string() + &"ab".repeat(40);
        let result = parse_hash(&hex);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("too long"));
    }

    #[test]
    fn test_parse_hash_invalid_chars() {
        let result = parse_hash("0xgg11223344556677889900112233445566778899001122334455667788990011");
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("hexadecimal"));
    }

    #[test]
    fn test_parse_hash_wrong_length() {
        let hex = "0x".to_string() + &"ab".repeat(16);
        let result = parse_hash(&hex);
        assert!(result.is_err());
        assert!(result.unwrap_err().message.contains("32 bytes"));
    }

    #[test]
    fn test_parse_account_valid() {
        let hex = "0x".to_string() + &"ef".repeat(32);
        let result = parse_account(&hex);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().0, [0xef; 32]);
    }

    #[test]
    fn test_validate_pagination_default() {
        assert_eq!(validate_pagination_first(None), DEFAULT_PAGE_SIZE);
    }

    #[test]
    fn test_validate_pagination_clamps_high() {
        assert_eq!(validate_pagination_first(Some(500)), MAX_PAGE_SIZE);
    }

    #[test]
    fn test_validate_pagination_clamps_low() {
        assert_eq!(validate_pagination_first(Some(0)), 1);
        assert_eq!(validate_pagination_first(Some(-5)), 1);
    }

    #[test]
    fn test_validate_pagination_within_range() {
        assert_eq!(validate_pagination_first(Some(50)), 50);
    }
}
