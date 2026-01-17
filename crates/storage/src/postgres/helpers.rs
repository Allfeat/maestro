//! Shared helper functions for PostgreSQL row conversion.

use maestro_core::error::{StorageError, StorageResult};

/// Convert a `Vec<u8>` to a fixed-size 32-byte array.
///
/// Returns an error if the length doesn't match.
pub fn bytes_to_hash32(bytes: Vec<u8>, field_name: &str) -> StorageResult<[u8; 32]> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        StorageError::SerializationError(format!(
            "{} has invalid length: expected 32, got {}",
            field_name,
            v.len()
        ))
    })
}

/// Convert a `Vec<u8>` to a 32-byte array, rejecting all-zero values as corrupt.
///
/// This is stricter than `bytes_to_hash32` and should be used for block hashes
/// where all-zeros indicates data corruption.
pub fn bytes_to_hash32_strict(bytes: Vec<u8>, field_name: &str) -> StorageResult<[u8; 32]> {
    let arr = bytes_to_hash32(bytes, field_name)?;

    if arr == [0u8; 32] {
        return Err(StorageError::SerializationError(format!(
            "{} is all zeros, which indicates data corruption",
            field_name
        )));
    }

    Ok(arr)
}

/// Convert an optional `Vec<u8>` to an optional 32-byte array.
pub fn bytes_to_optional_hash32(
    bytes: Option<Vec<u8>>,
    field_name: &str,
) -> StorageResult<Option<[u8; 32]>> {
    match bytes {
        Some(b) => Ok(Some(bytes_to_hash32(b, field_name)?)),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bytes_to_hash32_success() {
        let bytes = vec![1u8; 32];
        let result = bytes_to_hash32(bytes, "test.hash");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), [1u8; 32]);
    }

    #[test]
    fn test_bytes_to_hash32_wrong_length() {
        let bad_bytes = vec![1u8; 16];
        let result = bytes_to_hash32(bad_bytes, "block.parent_hash");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("block.parent_hash"));
        assert!(err.contains("expected 32"));
        assert!(err.contains("got 16"));
    }

    #[test]
    fn test_bytes_to_hash32_empty() {
        let empty = vec![];
        let result = bytes_to_hash32(empty, "test.field");
        let err = result.unwrap_err().to_string();
        assert!(err.contains("got 0"));
    }

    #[test]
    fn test_strict_accepts_valid_hash() {
        let valid = vec![1u8; 32];
        let result = bytes_to_hash32_strict(valid, "block.hash");
        assert!(result.is_ok());
    }

    #[test]
    fn test_strict_rejects_zero_hash() {
        let zeros = vec![0u8; 32];
        let result = bytes_to_hash32_strict(zeros, "block.hash");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("corruption"));
    }

    #[test]
    fn test_strict_rejects_wrong_length() {
        let bad = vec![1u8; 31];
        let result = bytes_to_hash32_strict(bad, "block.hash");
        assert!(result.is_err());
    }

    #[test]
    fn test_optional_none() {
        let result = bytes_to_optional_hash32(None, "block.author");
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn test_optional_some_valid() {
        let bytes = vec![42u8; 32];
        let result = bytes_to_optional_hash32(Some(bytes), "block.author");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some([42u8; 32]));
    }

    #[test]
    fn test_optional_some_invalid() {
        let bad = vec![1u8; 10];
        let result = bytes_to_optional_hash32(Some(bad), "block.author");
        assert!(result.is_err());
    }
}
