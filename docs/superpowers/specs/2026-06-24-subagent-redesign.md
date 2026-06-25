# Subagent Redesign Spec

## Overview

Re-introduce subagent as an **independent crate** (`nekocode-subagent`) alongside `nekocode-subthread`. Both crates share the same architectural pattern (middleware + tool suite + registry) but differ in persistence strategy: subthread is DB-persisted, subagent is purely in-memory.

## Design Principles

1. **Subagent ≠ Subthread** — same tool/registry/middleware pattern, different persistence layer
2. **No run_loop_core extraction** — subagent runs its own simplified loop directly; no `MessageStore` trait abstraction
3. **No SubAgentBuilder** — direct construction via middleware + context, mirroring `SubthreadMiddleware`
4. **In-memory only** — IDs from `AtomicU64`, state in `DashMap`, messages in `RwLock<Vec<>>`. No DB rows created.

## Crate Structure

```
crates/nekocode-subagent/
├── Cargo.toml
└── src/
    ├── lib.rs
    ├── middleware.rs      ← SubagentMiddleware
    ├── registry.rs        ← SubagentRegistry (in-memory run state)
    ├── config.rs          ← SubagentConfig
    ├── run_loop.rs        ← simplified memory-only run loop
    └── tool/
        ├── mod.rs
        ├── spawn_agent.rs
        ├── inspect_agent.rs
        ├── read_agent.rs
        ├── send_message.rs
        ├── wait_any_agent.rs
        ├── wait_all_agents.rs
        └── abort_agent.rs
```

## Dependencies

```
nekocode-core { path = "../nekocode-core" }
nekocode-provider { path = "../nekocode-provider" }
nekocode-types { path = "../nekocode-types" }
async-trait = "0.1"
dashmap = "6.0"
tokio = { version = "1.52", features = ["sync", "macros", "rt-multi-thread", "time"] }
anyhow = "1.0"
tracing = "0.1"
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
```

## Core Types

### `SubagentConfig`

Minimal config. Currently only controls nesting:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentConfig {
    /// Whether subagents spawned by this subagent may themselves spawn subagents.
    /// Defaults to false to bound recursion depth.
    #[serde(default)]
    pub allow_nested: bool,
}
```

Mirrors `SubthreadConfig::allow_subthread` naming and serialization.

### `SubagentRunState`

```rust
#[derive(Debug, Clone)]
pub enum SubagentRunState {
    Idle,        // Created but not started
    Running,     // Background run_loop task in flight
    Finished,    // Completed successfully
    Error(String), // Run_loop errored; carries error message
}

