//! GraphQL types and queries for MIDDS (Musical Industry Digital Distribution Standard).

use std::sync::Arc;

use async_graphql::{Context, Object, Result};
use chrono::{DateTime, Utc};

use maestro_core::ports::Pagination;
use maestro_graphql::{Order, PageInfo, convert_order};

use super::models::{
    Creator as CreatorModel, Date as DateModel, MusicalWork as MusicalWorkModel,
    PartyId as PartyIdModel, Recording as RecordingModel, Release as ReleaseModel,
};
use super::storage::{MiddsStorage, MusicalWorkFilter, RecordingFilter, ReleaseFilter};
use crate::graphql_utils::{parse_account, validate_cursor, validate_pagination_first};

// -----------------------------------------------------------------------------
// PartyId GraphQL Type
// -----------------------------------------------------------------------------

/// A party identifier (IPI, ISNI, or both).
#[derive(async_graphql::SimpleObject, Clone)]
pub struct PartyId {
    /// IPI (Interested Party Information) number, if available.
    pub ipi: Option<i64>,
    /// ISNI (International Standard Name Identifier), if available.
    pub isni: Option<String>,
}

impl From<&PartyIdModel> for PartyId {
    fn from(p: &PartyIdModel) -> Self {
        match p {
            PartyIdModel::Ipi { value } => PartyId {
                ipi: Some(*value as i64),
                isni: None,
            },
            PartyIdModel::Isni { value } => PartyId {
                ipi: None,
                isni: Some(value.clone()),
            },
            PartyIdModel::Both { ipi, isni } => PartyId {
                ipi: Some(*ipi as i64),
                isni: Some(isni.clone()),
            },
        }
    }
}

// -----------------------------------------------------------------------------
// Creator GraphQL Type
// -----------------------------------------------------------------------------

/// A creator of a musical work.
#[derive(async_graphql::SimpleObject, Clone)]
pub struct Creator {
    /// The party identifier.
    pub party_id: PartyId,
    /// The role of the creator (Author, Composer, Arranger, Adapter, Publisher).
    pub role: String,
}

impl From<&CreatorModel> for Creator {
    fn from(c: &CreatorModel) -> Self {
        Self {
            party_id: PartyId::from(&c.party_id),
            role: c.role.clone(),
        }
    }
}

// -----------------------------------------------------------------------------
// Date GraphQL Type
// -----------------------------------------------------------------------------

/// A date (year, month, day).
#[derive(async_graphql::SimpleObject, Clone)]
pub struct Date {
    pub year: i32,
    pub month: i32,
    pub day: i32,
}

impl From<&DateModel> for Date {
    fn from(d: &DateModel) -> Self {
        Self {
            year: d.year as i32,
            month: d.month as i32,
            day: d.day as i32,
        }
    }
}

// -----------------------------------------------------------------------------
// MusicalWork GraphQL Type
// -----------------------------------------------------------------------------

/// A musical work (composition) registered on-chain.
#[derive(async_graphql::SimpleObject)]
#[graphql(complex)]
pub struct MusicalWork {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded).
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

#[async_graphql::ComplexObject]
impl MusicalWork {
    /// Recordings of this musical work.
    async fn recordings<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Vec<Recording>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let recordings = storage
            .list_recordings_by_musical_work(self.id as u64)
            .await?;
        Ok(recordings.iter().map(Recording::from).collect())
    }
}

impl From<&MusicalWorkModel> for MusicalWork {
    fn from(w: &MusicalWorkModel) -> Self {
        Self {
            id: w.id as i64,
            provider: format!("0x{}", hex::encode(w.provider.0)),
            hash: format!("0x{}", hex::encode(w.hash)),
            data_cost: w.data_cost.to_string(),
            registered_at_block: w.registered_at_block as i64,
            registered_at_timestamp: w.registered_at_timestamp,
            iswc: w.iswc.clone(),
            title: w.title.clone(),
            creation_year: w.creation_year.map(|y| y as i32),
            instrumental: w.instrumental,
            language: w.language.clone(),
            bpm: w.bpm.map(|b| b as i32),
            key: w.key.clone(),
            work_type: w.work_type.clone(),
            creators: w.creators.iter().map(Creator::from).collect(),
        }
    }
}

