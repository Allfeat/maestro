//! Shared utilities for pallet handlers.
//!
//! This module provides common parsing and extraction functions used across
//! multiple pallet handlers to avoid code duplication.

use maestro_core::models::AccountId;

// =============================================================================
// Event field extraction
// =============================================================================

/// Extract a field from event data, trying multiple key names and falling back to index.
///
/// This function attempts to find a value in the event data by:
/// 1. First trying each key in the `keys` slice in order
/// 2. If no key matches, falling back to accessing by `index`
/// 3. Parsing the found value using the provided `parser` function
///
/// # Example
///
/// ```ignore
/// let from = extract_field(data, &["from", "who"], 0, parse_account);
/// ```
pub fn extract_field<T>(
    data: &serde_json::Value,
    keys: &[&str],
    index: usize,
    parser: fn(&serde_json::Value) -> Option<T>,
) -> Option<T> {
    keys.iter()
        .find_map(|key| data.get(*key))
        .or_else(|| data.get(index))
        .and_then(parser)
}

// =============================================================================
// Account parsing
// =============================================================================

/// Parse an account ID from various JSON representations.
///
/// Handles multiple formats that may be returned by Substrate nodes:
/// - Hex string: `"0x1234..."`
/// - Wrapped object: `{ "Id": "0x..." }`
/// - Array wrapper: `["0x..."]`
/// - Byte array: `[b0, b1, ..., b31]`
pub fn parse_account(value: &serde_json::Value) -> Option<AccountId> {
    match value {
        // Hex string: "0x1234..."
        serde_json::Value::String(s) => {
            let hex_str = s.strip_prefix("0x").unwrap_or(s);
            let bytes = hex::decode(hex_str).ok()?;
            let arr: [u8; 32] = bytes.try_into().ok()?;
            Some(AccountId(arr))
        }
        // Wrapped object: { "Id": "0x..." }
        serde_json::Value::Object(obj) => obj
            .get("Id")
            .or_else(|| obj.get("id"))
            .and_then(parse_account),
        // Array: either ["0x..."] or [b0, b1, ..., b31]
        serde_json::Value::Array(arr) => {
            if arr.len() == 1 {
                return parse_account(&arr[0]);
            }
            if arr.len() != 32 {
                return None;
            }
            let mut bytes = [0u8; 32];
            for (i, v) in arr.iter().enumerate() {
                bytes[i] = v.as_u64()? as u8;
            }
            Some(AccountId(bytes))
        }
        _ => None,
    }
}

// =============================================================================
// Numeric parsing
// =============================================================================

