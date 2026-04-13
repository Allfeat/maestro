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

        // Use subxt's dynamic storage API to construct the storage address.
        // The runtime metadata determines the hasher, making this more robust
        // than manual key construction.
        let storage_query = subxt::dynamic::storage::<
            Vec<subxt::dynamic::Value>,
            subxt::dynamic::Value,
        >(pallet, item);

        let at_block = self
            .client
            .at_block(hash)
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to resolve block: {}", e)))?;

        let entry = at_block.storage().entry(storage_query).map_err(|e| {
            ChainError::RpcError(format!("Failed to construct storage entry: {}", e))
        })?;

        match entry
            .try_fetch(vec![subxt::dynamic::Value::from_bytes(map_key)])
            .await
        {
            Ok(Some(value)) => Ok(Some(value.bytes().to_vec())),
            Ok(None) => Ok(None),
            Err(e) => Err(ChainError::RpcError(format!(
                "Failed to fetch storage: {}",
                e
            ))),
        }
    }

    async fn read_storage_map_u64(
        &self,
        block_hash: &BlockHash,
        pallet: &str,
        item: &str,
        key: u64,
    ) -> ChainResult<Option<Vec<u8>>> {
        let hash = subxt::utils::H256::from_slice(&block_hash.0);

        // Dynamic storage: the runtime metadata determines the hasher and
        // correctly encodes the u128 key.
        let storage_query = subxt::dynamic::storage::<
            Vec<subxt::dynamic::Value>,
            subxt::dynamic::Value,
        >(pallet, item);

        let at_block = self
            .client
            .at_block(hash)
            .await
            .map_err(|e| ChainError::RpcError(format!("Failed to resolve block: {}", e)))?;

        let entry = at_block.storage().entry(storage_query).map_err(|e| {
            ChainError::RpcError(format!("Failed to construct storage entry: {}", e))
        })?;

        match entry
            .try_fetch(vec![subxt::dynamic::Value::u128(key as u128)])
            .await
        {
            Ok(Some(value)) => Ok(Some(value.bytes().to_vec())),
            Ok(None) => Ok(None),
            Err(e) => Err(ChainError::RpcError(format!(
                "Failed to fetch storage: {}",
                e
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn dynamic_storage_map_builder_constructs_for_known_pallet() {
        // Reachability check: the subxt dynamic storage builder signature we rely on
        // in read_storage_map_u64 must compile for a known pallet/item pair.
        // Catches regressions where the builder shape silently drifts on upgrade.
        let _query = subxt::dynamic::storage::<Vec<subxt::dynamic::Value>, subxt::dynamic::Value>(
            "System", "Account",
        );
    }
}
