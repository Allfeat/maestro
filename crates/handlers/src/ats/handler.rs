//! Handler for the ATS (Allfeat Timestamp) pallet.
//!
//! This handler processes events from the Allfeat ATS pallet and extracts
//! timestamped work registrations, versions, and ownership transfers.
//!
//! # Supported Events
//!
//! - `ATSRegistered`: New ATS work registration
//! - `ATSUpdated`: New version of an existing ATS
//! - `ATSClaimed`: Ownership transfer via ZKP
//! - `VerificationKeyUpdated`: ZKP verification key update

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use maestro_core::error::DomainResult;
use maestro_core::models::{AccountId, Block};
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

use super::models::{AtsOwnershipTransfer, AtsVerificationKeyUpdate, AtsVersion, AtsWork};
use super::storage::AtsStorage;
use crate::utils::{extract_field, parse_account, parse_bytes, parse_hash256, parse_u32, parse_u64};

// =============================================================================
// Handler
// =============================================================================

/// Handler for the ATS pallet.
///
/// Extracts ATS events and persists them using its own storage.
pub struct AtsHandler {
    storage: Arc<dyn AtsStorage>,
}

impl AtsHandler {
    pub fn new(storage: Arc<dyn AtsStorage>) -> Self {
        Self { storage }
    }

    /// Process an ATSRegistered event.
    fn process_ats_registered(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> Option<(AtsWork, AtsVersion)> {
        let data = &event.data;

        let provider = extract_field(data, &["provider"], 0, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'provider' in ATSRegistered"
            );
            None
        })?;

        let ats_id = extract_field(data, &["ats_id"], 1, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in ATSRegistered"
            );
            None
        })?;

        let hash_commitment =
            extract_field(data, &["hash_commitment"], 2, parse_hash256).or_else(|| {
                warn!(
                    block = block.number,
                    event = event.index,
                    "Failed to parse 'hash_commitment' in ATSRegistered"
                );
                None
            })?;

        let work = AtsWork {
            id: ats_id,
            owner: provider,
            created_at_block: block.number,
            created_at_timestamp: block.timestamp,
            latest_version: 1,
        };

        let version = AtsVersion {
            id: format!("{}-{}", ats_id, 1),
            ats_id,
            version: 1,
            hash_commitment,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            event_index: event.index,
        };

        Some((work, version))
    }

    /// Process an ATSUpdated event.
    fn process_ats_updated(&self, event: &RawEvent, block: &Block) -> Option<(u64, AtsVersion)> {
        let data = &event.data;

        let ats_id = extract_field(data, &["ats_id"], 1, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in ATSUpdated"
            );
            None
        })?;

        let version_num = extract_field(data, &["version"], 2, parse_u32).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'version' in ATSUpdated"
            );
            None
        })?;

        let hash_commitment =
            extract_field(data, &["hash_commitment"], 3, parse_hash256).or_else(|| {
                warn!(
                    block = block.number,
                    event = event.index,
                    "Failed to parse 'hash_commitment' in ATSUpdated"
                );
                None
            })?;

        let version = AtsVersion {
            id: format!("{}-{}", ats_id, version_num),
            ats_id,
            version: version_num,
            hash_commitment,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            event_index: event.index,
        };

        Some((ats_id, version))
    }

    /// Process an ATSClaimed event.
    fn process_ats_claimed(&self, event: &RawEvent, block: &Block) -> Option<AtsOwnershipTransfer> {
        let data = &event.data;

        let old_owner = extract_field(data, &["old_owner"], 0, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'old_owner' in ATSClaimed"
            );
            None
        })?;

        let new_owner = extract_field(data, &["new_owner"], 1, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'new_owner' in ATSClaimed"
            );
            None
        })?;

        let ats_id = extract_field(data, &["ats_id"], 2, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in ATSClaimed"
            );
            None
        })?;

        Some(AtsOwnershipTransfer {
            id: format!("{}-{}", block.number, event.index),
            ats_id,
            old_owner,
            new_owner,
            block_number: block.number,
            event_index: event.index,
            timestamp: block.timestamp,
        })
    }

    /// Process a VerificationKeyUpdated event.
    fn process_vk_updated(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> Option<AtsVerificationKeyUpdate> {
        let data = &event.data;

        let vk = extract_field(data, &["vk"], 0, parse_bytes).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'vk' in VerificationKeyUpdated"
            );
            None
        })?;

        Some(AtsVerificationKeyUpdate {
            id: format!("{}-{}", block.number, event.index),
            vk,
            block_number: block.number,
            event_index: event.index,
            timestamp: block.timestamp,
        })
    }
}

