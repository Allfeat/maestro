# Creating Handler Bundles

Handler bundles are the primary extension mechanism in Maestro. This guide walks you through creating a custom bundle to index a Substrate pallet.

## Overview

A bundle is a self-contained unit that:

1. Defines database tables via migrations
2. Handles events from specific pallets
3. Optionally exposes GraphQL queries

## Quick Start

Let's create a bundle for a hypothetical `Rewards` pallet.

### 1. Create the Module Structure

```
crates/handlers/src/
├── rewards/
│   ├── mod.rs          # Bundle implementation
│   ├── handler.rs      # Event handler
│   ├── models.rs       # Domain models
│   ├── storage.rs      # Database operations
│   └── graphql.rs      # GraphQL queries (optional)
└── lib.rs              # Export the bundle
```

### 2. Define Your Models

```rust
// rewards/models.rs
use chrono::{DateTime, Utc};
use maestro_core::models::{AccountId, BlockHash};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reward {
    pub id: String,
    pub block_number: u64,
    pub block_hash: BlockHash,
    pub event_index: u32,
    pub account: AccountId,
    pub amount: u128,
    pub reward_type: RewardType,
    pub timestamp: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RewardType {
    Staking,
    Validator,
    Nominator,
}
```

### 3. Implement the Storage Layer

```rust
// rewards/storage.rs
use async_trait::async_trait;
use sqlx::PgPool;
use maestro_core::error::{StorageError, StorageResult};

use super::models::Reward;

#[async_trait]
pub trait RewardsStorage: Send + Sync {
    async fn insert_rewards(&self, rewards: &[Reward]) -> StorageResult<()>;
    async fn get_rewards_for_account(&self, account: &[u8]) -> StorageResult<Vec<Reward>>;
}

pub struct PgRewardsStorage {
    pool: PgPool,
}

impl PgRewardsStorage {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl RewardsStorage for PgRewardsStorage {
    async fn insert_rewards(&self, rewards: &[Reward]) -> StorageResult<()> {
        for reward in rewards {
            sqlx::query(
                r#"
                INSERT INTO rewards (id, block_number, block_hash, event_index,
                                     account, amount, reward_type, timestamp)
                VALUES ($1, $2, $3, $4, $5, $6::NUMERIC, $7, $8)
                ON CONFLICT (block_number, event_index) DO NOTHING
                "#,
            )
            .bind(&reward.id)
            .bind(reward.block_number as i64)
            .bind(&reward.block_hash.0[..])
            .bind(reward.event_index as i32)
            .bind(&reward.account.0[..])
            .bind(reward.amount.to_string())
            .bind(format!("{:?}", reward.reward_type))
            .bind(reward.timestamp)
            .execute(&self.pool)
            .await
            .map_err(|e| StorageError::QueryError(e.to_string()))?;
        }
        Ok(())
    }

    async fn get_rewards_for_account(&self, account: &[u8]) -> StorageResult<Vec<Reward>> {
        // Implementation...
        todo!()
    }
}

/// SQL migrations for this bundle
pub const MIGRATIONS: &[&str] = &[
    r#"
    CREATE TABLE rewards (
        id TEXT PRIMARY KEY,
        block_number BIGINT NOT NULL REFERENCES blocks(number) ON DELETE CASCADE,
        block_hash BYTEA NOT NULL,
        event_index INTEGER NOT NULL,
        account BYTEA NOT NULL,
        amount NUMERIC(39, 0) NOT NULL,
        reward_type TEXT NOT NULL,
        timestamp TIMESTAMPTZ,
        UNIQUE(block_number, event_index)
    );

    CREATE INDEX idx_rewards_account ON rewards(account);
    CREATE INDEX idx_rewards_block ON rewards(block_number);
    "#,
];
```

### 4. Implement the Event Handler

```rust
// rewards/handler.rs
use std::sync::Arc;
use async_trait::async_trait;
use tracing::warn;

use maestro_core::error::DomainResult;
use maestro_core::models::Block;
use maestro_core::ports::{HandlerOutputs, PalletHandler, RawEvent, RawExtrinsic};

use super::models::{Reward, RewardType};
use super::storage::RewardsStorage;

pub struct RewardsHandler {
    storage: Arc<dyn RewardsStorage>,
}

impl RewardsHandler {
    pub fn new(storage: Arc<dyn RewardsStorage>) -> Self {
        Self { storage }
    }

    fn process_reward_event(&self, event: &RawEvent, block: &Block) -> Option<Reward> {
        // Extract data from event.data (serde_json::Value)
        let account = event.data.get("who")?.as_str()?;
        let amount = event.data.get("amount")?.as_str()?.parse().ok()?;

        Some(Reward {
            id: format!("{}-{}", block.number, event.index),
            block_number: block.number,
            block_hash: block.hash.clone(),
            event_index: event.index,
            account: parse_account(account)?,
            amount,
            reward_type: RewardType::Staking,
            timestamp: block.timestamp,
        })
    }
}

#[async_trait]
impl PalletHandler for RewardsHandler {
    fn pallet_name(&self) -> &'static str {
        "Staking"  // Match the pallet name in events
    }

    async fn handle_event(
        &self,
        event: &RawEvent,
        block: &Block,
        _extrinsic: Option<&RawExtrinsic>,
    ) -> DomainResult<HandlerOutputs> {
        let mut outputs = HandlerOutputs::new();

        match event.name.as_str() {
            "Rewarded" => {
                if let Some(reward) = self.process_reward_event(event, block) {
                    outputs.add("rewards", "rewards", &reward)?;
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
        let rewards: Vec<Reward> = outputs.get_typed("rewards", "rewards");

        if !rewards.is_empty() {
            self.storage.insert_rewards(&rewards).await?;
        }

        Ok(HandlerOutputs::new())
    }

    fn priority(&self) -> i32 {
        0  // Default priority
    }
}
```

