//! Domain models for MIDDS (Musical Industry Digital Distribution Standard).
//!
//! These models represent the indexed data stored in PostgreSQL.
//! They are converted from the on-chain types defined in `allfeat-midds-v2`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use maestro_core::models::AccountId;

// =============================================================================
// Shared Types
// =============================================================================

/// A party identifier (composer, artist, producer, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PartyId {
    /// IPI (Interested Party Information) number only
    Ipi { value: u64 },
    /// ISNI (International Standard Name Identifier) only
    Isni { value: String },
    /// Both IPI and ISNI
    Both { ipi: u64, isni: String },
}

impl PartyId {
    /// Convert from the on-chain PartyId type.
    pub fn from_chain(party: &allfeat_midds::shared::PartyId) -> Self {
        match party {
            allfeat_midds::shared::PartyId::Ipi(ipi) => PartyId::Ipi { value: *ipi },
            allfeat_midds::shared::PartyId::Isni(isni) => PartyId::Isni {
                value: String::from_utf8_lossy(isni.as_slice()).to_string(),
            },
            allfeat_midds::shared::PartyId::Both(both) => PartyId::Both {
                ipi: both.ipi,
                isni: String::from_utf8_lossy(both.isni.as_slice()).to_string(),
            },
        }
    }
}

/// A date (year, month, day).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Date {
    pub year: u16,
    pub month: u8,
    pub day: u8,
}

impl Date {
    /// Convert from the on-chain Date type.
    pub fn from_chain(date: &allfeat_midds::shared::Date) -> Self {
        Self {
            year: date.year,
            month: date.month,
            day: date.day,
        }
    }
}

// =============================================================================
// Musical Work
// =============================================================================

/// A creator of a musical work.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Creator {
    /// The party identifier.
    pub party_id: PartyId,
    /// The role of the creator.
    pub role: String,
}

impl Creator {
    /// Convert from the on-chain Creator type.
    pub fn from_chain(creator: &allfeat_midds::musical_work::Creator) -> Self {
        let role = match creator.role {
            allfeat_midds::musical_work::CreatorRole::Author => "Author",
            allfeat_midds::musical_work::CreatorRole::Composer => "Composer",
            allfeat_midds::musical_work::CreatorRole::Arranger => "Arranger",
            allfeat_midds::musical_work::CreatorRole::Adapter => "Adapter",
            allfeat_midds::musical_work::CreatorRole::Publisher => "Publisher",
        };
        Self {
            party_id: PartyId::from_chain(&creator.id),
            role: role.to_string(),
        }
    }
}

/// A musical work (composition) indexed from the MusicalWorks pallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MusicalWork {
    /// Unique MIDDS identifier (on-chain).
    pub id: u64,
    /// Provider account that registered this work.
    pub provider: AccountId,
    /// Blake2-256 hash of the SCALE-encoded data.
    pub hash: [u8; 32],
    /// Cost deposited by the provider.
    pub data_cost: u128,
    /// Block number when registered.
    pub registered_at_block: u64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,

    // MIDDS-specific fields
    /// International Standard Musical Work Code.
    pub iswc: String,
    /// Title of the work.
    pub title: String,
    /// Year the work was created.
    pub creation_year: Option<u16>,
    /// Whether the work is instrumental (no lyrics).
    pub instrumental: Option<bool>,
    /// Language of the lyrics.
    pub language: Option<String>,
    /// Tempo in beats per minute.
    pub bpm: Option<u16>,
    /// Musical key.
    pub key: Option<String>,
    /// Type of work (Original, Medley, Mashup, Adaptation).
    pub work_type: Option<String>,
    /// List of creators with their roles.
    pub creators: Vec<Creator>,
}

// =============================================================================
// Recording
// =============================================================================

/// A recording indexed from the Recordings pallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Unique MIDDS identifier (on-chain).
    pub id: u64,
    /// Provider account that registered this recording.
    pub provider: AccountId,
    /// Blake2-256 hash of the SCALE-encoded data.
    pub hash: [u8; 32],
    /// Cost deposited by the provider.
    pub data_cost: u128,
    /// Block number when registered.
    pub registered_at_block: u64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,

    // MIDDS-specific fields
    /// International Standard Recording Code.
    pub isrc: String,
    /// Reference to the musical work this is a recording of.
    pub musical_work_id: u64,
    /// Main artist.
    pub artist: PartyId,
    /// Title of the recording.
    pub title: String,
    /// Year the recording was made.
    pub recording_year: Option<u16>,
    /// Duration in seconds.
    pub duration: Option<u16>,
    /// Tempo in beats per minute.
    pub bpm: Option<u16>,
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

// =============================================================================
// Release
// =============================================================================

