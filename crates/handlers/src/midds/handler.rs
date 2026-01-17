//! Handlers for MIDDS (Musical Industry Digital Distribution Standard) pallets.
//!
//! This module contains three handlers for the three MIDDS pallet instances:
//! - MusicalWorksHandler: Handles MusicalWorks pallet events
//! - RecordingsHandler: Handles Recordings pallet events
//! - ReleasesHandler: Handles Releases pallet events
//!
//! Each handler reads the full MIDDS data from on-chain storage when a
//! MIDDSRegistered event is received, since the event only contains minimal info.

use std::sync::Arc;

use async_trait::async_trait;
use codec::Decode;
use tracing::{debug, warn};

use maestro_core::error::{DomainError, DomainResult};
use maestro_core::models::{Block, BlockHash};
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic, StorageReader};

use super::models::{
    Creator, Date, MusicalWork, PartyId, Recording, Release, country_to_string, format_to_string,
    key_to_string, language_to_string, packaging_to_string, release_type_to_string,
    status_to_string, version_to_string, work_type_to_string,
};
use super::storage::MiddsStorage;
use crate::utils::{extract_field, parse_account, parse_amount, parse_u64};

// =============================================================================
// Musical Works Handler
// =============================================================================

/// Handler for the MusicalWorks pallet (MIDDS for musical compositions).
pub struct MusicalWorksHandler {
    storage: Arc<dyn MiddsStorage>,
    chain_reader: Arc<dyn StorageReader>,
}

impl MusicalWorksHandler {
    pub fn new(storage: Arc<dyn MiddsStorage>, chain_reader: Arc<dyn StorageReader>) -> Self {
        Self {
            storage,
            chain_reader,
        }
    }

    /// Fetch musical work data from on-chain storage.
    async fn fetch_musical_work_data(
        &self,
        block_hash: &BlockHash,
        midds_id: u64,
    ) -> DomainResult<Option<allfeat_midds::musical_work::MusicalWork>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, "MusicalWorks", "MiddsOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch storage: {}", e)))?;

        match bytes {
            Some(data) => {
                let work = allfeat_midds::musical_work::MusicalWork::decode(&mut &data[..])
                    .map_err(|e| {
                        maestro_core::error::DomainError::DecodingError(format!(
                            "Failed to decode MusicalWork: {}",
                            e
                        ))
                    })?;
                Ok(Some(work))
            }
            None => Ok(None),
        }
    }

    /// Fetch MiddsInfo from on-chain storage to get the hash.
    async fn fetch_midds_info(
        &self,
        block_hash: &BlockHash,
        pallet: &str,
        midds_id: u64,
    ) -> DomainResult<Option<[u8; 32]>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, pallet, "MiddsInfoOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch MiddsInfo: {}", e)))?;

        // MiddsInfo structure (SCALE encoded):
        // - provider: AccountId (32 bytes)
        // - registered_at: u64 (8 bytes) - timestamp in milliseconds
        // - hash: [u8; 32] (32 bytes)
        // - encoded_size: u32 (4 bytes)
        // - data_cost: u128 (16 bytes)
        // Total: 92 bytes
        match bytes {
            Some(data) => {
                // Skip provider (32 bytes) + registered_at (8 bytes) to get hash
                const HASH_OFFSET: usize = 32 + 8; // 40
                const HASH_END: usize = HASH_OFFSET + 32; // 72
                if data.len() >= HASH_END {
                    let mut hash = [0u8; 32];
                    hash.copy_from_slice(&data[HASH_OFFSET..HASH_END]);
                    Ok(Some(hash))
                } else {
                    Ok(None)
                }
            }
            None => Ok(None),
        }
    }

    /// Process a MIDDSRegistered event for a musical work.
    async fn process_registered(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> DomainResult<Option<MusicalWork>> {
        let data = &event.data;

        let provider = extract_field(data, &["provider"], 0, parse_account).ok_or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'provider' in MIDDSRegistered"
            );
            maestro_core::error::DomainError::DecodingError("Failed to parse provider".to_string())
        })?;

        let midds_id = extract_field(data, &["midds_id"], 1, parse_u64).ok_or_else(|| {
            warn!(
                block = block.number,
                event = event.index,
                "Failed to parse 'midds_id' in MIDDSRegistered"
            );
            maestro_core::error::DomainError::DecodingError("Failed to parse midds_id".to_string())
        })?;

        let data_cost = extract_field(data, &["data_cost", "data_colateral"], 2, parse_amount).unwrap_or(0);

        // Fetch full data from chain storage
        let block_hash = block.hash.clone();
        let work_data = self.fetch_musical_work_data(&block_hash, midds_id).await?;

        let work_data = match work_data {
            Some(w) => w,
            None => {
                warn!(
                    block = block.number,
                    midds_id = midds_id,
                    "MusicalWork not found in storage"
                );
                return Ok(None);
            }
        };

        // Get hash from MiddsInfo
        let hash = self
            .fetch_midds_info(&block_hash, "MusicalWorks", midds_id)
            .await?
            .unwrap_or([0u8; 32]);

        // Convert to domain model
        let work = MusicalWork {
            id: midds_id,
            provider,
            hash,
            data_cost,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            iswc: String::from_utf8_lossy(work_data.iswc.as_slice()).to_string(),
            title: String::from_utf8_lossy(work_data.title.as_slice()).to_string(),
            creation_year: work_data.creation_year,
            instrumental: work_data.instrumental,
            language: work_data.language.as_ref().map(language_to_string),
            bpm: work_data.bpm,
            key: work_data.key.as_ref().map(key_to_string),
            work_type: work_data.work_type.as_ref().map(work_type_to_string),
            creators: work_data.creators.iter().map(Creator::from_chain).collect(),
        };

        Ok(Some(work))
    }
}