// -----------------------------------------------------------------------------
// Recording GraphQL Type
// -----------------------------------------------------------------------------

/// A recording registered on-chain.
#[derive(async_graphql::SimpleObject)]
#[graphql(complex)]
pub struct Recording {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded).
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

#[async_graphql::ComplexObject]
impl Recording {
    /// The musical work this recording is based on.
    async fn musical_work<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Option<MusicalWork>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let work = storage
            .get_musical_work(self.musical_work_id as u64)
            .await?;
        Ok(work.as_ref().map(MusicalWork::from))
    }
}

impl From<&RecordingModel> for Recording {
    fn from(r: &RecordingModel) -> Self {
        Self {
            id: r.id as i64,
            provider: format!("0x{}", hex::encode(r.provider.0)),
            hash: format!("0x{}", hex::encode(r.hash)),
            data_cost: r.data_cost.to_string(),
            registered_at_block: r.registered_at_block as i64,
            registered_at_timestamp: r.registered_at_timestamp,
            isrc: r.isrc.clone(),
            musical_work_id: r.musical_work_id as i64,
            artist: PartyId::from(&r.artist),
            title: r.title.clone(),
            recording_year: r.recording_year.map(|y| y as i32),
            duration: r.duration.map(|d| d as i32),
            bpm: r.bpm.map(|b| b as i32),
            key: r.key.clone(),
            version: r.version.clone(),
            genres: r.genres.clone(),
            producers: r.producers.iter().map(PartyId::from).collect(),
            performers: r.performers.iter().map(PartyId::from).collect(),
        }
    }
}

// -----------------------------------------------------------------------------
// Release GraphQL Type
// -----------------------------------------------------------------------------

/// A release (album/single) registered on-chain.
#[derive(async_graphql::SimpleObject)]
#[graphql(complex)]
pub struct Release {
    /// Unique MIDDS identifier.
    pub id: i64,
    /// Provider account (hex encoded).
    pub provider: String,
    /// Blake2-256 hash of the SCALE-encoded data (hex encoded).
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

#[async_graphql::ComplexObject]
impl Release {
    /// The recordings in this release.
    async fn recordings<'ctx>(&self, ctx: &Context<'ctx>) -> Result<Vec<Recording>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let mut recordings = Vec::new();
        for id in &self.recording_ids {
            if let Some(recording) = storage.get_recording(*id as u64).await? {
                recordings.push(Recording::from(&recording));
            }
        }
        Ok(recordings)
    }
}

impl From<&ReleaseModel> for Release {
    fn from(r: &ReleaseModel) -> Self {
        Self {
            id: r.id as i64,
            provider: format!("0x{}", hex::encode(r.provider.0)),
            hash: format!("0x{}", hex::encode(r.hash)),
            data_cost: r.data_cost.to_string(),
            registered_at_block: r.registered_at_block as i64,
            registered_at_timestamp: r.registered_at_timestamp,
            ean_upc: r.ean_upc.clone(),
            creator: PartyId::from(&r.creator),
            title: r.title.clone(),
            release_type: r.release_type.clone(),
            format: r.format.clone(),
            packaging: r.packaging.clone(),
            status: r.status.clone(),
            date: Date::from(&r.date),
            country: r.country.clone(),
            recording_ids: r.recording_ids.iter().map(|id| *id as i64).collect(),
            distributor_name: r.distributor_name.clone(),
            manufacturer_name: r.manufacturer_name.clone(),
        }
    }
}

// -----------------------------------------------------------------------------
// Connection Types (Relay-style pagination)
// -----------------------------------------------------------------------------

