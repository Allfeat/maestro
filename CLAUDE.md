# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Maestro is a Substrate blockchain indexer that indexes finalized blocks and their data (extrinsics, events) from Substrate-based chains and exposes them via a GraphQL API. Built with Rust using hexagonal architecture (ports & adapters pattern).

## Build & Development Commands

```bash
# Build
cargo build                           # Debug build
cargo build --release                 # Release build (with LTO)
cargo build -p maestro-core           # Build specific crate

# Test
cargo test                            # All tests
cargo test -p maestro-core            # Test specific crate
cargo test -- --nocapture             # Show stdout

# Lint & Format
cargo clippy --all -- -D warnings     # Lint (treat warnings as errors)
cargo fmt                             # Format code

# Run
cargo run --bin maestro               # Run with defaults
cargo run --bin maestro -- --help     # Show CLI options
cargo run --bin maestro -- --migrate-only  # Run migrations and exit
cargo run --bin maestro -- --purge    # Purge all data (with confirmation)
cargo run --bin maestro -- --purge -y # Purge all data (skip confirmation)

# Database (Docker)
docker-compose up -d                  # Start PostgreSQL
docker-compose down                   # Stop PostgreSQL
```

## CLI Arguments

| Argument | Env Var | Default | Description |
|----------|---------|---------|-------------|
| `--ws-url` | `WS_URL` | `ws://127.0.0.1:9944` | Substrate node WebSocket URL |
| `--database-url` | `DATABASE_URL` | `postgres://localhost/maestro` | PostgreSQL connection URL |
| `--graphql-port` | `GRAPHQL_PORT` | `4000` | GraphQL server port |
| `--metrics-port` | `METRICS_PORT` | `9090` | Prometheus metrics port |
| `--log-level` | `LOG_LEVEL` | `info` | Log level (trace, debug, info, warn, error) |
| `--json-logs` | `JSON_LOGS` | `false` | Enable JSON log output |
| `--migrate-only` | - | - | Run migrations and exit |
| `--purge` | - | - | Purge all indexed data and exit |
| `-y, --yes` | - | - | Skip confirmation prompts |

## Architecture

```
bin/maestro/           # Binary - CLI, service orchestration
crates/
├── core/              # Domain layer - models, ports (traits), services
├── substrate/         # Substrate RPC adapter (implements BlockSource)
├── storage/           # PostgreSQL adapter (implements Repository traits)
└── graphql/           # GraphQL API server (Axum + async-graphql)
```

### Dependency Flow
```
maestro (binary)
├── maestro-core         (domain logic)
├── maestro-substrate    → maestro-core
├── maestro-storage      → maestro-core
└── maestro-graphql      → maestro-core, maestro-storage
```

### Key Patterns

- **Ports & Adapters**: Core defines traits (`BlockSource`, `Repository`, `Handler`), adapters implement them
- **Models**: `Block`, `Extrinsic`, `Event`, `IndexerCursor` - all in `maestro-core/src/models/`
- **Error Types**: `DomainError`, `StorageError`, `ChainError`, `IndexerError` - all use `thiserror`
- **Async**: Tokio runtime with `async-trait` for interface definitions

### Indexing Flow
1. Subscribe to finalized blocks via Substrate RPC (WebSocket)
2. Decode extrinsics/events from SCALE format
3. Call registered handlers (extensibility point for pallet-specific logic)
4. Store in PostgreSQL via SQLx
5. Update cursor for progress tracking

## Development Environment

```bash
# Nix (recommended)
direnv allow              # Auto-loads flake.nix on cd
# or manually: nix develop

# Provides: Rust toolchain, clang, protobuf, openssl, pkg-config
```

## Database

PostgreSQL 16 via Docker. Tables: `blocks`, `extrinsics`, `events`, `cursor`.
Migrations in `crates/storage/migrations/`.
Credentials: `maestro:maestro@localhost:5432/maestro`

## Key Dependencies

- **subxt 0.44**: Substrate RPC client
- **sqlx 0.8**: PostgreSQL with compile-time query checking
- **async-graphql 7.0**: GraphQL server
- **axum 0.8**: HTTP framework
- **parity-scale-codec**: SCALE encoding/decoding