#[async_trait]
impl PalletHandler for MusicalWorksHandler {
    fn pallet_name(&self) -> &'static str {
        "MusicalWorks"
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        match event.name.as_str() {
            "MIDDSRegistered" => {
                if let Ok(Some(work)) = self.process_registered(event, block).await {
                    debug!(
                        block = block.number,
                        midds_id = work.id,
                        iswc = %work.iswc,
                        "MusicalWork registered"
                    );
                    outputs.add("midds", "musical_works", &work)?;
                }
            }
            "MIDDSUnregistered" => {
                if let Some(midds_id) = extract_field(&event.data, &["midds_id"], 0, parse_u64) {
                    debug!(
                        block = block.number,
                        midds_id = midds_id,
                        "MusicalWork unregistered"
                    );
                    outputs.add("midds", "musical_work_deletions", midds_id)?;
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
        // Persist musical works
        let works: Vec<MusicalWork> = outputs.get_typed("midds", "musical_works");
        for work in &works {
            if let Err(e) = self.storage.insert_musical_work(work).await {
                warn!(
                    block = block.number,
                    midds_id = work.id,
                    error = ?e,
                    "Failed to persist MusicalWork"
                );
                return Err(e.into());
            }
        }

        // Handle deletions
        let deletions: Vec<u64> = outputs.get_typed("midds", "musical_work_deletions");
        for id in &deletions {
            if let Err(e) = self.storage.delete_musical_work(*id).await {
                warn!(
                    block = block.number,
                    midds_id = id,
                    error = ?e,
                    "Failed to delete MusicalWork"
                );
            }
        }

        if !works.is_empty() || !deletions.is_empty() {
            debug!(
                block = block.number,
                works = works.len(),
                deletions = deletions.len(),
                "MusicalWorks data persisted"
            );
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        10
    }
}

// =============================================================================
// Recordings Handler
// =============================================================================

/// Handler for the Recordings pallet (MIDDS for audio recordings).
pub struct RecordingsHandler {
    storage: Arc<dyn MiddsStorage>,
    chain_reader: Arc<dyn StorageReader>,
}

impl RecordingsHandler {
    pub fn new(storage: Arc<dyn MiddsStorage>, chain_reader: Arc<dyn StorageReader>) -> Self {
        Self {
            storage,
            chain_reader,
        }
    }

    /// Fetch recording data from on-chain storage.
    async fn fetch_recording_data(
        &self,
        block_hash: &BlockHash,
        midds_id: u64,
    ) -> DomainResult<Option<allfeat_midds::recording::Recording>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, "Recordings", "MiddsOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch Recording: {}", e)))?;

