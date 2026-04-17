//! Storage layer for MIDDS (Musical Industry Digital Distribution Standard).

use async_trait::async_trait;
use chrono::Datelike;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::AccountId;
use maestro_core::ports::{Connection, Cursor, Edge, OrderDirection, PageInfo, Pagination};

use super::models::{Creator, Date, MusicalWork, PartyId, Recording, Release};

// =============================================================================
// Filters
// =============================================================================

/// Filter options for musical work queries.
#[derive(Debug, Clone, Default)]
pub struct MusicalWorkFilter {
    pub provider: Option<AccountId>,
    pub iswc: Option<String>,
    pub language: Option<String>,
    pub work_type: Option<String>,
    pub created_at_block_gte: Option<u64>,
    pub created_at_block_lte: Option<u64>,
}

/// Filter options for recording queries.
#[derive(Debug, Clone, Default)]
pub struct RecordingFilter {
    pub provider: Option<AccountId>,
    pub isrc: Option<String>,
    pub musical_work_id: Option<u64>,
    pub version: Option<String>,
}

/// Filter options for release queries.
#[derive(Debug, Clone, Default)]
pub struct ReleaseFilter {
    pub provider: Option<AccountId>,
    pub ean_upc: Option<String>,
    pub release_type: Option<String>,
    pub status: Option<String>,
    pub country: Option<String>,
}

// =============================================================================
// Storage Trait
// =============================================================================

/// Storage trait for MIDDS pallet data.
#[async_trait]
pub trait MiddsStorage: Send + Sync {
    // -------------------------------------------------------------------------
    // Musical Works
    // -------------------------------------------------------------------------

    /// Insert a new musical work.
    async fn insert_musical_work(&self, work: &MusicalWork) -> StorageResult<()>;

    /// Get a musical work by ID.
    async fn get_musical_work(&self, id: u64) -> StorageResult<Option<MusicalWork>>;

    /// Find a musical work by ISWC.
    async fn find_musical_work_by_iswc(&self, iswc: &str) -> StorageResult<Option<MusicalWork>>;

