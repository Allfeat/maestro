# Historical block sync — Phase 2 design

**Date**: 2026-04-13
**Scope**: Phase 2 of the subxt upgrade effort. This document covers **only** the historical block sync feature: CLI-configurable backfill from a start block to the live tip, range-model cursor, parallel fetch with serial persist, V14 metadata floor enforcement, and a targeted refactor of the indexer service to cleanly separate backfill from live concerns. The subxt 0.44 → 0.50 API upgrade is **Phase 1** and is assumed complete when this phase starts.

---

## Context & motivation

After Phase 1, `maestro-substrate` is on subxt 0.50 and indexes finalized blocks from the chain head forward. The `use_historic_types(false)` hook is in place. Every Phase 1 cursor represents a `[0, last_indexed_block]` range by construction (no CLI knob exists to start anywhere else).

Phase 2 adds the capability the project has been blocked on: indexing historical blocks. A user running maestro against an existing chain with millions of blocks of history currently has no way to populate those blocks into the database — only new blocks arriving at the tip are indexed. This phase closes that gap.

## Phase decomposition

| Phase | Scope | Status |
|---|---|---|
| **1** | Subxt 0.50 upgrade, module split, timestamp cleanup, SCALE→JSON rewrite | **Complete** (assumed) |
| **2** (this doc) | Historical backfill, range cursor model, parallel fetch, V14 floor, bounded refactor | **Designed** |

## Non-goals for Phase 2

- Handler idempotency framework (X1 below; flagged as R-future)
- Pre-V14 block decoding — refused at startup per §4
- Full `process_block` decomposition beyond the one extraction named in R1
- Concurrent live + backfill (backfill runs to completion, then live starts)
- Bounded one-shot extraction (`--end-block`) — YAGNI
- Parallel persist / sharded backfill — YAGNI, `buffered(K)` fetch alone gives the throughput win
- Changes to `PalletHandler` trait or any handler bundle
- Changes to GraphQL schema or the graphql crate
- Changes to `RawBlock` / `RawEvent` / `RawExtrinsic` model shapes
- Changes to the `Repositories` trait shape (two new `CursorRepository` methods are additive)

## Acceptance criteria ("done" gate)

1. `cargo build --release` passes
2. `cargo clippy --all -- -D warnings` passes
3. `cargo test` passes — includes `BackfillPlan::compute` unit tests, retry math tests, and mock-based backfill integration tests
4. Fresh DB: `maestro --start-block 0 --backfill-concurrency 4` against a live Allfeat node indexes ≥ 1000 blocks, cursor advances monotonically, process transitions to live stream cleanly
5. Resume correctness: interrupt gate #4 at ~500 blocks with SIGINT, restart with identical args, assert resume continues from last persisted block without re-processing
6. `--live-only` on a fresh DB: starts at the current tip without any backfill, cursor initializes at tip, live blocks flow normally
7. All three handler bundles (balances, ats, midds) each show ≥1 record written during the backfill range of gate #4

Gates 1–3 automated. Gates 4–7 manual, documented in the PR description.

---

## 1. Architecture & module layout

Phase 2 adds a **historical backfill pipeline** alongside the existing live subscription. The new machinery is a thin layer over the per-block processing (transform, handler execution, atomic persist). Only block acquisition and loop control change.

### BlockSource trait additions

`crates/core/src/ports/block_source.rs`:

```rust
#[async_trait]
pub trait BlockSource: Send + Sync {
    // ... existing methods unchanged ...

    /// Fetch a block by number. Used by the backfill loop.
    /// Errors map to ChainError::RpcError on transient failures.
    async fn fetch_block_at(&self, number: u64) -> ChainResult<RawBlock>;

    /// Earliest block whose runtime metadata is V14 or later.
    /// Backfill below this block is refused at startup (§4).
    /// Returns 0 if the chain is V14-from-genesis.
    async fn earliest_v14_block(&self) -> ChainResult<u64>;
}
```

Both are additive. Phase 1's methods (`genesis_hash`, `finalized_head`, `best_head`, `subscribe_finalized`, `subscribe_best`, `runtime_version`) are untouched.

### New crate layout

```
crates/core/src/services/
├── indexer.rs        # IndexerService — dispatcher + live loop + pure per-block processor
├── backfill.rs       # BackfillRunner, BackfillPlan, BackfillRange, fetch_with_retry
└── mod.rs            # re-exports
```

`IndexerService::run` becomes a dispatcher that:
1. Verifies chain ID and cursor consistency
2. Enforces the V14 floor
3. Routes to `run_live_loop` directly if `--live-only` is set
4. Otherwise computes a `BackfillPlan`, executes its 0–2 ranges via `BackfillRunner`, then falls through to `run_live_loop`

### Substrate adapter changes