### 5. Create the Bundle

```rust
// rewards/mod.rs
mod handler;
pub mod models;
pub mod storage;

use std::sync::Arc;
use sqlx::PgPool;

use maestro_core::ports::PalletHandler;
use crate::HandlerBundle;

pub use handler::RewardsHandler;
pub use storage::{RewardsStorage, PgRewardsStorage, MIGRATIONS};

pub struct RewardsBundle {
    pool: PgPool,
}

impl RewardsBundle {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl HandlerBundle for RewardsBundle {
    fn name(&self) -> &'static str {
        "rewards"
    }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        let storage = Arc::new(PgRewardsStorage::new(self.pool.clone()));
        vec![Arc::new(RewardsHandler::new(storage))]
    }

    fn migrations(&self) -> &'static [&'static str] {
        MIGRATIONS
    }

    fn tables_to_purge(&self) -> &'static [&'static str] {
        &["rewards"]
    }

    fn priority(&self) -> i32 {
        50  // Run after core handlers
    }
}
```

### 6. Register the Bundle

```rust
// bin/maestro/src/main.rs
use maestro_handlers::RewardsBundle;

// In main():
let mut bundle_registry = BundleRegistry::new();
bundle_registry.register(Box::new(BalancesBundle::new(db.pool().clone())));
bundle_registry.register(Box::new(RewardsBundle::new(db.pool().clone())));  // Add here
```

## Adding GraphQL Queries

```rust
// rewards/graphql.rs
use async_graphql::{Context, Object, Result};
use std::sync::Arc;

use super::storage::RewardsStorage;

#[derive(Default)]
pub struct RewardsQuery;

#[Object]
impl RewardsQuery {
    async fn rewards_for_account(
        &self,
        ctx: &Context<'_>,
        account: String,
    ) -> Result<Vec<Reward>> {
        let storage = ctx.data::<Arc<dyn RewardsStorage>>()?;
        let account_bytes = hex::decode(account.trim_start_matches("0x"))?;

        Ok(storage.get_rewards_for_account(&account_bytes).await?)
    }
}
```

Then merge in `main.rs`:

```rust
#[derive(MergedObject, Default)]
struct Query(CoreQuery, BalancesQuery, RewardsQuery);
```

## Best Practices

### Event Parsing

Events can have different formats depending on the runtime version. Use fallback parsing:

```rust
fn parse_account(data: &serde_json::Value) -> Option<AccountId> {
    // Try named field first
    data.get("who")
        .or_else(|| data.get("account"))
        // Fall back to positional
        .or_else(|| data.get(0))
        .and_then(parse_account_value)
}
```

### Error Handling

- Log warnings for parsing failures, don't crash
- Use `?` for storage errors (they should propagate)
- Return `HandlerOutputs::new()` for events you don't handle

### Performance

- Batch inserts when possible
- Use `ON CONFLICT DO NOTHING` for idempotency
- Add indexes for common query patterns

### Testing

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_parse_reward_event() {
        let event = RawEvent {
            pallet: "Staking".into(),
            name: "Rewarded".into(),
            data: json!({
                "who": "0x" + &"aa".repeat(32),
                "amount": "1000000000000"
            }),
            // ...
        };

        // Test parsing logic
    }
}
```

## Migration Guidelines

1. **Always reference `blocks`** with `ON DELETE CASCADE`
2. **Use `NUMERIC(39, 0)`** for u128 amounts
3. **Add `UNIQUE` constraints** for idempotency
4. **Create indexes** for query patterns

## Troubleshooting

### Handler not receiving events

- Check `pallet_name()` matches exactly (case-sensitive)
- Verify the event exists in the runtime metadata

### Events missing data

- Check runtime version compatibility
- Log raw event data for debugging

### Migration failures

- Ensure tables reference `blocks(number)`
- Check for conflicts with existing tables