#[derive(async_graphql::SimpleObject)]
pub struct MusicalWorkEdge {
    pub node: MusicalWork,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct MusicalWorkConnection {
    pub edges: Vec<MusicalWorkEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<MusicalWorkModel>> for MusicalWorkConnection {
    fn from(conn: maestro_core::ports::Connection<MusicalWorkModel>) -> Self {
        Self {
            edges: conn
                .edges
                .iter()
                .map(|e| MusicalWorkEdge {
                    node: MusicalWork::from(&e.node),
                    cursor: e.cursor.value.clone(),
                })
                .collect(),
            page_info: PageInfo {
                has_next_page: conn.page_info.has_next_page,
                has_previous_page: conn.page_info.has_previous_page,
                start_cursor: conn
                    .page_info
                    .start_cursor
                    .as_ref()
                    .map(|c| c.value.clone()),
                end_cursor: conn.page_info.end_cursor.as_ref().map(|c| c.value.clone()),
            },
            total_count: conn.total_count,
        }
    }
}

#[derive(async_graphql::SimpleObject)]
pub struct RecordingEdge {
    pub node: Recording,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct RecordingConnection {
    pub edges: Vec<RecordingEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<RecordingModel>> for RecordingConnection {
    fn from(conn: maestro_core::ports::Connection<RecordingModel>) -> Self {
        Self {
            edges: conn
                .edges
                .iter()
                .map(|e| RecordingEdge {
                    node: Recording::from(&e.node),
                    cursor: e.cursor.value.clone(),
                })
                .collect(),
            page_info: PageInfo {
                has_next_page: conn.page_info.has_next_page,
                has_previous_page: conn.page_info.has_previous_page,
                start_cursor: conn
                    .page_info
                    .start_cursor
                    .as_ref()
                    .map(|c| c.value.clone()),
                end_cursor: conn.page_info.end_cursor.as_ref().map(|c| c.value.clone()),
            },
            total_count: conn.total_count,
        }
    }
}

#[derive(async_graphql::SimpleObject)]
pub struct ReleaseEdge {
    pub node: Release,
    pub cursor: String,
}

#[derive(async_graphql::SimpleObject)]
pub struct ReleaseConnection {
    pub edges: Vec<ReleaseEdge>,
    pub page_info: PageInfo,
    pub total_count: Option<i64>,
}

impl From<maestro_core::ports::Connection<ReleaseModel>> for ReleaseConnection {
    fn from(conn: maestro_core::ports::Connection<ReleaseModel>) -> Self {
        Self {
            edges: conn
                .edges
                .iter()
                .map(|e| ReleaseEdge {
                    node: Release::from(&e.node),
                    cursor: e.cursor.value.clone(),
                })
                .collect(),
            page_info: PageInfo {
                has_next_page: conn.page_info.has_next_page,
                has_previous_page: conn.page_info.has_previous_page,
                start_cursor: conn
                    .page_info
                    .start_cursor
                    .as_ref()
                    .map(|c| c.value.clone()),
                end_cursor: conn.page_info.end_cursor.as_ref().map(|c| c.value.clone()),
            },
            total_count: conn.total_count,
        }
    }
}

// -----------------------------------------------------------------------------
// Statistics Type
// -----------------------------------------------------------------------------

/// Statistics about MIDDS on-chain data.
#[derive(async_graphql::SimpleObject)]
pub struct MiddsStats {
    /// Total number of musical works registered.
    pub total_musical_works: i64,
    /// Total number of recordings registered.
    pub total_recordings: i64,
    /// Total number of releases registered.
    pub total_releases: i64,
}

// -----------------------------------------------------------------------------
// MIDDS Query
// -----------------------------------------------------------------------------

/// GraphQL query root for MIDDS (Musical Industry Digital Distribution Standard).
///
/// This can be merged with other query types using `#[derive(MergedObject)]`.
#[derive(Default)]
pub struct MiddsQuery;

#[Object]
impl MiddsQuery {
    // -------------------------------------------------------------------------
    // Musical Works
    // -------------------------------------------------------------------------

    /// Get a musical work by ID.
    async fn musical_work<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        id: i64,
    ) -> Result<Option<MusicalWork>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let work = storage.get_musical_work(id as u64).await?;
        Ok(work.as_ref().map(MusicalWork::from))
    }

    /// Find a musical work by ISWC.
    async fn musical_work_by_iswc<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        iswc: String,
    ) -> Result<Option<MusicalWork>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let work = storage.find_musical_work_by_iswc(&iswc).await?;
        Ok(work.as_ref().map(MusicalWork::from))
    }