`crates/substrate/src/`:
- `client.rs` gains `impl BlockSource::fetch_block_at` wrapping `client.at(hash).await` where the hash is looked up from a number via the RPC backend (exact 0.50 symbol name validated at first compile; fallback is the same ≤5-lines-per-call-site policy from Phase 1's R1).
- `client.rs` gains `impl BlockSource::earliest_v14_block` performing a cached binary search over runtime metadata versions (§4).
- `PolkadotConfig::builder().use_historic_types(false)` → `use_historic_types(true)`. Pure flag flip. No historical types are provided; the §4 floor check guarantees subxt never sees a block it cannot decode.

### What does NOT change

- `RawBlock`, `Block`, `Event`, `Extrinsic` domain model shapes
- `PalletHandler` trait — handlers don't know if a block is historical
- `Repositories` trait shape (`persist_block_atomic` signature unchanged; its body gains range-aware cursor logic; two additive `CursorRepository` methods)
- GraphQL schema and the graphql crate
- Storage reader (`StorageReader` trait and its three methods)
- Handler bundles: balances, ats, midds

---

## 1.5. Refactoring scope (in-phase)

Refactors earn their place in Phase 2 only if (a) they fall out of the backfill work naturally, or (b) they are ≤30-minute cleanups that make the new code readable. Anything larger is flagged and deferred.

### Included

**R1. Split `IndexerService::run` into dispatcher + pure per-block processor.**

`process_block` today conflates reorg-check (live-only), skip-already-indexed (live-only safety net), transform, handler execution, and atomic persist. After R1:

```rust
// Pure: takes a RawBlock, runs handlers, persists atomically. No loop state, no reorg check.
async fn index_single_block(&self, raw_block: RawBlock, mode: IndexMode) -> IndexerResult<()>;

// Live-loop wrapper: skip-if-indexed → reorg check → index_single_block(Live)
async fn live_process_block(&self, raw_block: RawBlock) -> IndexerResult<bool>;

// Backfill-loop wrapper: index_single_block(Backfill) — no skip, no reorg check
async fn backfill_process_block(&self, raw_block: RawBlock) -> IndexerResult<()>;
```

`IndexMode` is an enum `{ Live, Backfill }` passed into `index_single_block` purely so metrics and log lines can distinguish the two sources.

**R2. Rename `follow_blocks` → `run_live_loop`.** The old name made sense when live was the only loop. With `run_backfill_loop` alongside it the pair `run_live_loop` / `run_backfill_loop` reads clearly.

**R3. Nest new config knobs under `BackfillConfig`.**

```rust
pub struct IndexerConfig {
    pub chain_id: String,
    pub poll_interval: Duration,
    pub max_retries: u32,
    pub retry_delay: Duration,
    pub block_mode: BlockMode,
    pub backfill: BackfillConfig,   // ← new
}

pub struct BackfillConfig {
    pub start_block: u64,           // default 0
    pub live_only: bool,            // default false
    pub concurrency: usize,         // default 16
    pub max_fetch_retries: u32,     // default 5
}
```

Keeps existing flat fields stable (no touching of every construction site) and isolates new knobs.

**R4. Extract CLI parsing out of `main.rs`.** The `Cli` struct and its `TryFrom<Cli> for IndexerConfig` impl move to `bin/maestro/src/cli.rs`. Zero behavior change, pure organization. Four new flags (§5) would otherwise make `main.rs` unwieldy.

**R5. Consolidate cursor and chain verification at the top of `run`.** Today `verify_chain_id` runs in `run`, then `verify_consistency_on_reconnect` runs inside `follow_blocks`. After R1 both live in one startup sequence in `run` before the dispatcher picks a loop. One place to look for "what does the indexer do before processing".

**R6. Tighten error variants for new failure modes.** Add to `IndexerError`:
- `BackfillAborted { block: u64, reason: String }` — fetch retries exhausted for a specific block
- `PreV14BlockRequested { requested: u64, earliest_v14: u64 }` — §4 floor enforcement

Add to `StorageError`:
- `CursorGapViolation { block: u64, first: u64, last: u64 }` — `persist_block_atomic` detects a block that is neither adjacent above nor below the current cursor range. Propagates up through the existing `IndexerError::Storage` conversion.

These replace ad-hoc `anyhow::anyhow!` strings that would otherwise leak into the new code.

**R7. Per-block metrics recorder takes an `IndexMode`.** `record_block_indexed()` becomes `record_block_indexed(mode: IndexMode)` so Prometheus can break down live vs backfill throughput. Trivial edit at the one call site inside `index_single_block`.

### Deliberately excluded (flagged, not done)

**X1. Handler idempotency framework.** Handlers write to DB; if an already-indexed block is reprocessed, they would insert duplicates. Phase 2's block-level "skip if already indexed" check in `live_process_block` prevents this in practice. Fixing the handler contract itself (unique constraints, upsert, or delete-then-insert per handler) touches every handler bundle and is a separate phase. **Flagged as R-future.**

**X2. Full `process_block` decomposition.** R1 extracts `index_single_block`. The 170-line `process_block` → `transform_block` / `transform_events` / `transform_extrinsics` chain otherwise stays as-is.

**X3. Repository trait refinement.** `delete_from_block_atomic(0, chain_id)` as a "clear everything" gesture is clunky but out of scope for Phase 2.

**X4. `BlockMode` / `IndexMode` unification.** `BlockMode::{Finalized, Best}` and `IndexMode::{Live, Backfill}` are orthogonal axes (backfill finalized-only is valid; live best-only is valid). Resist merging; they stay two enums.

---

## 2. Cursor model & schema migration

### The model

```rust
pub struct IndexerCursor {
    pub chain_id: String,
    pub first_indexed_block: u64,     // ← new
    pub last_indexed_block: u64,
    pub last_indexed_hash: BlockHash,
    pub updated_at: DateTime<Utc>,
}
```

**Invariant**: `first_indexed_block ≤ last_indexed_block`, and the range `[first, last]` is contiguous — every block number in that interval exists in the `blocks` table for the given chain.

### Migration

New file: `crates/storage/migrations/NNN_add_cursor_first_indexed_block.sql` (NNN = next available number in the sequence).

```sql
ALTER TABLE cursor
  ADD COLUMN first_indexed_block BIGINT NOT NULL DEFAULT 0;

UPDATE cursor SET first_indexed_block = 0 WHERE first_indexed_block IS NULL;

ALTER TABLE cursor ALTER COLUMN first_indexed_block DROP DEFAULT;
```

This is correct because Phase 1 has no `--start-block` CLI flag — every pre-Phase-2 deployment indexed from genesis upward, so the existing `last_indexed_block` is the top of `[0, last]` by construction.

### Cursor update logic in `persist_block_atomic`

The block / events / extrinsics inserts are unchanged. Only the cursor update branches based on where the block sits relative to the current cursor range:

```rust
// Inside persist_block_atomic, after block/events/extrinsics inserted:
let block_num = data.block.number;
match self.cursor_repo.get_cursor(chain_id).await? {
    None => {
        // First block ever for this chain. Range initialized to [n, n].
        self.cursor_repo.set_cursor(&IndexerCursor {
            chain_id: chain_id.to_string(),
            first_indexed_block: block_num,
            last_indexed_block: block_num,
            last_indexed_hash: data.block.hash.clone(),
            updated_at: Utc::now(),
        }).await?;
    }
    Some(existing) if block_num == existing.last_indexed_block + 1 => {
        // Forward extension (live stream or forward backfill).
        self.cursor_repo.extend_upward(chain_id, block_num, &data.block.hash).await?;
    }
    Some(existing) if block_num + 1 == existing.first_indexed_block => {
        // Downward gap-fill.
        self.cursor_repo.extend_downward(chain_id, block_num).await?;
    }
    Some(existing) => {
        return Err(StorageError::CursorGapViolation {
            block: block_num,
            first: existing.first_indexed_block,
            last: existing.last_indexed_block,
        });
    }
}
```

### Two new `CursorRepository` methods (additive)

```rust
#[async_trait]
pub trait CursorRepository: Send + Sync {
    // ... existing methods unchanged ...

    /// Extend the indexed range upward by one block.
    /// Updates last_indexed_block, last_indexed_hash, updated_at.
    async fn extend_upward(
        &self,
        chain_id: &str,
        block: u64,
        hash: &BlockHash,
    ) -> StorageResult<()>;

    /// Extend the indexed range downward by one block.
    /// Updates first_indexed_block, updated_at. Does NOT touch last_indexed_hash
    /// (tied to the tip, not the bottom of the range).
    async fn extend_downward(
        &self,
        chain_id: &str,
        block: u64,
    ) -> StorageResult<()>;
}
```

Both are single-row `UPDATE cursor SET …` queries that execute inside the existing `persist_block_atomic` transaction, so cursor advancement is atomic with block data insertion.

### Startup cursor-vs-config reconciliation

`BackfillPlan::compute(start_block, existing_cursor, tip)` is a pure function encoding the reconciliation matrix:

```
None:                              normal fresh-start.
                                   → Upward [start_block, tip]

Some(c), start ≥ c.first:          already have as much/more history.
                                   Ignore start_block, resume upward.
                                   → Upward [c.last + 1, tip]  (or empty if c.last == tip)

Some(c), start < c.first:          user wants more history. Two-phase:
                                   → (1) Downward [start_block, c.first - 1]
                                   → (2) Upward [c.last + 1, tip]  (if c.last < tip)

live_only = true:                  skip the planning entirely; caller starts
                                   run_live_loop directly (warn if cursor gap).
```

The two-phase case (extending in both directions) runs phase 1 first: gap-fill has to complete before forward-extend starts. Otherwise a crash mid-gap-fill followed by a restart would see a non-contiguous range and fail the invariant. Downward-then-upward preserves the single-contiguous-range invariant across crashes.

**`BackfillPlan::compute` is a pure function**, trivially unit-testable (see §6).

---

## 3. Backfill loop

New module: `crates/core/src/services/backfill.rs`.

### Types

```rust
pub struct BackfillRunner<S: BlockSource> {
    block_source: Arc<S>,
    config: BackfillConfig,
    chain_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackfillDirection { Upward, Downward }

#[derive(Debug, Clone)]
pub struct BackfillRange {
    pub from: u64,       // inclusive
    pub to: u64,         // inclusive
    pub direction: BackfillDirection,
}

pub struct BackfillPlan {
    pub ranges: Vec<BackfillRange>,   // 0, 1, or 2 entries
}

impl BackfillPlan {
    pub fn compute(
        start_block: u64,
        existing_cursor: Option<&IndexerCursor>,
        tip: u64,
    ) -> Self { /* pure logic, see §2 */ }
}
```

### The core loop

One function, `run_range`, handles both directions. The caller passes in a closure for per-block processing so the runner doesn't need to hold an `IndexerService` reference (avoids circular ownership).

```rust
pub async fn run_range<F, Fut>(
    &self,
    range: BackfillRange,
    shutdown_rx: &mut watch::Receiver<bool>,
    mut index_single_block: F,
) -> IndexerResult<()>
where
    F: FnMut(RawBlock) -> Fut,
    Fut: std::future::Future<Output = IndexerResult<()>>,
{
    let block_numbers: Box<dyn Iterator<Item = u64> + Send> = match range.direction {
        BackfillDirection::Upward   => Box::new(range.from..=range.to),
        BackfillDirection::Downward => Box::new((range.from..=range.to).rev()),
    };

    let concurrency = self.config.concurrency;
    let source = self.block_source.clone();
    let max_retries = self.config.max_fetch_retries;

    let mut fetched = futures::stream::iter(block_numbers)
        .map(move |n| {
            let source = source.clone();
            async move { fetch_with_retry(&*source, n, max_retries).await }
        })
        .buffered(concurrency);

    while let Some(result) = fetched.next().await {
        if *shutdown_rx.borrow() {
            return Err(IndexerError::ShutdownRequested);
        }

        let raw_block = result.map_err(|(block, err)| IndexerError::BackfillAborted {
            block,
            reason: err.to_string(),
        })?;

        index_single_block(raw_block).await?;
    }

    Ok(())
}
```

### Key properties

- **`buffered(concurrency)` preserves input order** — blocks are persisted in strict monotonic order (ascending for Upward, descending for Downward) even though fetches run in parallel. This is what lets the cursor model stay single-integer-per-direction.
- **Direction-agnostic** — the same function runs upward and downward. The reversed iterator handles downward; `persist_block_atomic` routes the cursor update based on block number vs. current cursor range.
- **Shutdown check between blocks**, not mid-fetch. In-flight fetches are dropped on cancellation; no partial writes because shutdown is checked *before* the next `index_single_block` call.
- **The closure parameter** sidesteps a circular ownership problem: `BackfillRunner` would otherwise need to hold `Arc<IndexerService>`, and `IndexerService::run` would hold a `BackfillRunner`. The closure version lets `IndexerService::run` capture `&self` in a local closure and pass it to `run_range`.

### Retry policy

```rust
async fn fetch_with_retry<S: BlockSource>(
    source: &S,
    block: u64,
    max_retries: u32,
) -> Result<RawBlock, (u64, ChainError)> {
    let mut delay = Duration::from_millis(250);
    let mut attempt = 0u32;
    loop {
        match source.fetch_block_at(block).await {
            Ok(raw) => return Ok(raw),
            Err(e) if attempt < max_retries => {
                warn!(block, attempt, error = %e, "backfill fetch failed, retrying");
                record_backfill_fetch_retry();
                tokio::time::sleep(delay).await;
                delay = (delay * 2).min(Duration::from_secs(10));
                attempt += 1;
            }
            Err(e) => return Err((block, e)),
        }
    }
}
```

Bounded exponential backoff (250ms → 10s). After `max_retries` exhausts, the stream yields the error, `run_range` bubbles it up as `BackfillAborted`, and the whole run exits. Cursor sits at the last successfully-persisted block; the user restarts to pick up from there.

**Subtle interaction with `buffered(K)`**: when one in-flight fetch errors *earlier in the stream* than a later-numbered one that has already succeeded, the error blocks the later one from being yielded and its result is dropped. This is correct behavior (we don't want to persist `N+1` if `N` failed) and means up to `K-1` successful-but-dropped fetches per abort. Acceptable cost.

### Progress reporting

`tracing::info!` log every `N` blocks (default 1000), plus new Prometheus metrics in `crates/core/src/metrics.rs`:

```rust
pub fn record_backfill_block_indexed();
pub fn record_backfill_progress(indexed: u64, remaining: u64);
pub fn record_backfill_fetch_retry();
pub fn record_backfill_aborted();
```

Exposed via the existing `/metrics` endpoint on `METRICS_PORT`. Answers the "am I 30 minutes or 30 hours from catching up" ops question.

### Dispatcher wiring (consolidated `run` shape)

```rust
pub async fn run(&self, mut shutdown_rx: watch::Receiver<bool>) -> IndexerResult<()> {
    info!(mode = ?self.config.block_mode, "⛓️  Starting indexer");

    // 1. Chain verification + cursor consistency (moved up from follow_blocks per R5)
    self.verify_chain_id().await?;
    let existing_cursor = self.verify_consistency_on_startup().await?;

    // 2. V14 floor enforcement (§4)
    let earliest_v14 = self.block_source.earliest_v14_block().await?;
    self.enforce_v14_floor(earliest_v14)?;

    // 3. Live-only escape hatch
    if self.config.backfill.live_only {
        self.warn_if_cursor_gap(&existing_cursor).await;
        return self.run_live_loop(&mut shutdown_rx).await;
    }

    // 4. Plan and execute backfill
    let tip = self.block_source.finalized_head().await?.number;
    let plan = BackfillPlan::compute(
        self.config.backfill.start_block,
        existing_cursor.as_ref(),
        tip,
    );

    if !plan.ranges.is_empty() {
        info!(ranges = ?plan.ranges, "🕰  Starting backfill");
        let runner = BackfillRunner::new(
            self.block_source.clone(),
            self.config.backfill.clone(),
            self.config.chain_id.clone(),
        );
        for range in plan.ranges {
            runner.run_range(range, &mut shutdown_rx, |raw| {
                self.index_single_block(raw, IndexMode::Backfill)
            }).await?;
        }
        info!("✅ Backfill complete, switching to live stream");
    }

    // 5. Hand off to live loop
    self.run_live_loop(&mut shutdown_rx).await
}
```

Reads top-to-bottom as a story: verify, check floor, branch on live-only, plan, execute, hand off.

---

## 4. V14 floor detection

Subxt 0.50's `use_historic_types(true)` enables decoding blocks whose runtime predates self-describing metadata (V14, circa Substrate 2021). Pre-V14 blocks need caller-supplied type definitions. Phase 2 chooses to **refuse** pre-V14 backfill rather than ship historical types — but that requires actually *detecting* the V14 floor on any given chain.

### Strategy: binary search over metadata versions

```rust
// In crates/substrate/src/client.rs

async fn earliest_v14_block(&self) -> ChainResult<u64> {
    let tip = self.finalized_head().await?.number;

    // Cheap path: if genesis is already V14, floor is 0.
    if metadata_version_at(&self.client, 0).await? >= 14 {
        return Ok(0);
    }

    // Binary search: lowest block whose metadata version >= 14.
    let mut lo = 0u64;
    let mut hi = tip;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let version = metadata_version_at(&self.client, mid).await?;
        if version >= 14 {
            hi = mid;
        } else {
            lo = mid + 1;
        }
    }
    Ok(lo)
}

async fn metadata_version_at(
    client: &OnlineClient<PolkadotConfig>,
    block: u64,
) -> ChainResult<u32> {
    // Fetch block hash at `block`, fetch runtime metadata at that hash, return version.
    // Exact 0.50 API symbol names validated at first compile (same ≤5-lines fallback
    // policy as Phase 1 R1).
}
```

### Cost

`ceil(log2(N))` metadata fetches. At `N = 2M`, that is ~21 fetches of ~300KB each — roughly 6 MB and a few seconds of startup time. One-time cost at process start, acceptable. Cached in memory for the process lifetime.

### Allfeat reality

Allfeat is V14-from-genesis. The cheap path returns 0 after one probe. The binary search exists to make Phase 2 *correct* on any Substrate chain, not to solve an Allfeat-specific problem. It is no-op overhead on Allfeat.

### Fallback for unreliable historical RPC

If a specific chain-side RPC error prevents fetching historical metadata for a given block (e.g. runtime state pruning on the node), treat that block as pre-V14 and move `lo` forward. Conservative: may refuse slightly more than necessary, never decodes unsafe blocks.

### Optimization (deferred)

Cache `earliest_v14_block` in a new `chain_metadata` table so subsequent restarts skip the search. Deferred to R-future unless the ~2–3s startup becomes annoying.

### Error reported to the user

```
Error: Cannot backfill below block 473291 on this chain — its runtime uses
pre-V14 metadata, which Maestro does not decode. Re-run with
--start-block 473291 (or higher), or use --live-only to skip backfill entirely.
```

Friendly, actionable, names both knobs that unblock it. Emitted by `IndexerError::PreV14BlockRequested` Display impl.

---

## 5. CLI, config wiring, live-loop adjustments

### New CLI flags

`bin/maestro/src/cli.rs` (after R4):

```rust
/// Lowest block number to index. Defaults to 0 (genesis).
/// Cannot be below the chain's earliest V14 block.
#[arg(long, env = "START_BLOCK", default_value = "0")]
start_block: u64,

/// Skip backfill entirely and start indexing at the current chain head.
/// Any existing cursor gap will NOT be filled — use with care.
#[arg(long, env = "LIVE_ONLY", default_value = "false")]
live_only: bool,

/// Maximum parallel block fetches during backfill. Higher = faster, more RPC load.
#[arg(long, env = "BACKFILL_CONCURRENCY", default_value = "16")]
backfill_concurrency: usize,

/// Maximum retries per block fetch during backfill before aborting.
#[arg(long, env = "BACKFILL_MAX_RETRIES", default_value = "5")]
backfill_max_retries: u32,
```

### CLI table addendum for `CLAUDE.md`

Append to the existing CLI arguments table:

| Argument | Env Var | Default | Description |
|---|---|---|---|
| `--start-block` | `START_BLOCK` | `0` | Lowest block number to index |
| `--live-only` | `LIVE_ONLY` | `false` | Skip backfill, start at current head |
| `--backfill-concurrency` | `BACKFILL_CONCURRENCY` | `16` | Parallel fetches during backfill |
| `--backfill-max-retries` | `BACKFILL_MAX_RETRIES` | `5` | Per-block retry cap before abort |

### `TryFrom<Cli> for IndexerConfig`

Materialized in `cli.rs`, constructs the nested `BackfillConfig` sub-struct from the four new fields and leaves the rest of `IndexerConfig` untouched. Pure mechanical conversion; no behavior.

### Live-loop adjustments

Three small changes inside `run_live_loop` (the renamed `follow_blocks`):

**L1. No re-verification of cursor consistency on entry.** Moved up into `run` per R5. Remove the `verify_consistency_on_reconnect` call inside the live loop body; the outer dispatcher has already handled it.

**L2. Stricter handoff skip logging.** After backfill completes, the live stream's first block should be `cursor.last_indexed_block + 1` (the tip has moved forward by ~0 to a handful of blocks during backfill catch-up). Today's code in `process_block` skips silently on hash match; keep that behavior, but add a `debug!` log with the skip count so the handoff race (Q2/A) is observable.

**L3. Reorg check scoping** — already correct after R1. `check_and_handle_reorg` lives inside `live_process_block`; `backfill_process_block` never calls it. No code change needed beyond R1's extraction.

### Subxt config flip

In `crates/substrate/src/client.rs`:

```rust
// Phase 1
PolkadotConfig::builder().use_historic_types(false).build();

// Phase 2
PolkadotConfig::builder().use_historic_types(true).build();
```

Pure flag flip. §4 guarantees subxt never sees a pre-V14 block.

---

## 6. Testing strategy

Three layers plus a manual gate, proportional to risk.

### Layer 1 — Pure unit tests (no node, no DB)

In `crates/core/src/services/backfill.rs`:

```rust
#[cfg(test)]
mod plan_tests {
    use super::*;

    fn cursor(first: u64, last: u64) -> IndexerCursor {
        IndexerCursor {
            chain_id: "test".into(),
            first_indexed_block: first,
            last_indexed_block: last,
            last_indexed_hash: BlockHash([0u8; 32]),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn fresh_db_backfills_from_start_to_tip() {
        let plan = BackfillPlan::compute(0, None, 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn resume_ignores_start_when_covered() {
        let plan = BackfillPlan::compute(10, Some(&cursor(0, 50)), 100);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].from, 51);
        assert_eq!(plan.ranges[0].to, 100);
    }

    #[test]
    fn extend_below_triggers_two_phase() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 300);
        assert_eq!(plan.ranges.len(), 2);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
        assert_eq!(plan.ranges[0].from, 0);
        assert_eq!(plan.ranges[0].to, 99);
        assert_eq!(plan.ranges[1].direction, BackfillDirection::Upward);
        assert_eq!(plan.ranges[1].from, 201);
        assert_eq!(plan.ranges[1].to, 300);
    }

    #[test]
    fn already_at_tip_plans_nothing() {
        let plan = BackfillPlan::compute(0, Some(&cursor(0, 100)), 100);
        assert!(plan.ranges.is_empty());
    }

    #[test]
    fn downward_only_when_no_upward_needed() {
        let plan = BackfillPlan::compute(0, Some(&cursor(100, 200)), 200);
        assert_eq!(plan.ranges.len(), 1);
        assert_eq!(plan.ranges[0].direction, BackfillDirection::Downward);
    }
}
```

Five tests lock the plan matrix. New edge case → new test.

**`fetch_with_retry`** — a mock `BlockSource` that fails N times then succeeds. Assert success on retry `N+1`, error on retry `N+2`.

**Error Display formatting** — one assertion per new variant (`IndexerError::BackfillAborted`, `IndexerError::PreV14BlockRequested`, `StorageError::CursorGapViolation`) that the Display output contains the block number and other context. Catches copy-paste errors in user-facing messages.

### Layer 2 — Integration tests with mock `BlockSource`

New file: `crates/core/tests/backfill_integration.rs`.

A `MockBlockSource` returning canned `RawBlock`s from an in-memory `HashMap<u64, RawBlock>`, plus an in-memory `Repositories` fake (a `Mutex<BTreeMap>` keyed by block number — no SQLx, no Postgres). Scenarios:

- **End-to-end fresh backfill.** Seed with blocks `[0, 100]`. Assert all 101 blocks land in order and cursor ends at `[0, 100]`.
- **Downward gap-fill preserves invariant.** Seed repo with cursor `[50, 100]` and mock with blocks `[0, 100]`. Plan + execute for `start_block=0`. Final cursor `[0, 100]` with all 101 blocks in the repo in order.
- **Concurrency preserves order.** Mock adds random 10–50ms per-block delays. Run with `concurrency=16`. Blocks still persist in strict ascending order.
- **Abort on retry exhaustion.** Mock fails on block 42 every time. Run with `max_retries=3`. Run exits with `BackfillAborted { block: 42, .. }`, cursor sits at 41, blocks 0–41 all persisted.
- **Shutdown mid-backfill.** Start long backfill, signal shutdown after ~20 blocks. Clean exit with `ShutdownRequested`; cursor at a valid `[0, N]` for some `N ≥ 20`, no partial writes.
- **Handoff race simulation.** Backfill blocks `[0, 100]` from the mock; "live stream" (tokio channel) emits blocks `99, 100, 101, 102`. Assert 99 and 100 are skipped as already-indexed, 101 and 102 are persisted, final cursor `[0, 102]`.

### Layer 3 — Fixture-based decode test (extends Phase 1)

Phase 1 captured one balances block fixture to pin SCALE→JSON handler invariants. Phase 2 reuses it as-is; the `index_single_block` path is the same code both loops use. No new live-node fixtures required.

**Optional nice-to-have (not a gate):** a single pre-V14 fixture from any public Substrate chain (e.g. an old Kusama block), with a test asserting the V14 floor code refuses to decode it. If sourcing historical metadata is hard, skip this test and rely on Layer 2's mock-based floor-enforcement coverage.

### Layer 4 — Manual smoke test (gate criteria)

1. `cargo build --release` passes
2. `cargo clippy --all -- -D warnings` passes
3. `cargo test` passes (all new plan/integration tests)
4. Fresh DB + `maestro --start-block 0 --backfill-concurrency 4` against a live Allfeat node: ≥ 1000 blocks indexed, cursor advances monotonically, process exits to live stream cleanly
5. Resume mid-backfill: run #4 to ~500 blocks, SIGINT, restart with same args, resume picks up at last persisted block without re-processing
6. `--live-only` on fresh DB: starts at current tip without backfill, cursor initializes at tip, live blocks flow in normally
7. Handler outputs populated: balances/ats/midds bundles each show ≥1 record written during the backfill range

Gates 1–3 automated. Gates 4–7 manual, documented in the PR description.

### What is NOT tested

- **Real pre-V14 chain behavior.** Allfeat is V14-from-genesis, so the V14 floor path always returns 0 in practice. Layer 2 mocks cover the refusal logic; no live pre-V14 chain is exercised.
- **Very large backfills.** Layer 2 runs 101-block ranges. A 2M-block backfill correctness proof is inductive: 101 blocks work + cursor logic is range-based ⇒ scale is a performance question. Performance is validated only by gate #4's live smoke test.
- **Handler idempotency under re-processing.** X1. Out of scope.

---

## 7. Error handling

Phase 2 adds two `IndexerError` variants and one `StorageError` variant (R6). All other error sites stay at their Phase 1 granularity.

### New `IndexerError` variants

```rust
// In crates/core/src/error.rs

#[derive(thiserror::Error, Debug)]
pub enum IndexerError {
    // ... existing variants unchanged ...

    #[error("Backfill aborted at block {block}: {reason}")]
    BackfillAborted { block: u64, reason: String },

    #[error(
        "Cannot backfill below block {earliest_v14} on this chain — its runtime \
         uses pre-V14 metadata, which Maestro does not decode. Re-run with \
         --start-block {earliest_v14} (or higher), or use --live-only to skip \
         backfill entirely."
    )]
    PreV14BlockRequested { requested: u64, earliest_v14: u64 },
}
```

### New `StorageError` variant

`persist_block_atomic` returns `StorageResult<()>`, so the gap-violation error must live at the storage layer. It propagates upward through the existing `IndexerError::Storage(#[from] StorageError)` conversion — no extra plumbing required.

```rust
// In crates/core/src/error.rs (StorageError enum)

#[error("Cursor gap violation: block {block} does not extend range [{first}, {last}]")]
CursorGapViolation { block: u64, first: u64, last: u64 },
```

This variant is constructed in exactly one place: the final `match` arm in `persist_block_atomic`'s cursor-update logic (§2). In practice it is an "impossible state" error — reachable only if the caller bypasses `BackfillPlan::compute` — but making it a typed error instead of a panic keeps the indexer alive long enough to log diagnostic context.

### Abort semantics

- **Fetch retries exhausted** → `BackfillAborted { block, reason }` at the top of `run_range`. Cursor sits at the last successful block (last one that passed through `persist_block_atomic`).
- **Persist failure** (any `StorageError`) → aborts immediately with no retry. DB is local; failure means a bug or disk issue and retrying will loop forever.
- **Shutdown mid-backfill** → `ShutdownRequested`, cursor at last successful block, same resume story.
- **V14 floor violation** → `PreV14BlockRequested` at startup, before any block is fetched. User sees the error immediately.

All four preserve the "no silent gaps ever" contract.

### Per-block decode tolerance (unchanged from Phase 1)

Per-event decode skip-with-metric (Phase 1 §6) is preserved verbatim. If a specific event fails to decode inside a block, it is skipped with a `trace!` and a `record_decode_error` metric bump; the block still persists with its other events and extrinsics. Backfill and live loops inherit this behavior because they both go through `index_single_block`.

---

## 8. Risks & open questions

Captured for the writing-plans step. None block implementation.

### R1 — `fetch_block_at` exact 0.50 symbol

Subxt 0.50's API for fetching a block by number (rather than hash) involves a two-step lookup: RPC `block_hash(Some(n))` → `client.at(hash)`. Exact symbol names validated at first compile. Phase 1's ≤5-lines-per-call-site fallback policy applies. Not a design blocker.

### R2 — `metadata_at(hash)` for the V14 search

The binary search in §4 requires fetching runtime metadata at an arbitrary historical block hash. Subxt 0.50's `client.backend().metadata_at(hash)` is the likely target; if named differently, the fix is localized to one function. Not a blocker.

### R3 — Handler contract silent re-processing

X1 from §1.5. If a future bug causes `index_single_block` to run twice on the same block (outside the skip-if-indexed check), handlers would double-insert rows. Phase 2's integration tests exercise the skip-check path; beyond that, the contract stays "handlers assume one call per block per run".

### R4 — `live_only` with an existing gap

If user runs `--live-only` on a database with cursor `[0, 100]` and current tip `10000`, the 9900-block gap between 100 and the new tip will never be filled. Today's `warn_if_cursor_gap` emits a loud warning but does not refuse. Acceptable behavior; users who want strict gap protection can leave `--live-only` off.

### R5 — Backfill stalls indefinitely on a persistently-failing block

If block `N` is intrinsically undecodable (runtime upgrade corner case, unknown variant type), retries burn through the max and the run aborts. User has to investigate and either fix the decoder, raise `--start-block` above `N`, or switch to `--live-only`. No automatic skip — that would violate the contiguous-range invariant.

### R6 — `buffered(K)` memory pressure

With `concurrency=16`, up to 16 `RawBlock`s are held in memory simultaneously. At ~1–10 MB per block (worst case: block full of heavy extrinsics), this is 16–160 MB. Acceptable. If pathological blocks appear, lower the concurrency.

### R7 — Migration on an already-populated database

The migration in §2 adds `first_indexed_block` and backfills it to 0 for existing rows. This is correct only if every Phase 1 deployment indexed from genesis — which is true by construction (no CLI to start elsewhere). Not a risk in practice, but worth stating so future readers understand the assumption.

### R8 — Two-phase `BackfillPlan` crash safety

If a crash happens mid-downward-phase of a two-phase plan (user extending history below existing range), the cursor's `first_indexed_block` has advanced downward by some amount. On restart, the next `BackfillPlan::compute` sees a new `first_indexed_block` partway through the target range and continues correctly. Verified by integration test: "downward gap-fill preserves range invariant" + restart simulation.

---

## Phase 2 summary

| Area | Change |
|---|---|
| `BlockSource` trait | +`fetch_block_at(n)`, +`earliest_v14_block()`. Phase 1 methods unchanged. |
| `crates/core/src/services/` | New `backfill.rs`. `indexer.rs` refactored per R1/R2/R5. |
| `IndexerCursor` | +`first_indexed_block` field. Schema migration. |
| `CursorRepository` | +`extend_upward`, +`extend_downward`. |
| `persist_block_atomic` | Cursor update branches on block-number-vs-range. |
| `crates/substrate/src/client.rs` | +`fetch_block_at` impl, +`earliest_v14_block` (binary search). `use_historic_types` flipped to `true`. |
| `IndexerConfig` | +`BackfillConfig` sub-struct (4 new knobs). |
| `bin/maestro/src/main.rs` | `Cli` struct extracted to `cli.rs` (R4). 4 new CLI flags. |
| `IndexerError` | +2 variants (`BackfillAborted`, `PreV14BlockRequested`). |
| `StorageError` | +1 variant (`CursorGapViolation`). |
| `metrics.rs` | +4 backfill counters. `record_block_indexed` takes `IndexMode`. |
| Handlers (`balances`, `ats`, `midds`) | **Unchanged.** |
| `RawBlock` / domain models | **Unchanged.** |
| GraphQL / graphql crate | **Unchanged.** |
| Tests | +5 plan unit tests, +6 integration scenarios, manual smoke gate |

Next step after approval: `writing-plans` skill produces the implementation plan.
