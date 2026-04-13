# Codebase Cleanup & Refactor — Design

**Date**: 2026-04-13
**Status**: Approved (brainstorming phase complete, awaiting implementation plan)
**Next project unlocked**: Dynamic TUI refonte

## Context

Maestro is a Substrate blockchain indexer (~15k LoC Rust, hexagonal architecture). Before starting the next major project — a dynamic TUI replacing the current log-based operator view — the codebase needs a cleanup pass with the following goals, in this order:

1. **Decouple observability from logic** (new axis, surfaced during brainstorming). The TUI will need structured, typed access to indexer state. Today, most useful state lives inside `tracing::info!` strings, which is unusable from a TUI.
2. **Deduplicate handler boilerplate**. Balances, ATS, and MIDDS bundles repeat the same `handle_event → outputs → on_block_end → persist` pattern with minor variations. A shared trait can absorb this.
3. **Split oversized files**. Several files exceed 500 lines with multiple responsibilities — `midds/storage.rs` (1146), `midds/handler.rs` (743), `core/services/indexer.rs` (686), `midds/graphql.rs` (676), `ats/storage.rs` (578), `graphql/schema.rs` (552).
4. **Perf pass** (optional, last). Only optimizations guided by profiling, after the structural refactor.

ATS and MIDDS will be rewritten later by the user, so their deep refactor is out-of-scope — they only need minimal migration to fit the new pipeline.

## Goals

- Unlock TUI work by giving services a typed event stream.
- Reduce handler boilerplate so adding a new pallet is low-friction.
- Bring service and binary files under ~250 lines where reasonable.
- Keep `cargo test`, `cargo clippy --all -- -D warnings`, `cargo fmt --check` green at every commit.
- Zero functional regression (indexer keeps indexing, GraphQL keeps querying, metrics keep flowing).

## Non-Goals

- Reorgs / best-block handling.
- Multi-chain indexing.
- Event persistence (the bus is in-process only).
- DB schema changes.
- Rewriting ATS/MIDDS (user will do that in a later project).
- Implementing the TUI itself (this refactor prepares the ground; TUI is the next project).
- Async runtime or `subxt` upgrades.
- Speculative perf optimizations (only profiling-guided in Phase 5).

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                         bin/maestro                          │
│  main.rs (entry, ≤100 lines)                                 │
│  cli.rs, startup.rs, runtime.rs, commands.rs, event_sinks.rs │
└──────┬───────────────────────────────────────────────────────┘
       │
       ▼
┌──────────────────────────────────────────────────────────────┐
│                      maestro-core                            │
│                                                              │
│  services/indexer/ (split from indexer.rs)                   │
│    ├── mod.rs         IndexerService + run()                 │
│    ├── config.rs      IndexerConfig, IndexMode, Backfill     │
│    ├── dispatcher.rs  IndexMode decision (pure)              │
│    ├── live.rs        run_live_loop                          │
│    └── pipeline.rs    index_single_block                     │
│                                                              │
│  services/backfill.rs (unchanged, emits events)              │
│                                                              │
│  events/ ★ NEW ★                                             │
│    ├── mod.rs         EventBus, typed channels               │
│    ├── indexer.rs     IndexerEvent enum                      │
│    ├── backfill.rs    BackfillEvent enum                     │
│    ├── handler.rs     HandlerEvent enum                      │
│    └── chain.rs       ChainEvent enum                        │
└──────┬───────────────────────────────────────────────────────┘
       │        ▲
       │        │ (event subscribers)
       ▼        │
┌────────────┬──┴────────────┬──────────────────┐
│ Postgres   │  Prometheus   │  TUI (future)    │
│ (state)    │  (metrics)    │  (live view)     │
└────────────┴───────────────┴──────────────────┘