/// Parse an amount (u128) from JSON.
///
/// Handles both numeric and string representations, which is important
/// because JSON numbers are limited to u64 but Substrate amounts can be u128.
pub fn parse_amount(value: &serde_json::Value) -> Option<u128> {
    match value {
        serde_json::Value::Number(n) => n.as_u64().map(u128::from),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Parse a u64 from JSON.
pub fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    match value {
        serde_json::Value::Number(n) => n.as_u64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// Parse a u32 from JSON.
pub fn parse_u32(value: &serde_json::Value) -> Option<u32> {
    parse_u64(value).and_then(|v| v.try_into().ok())
}

// =============================================================================
// Hash/bytes parsing
// =============================================================================

/// Parse a 32-byte hash from JSON.
///
/// Handles:
/// - Hex string: `"0x1234..."`
/// - Byte array: `[b0, b1, ..., b31]`
pub fn parse_hash256(value: &serde_json::Value) -> Option<[u8; 32]> {
    match value {
        // Hex string: "0x1234..."
        serde_json::Value::String(s) => {
            let hex_str = s.strip_prefix("0x").unwrap_or(s);
            let bytes = hex::decode(hex_str).ok()?;
            bytes.try_into().ok()
        }
        // Array of bytes: [b0, b1, ..., b31]
        serde_json::Value::Array(arr) => {
            if arr.len() != 32 {
                return None;
            }
            let mut bytes = [0u8; 32];
            for (i, v) in arr.iter().enumerate() {
                bytes[i] = v.as_u64()? as u8;
            }
            Some(bytes)
        }
        _ => None,
    }
}

/// Parse arbitrary bytes from JSON.
///
/// Handles:
/// - Hex string: `"0x1234..."`
/// - Byte array: `[b0, b1, ...]`
pub fn parse_bytes(value: &serde_json::Value) -> Option<Vec<u8>> {
    match value {
        // Hex string: "0x1234..."
        serde_json::Value::String(s) => {
            let hex_str = s.strip_prefix("0x").unwrap_or(s);
            hex::decode(hex_str).ok()
        }
        // Array of bytes
        serde_json::Value::Array(arr) => {
            let mut bytes = Vec::with_capacity(arr.len());
            for v in arr {
                bytes.push(v.as_u64()? as u8);
            }
            Some(bytes)
        }
        _ => None,
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -------------------------------------------------------------------------
    // Account parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_account_hex_string() {
        let hex = "0x".to_string() + &"ab".repeat(32);
        let result = parse_account(&json!(hex));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, [0xab; 32]);
    }

    #[test]
    fn test_parse_account_without_prefix() {
        let hex = "cd".repeat(32);
        let result = parse_account(&json!(hex));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, [0xcd; 32]);
    }

    #[test]
    fn test_parse_account_wrapped_id() {
        let hex = "0x".to_string() + &"ef".repeat(32);
        let result = parse_account(&json!({"Id": hex}));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, [0xef; 32]);
    }

    #[test]
    fn test_parse_account_wrapped_lowercase_id() {
        let hex = "0x".to_string() + &"12".repeat(32);
        let result = parse_account(&json!({"id": hex}));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, [0x12; 32]);
    }

    #[test]
    fn test_parse_account_array_wrapper() {
        let hex = "0x".to_string() + &"34".repeat(32);
        let result = parse_account(&json!([hex]));
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, [0x34; 32]);
    }

    #[test]
    fn test_parse_account_byte_array() {
        let bytes: Vec<u8> = (0..32).collect();
        let result = parse_account(&json!(bytes));
        assert!(result.is_some());
        let expected: [u8; 32] = (0..32).collect::<Vec<u8>>().try_into().unwrap();
        assert_eq!(result.unwrap().0, expected);
    }

    #[test]
    fn test_parse_account_rejects_invalid_length() {
        let short_hex = "0x".to_string() + &"ab".repeat(16);
        assert!(parse_account(&json!(short_hex)).is_none());

        let short_array: Vec<u8> = (0..16).collect();
        assert!(parse_account(&json!(short_array)).is_none());
    }

    #[test]
    fn test_parse_account_rejects_invalid_hex() {
        assert!(parse_account(&json!("not_valid_hex")).is_none());
    }

    // -------------------------------------------------------------------------
    // Numeric parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_amount_number() {
        assert_eq!(parse_amount(&json!(12345)), Some(12345));
        assert_eq!(parse_amount(&json!(u64::MAX)), Some(u64::MAX as u128));
    }

    #[test]
    fn test_parse_amount_string() {
        assert_eq!(parse_amount(&json!("67890")), Some(67890));
        // Large amounts that exceed u64
        let large = "340282366920938463463374607431768211455"; // u128::MAX
        assert_eq!(parse_amount(&json!(large)), Some(u128::MAX));
    }

    #[test]
    fn test_parse_u64() {
        assert_eq!(parse_u64(&json!(12345)), Some(12345));
        assert_eq!(parse_u64(&json!("67890")), Some(67890));
        assert_eq!(parse_u64(&json!(u64::MAX)), Some(u64::MAX));
    }

    #[test]
    fn test_parse_u32() {
        assert_eq!(parse_u32(&json!(12345)), Some(12345));
        assert_eq!(parse_u32(&json!("67890")), Some(67890));
        // Should fail for values > u32::MAX
        assert!(parse_u32(&json!(u64::MAX)).is_none());
    }

    // -------------------------------------------------------------------------
    // Hash/bytes parsing tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_parse_hash256_hex() {
        let hex = "0x".to_string() + &"ab".repeat(32);
        let result = parse_hash256(&json!(hex));
        assert!(result.is_some());
        assert_eq!(result.unwrap(), [0xab; 32]);
    }

    #[test]
    fn test_parse_hash256_array() {
        let bytes: Vec<u8> = (0..32).collect();
        let result = parse_hash256(&json!(bytes));
        assert!(result.is_some());
        let expected: [u8; 32] = (0..32).collect::<Vec<u8>>().try_into().unwrap();
        assert_eq!(result.unwrap(), expected);
    }

    #[test]
    fn test_parse_hash256_rejects_wrong_length() {
        let short = "0x".to_string() + &"ab".repeat(16);
        assert!(parse_hash256(&json!(short)).is_none());

        let short_array: Vec<u8> = (0..16).collect();
        assert!(parse_hash256(&json!(short_array)).is_none());
    }

    #[test]
    fn test_parse_bytes_hex() {
        let result = parse_bytes(&json!("0xdeadbeef"));
        assert_eq!(result, Some(vec![0xde, 0xad, 0xbe, 0xef]));
    }

    #[test]
    fn test_parse_bytes_array() {
        let result = parse_bytes(&json!([1, 2, 3, 4]));
        assert_eq!(result, Some(vec![1, 2, 3, 4]));
    }

    // -------------------------------------------------------------------------
    // Field extraction tests
    // -------------------------------------------------------------------------

    #[test]
    fn test_extract_field_by_key() {
        let data = json!({
            "from": "0x".to_string() + &"ab".repeat(32),
            "amount": 1000
        });

        let from = extract_field(&data, &["from", "who"], 0, parse_account);
        assert!(from.is_some());
        assert_eq!(from.unwrap().0, [0xab; 32]);

        let amount = extract_field(&data, &["amount", "value"], 1, parse_amount);
        assert_eq!(amount, Some(1000));
    }

    #[test]
    fn test_extract_field_fallback_key() {
        let data = json!({
            "who": "0x".to_string() + &"cd".repeat(32)
        });

        // "from" doesn't exist, should fallback to "who"
        let from = extract_field(&data, &["from", "who"], 0, parse_account);
        assert!(from.is_some());
        assert_eq!(from.unwrap().0, [0xcd; 32]);
    }

    #[test]
    fn test_extract_field_by_index() {
        let data = json!([
            "0x".to_string() + &"ef".repeat(32),
            1000
        ]);

        // No matching keys, should use index
        let from = extract_field(&data, &["from", "who"], 0, parse_account);
        assert!(from.is_some());
        assert_eq!(from.unwrap().0, [0xef; 32]);

        let amount = extract_field(&data, &["amount", "value"], 1, parse_amount);
        assert_eq!(amount, Some(1000));
    }
}
