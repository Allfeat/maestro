# Contributing to Maestro

Thank you for your interest in contributing to Maestro! This document provides guidelines and information for contributors.

## Code of Conduct

By participating in this project, you agree to maintain a respectful and inclusive environment for everyone.

## Getting Started

### Development Environment

1. **Install Rust** (1.75 or later):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Clone the repository**:
   ```bash
   git clone https://github.com/allfeat/maestro.git
   cd maestro
   ```

3. **Using Nix** (recommended):
   ```bash
   # If you have direnv installed
   direnv allow

   # Or manually
   nix develop
   ```

4. **Start the database**:
   ```bash
   docker-compose up -d
   ```

### Running Tests

```bash
# Run all tests
cargo test

# Run tests for a specific crate
cargo test -p maestro-core

# Run tests with output
cargo test -- --nocapture
```

### Code Quality

Before submitting a PR, ensure your code passes all checks:

```bash
# Format code
cargo fmt

# Run linter (must pass with no warnings)
cargo clippy --all -- -D warnings

# Run all tests
cargo test
```

## Project Structure

```
maestro/
├── bin/maestro/       # Binary entrypoint
├── crates/
│   ├── core/          # Domain models and traits
│   ├── substrate/     # Substrate RPC client
│   ├── storage/       # PostgreSQL implementation
│   ├── graphql/       # GraphQL API
│   └── handlers/      # Pallet handler bundles
├── docs/              # Documentation
└── docker-compose.yml # Local development setup
```

## Making Changes

### Branch Naming

- `feat/description` — New features
- `fix/description` — Bug fixes
- `docs/description` — Documentation changes
- `refactor/description` — Code refactoring

### Commit Messages

We follow [Conventional Commits](https://www.conventionalcommits.org/):

```
type(scope): description

[optional body]
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`

Examples:
```
feat(handlers): add staking pallet handler
fix(storage): handle null timestamps in block queries
docs(readme): update quick start guide
```

### Pull Requests

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes
4. Ensure all tests pass
5. Submit a PR with a clear description

#### PR Checklist

- [ ] Code compiles without warnings
- [ ] All tests pass
- [ ] Code is formatted (`cargo fmt`)
- [ ] Clippy passes (`cargo clippy --all -- -D warnings`)
- [ ] Documentation is updated (if applicable)
- [ ] CHANGELOG is updated (for significant changes)

## Adding a Handler Bundle

To add indexing support for a new pallet:

1. Create a new module in `crates/handlers/src/`
2. Implement the `HandlerBundle` trait
3. Add SQL migrations for your schema
4. Register the bundle in `bin/maestro/src/main.rs`
5. Add GraphQL queries (optional)

See the `balances` bundle for a reference implementation.

## Architecture Guidelines

- **Hexagonal Architecture**: Keep domain logic in `core`, adapters in separate crates
- **Error Handling**: Use `thiserror` for error types, propagate with `?`
- **Async**: All I/O operations should be async
- **Testing**: Write unit tests for business logic, integration tests for adapters

## Questions?

- Open a [GitHub Discussion](https://github.com/allfeat/maestro/discussions)
- Join our [Discord](https://discord.gg/allfeat)

## License

By contributing, you agree that your contributions will be licensed under the Apache License 2.0.