    /// List musical works with pagination and filtering.
    async fn list_musical_works(
        &self,
        filter: MusicalWorkFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<MusicalWork>>;

    /// Delete a musical work by ID.
    async fn delete_musical_work(&self, id: u64) -> StorageResult<()>;

    /// Count all musical works.
    async fn count_musical_works(&self) -> StorageResult<u64>;

    // -------------------------------------------------------------------------
    // Recordings
    // -------------------------------------------------------------------------

    /// Insert a new recording.
    async fn insert_recording(&self, recording: &Recording) -> StorageResult<()>;

    /// Get a recording by ID.
    async fn get_recording(&self, id: u64) -> StorageResult<Option<Recording>>;

    /// Find a recording by ISRC.
    async fn find_recording_by_isrc(&self, isrc: &str) -> StorageResult<Option<Recording>>;

    /// List recordings with pagination and filtering.
    async fn list_recordings(
        &self,
        filter: RecordingFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Recording>>;

    /// List recordings for a specific musical work.
    async fn list_recordings_by_musical_work(&self, work_id: u64) -> StorageResult<Vec<Recording>>;

    /// Delete a recording by ID.
    async fn delete_recording(&self, id: u64) -> StorageResult<()>;

    /// Count all recordings.
    async fn count_recordings(&self) -> StorageResult<u64>;

    // -------------------------------------------------------------------------
    // Releases
    // -------------------------------------------------------------------------

    /// Insert a new release.
    async fn insert_release(&self, release: &Release) -> StorageResult<()>;

    /// Get a release by ID.
    async fn get_release(&self, id: u64) -> StorageResult<Option<Release>>;

    /// Find a release by EAN/UPC.
    async fn find_release_by_ean(&self, ean: &str) -> StorageResult<Option<Release>>;

    /// List releases with pagination and filtering.
    async fn list_releases(
        &self,
        filter: ReleaseFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Release>>;

    /// Delete a release by ID.
    async fn delete_release(&self, id: u64) -> StorageResult<()>;

    /// Count all releases.
    async fn count_releases(&self) -> StorageResult<u64>;

    // -------------------------------------------------------------------------
    // Reorg handling
    // -------------------------------------------------------------------------

    /// Delete all MIDDS data from a given block number (for reorg handling).
    async fn delete_from_block(&self, from_block: u64) -> StorageResult<u64>;
}

// =============================================================================
// PostgreSQL Implementation
// =============================================================================

/// PostgreSQL implementation of MiddsStorage.
pub struct PgMiddsStorage {
    pool: PgPool,
}

impl PgMiddsStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MiddsStorage for PgMiddsStorage {
    // -------------------------------------------------------------------------
    // Musical Works
    // -------------------------------------------------------------------------

    async fn insert_musical_work(&self, work: &MusicalWork) -> StorageResult<()> {
        let creators_json = serde_json::to_value(&work.creators)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        sqlx::query(
            r#"
            INSERT INTO midds_musical_works (
                id, provider, hash, data_cost, registered_at_block, registered_at_timestamp,
                iswc, title, creation_year, instrumental, language, bpm, musical_key, work_type, creators
            )
            VALUES ($1, $2, $3, $4::numeric, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(work.id as i64)
        .bind(&work.provider.0[..])
        .bind(&work.hash[..])
        .bind(work.data_cost.to_string())
        .bind(work.registered_at_block as i64)
        .bind(work.registered_at_timestamp)
        .bind(&work.iswc)
        .bind(&work.title)
        .bind(work.creation_year.map(|y| y as i16))
        .bind(work.instrumental)
        .bind(&work.language)
        .bind(work.bpm.map(|b| b as i16))
        .bind(&work.key)
        .bind(&work.work_type)
        .bind(creators_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(())
    }

    async fn get_musical_work(&self, id: u64) -> StorageResult<Option<MusicalWork>> {
        let row = sqlx::query_as::<_, MusicalWorkRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   iswc, title, creation_year, instrumental, language, bpm, musical_key, work_type, creators
            FROM midds_musical_works
            WHERE id = $1
            "#,
        )
        .bind(id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(MusicalWorkRow::into_model).transpose()
    }

    async fn find_musical_work_by_iswc(&self, iswc: &str) -> StorageResult<Option<MusicalWork>> {
        let row = sqlx::query_as::<_, MusicalWorkRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   iswc, title, creation_year, instrumental, language, bpm, musical_key, work_type, creators
            FROM midds_musical_works
            WHERE iswc = $1
            "#,
        )
        .bind(iswc)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(MusicalWorkRow::into_model).transpose()
    }

    async fn list_musical_works(
        &self,
        filter: MusicalWorkFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<MusicalWork>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.provider.is_some() {
            conditions.push(format!("provider = ${}", param_idx));
            param_idx += 1;
        }
        if filter.iswc.is_some() {
            conditions.push(format!("iswc = ${}", param_idx));
            param_idx += 1;
        }
        if filter.language.is_some() {
            conditions.push(format!("language = ${}", param_idx));
            param_idx += 1;
        }
        if filter.work_type.is_some() {
            conditions.push(format!("work_type = ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_at_block_gte.is_some() {
            conditions.push(format!("registered_at_block >= ${}", param_idx));
            param_idx += 1;
        }
        if filter.created_at_block_lte.is_some() {
            conditions.push(format!("registered_at_block <= ${}", param_idx));
            // param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   iswc, title, creation_year, instrumental, language, bpm, musical_key, work_type, creators
            FROM midds_musical_works
            {}
            ORDER BY id {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            limit + 1
        );

        let rows: Vec<MusicalWorkRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        } else {
            let mut q = sqlx::query_as::<_, MusicalWorkRow>(&query);
            if let Some(ref provider) = filter.provider {
                q = q.bind(&provider.0[..]);
            }
            if let Some(ref iswc) = filter.iswc {
                q = q.bind(iswc);
            }
            if let Some(ref language) = filter.language {
                q = q.bind(language);
            }
            if let Some(ref work_type) = filter.work_type {
                q = q.bind(work_type);
            }
            if let Some(block) = filter.created_at_block_gte {
                q = q.bind(block as i64);
            }
            if let Some(block) = filter.created_at_block_lte {
                q = q.bind(block as i64);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        };

        let has_more = rows.len() > limit as usize;
        let works: Vec<MusicalWork> = rows
            .into_iter()
            .take(limit as usize)
            .map(MusicalWorkRow::into_model)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<MusicalWork>> = works
            .into_iter()
            .map(|w| Edge {
                cursor: Cursor {
                    value: w.id.to_string(),
                },
                node: w,
            })
            .collect();

        let page_info = PageInfo {
            has_next_page: has_more,
            has_previous_page: pagination.after.is_some(),
            start_cursor: edges.first().map(|e| e.cursor.clone()),
            end_cursor: edges.last().map(|e| e.cursor.clone()),
        };

        Ok(Connection {
            edges,
            page_info,
            total_count: None,
        })
    }

    async fn delete_musical_work(&self, id: u64) -> StorageResult<()> {
        sqlx::query("DELETE FROM midds_musical_works WHERE id = $1")
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(())
    }

    async fn count_musical_works(&self) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM midds_musical_works")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(row.0 as u64)
    }

    // -------------------------------------------------------------------------
    // Recordings
    // -------------------------------------------------------------------------

    async fn insert_recording(&self, recording: &Recording) -> StorageResult<()> {
        let artist_json = serde_json::to_value(&recording.artist)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let genres_json = serde_json::to_value(&recording.genres)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let producers_json = serde_json::to_value(&recording.producers)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let performers_json = serde_json::to_value(&recording.performers)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        sqlx::query(
            r#"
            INSERT INTO midds_recordings (
                id, provider, hash, data_cost, registered_at_block, registered_at_timestamp,
                isrc, musical_work_id, artist, title, recording_year, duration, bpm, musical_key, version,
                genres, producers, performers
            )
            VALUES ($1, $2, $3, $4::numeric, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(recording.id as i64)
        .bind(&recording.provider.0[..])
        .bind(&recording.hash[..])
        .bind(recording.data_cost.to_string())
        .bind(recording.registered_at_block as i64)
        .bind(recording.registered_at_timestamp)
        .bind(&recording.isrc)
        .bind(recording.musical_work_id as i64)
        .bind(artist_json)
        .bind(&recording.title)
        .bind(recording.recording_year.map(|y| y as i16))
        .bind(recording.duration.map(|d| d as i16))
        .bind(recording.bpm.map(|b| b as i16))
        .bind(&recording.key)
        .bind(&recording.version)
        .bind(genres_json)
        .bind(producers_json)
        .bind(performers_json)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(())
    }

    async fn get_recording(&self, id: u64) -> StorageResult<Option<Recording>> {
        let row = sqlx::query_as::<_, RecordingRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   isrc, musical_work_id, artist, title, recording_year, duration, bpm, musical_key, version,
                   genres, producers, performers
            FROM midds_recordings
            WHERE id = $1
            "#,
        )
        .bind(id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(RecordingRow::into_model).transpose()
    }

    async fn find_recording_by_isrc(&self, isrc: &str) -> StorageResult<Option<Recording>> {
        let row = sqlx::query_as::<_, RecordingRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   isrc, musical_work_id, artist, title, recording_year, duration, bpm, musical_key, version,
                   genres, producers, performers
            FROM midds_recordings
            WHERE isrc = $1
            "#,
        )
        .bind(isrc)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(RecordingRow::into_model).transpose()
    }

    async fn list_recordings(
        &self,
        filter: RecordingFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Recording>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.provider.is_some() {
            conditions.push(format!("provider = ${}", param_idx));
            param_idx += 1;
        }
        if filter.isrc.is_some() {
            conditions.push(format!("isrc = ${}", param_idx));
            param_idx += 1;
        }
        if filter.musical_work_id.is_some() {
            conditions.push(format!("musical_work_id = ${}", param_idx));
            param_idx += 1;
        }
        if filter.version.is_some() {
            conditions.push(format!("version = ${}", param_idx));
            // param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   isrc, musical_work_id, artist, title, recording_year, duration, bpm, musical_key, version,
                   genres, producers, performers
            FROM midds_recordings
            {}
            ORDER BY id {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            limit + 1
        );

        let rows: Vec<RecordingRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        } else {
            let mut q = sqlx::query_as::<_, RecordingRow>(&query);
            if let Some(ref provider) = filter.provider {
                q = q.bind(&provider.0[..]);
            }
            if let Some(ref isrc) = filter.isrc {
                q = q.bind(isrc);
            }
            if let Some(work_id) = filter.musical_work_id {
                q = q.bind(work_id as i64);
            }
            if let Some(ref version) = filter.version {
                q = q.bind(version);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        };

        let has_more = rows.len() > limit as usize;
        let recordings: Vec<Recording> = rows
            .into_iter()
            .take(limit as usize)
            .map(RecordingRow::into_model)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Recording>> = recordings
            .into_iter()
            .map(|r| Edge {
                cursor: Cursor {
                    value: r.id.to_string(),
                },
                node: r,
            })
            .collect();

        let page_info = PageInfo {
            has_next_page: has_more,
            has_previous_page: pagination.after.is_some(),
            start_cursor: edges.first().map(|e| e.cursor.clone()),
            end_cursor: edges.last().map(|e| e.cursor.clone()),
        };

        Ok(Connection {
            edges,
            page_info,
            total_count: None,
        })
    }

    async fn list_recordings_by_musical_work(&self, work_id: u64) -> StorageResult<Vec<Recording>> {
        let rows = sqlx::query_as::<_, RecordingRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   isrc, musical_work_id, artist, title, recording_year, duration, bpm, musical_key, version,
                   genres, producers, performers
            FROM midds_recordings
            WHERE musical_work_id = $1
            ORDER BY id ASC
            "#,
        )
        .bind(work_id as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        rows.into_iter().map(RecordingRow::into_model).collect()
    }

    async fn delete_recording(&self, id: u64) -> StorageResult<()> {
        sqlx::query("DELETE FROM midds_recordings WHERE id = $1")
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(())
    }

    async fn count_recordings(&self) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM midds_recordings")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(row.0 as u64)
    }

    // -------------------------------------------------------------------------
    // Releases
    // -------------------------------------------------------------------------

    async fn insert_release(&self, release: &Release) -> StorageResult<()> {
        let creator_json = serde_json::to_value(&release.creator)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let recording_ids_json = serde_json::to_value(&release.recording_ids)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        let release_date = chrono::NaiveDate::from_ymd_opt(
            release.date.year as i32,
            release.date.month as u32,
            release.date.day as u32,
        );

        sqlx::query(
            r#"
            INSERT INTO midds_releases (
                id, provider, hash, data_cost, registered_at_block, registered_at_timestamp,
                ean_upc, creator, title, release_type, format, packaging, status, release_date,
                country, recording_ids, distributor_name, manufacturer_name
            )
            VALUES ($1, $2, $3, $4::numeric, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(release.id as i64)
        .bind(&release.provider.0[..])
        .bind(&release.hash[..])
        .bind(release.data_cost.to_string())
        .bind(release.registered_at_block as i64)
        .bind(release.registered_at_timestamp)
        .bind(&release.ean_upc)
        .bind(creator_json)
        .bind(&release.title)
        .bind(&release.release_type)
        .bind(&release.format)
        .bind(&release.packaging)
        .bind(&release.status)
        .bind(release_date)
        .bind(&release.country)
        .bind(recording_ids_json)
        .bind(&release.distributor_name)
        .bind(&release.manufacturer_name)
        .execute(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        Ok(())
    }

    async fn get_release(&self, id: u64) -> StorageResult<Option<Release>> {
        let row = sqlx::query_as::<_, ReleaseRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   ean_upc, creator, title, release_type, format, packaging, status, release_date,
                   country, recording_ids, distributor_name, manufacturer_name
            FROM midds_releases
            WHERE id = $1
            "#,
        )
        .bind(id as i64)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(ReleaseRow::into_model).transpose()
    }

    async fn find_release_by_ean(&self, ean: &str) -> StorageResult<Option<Release>> {
        let row = sqlx::query_as::<_, ReleaseRow>(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   ean_upc, creator, title, release_type, format, packaging, status, release_date,
                   country, recording_ids, distributor_name, manufacturer_name
            FROM midds_releases
            WHERE ean_upc = $1
            "#,
        )
        .bind(ean)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;

        row.map(ReleaseRow::into_model).transpose()
    }

    async fn list_releases(
        &self,
        filter: ReleaseFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Release>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.provider.is_some() {
            conditions.push(format!("provider = ${}", param_idx));
            param_idx += 1;
        }
        if filter.ean_upc.is_some() {
            conditions.push(format!("ean_upc = ${}", param_idx));
            param_idx += 1;
        }
        if filter.release_type.is_some() {
            conditions.push(format!("release_type = ${}", param_idx));
            param_idx += 1;
        }
        if filter.status.is_some() {
            conditions.push(format!("status = ${}", param_idx));
            param_idx += 1;
        }
        if filter.country.is_some() {
            conditions.push(format!("country = ${}", param_idx));
            // param_idx += 1;
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, provider, hash, data_cost::TEXT, registered_at_block, registered_at_timestamp,
                   ean_upc, creator, title, release_type, format, packaging, status, release_date,
                   country, recording_ids, distributor_name, manufacturer_name
            FROM midds_releases
            {}
            ORDER BY id {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            limit + 1
        );

        let rows: Vec<ReleaseRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        } else {
            let mut q = sqlx::query_as::<_, ReleaseRow>(&query);
            if let Some(ref provider) = filter.provider {
                q = q.bind(&provider.0[..]);
            }
            if let Some(ref ean) = filter.ean_upc {
                q = q.bind(ean);
            }
            if let Some(ref rt) = filter.release_type {
                q = q.bind(rt);
            }
            if let Some(ref status) = filter.status {
                q = q.bind(status);
            }
            if let Some(ref country) = filter.country {
                q = q.bind(country);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?
        };

        let has_more = rows.len() > limit as usize;
        let releases: Vec<Release> = rows
            .into_iter()
            .take(limit as usize)
            .map(ReleaseRow::into_model)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Release>> = releases
            .into_iter()
            .map(|r| Edge {
                cursor: Cursor {
                    value: r.id.to_string(),
                },
                node: r,
            })
            .collect();

        let page_info = PageInfo {
            has_next_page: has_more,
            has_previous_page: pagination.after.is_some(),
            start_cursor: edges.first().map(|e| e.cursor.clone()),
            end_cursor: edges.last().map(|e| e.cursor.clone()),
        };

        Ok(Connection {
            edges,
            page_info,
            total_count: None,
        })
    }

    async fn delete_release(&self, id: u64) -> StorageResult<()> {
        sqlx::query("DELETE FROM midds_releases WHERE id = $1")
            .bind(id as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(())
    }

    async fn count_releases(&self) -> StorageResult<u64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM midds_releases")
            .fetch_one(&self.pool)
            .await
            .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        Ok(row.0 as u64)
    }

    // -------------------------------------------------------------------------
    // Reorg handling
    // -------------------------------------------------------------------------

    async fn delete_from_block(&self, from_block: u64) -> StorageResult<u64> {
        let mut total_deleted = 0u64;

        // Delete in order of dependencies (children first)
        let releases_result =
            sqlx::query("DELETE FROM midds_releases WHERE registered_at_block >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        total_deleted += releases_result.rows_affected();

        let recordings_result =
            sqlx::query("DELETE FROM midds_recordings WHERE registered_at_block >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        total_deleted += recordings_result.rows_affected();

        let works_result =
            sqlx::query("DELETE FROM midds_musical_works WHERE registered_at_block >= $1")
                .bind(from_block as i64)
                .execute(&self.pool)
                .await
                .map_err(|e| StorageError::query_with_source(e.to_string(), e))?;
        total_deleted += works_result.rows_affected();

        Ok(total_deleted)
    }
}

// =============================================================================
// Row mapping
// =============================================================================

#[derive(sqlx::FromRow)]
struct MusicalWorkRow {
    id: i64,
    provider: Vec<u8>,
    hash: Vec<u8>,
    data_cost: String,
    registered_at_block: i64,
    registered_at_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    iswc: String,
    title: String,
    creation_year: Option<i16>,
    instrumental: Option<bool>,
    language: Option<String>,
    bpm: Option<i16>,
    musical_key: Option<String>,
    work_type: Option<String>,
    creators: serde_json::Value,
}

impl MusicalWorkRow {
    fn into_model(self) -> StorageResult<MusicalWork> {
        let creators: Vec<Creator> = serde_json::from_value(self.creators)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        Ok(MusicalWork {
            id: self.id as u64,
            provider: AccountId(bytes_to_hash32(self.provider, "musical_work.provider")?),
            hash: bytes_to_hash32(self.hash, "musical_work.hash")?,
            data_cost: parse_data_cost(&self.data_cost)?,
            registered_at_block: self.registered_at_block as u64,
            registered_at_timestamp: self.registered_at_timestamp,
            iswc: self.iswc,
            title: self.title,
            creation_year: self.creation_year.map(|y| y as u16),
            instrumental: self.instrumental,
            language: self.language,
            bpm: self.bpm.map(|b| b as u16),
            key: self.musical_key,
            work_type: self.work_type,
            creators,
        })
    }
}

#[derive(sqlx::FromRow)]
struct RecordingRow {
    id: i64,
    provider: Vec<u8>,
    hash: Vec<u8>,
    data_cost: String,
    registered_at_block: i64,
    registered_at_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    isrc: String,
    musical_work_id: i64,
    artist: serde_json::Value,
    title: String,
    recording_year: Option<i16>,
    duration: Option<i16>,
    bpm: Option<i16>,
    musical_key: Option<String>,
    version: Option<String>,
    genres: serde_json::Value,
    producers: serde_json::Value,
    performers: serde_json::Value,
}

impl RecordingRow {
    fn into_model(self) -> StorageResult<Recording> {
        let artist: PartyId = serde_json::from_value(self.artist)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let genres: Vec<String> = serde_json::from_value(self.genres)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let producers: Vec<PartyId> = serde_json::from_value(self.producers)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let performers: Vec<PartyId> = serde_json::from_value(self.performers)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        Ok(Recording {
            id: self.id as u64,
            provider: AccountId(bytes_to_hash32(self.provider, "recording.provider")?),
            hash: bytes_to_hash32(self.hash, "recording.hash")?,
            data_cost: parse_data_cost(&self.data_cost)?,
            registered_at_block: self.registered_at_block as u64,
            registered_at_timestamp: self.registered_at_timestamp,
            isrc: self.isrc,
            musical_work_id: self.musical_work_id as u64,
            artist,
            title: self.title,
            recording_year: self.recording_year.map(|y| y as u16),
            duration: self.duration.map(|d| d as u16),
            bpm: self.bpm.map(|b| b as u16),
            key: self.musical_key,
            version: self.version,
            genres,
            producers,
            performers,
        })
    }
}

#[derive(sqlx::FromRow)]
struct ReleaseRow {
    id: i64,
    provider: Vec<u8>,
    hash: Vec<u8>,
    data_cost: String,
    registered_at_block: i64,
    registered_at_timestamp: Option<chrono::DateTime<chrono::Utc>>,
    ean_upc: String,
    creator: serde_json::Value,
    title: String,
    release_type: String,
    format: String,
    packaging: String,
    status: String,
    release_date: Option<chrono::NaiveDate>,
    country: String,
    recording_ids: serde_json::Value,
    distributor_name: String,
    manufacturer_name: String,
}

impl ReleaseRow {
    fn into_model(self) -> StorageResult<Release> {
        let creator: PartyId = serde_json::from_value(self.creator)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;
        let recording_ids: Vec<u64> = serde_json::from_value(self.recording_ids)
            .map_err(|e| StorageError::serialization_with_source(e.to_string(), e))?;

        let date = self
            .release_date
            .map(|d| Date {
                year: d.year() as u16,
                month: d.month() as u8,
                day: d.day() as u8,
            })
            .unwrap_or(Date {
                year: 0,
                month: 0,
                day: 0,
            });

        Ok(Release {
            id: self.id as u64,
            provider: AccountId(bytes_to_hash32(self.provider, "release.provider")?),
            hash: bytes_to_hash32(self.hash, "release.hash")?,
            data_cost: parse_data_cost(&self.data_cost)?,
            registered_at_block: self.registered_at_block as u64,
            registered_at_timestamp: self.registered_at_timestamp,
            ean_upc: self.ean_upc,
            creator,
            title: self.title,
            release_type: self.release_type,
            format: self.format,
            packaging: self.packaging,
            status: self.status,
            date,
            country: self.country,
            recording_ids,
            distributor_name: self.distributor_name,
            manufacturer_name: self.manufacturer_name,
        })
    }
}

// =============================================================================
// Conversion helpers
// =============================================================================

/// Convert Vec<u8> to [u8; 32] with descriptive error.
fn bytes_to_hash32(bytes: Vec<u8>, field: &str) -> StorageResult<[u8; 32]> {
    bytes.try_into().map_err(|v: Vec<u8>| {
        StorageError::serialization(format!(
            "{} has invalid length: expected 32, got {}",
            field,
            v.len()
        ))
    })
}

/// Parse string to u128 (for data_cost stored as NUMERIC).
fn parse_data_cost(s: &str) -> StorageResult<u128> {
    // Remove decimal point if present (we store as integer)
    let s = s.split('.').next().unwrap_or(s);
    s.parse().map_err(|e| {
        StorageError::serialization(format!("data_cost parse error: {} (value: {})", e, s))
    })
}

// =============================================================================
// Migrations
// =============================================================================

/// SQL migrations for the MIDDS bundle.
pub const MIGRATIONS: &[&str] = &[
    // Migration 0: Create MIDDS tables
    r#"
-- Musical Works table
CREATE TABLE IF NOT EXISTS midds_musical_works (
    id BIGINT PRIMARY KEY,
    provider BYTEA NOT NULL,
    hash BYTEA NOT NULL,
    data_cost NUMERIC NOT NULL,
    registered_at_block BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    registered_at_timestamp TIMESTAMPTZ,

    iswc TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    creation_year SMALLINT,
    instrumental BOOLEAN,
    language TEXT,
    bpm SMALLINT,
    musical_key TEXT,
    work_type TEXT,
    creators JSONB NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_mw_provider ON midds_musical_works(provider);
CREATE INDEX IF NOT EXISTS idx_mw_iswc ON midds_musical_works(iswc);
CREATE INDEX IF NOT EXISTS idx_mw_block ON midds_musical_works(registered_at_block);
CREATE INDEX IF NOT EXISTS idx_mw_language ON midds_musical_works(language);

-- Recordings table
CREATE TABLE IF NOT EXISTS midds_recordings (
    id BIGINT PRIMARY KEY,
    provider BYTEA NOT NULL,
    hash BYTEA NOT NULL,
    data_cost NUMERIC NOT NULL,
    registered_at_block BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    registered_at_timestamp TIMESTAMPTZ,

    isrc TEXT NOT NULL UNIQUE,
    musical_work_id BIGINT NOT NULL,
    artist JSONB NOT NULL,
    title TEXT NOT NULL,
    recording_year SMALLINT,
    duration SMALLINT,
    bpm SMALLINT,
    musical_key TEXT,
    version TEXT,
    genres JSONB NOT NULL DEFAULT '[]',
    producers JSONB NOT NULL DEFAULT '[]',
    performers JSONB NOT NULL DEFAULT '[]'
);

CREATE INDEX IF NOT EXISTS idx_rec_provider ON midds_recordings(provider);
CREATE INDEX IF NOT EXISTS idx_rec_isrc ON midds_recordings(isrc);
CREATE INDEX IF NOT EXISTS idx_rec_musical_work ON midds_recordings(musical_work_id);
CREATE INDEX IF NOT EXISTS idx_rec_block ON midds_recordings(registered_at_block);
CREATE INDEX IF NOT EXISTS idx_rec_version ON midds_recordings(version);

-- Releases table
CREATE TABLE IF NOT EXISTS midds_releases (
    id BIGINT PRIMARY KEY,
    provider BYTEA NOT NULL,
    hash BYTEA NOT NULL,
    data_cost NUMERIC NOT NULL,
    registered_at_block BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
    registered_at_timestamp TIMESTAMPTZ,

    ean_upc TEXT NOT NULL UNIQUE,
    creator JSONB NOT NULL,
    title TEXT NOT NULL,
    release_type TEXT NOT NULL,
    format TEXT NOT NULL,
    packaging TEXT NOT NULL,
    status TEXT NOT NULL,
    release_date DATE,
    country TEXT NOT NULL,
    recording_ids JSONB NOT NULL DEFAULT '[]',
    distributor_name TEXT NOT NULL,
    manufacturer_name TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_rel_provider ON midds_releases(provider);
CREATE INDEX IF NOT EXISTS idx_rel_ean ON midds_releases(ean_upc);
CREATE INDEX IF NOT EXISTS idx_rel_type ON midds_releases(release_type);
CREATE INDEX IF NOT EXISTS idx_rel_country ON midds_releases(country);
CREATE INDEX IF NOT EXISTS idx_rel_status ON midds_releases(status);
CREATE INDEX IF NOT EXISTS idx_rel_block ON midds_releases(registered_at_block);
"#,
];