        match bytes {
            Some(data) => {
                let recording = allfeat_midds::recording::Recording::decode(&mut &data[..])
                    .map_err(|e| {
                        maestro_core::error::DomainError::DecodingError(format!(
                            "Failed to decode Recording: {}",
                            e
                        ))
                    })?;
                Ok(Some(recording))
            }
            None => Ok(None),
        }
    }

    /// Process a MIDDSRegistered event for a recording.
    async fn process_registered(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> DomainResult<Option<Recording>> {
        let data = &event.data;

        let provider = extract_field(data, &["provider"], 0, parse_account).ok_or_else(|| {
            maestro_core::error::DomainError::DecodingError("Failed to parse provider".to_string())
        })?;

        let midds_id = extract_field(data, &["midds_id"], 1, parse_u64).ok_or_else(|| {
            maestro_core::error::DomainError::DecodingError("Failed to parse midds_id".to_string())
        })?;

        let data_cost = extract_field(data, &["data_cost", "data_colateral"], 2, parse_amount).unwrap_or(0);

        // Fetch full data from chain storage
        let block_hash = block.hash.clone();
        let rec_data = self.fetch_recording_data(&block_hash, midds_id).await?;

        let rec_data = match rec_data {
            Some(r) => r,
            None => {
                warn!(
                    block = block.number,
                    midds_id = midds_id,
                    "Recording not found in storage"
                );
                return Ok(None);
            }
        };

        // Get hash from MiddsInfo
        let hash = self
            .fetch_midds_info(&block_hash, midds_id)
            .await?
            .unwrap_or([0u8; 32]);

        // Convert to domain model
        let recording = Recording {
            id: midds_id,
            provider,
            hash,
            data_cost,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            isrc: String::from_utf8_lossy(rec_data.isrc.as_slice()).to_string(),
            musical_work_id: rec_data.musical_work,
            artist: PartyId::from_chain(&rec_data.artist),
            title: String::from_utf8_lossy(rec_data.title.as_slice()).to_string(),
            recording_year: rec_data.recording_year,
            duration: rec_data.duration,
            bpm: rec_data.bpm,
            key: rec_data.key.as_ref().map(key_to_string),
            version: rec_data.version.as_ref().map(version_to_string),
            genres: rec_data.genres.iter().map(|g| format!("{:?}", g)).collect(),
            producers: rec_data.producers.iter().map(PartyId::from_chain).collect(),
            performers: rec_data
                .performers
                .iter()
                .map(PartyId::from_chain)
                .collect(),
        };

        Ok(Some(recording))
    }

    /// Fetch MiddsInfo from on-chain storage.
    async fn fetch_midds_info(
        &self,
        block_hash: &BlockHash,
        midds_id: u64,
    ) -> DomainResult<Option<[u8; 32]>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, "Recordings", "MiddsInfoOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch MiddsInfo: {}", e)))?;

        // MiddsInfo structure (SCALE encoded):
        // - provider: AccountId (32 bytes)
        // - registered_at: u64 (8 bytes)
        // - hash: [u8; 32] (32 bytes)
        // - encoded_size: u32 (4 bytes)
        // - data_cost: u128 (16 bytes)
        const HASH_OFFSET: usize = 32 + 8;
        const HASH_END: usize = HASH_OFFSET + 32;
        match bytes {
            Some(data) if data.len() >= HASH_END => {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&data[HASH_OFFSET..HASH_END]);
                Ok(Some(hash))
            }
            _ => Ok(None),
        }
    }
}

