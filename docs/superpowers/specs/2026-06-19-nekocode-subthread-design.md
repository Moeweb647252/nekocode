# nekocode-subthread Design

**Date:** 2026-06-19
**Status:** Approved for planning
**Author:** Misaka Xiaxun (designed with brainstorming skill)

## Overview

Add a subthread system that lets a parent thread spawn child threads which run independently, expose their message history, and can be polled for completion. Subthreads share the same middleware architecture as top-level threads but use a new dedicated `SubthreadMiddleware` that registers nine tools into the parent's `ToolRegistry`.

## Goals

- Allow a parent thread to spawn child threads with a constrained `working_directory`.
- Allow the parent to start, inspect, read history from, and configure subthreads.
- Allow the parent to fan-out to multiple subthreads and synchronize on completion (`wait_any_subthread`, `wait_all_subthreads`).
- Persist subthread relationships in the database (`Thread.own_by_id`).
- Cascade delete subthreads when their parent is deleted.

## Non-Goals

- Cross-thread tool sharing at runtime (subthreads each have their own `active_threads` entry; cross-thread coordination is via DB only).
- Streaming subthread output back to the parent in real time (parent reads results via `read_subthread` after completion).
- Subthreads persisting their run state across server restarts (in-memory `SubthreadRegistry` only; DB has the messages).

## Architecture

### Components

1. **`nekocode-entities`** — Add `own_by_id: Option<u64>` to `Thread`.
2. **`nekocode-subthread` crate** — `SubthreadMiddleware` + nine tool implementations + `SubthreadRegistry` shared state.
3. **`nekocode` API layer** — Register `SubthreadRegistry` in `AppState`, register the subthread middleware in `activate_thread`, modify `delete_thread` for cascade delete.

### Data Model

**Thread entity** (`crates/nekocode-entities/src/thread.rs`):

```rust
/// ID of the parent thread that owns this subthread. `None` for top-level threads.
/// Used to express the subthread relationship in the database.
#[index]
pub own_by_id: Option<u64>,
```

`None` means top-level thread; `Some(parent_id)` means subthread. Indexed for efficient "find all subthreads of thread X" queries. Nullable so the column add succeeds against legacy DBs without backfill.

### In-Memory State: SubthreadRegistry

A `DashMap<u64, SubthreadState>` lives in `AppState` (shared across all subthread tools within a server process):

```rust
pub enum SubthreadRunState {
    Idle,
    Running,
    Finished,
    Error(String),
}

pub struct SubthreadState {
    pub thread_id: u64,
    /// Which parent thread owns this subthread. Mirrors `Thread.own_by_id` and
    /// lets `wait_all_subthreads` filter to the calling parent without a DB hit.
    pub parent_thread_id: u64,
    pub run_state: SubthreadRunState,
    pub task_handle: Option<tokio::task::JoinHandle<()>>,
    pub notify: Arc<tokio::sync::Notify>,
}
```

The `Notify` is fired via `notify_waiters()` whenever `run_state` transitions from `Running` to `Finished`/`Error`, powering `wait_any_subthread` / `wait_all_subthreads`.

### AppState Sharing

The subthread tools need to mutate the same `active_threads` and `generate_states` maps the API layer uses, so `SubthreadMiddleware` carries `Arc` clones of both alongside the `SubthreadRegistry`. The middleware's constructor receives them from `activate_thread`.

```rust
pub struct SubthreadMiddleware {
    pub db: toasty::Db,
    pub parent_thread_id: u64,
    pub parent_working_directory: String,
    pub registry: Arc<SubthreadRegistry>,
    pub active_threads: Arc<dashmap::DashMap<u64, Arc<RwLock<Agent>>>>,
    pub generate_states: Arc<dashmap::DashMap<u64, Arc<GenerateState>>>,
    pub config_models: Arc<RwLock<Config>>, // For provider lookup on start_subthread
    pub config: Arc<SubthreadConfig>,
}
```

Because `Agent` and `GenerateState` are defined in the API crate (`nekocode`), the `nekocode-subthread` crate cannot reference them directly. To keep the dependency direction sound, the API layer constructs the tool implementations and passes them to the middleware, OR the subthread crate defines a small trait abstracting "activate a thread" / "register a running generation". Concrete approach is decided during planning; the spec only fixes the data the tools need.

## Tool Specifications