#[async_trait]
impl PalletHandler for AtsHandler {
    fn pallet_name(&self) -> &'static str {
        "Ats"
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        match event.name.as_str() {
            "ATSRegistered" => {
                if let Some((work, version)) = self.process_ats_registered(event, block) {
                    debug!(
                        block = block.number,
                        ats_id = work.id,
                        owner = %hex::encode(work.owner.0),
                        "ATS registered"
                    );
                    outputs.add("ats", "works", &work)?;
                    outputs.add("ats", "versions", &version)?;
                }
            }
            "ATSUpdated" => {
                if let Some((ats_id, version)) = self.process_ats_updated(event, block) {
                    debug!(
                        block = block.number,
                        ats_id = ats_id,
                        version = version.version,
                        "ATS updated"
                    );
                    outputs.add("ats", "versions", &version)?;
                    outputs.add("ats", "version_updates", (ats_id, version.version))?;
                }
            }
            "ATSClaimed" => {
                if let Some(transfer) = self.process_ats_claimed(event, block) {
                    debug!(
                        block = block.number,
                        ats_id = transfer.ats_id,
                        old_owner = %hex::encode(transfer.old_owner.0),
                        new_owner = %hex::encode(transfer.new_owner.0),
                        "ATS claimed"
                    );
                    outputs.add("ats", "transfers", &transfer)?;
                    outputs.add(
                        "ats",
                        "owner_updates",
                        (transfer.ats_id, transfer.new_owner.clone()),
                    )?;
                }
            }
            "VerificationKeyUpdated" => {
                if let Some(vk_update) = self.process_vk_updated(event, block) {
                    debug!(
                        block = block.number,
                        vk_size = vk_update.vk.len(),
                        "Verification key updated"
                    );
                    outputs.add("ats", "vk_updates", &vk_update)?;
                }
            }
            _ => {}
        }

