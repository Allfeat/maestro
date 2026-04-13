//! `StorageReader` implementation for `SubstrateClient`.

use async_trait::async_trait;

use maestro_core::error::{ChainError, ChainResult};
use maestro_core::models::BlockHash;
use maestro_core::ports::StorageReader;
use subxt::error::StorageError;

use crate::client::SubstrateClient;

/// Translate a `fetch_raw` result: `NoValueFound` -> `Ok(None)`, any other
/// error -> `Err(ChainError::RpcError)`.
fn optional_fetch_raw(result: Result<Vec<u8>, StorageError>) -> ChainResult<Option<Vec<u8>>> {
    match result {
        Ok(bytes) => Ok(Some(bytes)),
        Err(StorageError::NoValueFound) => Ok(None),
        Err(e) => Err(ChainError::RpcError(format!(
            "Failed to fetch storage: {}",
            e
        ))),
    }
}

#[async_trait]
impl StorageReader for SubstrateClient {
    async fn read_storage(
        &self,
        block_hash: &BlockHash,
        key: &[u8],
    ) -> ChainResult<Option<Vec<u8>>> {
        let hash = subxt::utils::H256::from_slice(&block_hash.0);

        let at_block = self
            .client
            .at_block(hash)
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to resolve block: {}", e)))?;

        optional_fetch_raw(at_block.storage().fetch_raw(key.to_vec()).await)
    }

    async fn read_storage_map(
        &self,
        block_hash: &BlockHash,
        pallet: &str,
        item: &str,
        map_key: &[u8],
    ) -> ChainResult<Option<Vec<u8>>> {
        let hash = subxt::utils::H256::from_slice(&block_hash.0);

        // High-level dynamic storage (see subxt/examples/dynamic.rs): the
        // runtime metadata drives the hasher, and the typed-tuple key tells
        // subxt how to scale-encode the lookup value.
        let addr = subxt::dynamic::storage::<(Vec<u8>,), subxt::dynamic::Value>(pallet, item);

        let at_block = self
            .client
            .at_block(hash)
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to resolve block: {}", e)))?;

        let value = at_block
            .storage()
            .try_fetch(addr, (map_key.to_vec(),))
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to fetch storage: {}", e)))?;

        Ok(value.map(|v| v.bytes().to_vec()))
    }

    async fn read_storage_map_u64(
        &self,
        block_hash: &BlockHash,
        pallet: &str,
        item: &str,
        key: u64,
    ) -> ChainResult<Option<Vec<u8>>> {
        let hash = subxt::utils::H256::from_slice(&block_hash.0);

        // High-level dynamic storage. The stored key is `u128` because
        // runtime-side indices are typed that way (matches the pre-refactor
        // `Value::u128(key as u128)` encoding; guarded by the unit test in
        // this module).
        let addr = subxt::dynamic::storage::<(u128,), subxt::dynamic::Value>(pallet, item);

        let at_block = self
            .client
            .at_block(hash)
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to resolve block: {}", e)))?;

        let value = at_block
            .storage()
            .try_fetch(addr, (key as u128,))
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to fetch storage: {}", e)))?;

        Ok(value.map(|v| v.bytes().to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use codec::Encode;
    use subxt::dynamic::Value;

    /// Reachability guard: the typed-tuple dynamic-storage builder shapes
    /// used by `read_storage_map_u64` and `read_storage_map` must compile
    /// against subxt's public trait bounds. If a future subxt release
    /// restricts the `Keys` type parameter so that these tuples no longer
    /// satisfy it, this test catches it before the manual smoke run.
    /// Replaces the pre-refactor `dynamic_storage_map_builder_constructs_for_known_pallet`.
    #[test]
    fn typed_tuple_dynamic_storage_builders_compile() {
        let _u64_shape = subxt::dynamic::storage::<(u128,), Value>("System", "Account");
        let _bytes_shape = subxt::dynamic::storage::<(Vec<u8>,), Value>("System", "Account");
    }

    /// SCALE-framing guard for spec §4.7 R2: a single-element Rust tuple
    /// SCALE-encodes to the same bytes as its inner value. This pins the
    /// framing of the new typed-tuple keys at the codec layer, so Task 6
    /// cannot silently add extra length prefixes or padding. The full
    /// byte-equivalence check against the pre-refactor `Value::u128` path
    /// runs against a live runtime in Task 9 (manual smoke).
    #[test]
    fn single_element_u128_tuple_encodes_as_plain_u128() {
        let k: u128 = 0x0123_4567_89ab_cdef_0123_4567_89ab_cdef;
        let tuple_bytes = (k,).encode();
        let plain_bytes = k.encode();
        assert_eq!(tuple_bytes, plain_bytes);
        assert_eq!(tuple_bytes.len(), 16, "u128 is 16 little-endian bytes");
    }

    /// Same framing guard, byte-key variant. `Vec<u8>` SCALE-encodes with a
    /// compact length prefix, and a single-element tuple wrapping it adds
    /// nothing. Pins the framing for `read_storage_map`.
    #[test]
    fn single_element_bytes_tuple_encodes_as_plain_bytes() {
        let raw: Vec<u8> = (0u8..32).collect();
        let tuple_bytes = (raw.clone(),).encode();
        let plain_bytes = raw.encode();
        assert_eq!(tuple_bytes, plain_bytes);
    }
}