All tools are registered by `SubthreadMiddleware` during `before_generate`. They operate on the parent's DB connection, parent's `SubthreadRegistry`, and the parent's `parent_thread_id` / `parent_working_directory` carried in the middleware.

### 1. `spawn_subthread`

Create a subthread entity and seed default middlewares.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `working_directory` | string | yes | Must be the parent's working_directory or a subdirectory of it. |
| `allow_subthread` | boolean | no | Whether this subthread can spawn its own sub-subthreads. Default `false`. |

**Returns:** `{ subthread_id: u64 }`

**Behavior:**
- Validate `working_directory` is within parent's `working_directory` (relaxed containment: equal or descendant).
- Canonicalize both paths before comparison to defeat symlink traversal.
- Create `Thread` with `own_by_id = Some(parent_thread_id)`, `workspace_id` inherited from parent.
- Seed default middlewares:
  - `shell` with `working_directory = subthread.working_directory`
  - `tool` with `working_directory = subthread.working_directory`
  - `subthread` (only when `allow_subthread = true`) with `SubthreadConfig { allow_subthread: false }` (no transitive recursion by default)
- Insert into `SubthreadRegistry` as `Idle`.
- Does **not** activate the subthread.

### 2. `list_subthreads`

List all subthreads of the current thread.

**Parameters:** none

**Returns:** `{ subthreads: [{ subthread_id, working_directory, run_state, allow_subthread }] }`

**Behavior:**
- Query DB: `Thread FILTER .own_by_id == #(parent_thread_id)`.
- Enrich each row with `run_state` from `SubthreadRegistry` (default `Idle` if not present).
- Read `allow_subthread` from the subthread's middleware list: `true` if a `subthread` middleware row exists with `enabled = true`.

### 3. `start_subthread`

Activate a subthread and run it with a prompt in the background.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_id` | integer | yes | ID returned by `spawn_subthread`. |
| `prompt` | string | yes | User message to send as the first turn. |

**Returns:** `{ subthread_id, status: "started" }`

**Behavior:**
- Validate subthread exists and `own_by_id == parent_thread_id`; otherwise `ItemNotFound`.
- Reject if `SubthreadRegistry` shows `Running` (no concurrent runs).
- Load subthread's middlewares from DB and build the `Agent` (same logic as `activate_thread` API).
- Insert into `state.active_threads`; reject if already present (`ThreadAlreadyActivated`).
- Spawn a background `tokio::task` calling `agent.run_loop(prompt, tx)`; the `tx` channel's events are discarded (results persisted to DB, read via `read_subthread`).
- Update `SubthreadRegistry` state to `Running`; store `JoinHandle`; arm `Notify`.
- On task completion: update state to `Finished` or `Error(msg)`, fire `Notify`, remove from `active_threads` and any `generate_states` entry, drop the `JoinHandle`.

### 4. `inspect_subthread`

Check a subthread's current state.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_id` | integer | yes | |

**Returns:** `{ subthread_id, run_state, working_directory, allow_subthread }`

**Behavior:**
- Validate subthread exists and `own_by_id == parent_thread_id`; otherwise `ItemNotFound`.
- Read `run_state` from `SubthreadRegistry` (default `Idle` if absent).
- Read `working_directory` from the thread row.
- Read `allow_subthread` from the middleware list (same logic as `list_subthreads`).

### 5. `read_subthread`

Read a subthread's persisted message history.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_id` | integer | yes | |
| `start_turn` | integer | no | 0-based turn index to start from. Default `0`. |
| `limit` | integer | no | Max turns to return. Default `10`. |

**Returns:** `{ subthread_id, turns: [{ turn_index, messages: [...] }] }`

**Behavior:**
- Validate subthread exists and `own_by_id == parent_thread_id`; otherwise `ItemNotFound`.
- Query `Turn FILTER .thread_id == #(subthread_id) ORDER BY .turn_index ASC`.
- Apply `LIMIT`/`OFFSET` via `start_turn` / `limit`.
- For each turn, include its messages (same pattern as `Agent::run_loop` history loading).
- Returns the same shape used by the existing thread history API.

### 6. `subthread_settings`

View a subthread's middleware settings.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_id` | integer | yes | |

**Returns:** `{ subthread_id, middlewares: [{ id, name, config, enabled }] }`