#[async_trait]
impl PalletHandler for RecordingsHandler {
    fn pallet_name(&self) -> &'static str {
        "Recordings"
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        match event.name.as_str() {
            "MIDDSRegistered" => {
                if let Ok(Some(recording)) = self.process_registered(event, block).await {
                    debug!(
                        block = block.number,
                        midds_id = recording.id,
                        isrc = %recording.isrc,
                        "Recording registered"
                    );
                    outputs.add("midds", "recordings", &recording)?;
                }
            }
            "MIDDSUnregistered" => {
                if let Some(midds_id) = extract_field(&event.data, &["midds_id"], 0, parse_u64) {
                    debug!(
                        block = block.number,
                        midds_id = midds_id,
                        "Recording unregistered"
                    );
                    outputs.add("midds", "recording_deletions", midds_id)?;
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
        // Persist recordings
        let recordings: Vec<Recording> = outputs.get_typed("midds", "recordings");
        for recording in &recordings {
            if let Err(e) = self.storage.insert_recording(recording).await {
                warn!(
                    block = block.number,
                    midds_id = recording.id,
                    error = ?e,
                    "Failed to persist Recording"
                );
                return Err(e.into());
            }
        }

        // Handle deletions
        let deletions: Vec<u64> = outputs.get_typed("midds", "recording_deletions");
        for id in &deletions {
            if let Err(e) = self.storage.delete_recording(*id).await {
                warn!(
                    block = block.number,
                    midds_id = id,
                    error = ?e,
                    "Failed to delete Recording"
                );
            }
        }

        if !recordings.is_empty() || !deletions.is_empty() {
            debug!(
                block = block.number,
                recordings = recordings.len(),
                deletions = deletions.len(),
                "Recordings data persisted"
            );
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        10
    }
}

// =============================================================================
// Releases Handler
// =============================================================================

/// Handler for the Releases pallet (MIDDS for album releases).
pub struct ReleasesHandler {
    storage: Arc<dyn MiddsStorage>,
    chain_reader: Arc<dyn StorageReader>,
}

impl ReleasesHandler {
    pub fn new(storage: Arc<dyn MiddsStorage>, chain_reader: Arc<dyn StorageReader>) -> Self {
        Self {
            storage,
            chain_reader,
        }
    }

    /// Fetch release data from on-chain storage.
    async fn fetch_release_data(
        &self,
        block_hash: &BlockHash,
        midds_id: u64,
    ) -> DomainResult<Option<allfeat_midds::release::Release>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, "Releases", "MiddsOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch Release: {}", e)))?;

