//! Event repository implementation for PostgreSQL.

use async_trait::async_trait;
use sqlx::PgPool;

use maestro_core::error::{StorageError, StorageResult};
use maestro_core::models::{BlockHash, Event};
use maestro_core::ports::{
    Connection, Cursor, Edge, EventFilter, EventRepository, OrderDirection, PageInfo, Pagination,
};

use super::helpers::bytes_to_hash32;

// =============================================================================
// Repository Implementation
// =============================================================================

/// PostgreSQL implementation of EventRepository.
pub struct PgEventRepository {
    pool: PgPool,
}

impl PgEventRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventRepository for PgEventRepository {
    async fn insert_events(&self, events: &[Event]) -> StorageResult<()> {
        if events.is_empty() {
            return Ok(());
        }

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        for event in events {
            sqlx::query(
                r#"
                INSERT INTO events (
                    id, block_number, block_hash, index, extrinsic_index,
                    pallet, name, data, topics
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                ON CONFLICT (block_number, index) DO NOTHING
                "#,
            )
            .bind(&event.id)
            .bind(event.block_number as i64)
            .bind(&event.block_hash.0[..])
            .bind(event.index as i32)
            .bind(event.extrinsic_index.map(|i| i as i32))
            .bind(&event.pallet)
            .bind(&event.name)
            .bind(&event.data)
            .bind(&event.topics)
            .execute(&mut *tx)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }

        tx.commit()
            .await
            .map_err(|e| StorageError::TransactionError(e.to_string()))?;

        Ok(())
    }

    async fn get_event(&self, id: &str) -> StorageResult<Option<Event>> {
        let row = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, block_number, block_hash, index, extrinsic_index,
                   pallet, name, data, topics
            FROM events
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        row.map(EventRow::into_event).transpose()
    }

    async fn list_events_for_block(&self, block_number: u64) -> StorageResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, block_number, block_hash, index, extrinsic_index,
                   pallet, name, data, topics
            FROM events
            WHERE block_number = $1
            ORDER BY index ASC
            "#,
        )
        .bind(block_number as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter().map(EventRow::into_event).collect()
    }

    async fn list_events_for_extrinsic(
        &self,
        block_number: u64,
        extrinsic_index: u32,
    ) -> StorageResult<Vec<Event>> {
        let rows = sqlx::query_as::<_, EventRow>(
            r#"
            SELECT id, block_number, block_hash, index, extrinsic_index,
                   pallet, name, data, topics
            FROM events
            WHERE block_number = $1 AND extrinsic_index = $2
            ORDER BY index ASC
            "#,
        )
        .bind(block_number as i64)
        .bind(extrinsic_index as i32)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StorageError::QueryError(e.to_string()))?;

        rows.into_iter().map(EventRow::into_event).collect()
    }

    async fn list_events(
        &self,
        filter: EventFilter,
        pagination: Pagination,
        order: OrderDirection,
    ) -> StorageResult<Connection<Event>> {
        let limit = pagination.first.or(pagination.last).unwrap_or(20).min(100);
        let order_sql = match order {
            OrderDirection::Asc => "ASC",
            OrderDirection::Desc => "DESC",
        };

        // Build dynamic query.
        //
        // SAFETY: This dynamic SQL is safe from injection because:
        // 1. Column names are hardcoded (block_number, extrinsic_index, pallet, name)
        // 2. Operators (=, AND) are hardcoded
        // 3. All VALUES are parameterized via $1, $2, etc. and bound separately
        // 4. Order direction comes from enum (ASC/DESC), not user strings
        let mut conditions = Vec::new();
        let mut param_idx = 1;

        if filter.block_number.is_some() {
            conditions.push(format!("block_number = ${}", param_idx));
            param_idx += 1;
        }
        if filter.extrinsic_index.is_some() {
            conditions.push(format!("extrinsic_index = ${}", param_idx));
            param_idx += 1;
        }
        if filter.pallet.is_some() {
            conditions.push(format!("pallet = ${}", param_idx));
            param_idx += 1;
        }
        if filter.name.is_some() {
            conditions.push(format!("name = ${}", param_idx));
        }

        let where_clause = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };

        let query = format!(
            r#"
            SELECT id, block_number, block_hash, index, extrinsic_index,
                   pallet, name, data, topics
            FROM events
            {}
            ORDER BY block_number {}, index {}
            LIMIT {}
            "#,
            where_clause,
            order_sql,
            order_sql,
            limit + 1
        );

        let rows: Vec<EventRow> = if conditions.is_empty() {
            sqlx::query_as(&query)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        } else {
            let mut q = sqlx::query_as::<_, EventRow>(&query);
            if let Some(bn) = filter.block_number {
                q = q.bind(bn as i64);
            }
            if let Some(ei) = filter.extrinsic_index {
                q = q.bind(ei as i32);
            }
            if let Some(ref p) = filter.pallet {
                q = q.bind(p);
            }
            if let Some(ref n) = filter.name {
                q = q.bind(n);
            }
            q.fetch_all(&self.pool)
                .await
                .map_err(|e| StorageError::QueryError(e.to_string()))?
        };

        let has_more = rows.len() > limit as usize;
        let events: Vec<Event> = rows
            .into_iter()
            .take(limit as usize)
            .map(EventRow::into_event)
            .collect::<StorageResult<Vec<_>>>()?;

        let edges: Vec<Edge<Event>> = events
            .into_iter()
            .map(|evt| Edge {
                cursor: Cursor {
                    value: evt.id.clone(),
                },
                node: evt,
            })
            .collect();

        Ok(Connection {
            edges: edges.clone(),
            page_info: PageInfo {
                has_next_page: has_more,
                has_previous_page: pagination.after.is_some(),
                start_cursor: edges.first().map(|e| e.cursor.clone()),
                end_cursor: edges.last().map(|e| e.cursor.clone()),
            },
            total_count: None,
        })
    }

    async fn delete_events_from(&self, from_block: u64) -> StorageResult<u64> {
        let result = sqlx::query("DELETE FROM events WHERE block_number >= $1")
            .bind(from_block as i64)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        Ok(result.rows_affected())
    }
}

// =============================================================================
// Row Mapping
// =============================================================================

#[derive(sqlx::FromRow)]
struct EventRow {
    id: String,
    block_number: i64,
    block_hash: Vec<u8>,
    index: i32,
    extrinsic_index: Option<i32>,
    pallet: String,
    name: String,
    data: serde_json::Value,
    topics: Vec<String>,
}

impl EventRow {
    fn into_event(self) -> StorageResult<Event> {
        let block_hash = bytes_to_hash32(self.block_hash, "event.block_hash")?;

        Ok(Event {
            id: self.id,
            block_number: self.block_number as u64,
            block_hash: BlockHash(block_hash),
            index: self.index as u32,
            extrinsic_index: self.extrinsic_index.map(|i| i as u32),
            pallet: self.pallet,
            name: self.name,
            data: self.data,
            topics: self.topics,
        })
    }
}