**Behavior:**
- Validate subthread exists and `own_by_id == parent_thread_id`; otherwise `ItemNotFound`.
- Query `Middleware FILTER .thread_id == #(subthread_id)`.
- Return shape matches the existing `list_middlewares` API.

### 7. `set_subthread_settings`

Modify a subthread's middleware settings.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_id` | integer | yes | |
| `middleware_id` | integer | yes | Which middleware row to update. |
| `config` | object | no | New config JSON. |
| `enabled` | boolean | no | Toggle enabled flag. |

**Returns:** `{ subthread_id, middleware_id, updated: true }`

**Behavior:**
- Validate subthread exists, is owned by parent, and the middleware row belongs to the subthread; otherwise `ItemNotFound`.
- Reject if subthread is currently `Running` (`ThreadGenerating`); parent must wait for completion.
- Update the middleware row in DB (same pattern as `update_middleware` API).
- Evict subthread from `active_threads` so the next `start_subthread` rebuilds the agent with the new config.

### 8. `wait_any_subthread`

Wait until any one of the specified subthreads becomes ready (Finished or Error), or until the timeout elapses.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_ids` | array&lt;integer&gt; | yes | Subthreads to wait on. |
| `timeout` | number | yes | Maximum seconds to wait. Must be positive. |

**Returns:**
- Ready: `{ status: "ready", subthread_id, run_state }`
- Timeout: `{ status: "timeout", pending: [subthread_id, ...] }`

**Behavior:**
- Validate all IDs exist and `own_by_id == parent_thread_id`; otherwise `ItemNotFound`.
- If any are already in `Finished`/`Error`, return immediately with the first one found.
- Loop: `notify.notified().await` (across all entries), then re-check; break when one is ready or deadline elapses.
- Timeout does **not** kill or affect running subthreads; the parent can call again or proceed.

### 9. `wait_all_subthreads`

Wait until all specified subthreads are ready, or until the timeout elapses.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `subthread_ids` | array&lt;integer&gt; | no | Defaults to all subthreads of parent that are currently `Running`. |
| `timeout` | number | yes | Maximum seconds to wait. Must be positive. |

**Returns:**
- All ready: `{ status: "ready", results: [{ subthread_id, run_state }] }`
- Timeout: `{ status: "timeout", ready: [...], pending: [...] }`

**Behavior:**
- Validate explicit IDs (if provided) all exist and are owned by parent; otherwise `ItemNotFound`.
- Default scope: every subthread in `SubthreadRegistry` with `own_by_id == parent_thread_id` and `run_state == Running`. (Excludes `Idle` so the call doesn't block forever on subthreads that were never started.)
- If the resolved set is empty, return `ready` with empty `results`.
- Loop waiting on `Notify`, re-checking each entry; break when all are ready or deadline elapses.

## "ready" Semantics

`SubthreadRunState::Finished` and `SubthreadRunState::Error(_)` are both considered "ready" — the subthread has completed a `run_loop` and its message history is persisted. `Idle` (never started) and `Running` are not ready.

## Working Directory Validation

`spawn_subthread` validates containment by canonicalizing both paths and applying `child == parent || child.starts_with(parent)`. Symlinks are resolved to prevent `..` / symlink-based escapes. Path comparison uses `std::path::Path::starts_with` on the canonical forms.

## Cascade Delete

`delete_thread` (in `crates/nekocode/src/api/thread/delete.rs`) is modified to cascade:

1. Refuse if parent is mid-generation (`ThreadGenerating`).
2. Collect all threads to delete: the parent plus every thread reachable via `own_by_id` (recursive).
3. For each thread in the set:
   - Cancel any in-flight `task_handle` via `SubthreadRegistry` (call `task_handle.abort()`).
   - Remove from `active_threads` and `generate_states`.
   - In a single transaction, delete `Message` (per turn) → `Turn` → `Middleware` → `Thread` rows.
4. Commit transaction; remove parent from `active_threads` last.

The set-collection step uses a single query for direct subthreads, then recurses for transitive descendants. The depth is bounded by recursion since each subthread can only spawn sub-subthreads when its `subthread` middleware is enabled.

## SubthreadMiddleware Wiring

A new middleware crate `nekocode-subthread` exposes:

```rust
pub struct SubthreadConfig {
    #[serde(default)]
    pub allow_subthread: bool,
}

