//! Handler for the ATS (Allfeat Timestamp) pallet.
//!
//! This handler processes events from the Allfeat ATS pallet and extracts
//! timestamped work registrations, versions, and revocations.
//!
//! # Supported Events
//!
//! - `AtsCreated`: New ATS work registration
//! - `AtsUpdated`: New version of an existing ATS
//! - `AtsRevoked`: ATS revocation (deletion)

use std::sync::Arc;

use async_trait::async_trait;
use tracing::{debug, warn};

use maestro_core::error::DomainResult;
use maestro_core::models::Block;
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

use super::models::{AtsVersion, AtsWork};
use super::storage::AtsStorage;
use crate::utils::{extract_field, parse_account, parse_hash256, parse_u8, parse_u32, parse_u64};

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

    /// Process an AtsCreated event.
    fn process_ats_created(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> Option<(AtsWork, AtsVersion)> {
        let data = &event.data;

        let ats_id = extract_field(data, &["ats_id"], 0, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in AtsCreated"
            );
            None
        })?;

        let owner = extract_field(data, &["owner"], 1, parse_account).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'owner' in AtsCreated"
            );
            None
        })?;

        let commitment = extract_field(data, &["commitment"], 2, parse_hash256).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'commitment' in AtsCreated"
            );
            None
        })?;

        let protocol_version =
            extract_field(data, &["protocol_version"], 3, parse_u8).or_else(|| {
                warn!(
                    block = block.number,
                    event = event.index,
                    "Failed to parse 'protocol_version' in AtsCreated"
                );
                None
            })?;

        let work = AtsWork {
            id: ats_id,
            owner,
            created_at_block: block.number,
            created_at_timestamp: block.timestamp,
            latest_version: 1,
        };

        let version = AtsVersion {
            id: format!("{}-{}", ats_id, 1),
            ats_id,
            version: 1,
            commitment,
            protocol_version,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            event_index: event.index,
        };

        Some((work, version))
    }

    /// Process an AtsUpdated event.
    fn process_ats_updated(&self, event: &RawEvent, block: &Block) -> Option<(u64, AtsVersion)> {
        let data = &event.data;

        let ats_id = extract_field(data, &["ats_id"], 0, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in AtsUpdated"
            );
            None
        })?;

        let version_num = extract_field(data, &["version"], 1, parse_u32).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'version' in AtsUpdated"
            );
            None
        })?;

        let commitment = extract_field(data, &["commitment"], 2, parse_hash256).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'commitment' in AtsUpdated"
            );
            None
        })?;

        let protocol_version =
            extract_field(data, &["protocol_version"], 3, parse_u8).or_else(|| {
                warn!(
                    block = block.number,
                    event = event.index,
                    "Failed to parse 'protocol_version' in AtsUpdated"
                );
                None
            })?;

        let version = AtsVersion {
            id: format!("{}-{}", ats_id, version_num),
            ats_id,
            version: version_num,
            commitment,
            protocol_version,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            event_index: event.index,
        };

        Some((ats_id, version))
    }

    /// Process an AtsRevoked event.
    fn process_ats_revoked(&self, event: &RawEvent, block: &Block) -> Option<u64> {
        let data = &event.data;

        let ats_id = extract_field(data, &["ats_id"], 0, parse_u64).or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'ats_id' in AtsRevoked"
            );
            None
        })?;

        Some(ats_id)
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
            "AtsCreated" => {
                if let Some((work, version)) = self.process_ats_created(event, block) {
                    debug!(
                        block = block.number,
                        ats_id = work.id,
                        owner = %hex::encode(work.owner.0),
                        "ATS created"
                    );
                    outputs.add("ats", "works", &work)?;
                    outputs.add("ats", "versions", &version)?;
                }
            }
            "AtsUpdated" => {
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
            "AtsRevoked" => {
                if let Some(ats_id) = self.process_ats_revoked(event, block) {
                    debug!(block = block.number, ats_id = ats_id, "ATS revoked");
                    outputs.add("ats", "revocations", ats_id)?;
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

        // Handle revocations (delete ATS work + cascaded versions)
        let revocations: Vec<u64> = outputs.get_typed("ats", "revocations");
        for ats_id in &revocations {
            if let Err(e) = self.storage.delete_ats_work(*ats_id).await {
                warn!(
                    block = block.number,
                    ats_id = ats_id,
                    error = ?e,
                    "Failed to delete revoked ATS work"
                );
                return Err(e.into());
            }
        }

        if !works.is_empty() || !versions.is_empty() || !revocations.is_empty() {
            debug!(
                block = block.number,
                works = works.len(),
                versions = versions.len(),
                revocations = revocations.len(),
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

    use super::super::storage::AtsWorkFilter;

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
        async fn list_ats_by_owner(
            &self,
            _owner: &maestro_core::models::AccountId,
        ) -> StorageResult<Vec<AtsWork>> {
            Ok(vec![])
        }
        async fn count_ats_works(&self) -> StorageResult<u64> {
            Ok(0)
        }
        async fn count_ats_by_owner(
            &self,
            _owner: &maestro_core::models::AccountId,
        ) -> StorageResult<u64> {
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
        async fn find_by_commitment(&self, _hash: &[u8; 32]) -> StorageResult<Option<AtsVersion>> {
            Ok(None)
        }
        async fn delete_ats_work(&self, _id: u64) -> StorageResult<()> {
            Ok(())
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
    fn test_process_ats_created_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "AtsCreated",
            json!({
                "ats_id": 42,
                "owner": "0x".to_string() + &"ab".repeat(32),
                "commitment": "0x".to_string() + &"cd".repeat(32),
                "protocol_version": 1
            }),
        );

        let result = handler.process_ats_created(&event, &block);
        assert!(result.is_some());

        let (work, version) = result.unwrap();
        assert_eq!(work.id, 42);
        assert_eq!(work.owner.0, [0xab; 32]);
        assert_eq!(work.created_at_block, 100);
        assert_eq!(work.latest_version, 1);

        assert_eq!(version.ats_id, 42);
        assert_eq!(version.version, 1);
        assert_eq!(version.commitment, [0xcd; 32]);
        assert_eq!(version.protocol_version, 1);
    }

    #[test]
    fn test_process_ats_created_missing_owner() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(100);

        let event = mock_event(
            "AtsCreated",
            json!({
                "ats_id": 42,
                "commitment": "0x".to_string() + &"cd".repeat(32),
                "protocol_version": 1
            }),
        );

        let result = handler.process_ats_created(&event, &block);
        assert!(result.is_none());
    }

    #[test]
    fn test_process_ats_updated_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(200);

        let event = mock_event(
            "AtsUpdated",
            json!({
                "ats_id": 42,
                "version": 3,
                "commitment": "0x".to_string() + &"ef".repeat(32),
                "protocol_version": 2
            }),
        );

        let result = handler.process_ats_updated(&event, &block);
        assert!(result.is_some());

        let (ats_id, version) = result.unwrap();
        assert_eq!(ats_id, 42);
        assert_eq!(version.ats_id, 42);
        assert_eq!(version.version, 3);
        assert_eq!(version.commitment, [0xef; 32]);
        assert_eq!(version.protocol_version, 2);
        assert_eq!(version.registered_at_block, 200);
    }

    #[test]
    fn test_process_ats_revoked_valid() {
        let handler = AtsHandler::new(Arc::new(MockStorage));
        let block = mock_block(300);

        let event = mock_event(
            "AtsRevoked",
            json!({
                "ats_id": 99,
                "owner": "0x".to_string() + &"aa".repeat(32)
            }),
        );

        let result = handler.process_ats_revoked(&event, &block);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), 99);
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
