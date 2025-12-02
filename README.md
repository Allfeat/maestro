<div align="center">

<img src="assets/banner.png" alt="Maestro" width="600" />

**A high-performance Substrate blockchain indexer**

[![Build Status](https://img.shields.io/github/actions/workflow/status/allfeat/maestro/ci.yml?branch=main&style=flat-square)](https://github.com/allfeat/maestro/actions)
[![License](https://img.shields.io/badge/license-Apache%202.0-blue?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75+-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)

[Features](#features) • [Quick Start](#quick-start) • [Documentation](#documentation) • [Architecture](#architecture) • [Contributing](#contributing)

</div>

---

## Overview

Maestro is a fast, modular blockchain indexer for Substrate-based chains. It subscribes to finalized blocks, decodes extrinsics and events, and exposes the data through a GraphQL API.

Built with a plugin architecture, Maestro makes it easy to add custom indexing logic for any pallet without modifying the core codebase.

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│  Substrate Node │────▶│     Maestro     │────▶│   PostgreSQL    │
│   (WebSocket)   │     │    (Indexer)    │     │                 │
└─────────────────┘     └────────┬────────┘     └─────────────────┘
                                 │
                                 ▼
                        ┌─────────────────┐
                        │   GraphQL API   │
                        │   (Port 4000)   │
                        └─────────────────┘
```

## Features

- **Real-time Indexing** — Subscribes to finalized blocks via WebSocket for instant updates
- **GraphQL API** — Query blocks, extrinsics, and events with a powerful, typed API
- **Plugin System** — Add custom handlers for any pallet with the bundle architecture
- **Reorg Handling** — Automatically handles chain reorganizations
- **Prometheus Metrics** — Built-in metrics for monitoring and alerting
- **Type-Safe** — Written in Rust with compile-time guarantees

## Quick Start

### Prerequisites

- Rust 1.75+
- PostgreSQL 14+
- A running Substrate node

### Installation

```bash
# Clone the repository
git clone https://github.com/allfeat/maestro.git
cd maestro

# Build in release mode
cargo build --release
```

### Running

```bash
# Start PostgreSQL (using Docker)
docker-compose up -d

# Run the indexer
./target/release/maestro \
  --ws-url ws://localhost:9944 \
  --database-url postgres://maestro:maestro@localhost:5432/maestro
```

The GraphQL playground will be available at `http://localhost:4000/graphql`.

### Example Query

```graphql
query {
  blocks(first: 5) {
    edges {
      node {
        number
        hash
        timestamp
        extrinsicCount
        eventCount
      }
    }
  }
}
```

## Configuration

Maestro can be configured via CLI arguments or environment variables:

| Option           | Environment    | Default                        | Description                  |
| ---------------- | -------------- | ------------------------------ | ---------------------------- |
| `--ws-url`       | `WS_URL`       | `ws://127.0.0.1:9944`          | Substrate node WebSocket URL |
| `--database-url` | `DATABASE_URL` | `postgres://localhost/maestro` | PostgreSQL connection string |
| `--graphql-port` | `GRAPHQL_PORT` | `4000`                         | GraphQL server port          |
| `--metrics-port` | `METRICS_PORT` | `9090`                         | Prometheus metrics port      |
| `--log-level`    | `LOG_LEVEL`    | `info`                         | Log verbosity                |
| `--block-mode`   | `BLOCK_MODE`   | `finalized`                    | Subscription mode (finalized/best) |

### Maintenance Commands

```bash
# Run database migrations only
maestro --migrate-only

# Purge all indexed data (restart from genesis)
maestro --purge

# Purge without confirmation (for scripts)
maestro --purge -y
```

## Architecture

Maestro follows a hexagonal (ports & adapters) architecture:

```
bin/maestro/           # CLI and service orchestration
crates/
├── core/              # Domain models, ports (traits), business logic
├── substrate/         # Substrate RPC adapter
├── storage/           # PostgreSQL adapter
├── graphql/           # GraphQL API server
└── handlers/          # Pallet-specific handler bundles
```

### Handler Bundles

Bundles are the extension point for custom indexing logic. Each bundle can:

- Define its own database schema via migrations
- Register handlers for specific pallets
- Expose custom GraphQL queries

```rust
impl HandlerBundle for MyBundle {
    fn name(&self) -> &'static str { "my_bundle" }

    fn handlers(&self) -> Vec<Arc<dyn PalletHandler>> {
        vec![Arc::new(MyHandler::new(self.storage.clone()))]
    }

    fn migrations(&self) -> &'static [&'static str] {
        &[include_str!("migrations/001_init.sql")]
    }
}
```

See the [handlers documentation](docs/handlers.md) for a complete guide.

## Documentation

- [Architecture Guide](docs/architecture.md)
- [Creating Handler Bundles](docs/handlers.md)
- [GraphQL Schema Reference](docs/graphql.md)

## Performance

Benchmarks on commodity hardware (AMD Ryzen 5, 32GB RAM, NVMe SSD):

| Metric         | Value                             |
| -------------- | --------------------------------- |
| Indexing speed | ~500 blocks/sec (historical sync) |
| Query latency  | <10ms (p99)                       |
| Memory usage   | ~200MB (steady state)             |

## Contributing

Contributions are welcome! Please read our [Contributing Guide](CONTRIBUTING.md) before submitting a PR.

```bash
# Run tests
cargo test

# Run linter
cargo clippy --all -- -D warnings

# Format code
cargo fmt
```

## Compatibility

> **Note**: The core indexer and the Balances handler have been tested on a Substrate chain running **polkadot-stable2509** (Polkadot SDK). Compatibility with other versions is not guaranteed and may require adjustments. If you encounter issues with a different runtime version, please open an issue.

> **Best block mode**: When using `--block-mode best`, blocks may be reorged. The indexer will automatically detect and reindex affected blocks, but data may temporarily be inconsistent during reorgs.

## Note on Development

This project was developed with the assistance of Large Language Models (LLMs), specifically Claude by Anthropic. While all code has been reviewed and tested, please report any issues you encounter.

## License

Maestro is licensed under the [Apache License 2.0](LICENSE).

---

<div align="center">

**[Website](https://allfeat.org)** • **[Discord](https://discord.allfeat.com)** • **[Twitter](https://twitter.com/allfeat_IP)**

Built with ❤️ by [Allfeat](https://github.com/allfeat)

</div>