pub struct SubthreadMiddleware {
    pub db: toasty::Db,
    pub parent_thread_id: u64,
    pub parent_working_directory: String,
    pub registry: Arc<SubthreadRegistry>,
    pub config: Arc<SubthreadConfig>,
}
```

`SubthreadMiddleware::before_generate` registers all nine tools into the `ToolRegistry`, each capturing the fields above (and `Db`, `Arc` handles where needed).

`SubthreadMiddleware::after_generate` is a no-op (subthreads have no `AgentControlFlow` override).

In `activate_thread` (API layer), the middleware loop gains a new arm:

```rust
"subthread" => {
    let cfg = SubthreadConfig::from_value(&i.config);
    let parent_wd = state.db.query_thread_wd(thread_id).await?;
    middlewares.push(Box::new(SubthreadMiddleware::new(
        state.db.clone(),
        thread_id,
        parent_wd,
        state.subthread_registry.clone(),
        cfg,
    )));
}
```

`AppState` gains `subthread_registry: Arc<SubthreadRegistry>` so the API layer can drive cascade delete and so the middleware can register tools against the shared registry.

## Error Cases

| Scenario | Error |
|---|---|
| `working_directory` outside parent's tree | `InvalidInput` |
| `start_subthread` on already-running subthread | `InvalidInput("subthread already running")` |
| `start_subthread` on non-existent subthread | `ItemNotFound` |
| `start_subthread` on subthread not owned by this parent | `ItemNotFound` |
| `set_subthread_settings` while subthread is `Running` | `ThreadGenerating` |
| Sub-subthread spawn without `allow_subthread = true` | Tool not found (middleware not registered) |
| `wait_*_subthread` references subthread not owned by this parent | `ItemNotFound` |

## Testing Strategy

- **Unit tests:** working-directory containment (canonicalization, symlink defeat); config serde round-trips.
- **Integration tests against a temp DB:**
  - Spawn → list → start → inspect (Running) → wait → inspect (Finished) → read.
  - Cascade delete: spawn two subthreads, delete parent, assert all three threads and their messages are gone.
  - `wait_any` with one already-Finished and one Running: returns immediately with the Finished one.
  - `wait_all` excludes `Idle` subthreads from the default set.
  - `set_subthread_settings` while Running returns `ThreadGenerating`.
  - `spawn_subthread` with a `working_directory` outside the parent tree returns `InvalidInput`.
- **Recursive spawn:** spawn a subthread with `allow_subthread = true`, then from inside it spawn a sub-subthread and assert the chain.

## Out-of-Scope (Deferred)

- Cross-thread tool-call routing (parent calling a tool on a subthread).
- Real-time streaming of subthread deltas to the parent.
- Persisting `SubthreadRunState` to DB across restarts.
- Concurrency limits (max parallel subthreads per parent).
## Verification

**Automated** (run from repo root):

- `cargo build --workspace` — full workspace builds clean (only pre-existing dead-code warnings in `nekocode/src/api/mod.rs`).
- `cargo test --workspace` — all tests pass, including:
  - 1 entity test (`own_by_id_roundtrips`) confirming the schema change persists.
  - 13 `nekocode-subthread` unit tests (config serde, registry state machine, path containment).
  - 6 `nekocode-subthread` integration tests (`tests/integration.rs`) covering spawn/list/inspect/read/settings + cascade delete + cross-parent isolation.
  - All other crates' suites (shell, tool, types, etc.) unaffected.
- `cargo clippy -p nekocode-subthread --all-targets --no-deps -- -D warnings` — clean.

**Manual API smoke test** (requires a running server with a configured model):

1. `POST /api/thread/create` with a working_directory → note `thread_id`.
2. `POST /api/middleware/create` with `{ thread_id, name: "subthread", config: {} }`.
3. `POST /api/thread/activate` to register the subthread middleware.
4. `POST /api/generate/stream` (WebSocket) with a prompt asking the agent to:
   - `spawn_subthread` in a subdirectory,
   - `start_subthread` with a prompt,
   - poll via `inspect_subthread` until `run_state == "finished"`,
   - `read_subthread` to confirm the subthread's message history is persisted.
5. Optional fan-out: spawn N subthreads, start them in parallel, then `wait_all_subthreads` to synchronize.
6. `POST /api/thread/delete` on the parent; verify subthreads are gone (their rows are cascade-deleted in a single transaction).