        Ok(outputs)
    }

    async fn on_block_end(
        &self,
        block: &Block,
        outputs: &HandlerOutputs,
    ) -> DomainResult<HandlerOutputs> {
        // Persist ATS works
        let works: Vec<AtsWork> = outputs.get_typed("ats", "works");
        for work in &works {
            if let Err(e) = self.storage.insert_ats_work(work).await {
                warn!(
                    block = block.number,
                    ats_id = work.id,
                    error = ?e,
                    "Failed to persist ATS work"
                );
                return Err(e.into());
            }
        }

        // Persist ATS versions
        let versions: Vec<AtsVersion> = outputs.get_typed("ats", "versions");
        for version in &versions {
            if let Err(e) = self.storage.insert_ats_version(version).await {
                warn!(
                    block = block.number,
                    ats_id = version.ats_id,
                    version = version.version,
                    error = ?e,
                    "Failed to persist ATS version"
                );
                return Err(e.into());
            }
        }

        // Update latest versions for existing ATS
        let version_updates: Vec<(u64, u32)> = outputs.get_typed("ats", "version_updates");
        for (ats_id, new_version) in &version_updates {
            if let Err(e) = self
                .storage
                .update_ats_latest_version(*ats_id, *new_version)
                .await
            {
                warn!(
                    block = block.number,
                    ats_id = ats_id,
                    version = new_version,
                    error = ?e,
                    "Failed to update ATS latest version"
                );
                return Err(e.into());
            }
        }

        // Persist ownership transfers
        let transfers: Vec<AtsOwnershipTransfer> = outputs.get_typed("ats", "transfers");
        for transfer in &transfers {
            if let Err(e) = self.storage.insert_ownership_transfer(transfer).await {
                warn!(
                    block = block.number,
                    ats_id = transfer.ats_id,
                    error = ?e,
                    "Failed to persist ATS ownership transfer"
                );
                return Err(e.into());
            }
        }

        // Update owners for claimed ATS
        let owner_updates: Vec<(u64, AccountId)> = outputs.get_typed("ats", "owner_updates");
        for (ats_id, new_owner) in &owner_updates {
            if let Err(e) = self.storage.update_ats_owner(*ats_id, new_owner).await {
                warn!(
                    block = block.number,
                    ats_id = ats_id,
                    error = ?e,
                    "Failed to update ATS owner"
                );
                return Err(e.into());
            }
        }

        // Persist verification key updates
        let vk_updates: Vec<AtsVerificationKeyUpdate> = outputs.get_typed("ats", "vk_updates");
        for vk_update in &vk_updates {
            if let Err(e) = self.storage.insert_vk_update(vk_update).await {
                warn!(
                    block = block.number,
                    error = ?e,
                    "Failed to persist verification key update"
                );
                return Err(e.into());
            }
        }

        if !works.is_empty()
            || !versions.is_empty()
            || !transfers.is_empty()
            || !vk_updates.is_empty()
        {
            debug!(
                block = block.number,
                works = works.len(),
                versions = versions.len(),
                transfers = transfers.len(),
                vk_updates = vk_updates.len(),
                "ATS data persisted"
            );
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        10
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use maestro_core::error::StorageResult;
    use maestro_core::models::BlockHash;
    use maestro_core::ports::{Connection, OrderDirection, PageInfo, Pagination};
    use serde_json::json;

    use super::super::storage::{AtsTransferFilter, AtsWorkFilter};

    /// Mock storage for testing (doesn't persist anything).
    struct MockStorage;

    #[async_trait]
    impl AtsStorage for MockStorage {
        async fn insert_ats_work(&self, _work: &AtsWork) -> StorageResult<()> {
            Ok(())
        }
        async fn get_ats_work(&self, _id: u64) -> StorageResult<Option<AtsWork>> {
            Ok(None)
        }
        async fn update_ats_owner(&self, _id: u64, _owner: &AccountId) -> StorageResult<()> {
            Ok(())
        }
        async fn update_ats_latest_version(&self, _id: u64, _version: u32) -> StorageResult<()> {
            Ok(())
        }
        async fn list_ats_works(
            &self,
            _filter: AtsWorkFilter,
            _pagination: Pagination,
            _order: OrderDirection,
        ) -> StorageResult<Connection<AtsWork>> {
            Ok(Connection {
                edges: vec![],
                page_info: PageInfo {
                    has_next_page: false,
                    has_previous_page: false,
                    start_cursor: None,
                    end_cursor: None,
                },
                total_count: Some(0),
            })
        }
        async fn list_ats_by_owner(&self, _owner: &AccountId) -> StorageResult<Vec<AtsWork>> {
            Ok(vec![])
        }
        async fn count_ats_works(&self) -> StorageResult<u64> {
            Ok(0)
        }
        async fn count_ats_by_owner(&self, _owner: &AccountId) -> StorageResult<u64> {
            Ok(0)
        }
        async fn insert_ats_version(&self, _version: &AtsVersion) -> StorageResult<()> {
            Ok(())
        }
        async fn get_ats_version(
            &self,
            _ats_id: u64,
            _version: u32,
        ) -> StorageResult<Option<AtsVersion>> {
            Ok(None)
        }
        async fn list_versions_for_ats(&self, _ats_id: u64) -> StorageResult<Vec<AtsVersion>> {
            Ok(vec![])
        }
        async fn find_by_hash_commitment(
            &self,
            _hash: &[u8; 32],
        ) -> StorageResult<Option<AtsVersion>> {
            Ok(None)
        }
        async fn insert_ownership_transfer(
            &self,
            _transfer: &AtsOwnershipTransfer,
        ) -> StorageResult<()> {
            Ok(())
        }
        async fn list_transfers_for_ats(
            &self,
            _ats_id: u64,
        ) -> StorageResult<Vec<AtsOwnershipTransfer>> {
            Ok(vec![])
        }
        async fn list_transfers(
            &self,
            _filter: AtsTransferFilter,
            _pagination: Pagination,
            _order: OrderDirection,
        ) -> StorageResult<Connection<AtsOwnershipTransfer>> {
            Ok(Connection {
                edges: vec![],
                page_info: PageInfo {
                    has_next_page: false,
                    has_previous_page: false,
                    start_cursor: None,
                    end_cursor: None,
                },
                total_count: Some(0),
            })
        }
        async fn insert_vk_update(&self, _update: &AtsVerificationKeyUpdate) -> StorageResult<()> {
            Ok(())
        }
        async fn get_latest_vk(&self) -> StorageResult<Option<AtsVerificationKeyUpdate>> {
            Ok(None)
        }
        async fn list_vk_updates(&self) -> StorageResult<Vec<AtsVerificationKeyUpdate>> {
            Ok(vec![])
        }
        async fn delete_from_block(&self, _from_block: u64) -> StorageResult<u64> {
            Ok(0)
        }
    }

    fn mock_block(number: u64) -> Block {
        Block {
            number,
            hash: BlockHash([0xaa; 32]),
            parent_hash: BlockHash([0xbb; 32]),
            state_root: BlockHash([0xcc; 32]),
            extrinsics_root: BlockHash([0xdd; 32]),
            author: None,
            timestamp: Some(Utc::now()),
            extrinsic_count: 0,
            event_count: 0,
            indexed_at: Utc::now(),
        }
    }

    fn mock_event(name: &str, data: serde_json::Value) -> RawEvent {
        RawEvent {
            index: 0,
            extrinsic_index: Some(0),
            pallet: "Ats".to_string(),
            name: name.to_string(),
            data,
            topics: vec![],
        }
    }

    #[test]
    fn test_process_ats_registered_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "ATSRegistered",
            json!({
                "provider": "0x".to_string() + &"ab".repeat(32),
                "ats_id": 42,
                "hash_commitment": "0x".to_string() + &"cd".repeat(32)
            }),
        );

        let result = handler.process_ats_registered(&event, &block);
        assert!(result.is_some());

        let (work, version) = result.unwrap();
        assert_eq!(work.id, 42);
        assert_eq!(work.owner.0, [0xab; 32]);
        assert_eq!(work.created_at_block, 100);
        assert_eq!(work.latest_version, 1);

        assert_eq!(version.ats_id, 42);
        assert_eq!(version.version, 1);
        assert_eq!(version.hash_commitment, [0xcd; 32]);
    }

    #[test]
    fn test_process_ats_registered_missing_provider() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "ATSRegistered",
            json!({
                "ats_id": 42,
                "hash_commitment": "0x".to_string() + &"cd".repeat(32)
            }),
        );

        let result = handler.process_ats_registered(&event, &block);
        assert!(result.is_none());
    }

    #[test]
    fn test_process_ats_updated_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(200);

        let event = mock_event(
            "ATSUpdated",
            json!({
                "provider": "0x".to_string() + &"11".repeat(32), // position 0
                "ats_id": 42,                                     // position 1
                "version": 3,                                     // position 2
                "hash_commitment": "0x".to_string() + &"ef".repeat(32) // position 3
            }),
        );

        let result = handler.process_ats_updated(&event, &block);
        assert!(result.is_some());

        let (ats_id, version) = result.unwrap();
        assert_eq!(ats_id, 42);
        assert_eq!(version.ats_id, 42);
        assert_eq!(version.version, 3);
        assert_eq!(version.hash_commitment, [0xef; 32]);
        assert_eq!(version.registered_at_block, 200);
    }

    #[test]
    fn test_process_ats_claimed_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(300);

        let event = mock_event(
            "ATSClaimed",
            json!({
                "old_owner": "0x".to_string() + &"aa".repeat(32),
                "new_owner": "0x".to_string() + &"bb".repeat(32),
                "ats_id": 99
            }),
        );

        let result = handler.process_ats_claimed(&event, &block);
        assert!(result.is_some());

        let transfer = result.unwrap();
        assert_eq!(transfer.ats_id, 99);
        assert_eq!(transfer.old_owner.0, [0xaa; 32]);
        assert_eq!(transfer.new_owner.0, [0xbb; 32]);
        assert_eq!(transfer.block_number, 300);
    }

    #[test]
    fn test_process_vk_updated_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(400);

        let event = mock_event(
            "VerificationKeyUpdated",
            json!({
                "vk": "0xdeadbeefcafe"
            }),
        );

        let result = handler.process_vk_updated(&event, &block);
        assert!(result.is_some());

        let vk_update = result.unwrap();
        assert_eq!(vk_update.vk, vec![0xde, 0xad, 0xbe, 0xef, 0xca, 0xfe]);
        assert_eq!(vk_update.block_number, 400);
    }

    #[test]
    fn test_pallet_name() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        assert_eq!(handler.pallet_name(), "Ats");
    }

    #[test]
    fn test_priority() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        assert_eq!(handler.priority(), 10);
    }
}