    /// List musical works with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn musical_works<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        provider: Option<String>,
        iswc: Option<String>,
        language: Option<String>,
        work_type: Option<String>,
        created_at_block_gte: Option<i64>,
        created_at_block_lte: Option<i64>,
        #[graphql(default)] order: Order,
    ) -> Result<MusicalWorkConnection> {
        validate_cursor(&after, "after")?;

        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;

        let filter = MusicalWorkFilter {
            provider: provider.map(|s| parse_account(&s)).transpose()?,
            iswc,
            language,
            work_type,
            created_at_block_gte: created_at_block_gte.map(|n| n as u64),
            created_at_block_lte: created_at_block_lte.map(|n| n as u64),
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage
            .list_musical_works(filter, pagination, convert_order(order))
            .await?;
        Ok(MusicalWorkConnection::from(connection))
    }

    // -------------------------------------------------------------------------
    // Recordings
    // -------------------------------------------------------------------------

    /// Get a recording by ID.
    async fn recording<'ctx>(&self, ctx: &Context<'ctx>, id: i64) -> Result<Option<Recording>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let recording = storage.get_recording(id as u64).await?;
        Ok(recording.as_ref().map(Recording::from))
    }

    /// Find a recording by ISRC.
    async fn recording_by_isrc<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        isrc: String,
    ) -> Result<Option<Recording>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let recording = storage.find_recording_by_isrc(&isrc).await?;
        Ok(recording.as_ref().map(Recording::from))
    }

    /// List recordings with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn recordings<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        provider: Option<String>,
        isrc: Option<String>,
        musical_work_id: Option<i64>,
        version: Option<String>,
        #[graphql(default)] order: Order,
    ) -> Result<RecordingConnection> {
        validate_cursor(&after, "after")?;

        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;

        let filter = RecordingFilter {
            provider: provider.map(|s| parse_account(&s)).transpose()?,
            isrc,
            musical_work_id: musical_work_id.map(|n| n as u64),
            version,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage
            .list_recordings(filter, pagination, convert_order(order))
            .await?;
        Ok(RecordingConnection::from(connection))
    }

    /// List recordings for a specific musical work.
    async fn recordings_for_musical_work<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        musical_work_id: i64,
    ) -> Result<Vec<Recording>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let recordings = storage
            .list_recordings_by_musical_work(musical_work_id as u64)
            .await?;
        Ok(recordings.iter().map(Recording::from).collect())
    }

    // -------------------------------------------------------------------------
    // Releases
    // -------------------------------------------------------------------------

    /// Get a release by ID.
    async fn release<'ctx>(&self, ctx: &Context<'ctx>, id: i64) -> Result<Option<Release>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let release = storage.get_release(id as u64).await?;
        Ok(release.as_ref().map(Release::from))
    }

    /// Find a release by EAN/UPC.
    async fn release_by_ean<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        ean: String,
    ) -> Result<Option<Release>> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;
        let release = storage.find_release_by_ean(&ean).await?;
        Ok(release.as_ref().map(Release::from))
    }

    /// List releases with pagination and filtering.
    #[allow(clippy::too_many_arguments)]
    async fn releases<'ctx>(
        &self,
        ctx: &Context<'ctx>,
        #[graphql(default = 20)] first: Option<i32>,
        after: Option<String>,
        provider: Option<String>,
        ean_upc: Option<String>,
        release_type: Option<String>,
        status: Option<String>,
        country: Option<String>,
        #[graphql(default)] order: Order,
    ) -> Result<ReleaseConnection> {
        validate_cursor(&after, "after")?;

        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;

        let filter = ReleaseFilter {
            provider: provider.map(|s| parse_account(&s)).transpose()?,
            ean_upc,
            release_type,
            status,
            country,
        };

        let pagination = Pagination {
            first: Some(validate_pagination_first(first)),
            after: after.map(|v| maestro_core::ports::Cursor { value: v }),
            ..Default::default()
        };

        let connection = storage
            .list_releases(filter, pagination, convert_order(order))
            .await?;
        Ok(ReleaseConnection::from(connection))
    }

    // -------------------------------------------------------------------------
    // Statistics
    // -------------------------------------------------------------------------

    /// Get MIDDS statistics.
    async fn midds_stats<'ctx>(&self, ctx: &Context<'ctx>) -> Result<MiddsStats> {
        let storage = ctx.data::<Arc<dyn MiddsStorage>>()?;

        let total_musical_works = storage.count_musical_works().await? as i64;
        let total_recordings = storage.count_recordings().await? as i64;
        let total_releases = storage.count_releases().await? as i64;

        Ok(MiddsStats {
            total_musical_works,
            total_recordings,
            total_releases,
        })
    }
}