        match bytes {
            Some(data) => {
                let release =
                    allfeat_midds::release::Release::decode(&mut &data[..]).map_err(|e| {
                        maestro_core::error::DomainError::DecodingError(format!(
                            "Failed to decode Release: {}",
                            e
                        ))
                    })?;
                Ok(Some(release))
            }
            None => Ok(None),
        }
    }

    /// Fetch MiddsInfo from on-chain storage.
    async fn fetch_midds_info(
        &self,
        block_hash: &BlockHash,
        midds_id: u64,
    ) -> DomainResult<Option<[u8; 32]>> {
        let bytes = self
            .chain_reader
            .read_storage_map_u64(block_hash, "Releases", "MiddsInfoOf", midds_id)
            .await
            .map_err(|e| DomainError::DecodingError(format!("Failed to fetch MiddsInfo: {}", e)))?;

        // MiddsInfo structure (SCALE encoded):
        // - provider: AccountId (32 bytes)
        // - registered_at: u64 (8 bytes)
        // - hash: [u8; 32] (32 bytes)
        // - encoded_size: u32 (4 bytes)
        // - data_cost: u128 (16 bytes)
        const HASH_OFFSET: usize = 32 + 8;
        const HASH_END: usize = HASH_OFFSET + 32;
        match bytes {
            Some(data) if data.len() >= HASH_END => {
                let mut hash = [0u8; 32];
                hash.copy_from_slice(&data[HASH_OFFSET..HASH_END]);
                Ok(Some(hash))
            }
            _ => Ok(None),
        }
    }

    /// Process a MIDDSRegistered event for a release.
    async fn process_registered(
        &self,
        event: &RawEvent,
        block: &Block,
    ) -> DomainResult<Option<Release>> {
        let data = &event.data;

        let provider = extract_field(data, &["provider"], 0, parse_account).ok_or_else(|| {
            maestro_core::error::DomainError::DecodingError("Failed to parse provider".to_string())
        })?;

        let midds_id = extract_field(data, &["midds_id"], 1, parse_u64).ok_or_else(|| {
            maestro_core::error::DomainError::DecodingError("Failed to parse midds_id".to_string())
        })?;

        let data_cost = extract_field(data, &["data_cost", "data_colateral"], 2, parse_amount).unwrap_or(0);

        // Fetch full data from chain storage
        let block_hash = block.hash.clone();
        let rel_data = self.fetch_release_data(&block_hash, midds_id).await?;

        let rel_data = match rel_data {
            Some(r) => r,
            None => {
                warn!(
                    block = block.number,
                    midds_id = midds_id,
                    "Release not found in storage"
                );
                return Ok(None);
            }
        };

        // Get hash from MiddsInfo
        let hash = self
            .fetch_midds_info(&block_hash, midds_id)
            .await?
            .unwrap_or([0u8; 32]);

        // Convert to domain model
        let release = Release {
            id: midds_id,
            provider,
            hash,
            data_cost,
            registered_at_block: block.number,
            registered_at_timestamp: block.timestamp,
            ean_upc: String::from_utf8_lossy(rel_data.ean_upc.as_slice()).to_string(),
            creator: PartyId::from_chain(&rel_data.creator),
            title: String::from_utf8_lossy(rel_data.title.as_slice()).to_string(),
            release_type: release_type_to_string(&rel_data.release_type),
            format: format_to_string(&rel_data.format),
            packaging: packaging_to_string(&rel_data.packaging),
            status: status_to_string(&rel_data.status),
            date: Date::from_chain(&rel_data.date),
            country: country_to_string(&rel_data.country),
            recording_ids: rel_data.recordings.iter().copied().collect(),
            distributor_name: String::from_utf8_lossy(rel_data.distributor_name.as_slice())
                .to_string(),
            manufacturer_name: String::from_utf8_lossy(rel_data.manufacturer_name.as_slice())
                .to_string(),
        };

        Ok(Some(release))
    }
}

#[async_trait]
impl PalletHandler for ReleasesHandler {
    fn pallet_name(&self) -> &'static str {
        "Releases"
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        match event.name.as_str() {
            "MIDDSRegistered" => {
                if let Ok(Some(release)) = self.process_registered(event, block).await {
                    debug!(
                        block = block.number,
                        midds_id = release.id,
                        ean = %release.ean_upc,
                        "Release registered"
                    );
                    outputs.add("midds", "releases", &release)?;
                }
            }
            "MIDDSUnregistered" => {
                if let Some(midds_id) = extract_field(&event.data, &["midds_id"], 0, parse_u64) {
                    debug!(
                        block = block.number,
                        midds_id = midds_id,
                        "Release unregistered"
                    );
                    outputs.add("midds", "release_deletions", midds_id)?;
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
        // Persist releases
        let releases: Vec<Release> = outputs.get_typed("midds", "releases");
        for release in &releases {
            if let Err(e) = self.storage.insert_release(release).await {
                warn!(
                    block = block.number,
                    midds_id = release.id,
                    error = ?e,
                    "Failed to persist Release"
                );
                return Err(e.into());
            }
        }

        // Handle deletions
        let deletions: Vec<u64> = outputs.get_typed("midds", "release_deletions");
        for id in &deletions {
            if let Err(e) = self.storage.delete_release(*id).await {
                warn!(
                    block = block.number,
                    midds_id = id,
                    error = ?e,
                    "Failed to delete Release"
                );
            }
        }

        if !releases.is_empty() || !deletions.is_empty() {
            debug!(
                block = block.number,
                releases = releases.len(),
                deletions = deletions.len(),
                "Releases data persisted"
            );
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        10
    }
}