┌──────────────────────────────────────────────────────────────┐
│                    maestro-handlers                          │
│                                                              │
│  core/ ★ NEW ★                                               │
│    └── pallet_handler_ext.rs  PalletHandlerExt trait         │
│                                                              │
│  balances/  ats/  midds/*     (bundles migrated to trait)    │
└──────────────────────────────────────────────────────────────┘
```

## Component 1 — Event Bus (`maestro_core::events`)

The event bus is the critical new component. It decouples services from observability consumers.

### Event Types

Four enum families, one per subsystem:

```rust
// events/indexer.rs
#[derive(Debug, Clone)]
pub enum IndexerEvent {
    Started { mode: IndexMode, start_block: u64 },
    BlockIndexed { number: u64, hash: BlockHash, extrinsics: u32, events: u32, duration_ms: u32 },
    CursorAdvanced { head: u64, tail: u64 },
    LiveModeEntered { from_block: u64 },
    Stopped { reason: StopReason },
}

// events/backfill.rs
#[derive(Debug, Clone)]
pub enum BackfillEvent {
    Planned { from: u64, to: u64, total: u64 },
    BlockFetched { number: u64 },
    BlockPersisted { number: u64 },
    RangeCompleted { from: u64, to: u64 },
    FetchRetried { number: u64, attempt: u32, error: String },
    Aborted { reason: String },
}

// events/handler.rs
#[derive(Debug, Clone)]
pub enum HandlerEvent {
    EventProcessed { pallet: &'static str, event_name: String, block: u64 },
    Persisted { pallet: &'static str, table: &'static str, count: usize, block: u64 },
    Error { pallet: &'static str, block: u64, error: String },
}

// events/chain.rs
#[derive(Debug, Clone)]
pub enum ChainEvent {
    RpcConnected { url: String },
    RpcDisconnected { reason: String },
    RpcReconnecting { attempt: u32 },
    RuntimeUpgraded { spec_version: u32 },
}
```

### Bus

```rust
// events/mod.rs
use tokio::sync::{broadcast, watch};

#[derive(Clone)]
pub struct EventBus {
    indexer:  broadcast::Sender<IndexerEvent>,
    backfill: broadcast::Sender<BackfillEvent>,
    handler:  broadcast::Sender<HandlerEvent>,
    chain:    broadcast::Sender<ChainEvent>,
    cursor_state: watch::Sender<CursorState>,
    chain_state:  watch::Sender<ChainState>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self { /* ... */ }

    // Noop variant for tests (live bus with no subscribers; emits are dropped silently).
    pub fn noop() -> Self { Self::new(16) }

    pub fn emit_indexer(&self, ev: IndexerEvent)   { let _ = self.indexer.send(ev); }
    pub fn emit_backfill(&self, ev: BackfillEvent) { let _ = self.backfill.send(ev); }
    pub fn emit_handler(&self, ev: HandlerEvent)   { let _ = self.handler.send(ev); }
    pub fn emit_chain(&self, ev: ChainEvent)       { let _ = self.chain.send(ev); }

    pub fn update_cursor(&self, s: CursorState) { let _ = self.cursor_state.send(s); }
    pub fn update_chain(&self, s: ChainState)   { let _ = self.chain_state.send(s); }

    pub fn subscribe_indexer(&self)  -> broadcast::Receiver<IndexerEvent>  { self.indexer.subscribe() }
    pub fn subscribe_backfill(&self) -> broadcast::Receiver<BackfillEvent> { self.backfill.subscribe() }
    pub fn subscribe_handler(&self)  -> broadcast::Receiver<HandlerEvent>  { self.handler.subscribe() }
    pub fn subscribe_chain(&self)    -> broadcast::Receiver<ChainEvent>    { self.chain.subscribe() }
    pub fn watch_cursor(&self) -> watch::Receiver<CursorState> { self.cursor_state.subscribe() }
    pub fn watch_chain(&self)  -> watch::Receiver<ChainState>  { self.chain_state.subscribe() }
}
```

**Broadcast capacity**: 1024 per channel. Sufficient for a 60Hz TUI consumer.

### Why Two Channel Types

- **`broadcast`** for discrete events (block indexed, error). Multiple subscribers, each at their own pace. `Lagged` is acceptable — the TUI does not need every event.
- **`watch`** for "current state" values (latest cursor, RPC state). Latest-wins. Perfect for a TUI that displays the current head number.

### Phase 0 Consumers

Two consumers are wired immediately to validate the design:

1. **`events::logger`** — transforms events into structured `tracing::info!/warn!` calls. Replaces ad-hoc log sites progressively.
2. **`events::metrics_bridge`** — updates existing Prometheus counters/gauges from events. Moves the "increment metric" logic out of business services.

The TUI will be a third consumer in the next project.

### Not in Scope

- Event persistence / replay.
- Cross-process distribution (broadcast is in-process only).

## Component 2 — `PalletHandlerExt` trait (`handlers::core`)

### Trait

```rust
#[async_trait]
pub trait PalletHandlerExt: Send + Sync + 'static {
    type Model: Clone + Send + Sync
              + serde::Serialize + serde::de::DeserializeOwned
              + 'static;

    fn pallet_name(&self) -> &'static str;
    fn bundle_name(&self) -> &'static str;
    fn table_name(&self)  -> &'static str;
    fn priority(&self)    -> i32 { 0 }

    fn parse(
        &self,
        event: &RawEvent,
        block: &Block,
        extrinsic: Option<&RawExtrinsic>,
    ) -> Option<Self::Model>;

    async fn persist(&self, models: &[Self::Model]) -> StorageResult<()>;

    // EventBus is required (not Option). Tests use EventBus::noop().
    fn event_bus(&self) -> &EventBus;
}
```

### Blanket impl

`impl<H: PalletHandlerExt> PalletHandler for H` takes care of:

- Dispatching `handle_event` → `self.parse` → add to `HandlerOutputs`.
- Emitting `HandlerEvent::EventProcessed` on each successful parse.
- Dispatching `on_block_end` → `self.persist` → emit `HandlerEvent::Persisted` or `HandlerEvent::Error`.

### What a Migrated Handler Looks Like

Balances goes from ~135 lines of `PalletHandler` logic to ~35 lines implementing `PalletHandlerExt`. The four trait methods (`parse`, `persist`, accessors, `event_bus`) are the only thing the handler author writes.

### Trait over Macro

Rejected: declarative macros. Reasoning:

- Trait preserves IDE tooling (go-to-def, completion, doc-on-hover).
- Compiler errors are readable.
- A handler with atypical needs can still `impl PalletHandler` directly, bypassing the blanket. Macros are all-or-nothing.

Macros stay in reserve if Phase 2 reveals boilerplate a trait cannot absorb.

### Known Constraint: One Model per Handler

The trait assumes each concrete handler produces exactly one model type. If a pallet needs to produce multiple model types from one event, the plan is to split it into N concrete structs, each implementing `PalletHandlerExt` with its own `Model`. Decision deferred until the need is real (YAGNI).

### Phase 0 deviation: crate location

`PalletHandlerExt` and its blanket `impl<H> PalletHandler for H` were
placed in `maestro_core::ports::pallet_handler_ext`, not in
`maestro_handlers::core::pallet_handler_ext` as originally diagrammed.
The orphan rule forbids a downstream crate from providing a blanket
foreign-trait impl over a bare type parameter. Downstream handlers
import via `maestro_core::ports::PalletHandlerExt`.

## Phases

### Phase 0 — Foundations

Build `maestro_core::events` and `handlers::core::PalletHandlerExt` + blanket impl. Wire the `logger` and `metrics_bridge` consumers in the binary. **No handler is migrated yet.**

**Critères de sortie**:

- `cargo test` green, `cargo clippy --all -- -D warnings` green, `cargo fmt --check` green.
- ~6 new unit/integration tests on the event bus (construction, fanout, lag tolerance, watch latest-wins, noop variant, logger consumer, metrics_bridge consumer).
- Binary starts and runs normally with the two new consumers attached, no regression in existing metrics.

### Phase 1 — Pilot: `balances`

Migrate `BalancesHandler` from `impl PalletHandler` to `impl PalletHandlerExt`. `BalancesBundle::new` accepts `EventBus`. Existing tests (8 on `process_transfer`) adapted to `parse`. Two new tests assert `HandlerEvent::Persisted` and `HandlerEvent::Error` are emitted.

Prometheus metrics for balances are temporarily **duplicated** (old emission + new via `metrics_bridge`), to detect regressions. The old emission is removed in Phase 2.

**Critères de sortie**:

- `cargo test -p maestro-handlers` green, global `cargo test` green.
- `cargo clippy --all -- -D warnings` green.
- Live smoke test on a local node: binary indexes a few blocks, `HandlerEvent::Persisted` received for Balances.
- `handlers/src/balances/handler.rs` ≤100 lines outside tests.
- Existing Prometheus metrics for balances still populated.

**Non-goals Phase 1**: ATS, MIDDS, `indexer.rs`, `main.rs`, GraphQL, perf.

### Phase 2 — Minimal migration of `ats` and `midds`

Reduced from initial plan because ATS and MIDDS will be rewritten later.

- All handlers in ATS and MIDDS migrate from `impl PalletHandler` to `impl PalletHandlerExt` (mechanical, ~50 lines touched per handler).
- `EventBus` injected into their bundle constructors.
- **No file splits** for `midds/handler.rs`, `midds/storage.rs`, `ats/handler.rs`, `ats/storage.rs`. They stay as-is.
- Prometheus duplication removed — `metrics_bridge` becomes the sole path.

**Critères de sortie**:

- All handlers in the project implement `PalletHandlerExt`.
- No direct `impl PalletHandler` remaining in `crates/handlers/`.
- Prometheus duplication removed.
- `cargo test` green, `cargo clippy --all -- -D warnings` green.

**Non-goal**: size constraints on MIDDS/ATS files.

### Phase 3 — Split `indexer.rs` and `main.rs`

**`crates/core/src/services/indexer.rs` → `services/indexer/`**:

```
mod.rs        (~80 lines — IndexerService struct, run(), re-exports)
config.rs     (~150 lines — IndexerConfig, BackfillConfig, IndexMode)
dispatcher.rs (~120 lines — live vs backfill decision, pure)
live.rs       (~180 lines — run_live_loop)
pipeline.rs   (~150 lines — index_single_block)
```

Emission of `IndexerEvent`s from `live.rs` / `pipeline.rs` (natural insertion points).

**`bin/maestro/src/main.rs` → split**:

```
main.rs         (~60 lines — minimal entry)
cli.rs          (~130 lines — unchanged)
startup.rs      (~150 lines — logging, DB, RPC, bundles)
commands.rs     (~100 lines — purge, migrate-only, export-schema)
runtime.rs      (~120 lines — service wiring, signals, shutdown)
event_sinks.rs  (~80 lines — logger + metrics_bridge + future TUI)
```

`runtime.rs` is the natural insertion point for the future TUI sink.

**Critères de sortie**:

- `cargo test` green, `cargo clippy --all -- -D warnings` green.
- No file in `crates/core/src/services/` or `bin/maestro/src/` exceeds **250 lines**.
- `main.rs` ≤100 lines.
- `backfill_integration` tests pass with only `use`-path updates.
- Local startup on a real node: full startup, a few blocks indexed, clean shutdown via `Ctrl+C`.
- New unit test on `dispatcher::decide_mode(cursor, config)` (pure logic).

### Phase 4 — GraphQL cleanup

Audit and deduplicate patterns in `graphql/schema.rs` (552), `balances/graphql.rs` (160), `ats/graphql.rs` (274), `midds/graphql.rs` (676).

**Process**:

1. Read the four files and identify concrete duplicated patterns (likely: pagination / `Connection<T>` / `PageInfo` construction, domain-to-GraphQL conversions, filter inputs).
2. Extract shared helpers into `handlers::core::graphql_utils` or a new module in the `graphql` crate.
3. Refactor `balances/graphql.rs` first (pilot).
4. Refactor `ats/graphql.rs` minimally.
5. **Skip `midds/graphql.rs`** — will be rewritten later (same rationale as Phase 2).

**Critères de sortie**:

- `crates/graphql/src/schema.rs` ≤350 lines.
- `balances/graphql.rs` ≤120 lines.
- Pagination/connection helpers in a single location.
- `cargo test` green, `cargo clippy --all -- -D warnings` green.
- Smoke test: a live GraphQL query works (`{ blocks(first: 10) { edges { node { number } } } }`).

### Phase 5 — Perf pass (optional)

Only starts **after** Phases 0–4 are merged. Code has its final shape — profiling is meaningful.

**Method**:

1. Install two benchmarks:
   - **Backfill throughput**: backfill N blocks on a local node, measure blocks/sec and allocations via `cargo flamegraph` or `samply`.
   - **Live steady-state**: 15 min of live indexing, measure per-block latency via the `IndexerEvent::BlockIndexed { duration_ms }` field.
2. **Time budget**: 2–3 days max. Goal is fixing obvious hotspots from profiling, not exhaustive optimization.
3. **Hotspot candidates** (to confirm by measurement):
   - SCALE decoding in `substrate/src/decode/`.
   - JSON serialization in `substrate/src/scale_json.rs` (302 lines, allocation-heavy suspect).
   - SQL inserts — confirm batch/COPY is used, audit `extrinsic_repo.rs` (310 lines).
   - Broadcast channel capacity if `metrics_bridge` lags.

**Rules**:

- No optimization without a before/after measurement.
- One commit = one measured optimization. No batched "perf cleanup".
- No new `unsafe` for perf.

**Critères de sortie**:

- Documented, reproducible bench commands.
- Each optimization commit includes a before/after number.
- No functional regression.

Phase 5 can be abandoned entirely if profiling shows the code is already fine.

## Test Strategy

Existing tests are the safety net. They are never deleted — only adapted to new paths.

| Level | Phase 0 | Phase 1 | Phase 2 | Phase 3 | Phase 4 |
|---|---|---|---|---|---|
| Unit (pure) | event types, bus construction | balances parse | — | dispatcher::decide_mode | pagination helpers |
| Integration (mock) | bus fanout, lag tolerance | balances end-to-end via mock storage | — | indexer orchestration with mock BlockSource | resolvers with mock repo |
| Live (real node) | — | exit criterion | exit criterion | exit criterion | smoke query |

Estimated ~13–15 new tests, ~200 lines of test code across all phases.

### Commit Gate

Every commit must pass:

```bash
cargo fmt --check && \
cargo clippy --all --all-targets -- -D warnings && \
cargo test --all
```

If any of the three fails, the commit does not land.

## Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Blanket `impl<H: PalletHandlerExt> PalletHandler for H` conflicts with direct `impl PalletHandler` on ATS/MIDDS during transition | Low | The blanket only applies to types implementing `PalletHandlerExt`. Types with manual impls are unaffected. Verified in CI. |
| `EventBus::noop()` creates a receiver with no subscribers → `broadcast::send` returns `Err(SendError)` | Certain | Emit methods deliberately ignore errors (`let _ = send`). |
| Test races an event (too early/late) | Medium | `broadcast::Receiver::recv()` wrapped in `tokio::time::timeout(100ms)`. Timeout = explicit fail. |
| Migration silently breaks a Prometheus metric | Medium | Phase 1 duplicates metric emission (old + `metrics_bridge`). Only Phase 2 removes the old path. |
| ATS has a multi-model handler we didn't anticipate | Low | Documented plan B: split into N structs, each with `impl PalletHandlerExt`. Trait variant only if N blows up. |
| Moving `midds/storage.rs` breaks `use` imports across the crate | Low | Not applicable — Phase 2 leaves midds storage in place. |
| Splitting `indexer.rs` breaks complex generic lifetimes (`IndexerService<S: BlockSource, R: Repositories>`) | Medium | Generic types stay in `mod.rs`. Submodules contain `impl` blocks of the same type or free functions taking `&IndexerService<S,R>`. |
| Splitting `main.rs` introduces circular deps | Low | `runtime.rs` depends on `startup.rs`, not the reverse. Linear flow. |
| Shutdown signal regressed during refactor | Medium | Manual test in Phase 3 exit criteria + a `tokio::time::timeout` test that `serve()` exits on `CancellationToken::cancel()`. |
| Context loss across phases | Medium | This design doc committed at start. Each phase = a branch with description linking back. |
| Merge conflict with parallel work on `develop` | Medium | Reserve a quiet period. Rebase often if unavoidable. |
| Phase 5 reveals a deeper architectural issue | Medium | Accepted — it's the purpose of profiling. New design doc if it happens. |

## Size Estimate

| Phase | Duration | Net line delta |
|---|---|---|
| Phase 0 | ~1 day | +400 lines (new infra) |
| Phase 1 | ~0.5 day | −100 lines (balances shrinks) |
| Phase 2 | ~0.5 day | −300 to −500 lines (dedup + prometheus cleanup) |
| Phase 3 | ~1 day | 0 (pure split) |
| Phase 4 | ~1 day | −200 to −400 lines |
| Phase 5 | ~1–2 days (optional) | minimal |

**Total**: ~4–6 working days of refactor before TUI work begins.

## Success Criteria (whole refactor)

- All three exit criteria gates (`cargo fmt --check`, `cargo clippy --all -- -D warnings`, `cargo test --all`) remain green at every commit.
- No functional regression (indexer indexes, GraphQL queries, metrics flow).
- `maestro_core::events::EventBus` exists and is subscribed by `logger` + `metrics_bridge`.
- All handlers implement `PalletHandlerExt`. Zero direct `impl PalletHandler` left in `crates/handlers/`.
- `crates/core/src/services/` and `bin/maestro/src/` have no file >250 lines.
- `main.rs` ≤100 lines. Balances handler ≤100 lines (outside tests).
- TUI consumer can be added later by wiring a third subscriber in `event_sinks.rs` — no other change required.
