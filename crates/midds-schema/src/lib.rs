//! MIDDS (Musical Industry Digital Distribution Standard) GraphQL types for the Maestro blockchain indexer.
//!
//! This crate provides GraphQL schema types for the Allfeat MIDDS pallets,
//! which handle musical works, recordings, and releases.
//!
//! # Example
//!
//! ```rust
//! use maestro_midds_schema::{MusicalWork, Recording, Release, PartyId};
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use maestro_graphql_schema::PageInfo;

// =============================================================================
// Party Identifier Types
// =============================================================================

/// A party identifier (IPI, ISNI, or both).
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct PartyId {
    /// IPI (Interested Party Information) number, if available.
    pub ipi: Option<i64>,
    /// ISNI (International Standard Name Identifier), if available.
    pub isni: Option<String>,
}

/// A creator of a musical work.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct Creator {
    /// The party identifier.
    pub party_id: PartyId,
    /// The role of the creator (Author, Composer, Arranger, Adapter, Publisher).
    pub role: String,
}

/// A date (year, month, day).
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct Date {
    /// Year component.
    pub year: i32,
    /// Month component (1-12).
    pub month: i32,
    /// Day component (1-31).
    pub day: i32,
}

// =============================================================================
// Musical Work Types
// =============================================================================

/// A musical work (composition) registered on-chain.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct MusicalWork {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded with 0x prefix).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded with 0x prefix).
    pub hash: String,
    /// Cost deposited by the provider.
    pub data_cost: String,
    /// Block number when registered.
    pub registered_at_block: i64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// International Standard Musical Work Code.
    pub iswc: String,
    /// Title of the work.
    pub title: String,
    /// Year the work was created.
    pub creation_year: Option<i32>,
    /// Whether the work is instrumental (no lyrics).
    pub instrumental: Option<bool>,
    /// Language of the lyrics.
    pub language: Option<String>,
    /// Tempo in beats per minute.
    pub bpm: Option<i32>,
    /// Musical key.
    pub key: Option<String>,
    /// Type of work (Original, Medley, Mashup, Adaptation).
    pub work_type: Option<String>,
    /// List of creators with their roles.
    pub creators: Vec<Creator>,
}

/// A single musical work in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct MusicalWorkEdge {
    /// The musical work.
    pub node: MusicalWork,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of musical works.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct MusicalWorkConnection {
    /// List of musical work edges.
    pub edges: Vec<MusicalWorkEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of musical works (if available).
    pub total_count: Option<i64>,
}

// =============================================================================
// Recording Types
// =============================================================================

/// A recording registered on-chain.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct Recording {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded with 0x prefix).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded with 0x prefix).
    pub hash: String,
    /// Cost deposited by the provider.
    pub data_cost: String,
    /// Block number when registered.
    pub registered_at_block: i64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// International Standard Recording Code.
    pub isrc: String,
    /// Reference to the musical work this is a recording of.
    pub musical_work_id: i64,
    /// Main artist.
    pub artist: PartyId,
    /// Title of the recording.
    pub title: String,
    /// Year the recording was made.
    pub recording_year: Option<i32>,
    /// Duration in seconds.
    pub duration: Option<i32>,
    /// Tempo in beats per minute.
    pub bpm: Option<i32>,
    /// Musical key.
    pub key: Option<String>,
    /// Version type (Original, Live, Remix, etc.).
    pub version: Option<String>,
    /// Genre names.
    pub genres: Vec<String>,
    /// Producer party IDs.
    pub producers: Vec<PartyId>,
    /// Performer party IDs.
    pub performers: Vec<PartyId>,
}

/// A single recording in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct RecordingEdge {
    /// The recording.
    pub node: Recording,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of recordings.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct RecordingConnection {
    /// List of recording edges.
    pub edges: Vec<RecordingEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of recordings (if available).
    pub total_count: Option<i64>,
}

// =============================================================================
// Release Types
// =============================================================================

/// A release (album/single) registered on-chain.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded with 0x prefix).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded with 0x prefix).
    pub hash: String,
    /// Cost deposited by the provider.
    pub data_cost: String,
    /// Block number when registered.
    pub registered_at_block: i64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,
    /// European Article Number / Universal Product Code.
    pub ean_upc: String,
    /// Creator of the release.
    pub creator: PartyId,
    /// Title of the release.
    pub title: String,
    /// Type of release (Lp, Ep, Single, etc.).
    pub release_type: String,
    /// Format (CD, Vinyl, Digital, etc.).
    pub format: String,
    /// Packaging type.
    pub packaging: String,
    /// Release status.
    pub status: String,
    /// Release date.
    pub date: Date,
    /// Country of release (ISO 3166-1 alpha-2).
    pub country: String,
    /// IDs of recordings in this release.
    pub recording_ids: Vec<i64>,
    /// Name of the distributor.
    pub distributor_name: String,
    /// Name of the manufacturer.
    pub manufacturer_name: String,
}

/// A single release in a paginated list.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseEdge {
    /// The release.
    pub node: Release,
    /// Cursor for pagination.
    pub cursor: String,
}

/// Paginated list of releases.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseConnection {
    /// List of release edges.
    pub edges: Vec<ReleaseEdge>,
    /// Pagination information.
    pub page_info: PageInfo,
    /// Total count of releases (if available).
    pub total_count: Option<i64>,
}

// =============================================================================
// Statistics Type
// =============================================================================

/// Statistics about MIDDS on-chain data.
#[derive(async_graphql::SimpleObject, Clone, Debug, Serialize, Deserialize)]
pub struct MiddsStats {
    /// Total number of musical works registered.
    pub total_musical_works: i64,
    /// Total number of recordings registered.
    pub total_recordings: i64,
    /// Total number of releases registered.
    pub total_releases: i64,
}
