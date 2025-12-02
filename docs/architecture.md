# Architecture Guide

Maestro follows a **hexagonal architecture** (also known as ports & adapters), which separates business logic from external concerns like databases and APIs.

## Overview

```
                    ┌─────────────────────────────────────────────┐
                    │                   Adapters                   │
                    │  ┌───────────┐  ┌───────────┐  ┌──────────┐ │
                    │  │ Substrate │  │ PostgreSQL│  │  GraphQL │ │
                    │  │   RPC     │  │  Storage  │  │   API    │ │
                    │  └─────┬─────┘  └─────┬─────┘  └────┬─────┘ │
                    │        │              │             │       │
                    └────────┼──────────────┼─────────────┼───────┘
                             │              │             │
                    ┌────────┼──────────────┼─────────────┼───────┐
                    │        ▼              ▼             ▼       │
                    │  ┌───────────────────────────────────────┐  │
                    │  │              Ports (Traits)            │  │
                    │  │  BlockSource  Repository  Handler     │  │
                    │  └───────────────────────────────────────┘  │
                    │                      │                      │
                    │                      ▼                      │
                    │  ┌───────────────────────────────────────┐  │
                    │  │            Domain (Core)              │  │
                    │  │   Models   Services   Business Logic  │  │
                    │  └───────────────────────────────────────┘  │
                    │                    Core                     │
                    └─────────────────────────────────────────────┘
```

## Crate Structure

### `maestro-core`

The heart of the system. Contains:

- **Models**: `Block`, `Extrinsic`, `Event`, `IndexerCursor`
- **Ports (Traits)**: `BlockSource`, `Repository`, `PalletHandler`
- **Services**: `IndexerService` - the main indexing loop
- **Errors**: Typed error hierarchy with automatic conversion

```rust
// Example: The BlockSource port
#[async_trait]
pub trait BlockSource: Send + Sync {
    async fn genesis_hash(&self) -> ChainResult<BlockHash>;
    async fn finalized_head(&self) -> ChainResult<Block>;
    async fn get_block(&self, hash: &BlockHash) -> ChainResult<Option<RawBlock>>;
    async fn subscribe_finalized(&self) -> ChainResult<BlockStream>;
}
```

### `maestro-substrate`

Adapter implementing `BlockSource` for Substrate nodes via WebSocket RPC.

- Uses `subxt` for type-safe RPC calls
- Handles connection management and reconnection
- Decodes SCALE-encoded data

### `maestro-storage`

PostgreSQL adapter implementing repository traits.

- Connection pooling with `sqlx`
- Atomic transactions for block persistence
- Migration management

### `maestro-graphql`

HTTP API layer using Axum and async-graphql.

- Relay-style pagination
- Filtering and sorting
- Playground for development

### `maestro-handlers`

Plugin system for pallet-specific indexing.

- `HandlerBundle` trait for self-contained plugins
- Per-bundle migrations
- Custom GraphQL queries

## Data Flow

### Indexing Flow

```
1. Subscribe to finalized blocks
         │
         ▼
2. Fetch full block data (extrinsics, events)
         │
         ▼
3. Decode SCALE data to domain models
         │
         ▼
4. Route events to registered handlers
         │
         ▼
5. Handlers extract domain entities
         │
         ▼
6. Persist all data in single transaction
         │
         ▼
7. Update cursor position
```

### Query Flow

```
1. GraphQL request received
         │
         ▼
2. Parse and validate query
         │
         ▼
3. Execute against repositories
         │
         ▼
4. Transform to GraphQL types
         │
         ▼
5. Return JSON response
```

## Error Handling

Errors are organized in a hierarchy that enables the `?` operator across boundaries:

```
IndexerError
├── Domain(DomainError)
│   ├── BlockNotFound
│   ├── DecodingError
│   ├── ReorgDetected
│   └── Storage(StorageError)
├── Storage(StorageError)
│   ├── ConnectionError
│   ├── QueryError
│   └── TransactionError
└── Chain(ChainError)
    ├── ConnectionFailed
    ├── RpcError
    └── SubscriptionError
```

## Concurrency Model

- **Tokio runtime** for async I/O
- **Single indexer task** processes blocks sequentially (ensures ordering)
- **GraphQL server** handles requests concurrently
- **Connection pools** for database access

## Extension Points

### Adding a New Handler

1. Create a struct implementing `PalletHandler`
2. Bundle it with `HandlerBundle`
3. Register in `main.rs`

### Adding a New Storage Backend

1. Implement repository traits from `maestro-core`
2. Swap implementation in `main.rs`

### Adding Custom GraphQL Queries

1. Create query type with `#[Object]`
2. Merge with `MergedObject` in `main.rs`

## Testing Strategy

- **Unit tests**: Domain logic in `core`
- **Integration tests**: Storage with test database
- **Doc tests**: API examples (marked `ignore` when requiring infrastructure)