impl SubagentRunState {
    pub fn is_ready(&self) -> bool {
        matches!(self, SubagentRunState::Finished | SubagentRunState::Error(_))
    }
}
```

Identical to `SubthreadRunState`. Not shared across crates — each crate defines its own enum to avoid coupling.

### `SubagentState`

Per-subagent in-memory bookkeeping:

```rust
#[derive(Debug)]
pub struct SubagentState {
    pub agent_id: u64,                      // AtomicU64-assigned ID
    pub run_state: SubagentRunState,
    pub task_handle: Option<tokio::task::JoinHandle<()>>,
    pub notify: Arc<tokio::sync::Notify>,    // For wait_any/wait_all
    pub messages: tokio::sync::RwLock<Vec<AgentMessage>>,  // Conversation history
}
```

Key difference from `SubthreadState`: no `thread_id` → `agent_id`; includes `messages` for in-memory history.

### `SubagentRegistry`

```rust
pub struct SubagentRegistry {
    states: dashmap::DashMap<u64, SubagentState>,
}
```

Owned per-parent agent, stored in `Agent.extensions["subagent"]`.

Methods:
- `new()` → empty registry
- `insert_idle(agent_id)` → register an idle subagent
- `run_state(agent_id)` → snapshot of current state, defaults to Idle
- `set_running(agent_id, handle)` → mark Running + store JoinHandle
- `set_finished(agent_id)` → mark Finished, wake waiters
- `set_error(agent_id, msg)` → mark Error, wake waiters
- `abort(agent_id)` → abort task if running, remove entry
- `abort_all_and_clear()` → abort all running tasks, return aborted IDs
- `notify(agent_id)` → clone of the Notify handle for waiters
- `all_agent_ids()` → all tracked IDs

### `SubagentContext`

Shared context for all tools, cheaply cloneable (all fields are Arc/Clone types):

```rust
#[derive(Clone)]
pub struct SubagentContext {
    pub provider: Arc<dyn nekocode_core::provider::Provider>,
    pub parent_thread_id: u64,
    pub parent_extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
    pub registry: Arc<SubagentRegistry>,
    pub config: Arc<SubagentConfig>,
}
```

### `SubagentMiddleware`

```rust
pub struct SubagentMiddleware {
    ctx: SubagentContext,
}
```

Constructor:
```rust
pub fn new(
    provider: Arc<dyn Provider>,
    parent_thread_id: u64,
    extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    config: SubagentConfig,
) -> Self
```

In `before_generate`: registers all 7 subagent tools into the ToolRegistry.
Publishes registry to `Agent.extensions["subagent"]`.

### Extension Key

```rust
pub const SUBAGENT_EXTENSION_KEY: &str = "subagent";
```

## Run Loop

The simplified memory-only run loop. Operates on `Arc<RwLock<Vec<AgentMessage>>>` for conversation history:

```rust
pub async fn subagent_run_loop(
    messages: Arc<tokio::sync::RwLock<Vec<AgentMessage>>>,
    provider: Arc<dyn Provider>,
    middlewares: &[Box<dyn Middleware>],
    prompt: String,
    mut sender: Option<UnboundedSender<AgentEvent>>,
) -> Result<SubagentRunSummary, AgentError>
```

Implementation outline:
1. Push user message (prompt) to `messages`
2. Outer loop (middleware-driven regeneration):
   - Run `before_generate` on all middlewares → build `GenerateRequest` from `messages`
   - Inner loop (tool-call resolution):
     - Emit `MessageStart` event
     - `provider.stream_generate` → collect provider events, forward to sender
     - Push assistant message to `messages` + DB-write-equivalent (just memory)
     - Execute tool calls → push results to `messages`
   - Break inner loop when no tool calls in response
   - Run `after_generate`; handle `AgentControlFlow::GenerateWith`
3. Emit `TurnEnd` event

**No DB operations.** All persistence is the in-memory `Vec`. No `Turn` creation, no `Message` DB rows.

Message type for in-memory storage: `nekocode_types::generate::Message` — same enum used by the DB model, so no type mismatch.

## Tools

All tools follow the `Tool` trait from `nekocode-types`. Each takes `SubagentContext`, implements `spec()` and `call()`.

### `SpawnSubagentTool`

- **Name**: `spawn_subagent`
- **Params**: `allow_nested: bool` (optional, default false)
- **Behavior**:
  1. Get next `agent_id` from `AtomicU64`
  2. Create `SubagentState` with Idle, new `RwLock<Vec<>>`, empty history
  3. Register in `SubagentRegistry`
  4. Return `{ agent_id, status: "idle" }`

### `InspectSubagentTool`

- **Name**: `inspect_subagent`
- **Params**: `agent_id: u64`
- **Returns**: `{ agent_id, status: "idle"|"running"|"finished"|"error" }`

### `ReadSubagentTool`

- **Name**: `read_subagent`
- **Params**: `agent_id: u64`, `start_turn: u64` (default 0), `limit: u64` (default 10), `text_only: bool` (default true)
- **Returns**: `{ agent_id, turns: [...] }` — same shape as `ReadSubthreadTool`
- `text_only` filters out reasoning blocks and tool-call-result messages

### `SendMessageToSubagentTool`

- **Name**: `send_message_to_subagent`
- **Params**: `agent_id: u64`, `content: string`
- **Behavior**:
  1. Validate ownership (agent belongs to this parent, checked via registry)
  2. If state is Running, push the user message into the subagent's message buffer
  3. The running subagent's next iteration will pick it up from `messages`
- **Note**: This is a "fire-and-forget" message injection. The tool returns immediately; the subagent processes it on its next loop iteration.

### `WaitAnySubagentTool`

- **Name**: `wait_any_subagent`
- **Params**: `agent_ids: [u64]`, `timeout: f64` (seconds)
- **Returns**: On ready → `{ status: "ready", agent_id, run_state }`; on timeout → `{ status: "timeout", pending: [agent_ids] }`

### `WaitAllSubagentsTool`

- **Name**: `wait_all_subagents`
- **Params**: `agent_ids: [u64]` (optional, defaults to all Running subagents), `timeout: f64` (seconds)
- **Returns**: On all ready → `{ status: "ready", results: [...] }`; on timeout → `{ status: "timeout", ready: [...], pending: [...] }`

### `AbortSubagentTool`

- **Name**: `abort_subagent`
- **Params**: `agent_id: u64`
- **Behavior**:
  1. Find subagent state in registry
  2. Abort JoinHandle if running
  3. Remove from registry
  4. Return `{ agent_id, aborted: true }`

## API Layer Integration

Dependencies to add back:
- Root `Cargo.toml`: `nekocode-subagent = { path = "crates/nekocode-subagent" }`
- `nekocode/Cargo.toml`: `nekocode-subagent.workspace = true`

### `thread/mod.rs`

**MiddlewareBuildContext**: re-add `provider` field:
```rust
pub(crate) struct MiddlewareBuildContext {
    pub db: toasty::Db,
    pub config: Arc<tokio::sync::RwLock<nekocode_types::config::Config>>,
    pub extensions: Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
    pub thread_id: u64,
    pub working_directory: String,
    pub subthread_activator: Arc<dyn nekocode_subthread::activator::ThreadActivator>,
    pub provider: Arc<dyn nekocode_core::provider::Provider>,
}
```

**build_middlewares**: add `"subagent"` case:
```rust
"subagent" => {
    let cfg = nekocode_subagent::SubagentConfig::from_value(&i.config);
    middlewares.push(Box::new(nekocode_subagent::SubagentMiddleware::new(
        ctx.provider.clone(),
        ctx.thread_id,
        ctx.extensions.clone(),
        cfg,
    )));
}
```

### `activate.rs`

Re-add `provider.clone()` to `MiddlewareBuildContext`. Subagent needs it; subthread doesn't (it resolves model itself in `activate`).

### `subthread_activator.rs`

Same: re-add `provider.clone()` to context.

## Cascade Cleanup

When a parent thread is deleted (`delete_threads_cascade`), any subagent running under it must be aborted. Cascade logic:
1. Find `SubagentRegistry` in `Agent.extensions["subagent"]` for the parent
2. Call `registry.abort_all_and_clear()` — aborts all running subagent tasks
3. Subagent state is purely in-memory, no DB cascade needed

## Boundary with Subthread

| Concern | Subthread | Subagent |
|---------|-----------|----------|
| Crate | `nekocode-subthread` | `nekocode-subagent` |
| Config struct | `SubthreadConfig` | `SubagentConfig` |
| Registry | `SubthreadRegistry` | `SubagentRegistry` |
| Run state enum | `SubthreadRunState` | `SubagentRunState` |
| Context struct | `SubthreadContext` | `SubagentContext` |
| Middleware | `SubthreadMiddleware` | `SubagentMiddleware` |
| Extension key | `"subthread"` | `"subagent"` |
| Tool prefix | `spawn_subthread`, etc. | `spawn_subagent`, etc. |
| ID type | DB `Thread.id` (u64) | AtomicU64 (u64) |
| Store type | DB (Toasty ORM) | `RwLock<Vec<AgentMessage>>` |
| Activator | `ThreadActivator` trait | None (direct Arc<Agent>) |

Types from `nekocode-core` shared:
- `Agent`, `AgentEvent`, `AgentEventType`, `AgentError`
- `Middleware` trait, `AgentControlFlow`
- `Provider` trait (via `nekocode-provider`)
- `GenerateRequest`, `GenerateResponse`
- `ToolRegistry`

Types from `nekocode-types` shared:
- `Tool`, `ToolSpec`, `ToolError`, `ToolCallResult`
- `StreamEvent`, `StreamEventData`
- `AgentMessage`, `MessageContent`, `AssistantMessage`

## No Shared Abstractions

Do NOT create shared traits or base types between the two crates. Each crate defines its own:
- Registry (even though structurally identical)
- Run state enum (even though variants are identical)
- Context struct (even though fields overlap)
- Config struct

This avoids coupling and allows each crate to evolve independently. The only shared layer is `nekocode-core`'s `Agent::run_loop` and the `Middleware`/`Provider` traits.

## Implementation Phases

### Phase 1: Skeleton
- Create `crates/nekocode-subagent/Cargo.toml`
- Create `src/lib.rs`, `src/middleware.rs`, `src/registry.rs`, `src/config.rs`, `src/run_loop.rs`
- Add workspace + crate dependencies

### Phase 2: Registry + Config
- Implement `SubagentRegistry` (all methods + tests)
- Implement `SubagentConfig` (derive Serialize/Deserialize, roundtrip tests)

### Phase 3: Run Loop
- Implement `subagent_run_loop` in `run_loop.rs`
- Unit test with MockProvider against in-memory messages

### Phase 4: Tools
- Implement all 7 tools under `src/tool/`
- Integration test: spawn → inspect → read → wait → abort lifecycle

### Phase 5: Middleware + API Integration
- `SubagentMiddleware` registers tools, publishes registry
- Update `build_middlewares`, `MiddlewareBuildContext`, `activate.rs`, `subthread_activator.rs`
- Add workspace/crate dependencies
- End-to-end test through API