/// A release indexed from the Releases pallet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    /// Unique MIDDS identifier (on-chain).
    pub id: u64,
    /// Provider account that registered this release.
    pub provider: AccountId,
    /// Blake2-256 hash of the SCALE-encoded data.
    pub hash: [u8; 32],
    /// Cost deposited by the provider.
    pub data_cost: u128,
    /// Block number when registered.
    pub registered_at_block: u64,
    /// Timestamp when registered.
    pub registered_at_timestamp: Option<DateTime<Utc>>,

    // MIDDS-specific fields
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
    pub recording_ids: Vec<u64>,
    /// Name of the distributor.
    pub distributor_name: String,
    /// Name of the manufacturer.
    pub manufacturer_name: String,
}

// =============================================================================
// Conversion helpers
// =============================================================================

/// Convert a Language enum to a string.
pub fn language_to_string(lang: &allfeat_midds::shared::Language) -> String {
    use allfeat_midds::shared::Language::*;
    match lang {
        English => "English",
        French => "French",
        Spanish => "Spanish",
        German => "German",
        Italian => "Italian",
        Portuguese => "Portuguese",
        Russian => "Russian",
        Chinese => "Chinese",
        Japanese => "Japanese",
        Korean => "Korean",
        Arabic => "Arabic",
        Hindi => "Hindi",
        Dutch => "Dutch",
        Swedish => "Swedish",
        Norwegian => "Norwegian",
        Finnish => "Finnish",
        Polish => "Polish",
        Turkish => "Turkish",
        Hebrew => "Hebrew",
        Greek => "Greek",
        Latin => "Latin",
        Esperanto => "Esperanto",
    }
    .to_string()
}

/// Convert a Key enum to a string.
pub fn key_to_string(key: &allfeat_midds::shared::Key) -> String {
    format!("{:?}", key)
}

/// Convert a Country enum to a string.
pub fn country_to_string(country: &allfeat_midds::shared::Country) -> String {
    format!("{:?}", country)
}

/// Convert a MusicalWorkType enum to a string.
pub fn work_type_to_string(wt: &allfeat_midds::musical_work::MusicalWorkType) -> String {
    use allfeat_midds::musical_work::MusicalWorkType::*;
    match wt {
        Original => "Original",
        Medley(_) => "Medley",
        Mashup(_) => "Mashup",
        Adaptation(_) => "Adaptation",
    }
    .to_string()
}

/// Convert a RecordingVersion enum to a string.
pub fn version_to_string(version: &allfeat_midds::recording::RecordingVersion) -> String {
    use allfeat_midds::recording::RecordingVersion::*;
    match version {
        Original => "Original",
        Live => "Live",
        RadioEdit => "RadioEdit",
        TvTrack => "TvTrack",
        Single => "Single",
        Remix => "Remix",
        Cover => "Cover",
        Acoustic => "Acoustic",
        Acapella => "Acapella",
        Instrumental => "Instrumental",
        Orchestral => "Orchestral",
        Extended => "Extended",
        AlternateTake => "AlternateTake",
        ReRecorded => "ReRecorded",
        Karaoke => "Karaoke",
        Dance => "Dance",
        Dub => "Dub",
        Clean => "Clean",
        Rehearsal => "Rehearsal",
        Demo => "Demo",
        Edit => "Edit",
    }
    .to_string()
}

/// Convert a ReleaseType enum to a string.
pub fn release_type_to_string(rt: &allfeat_midds::release::ReleaseType) -> String {
    use allfeat_midds::release::ReleaseType::*;
    match rt {
        Lp => "Lp",
        DoubleLp => "DoubleLp",
        Ep => "Ep",
        Single => "Single",
        Mixtape => "Mixtape",
        Compilation => "Compilation",
    }
    .to_string()
}

/// Convert a ReleaseFormat enum to a string.
pub fn format_to_string(fmt: &allfeat_midds::release::ReleaseFormat) -> String {
    format!("{:?}", fmt)
}

/// Convert a ReleasePackaging enum to a string.
pub fn packaging_to_string(pkg: &allfeat_midds::release::ReleasePackaging) -> String {
    format!("{:?}", pkg)
}

/// Convert a ReleaseStatus enum to a string.
pub fn status_to_string(status: &allfeat_midds::release::ReleaseStatus) -> String {
    use allfeat_midds::release::ReleaseStatus::*;
    match status {
        Official => "Official",
        Promotional => "Promotional",
        ReRelease => "ReRelease",
        SpecialEdition => "SpecialEdition",
        Remastered => "Remastered",
        Bootleg => "Bootleg",
        PseudoRelease => "PseudoRelease",
        Withdrawn => "Withdrawn",
        Expunged => "Expunged",
        Cancelled => "Cancelled",
    }
    .to_string()
}
