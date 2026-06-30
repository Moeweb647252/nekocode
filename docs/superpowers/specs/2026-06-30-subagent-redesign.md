# Subagent Redesign Spec

## Overview

Re-introduce subagent as an **independent leaf crate** (`nekocode-subagent`) alongside `nekocode-subthread`. Both crates share the same architectural pattern (middleware + tool suite + per-parent registry in `Agent.extensions`), but subagent is **lighter** than subthread: purely in-memory, single-turn, no DB, no `ThreadActivator`-style trait, and it **reuses `Agent::run_loop` directly** rather than reimplementing a parallel loop.

This redesign supersedes the removed `2026-06-24-subagent-redesign.md`, whose core premise ("`Agent::run_loop` weaves DB persistence into every step, so subagent must reimplement the loop") was invalidated by commit `6ec5423` ("refactor(agent): decouple Agent::run_loop from Storage"). `run_loop` now takes history as a value (`Vec<Turn>`), returns a `Turn`, and never touches the DB — so a subagent gets isolation by construction and persistence is the API layer's concern (optional for ephemeral subagents).

## Design Principles

1. **Subagent ≠ Subthread** — same middleware+tool+registry pattern, but lighter: no DB, no multi-turn, no activator-as-trait-for-thread-activation. The one trait this design introduces (`SubagentMiddlewareFactory`) builds isolated middleware *instances*, not DB threads.
2. **Reuse `Agent::run_loop`** — no parallel loop, no `MessageStore` abstraction, no "keep in lockstep" maintenance burden. The subagent runner calls `run_loop` with empty `old_turns` and captures the returned `Turn`.
3. **Single-turn only** — a subagent is spawned with a prompt, runs `run_loop` once, stores the result, and is done. No `send_message`, no history continuation, no run-in-progress injection. This matches `run_loop`'s value-semantics contract with no contortion.
4. **Independent middleware instances** — a child does not share its parent's middleware instances. Each child gets fresh instances (own shell session map, own `thread_id`). Instances are built at spawn time via a trait, because only the API layer can see the `shell/file/mcp/skills` constructors.
5. **`nekocode-subagent` depends only on `nekocode-core` + `nekocode-types`** — it never imports `nekocode-shell/file/mcp/skills/entities/provider/subthread`. All middleware/provider construction is delegated to the API layer via the `SubagentMiddlewareFactory` trait.
6. **No shared abstractions across the two crates** — each crate owns its own registry, run-state enum, context struct, config struct, and tool helpers (per the "no cross-crate shared types" guideline from the prior design). They evolve independently.

## Crate Structure

```
crates/nekocode-subagent/
├── Cargo.toml
└── src/
    ├── lib.rs                 ← re-exports, SUBAGENT_EXTENSION_KEY
    ├── config.rs              ← SubagentConfig (parent middleware config: max_depth)
    ├── profile.rs             ← SubagentProfile, ProfileCatalog, load + merge
    ├── registry.rs            ← SubagentRegistry, SubagentState, SubagentRunState, SubagentRunResult
    ├── factory.rs             ← SubagentMiddlewareFactory trait
    ├── runner.rs              ← run_subagent: build child Agent, call run_loop, capture Turn
    ├── middleware.rs          ← SubagentMiddleware (registers 6 tools, publishes registry)
    └── tool/
        ├── mod.rs             ← parse helpers, run_state_name, notify_any
        ├── spawn_subagent.rs
        ├── inspect_subagent.rs
        ├── read_subagent.rs
        ├── wait_any_subagent.rs
        ├── wait_all_subagents.rs
        └── abort_subagent.rs
```

## Dependencies

`nekocode-subagent/Cargo.toml` — strictly lighter than `nekocode-subthread` (no `nekocode-entities`):

```toml
[package]
name = "nekocode-subagent"
version = "0.1.0"
edition = "2024"

[dependencies]
nekocode-core.workspace = true
nekocode-types.workspace = true
toasty.workspace = true            # only to name Agent.db's toasty::Db type; never queried
async-trait.workspace = true
anyhow.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio.workspace = true
tokio-util.workspace = true
tracing.workspace = true
dashmap.workspace = true
jiff.workspace = true
futures-util.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

Notably **absent** vs `nekocode-subthread`: `nekocode-entities` (no DB queries, no entity types). `toasty` is present only because `nekocode_core::agent::Agent.db: toasty::Db` requires the type to be named; the handle is never used by subagent code.

Root `Cargo.toml` gains `nekocode-subagent = { path = "crates/nekocode-subagent" }` in `[workspace.dependencies]`.

## Core Types

### `MiddlewareSpec` (in `nekocode-core`)

```rust
// nekocode-core/src/middleware.rs
/// Name + raw config — enough for the API layer to rebuild an isolated
/// instance. Defined in nekocode-core so nekocode-subagent (core+types only)
/// can refer to it by name without seeing the middleware crates.
#[derive(Debug, Clone)]
pub struct MiddlewareSpec {
    pub name: String,
    pub config: serde_json::Value,
}
```

This is the only addition to `nekocode-core`: a tiny data struct, no behavior, no new dependencies.

### `SubagentMiddlewareFactory` trait (in `nekocode-subagent`)

```rust
// nekocode-subagent/src/factory.rs
use std::any::Any;
use std::sync::Arc;
use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};

/// Builds an isolated child middleware instance from a spec.
/// `subagent_id` is the AtomicU64-allocated ID (not a DB id); `extensions`
/// is the child's fresh DashMap so middleware like shell gets its own
/// session map. Implemented by the API crate, which is the only layer that
/// can see the shell/file/mcp/skills constructors.
#[async_trait]
pub trait SubagentMiddlewareFactory: Send + Sync {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware>;
}
```

This trait is the **single** dependency-inversion point. It is named `SubagentMiddlewareFactory` (not "Activator") to distinguish it from `nekocode-subthread`'s `ThreadActivator`: the latter activates DB-persisted threads; this one builds in-memory middleware instances. The name describes the responsibility exactly.

### `SubagentConfig` (parent middleware config)

```rust
// nekocode-subagent/src/config.rs
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SubagentConfig {
    /// Maximum *nesting* depth — how many levels of subagents may spawn
    /// further subagents. The top-level thread spawns level-1 subagents
    /// (depth 0 → child at depth 1); `max_depth` bounds how deep those may
    /// nest. `max_depth = 0` (the default) means level-1 subagents cannot
    /// themselves spawn (depth 1 + 1 > 0). `max_depth = 1` allows one level
    /// of nesting (a level-1 subagent may spawn a level-2 subagent, but the
    /// level-2 one cannot). Propagated unchanged down the chain as the
    /// single tree-wide bound.
    #[serde(default)]
    pub max_depth: u32,
}

impl SubagentConfig {
    pub fn from_value(v: &serde_json::Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}
```

### `SubagentRunState`

```rust
// nekocode-subagent/src/registry.rs
#[derive(Debug, Clone)]
pub enum SubagentRunState {
    /// Unused under the single-turn model (spawn transitions straight to
    /// Running). Retained for symmetry with SubthreadRunState and future
    /// extension; a dead branch in v1.
    Idle,
    /// A background run_loop task is in flight.
    Running,
    /// The background task completed; result stored in SubagentState.result.
    Finished,
    /// The background task errored; carries the error message.
    Error(String),
}

impl SubagentRunState {
    pub fn is_ready(&self) -> bool {
        matches!(self, SubagentRunState::Finished | SubagentRunState::Error(_))
    }
}
```

Defined separately from `SubthreadRunState` (no cross-crate sharing). `Idle` is retained for symmetry but is a dead branch in the single-turn model — accepted asymmetry, not over-engineering.

### `SubagentRunResult`

```rust
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRunResult {
    pub usage: nekocode_types::generate::Usage,
    /// All messages from the captured Turn (assistant, tool-call, tool-result).
    /// read_subagent extracts the last assistant text from here.
    pub messages: Vec<nekocode_types::generate::Message>,
    pub finished: bool,
}
```

This replaces the DB that `ReadSubthreadTool` reads from: the runner stores the captured `Turn` here so `read_subagent` can return the last assistant text without any DB.

### `SubagentState`

```rust
#[derive(Debug)]
pub struct SubagentState {
    pub agent_id: u64,                       // AtomicU64-assigned, not a DB id
    pub run_state: SubagentRunState,
    pub task_handle: Option<tokio::task::JoinHandle<()>>,
    pub notify: Arc<tokio::sync::Notify>,
    /// The subagent's run_loop result. None until Finished. read_subagent
    /// reads the last assistant message from here.
    pub result: tokio::sync::RwLock<Option<SubagentRunResult>>,
}

impl SubagentState {
    pub fn new(agent_id: u64) -> Self {
        Self {
            agent_id,
            run_state: SubagentRunState::Running,  // spawn → Running immediately
            task_handle: None,
            notify: Arc::new(Notify::new()),
            result: tokio::sync::RwLock::new(None),
        }
    }
}
```

Key difference from `SubthreadState`: `agent_id` (AtomicU64) instead of `thread_id` (DB), and a `result` field (in-memory captured Turn) instead of DB rows.

### `SubagentRegistry`

Per-parent, owned by the parent's `Agent.extensions["subagent"]` (NOT a process-global singleton). Owns the ID allocator (subthread gets IDs from DB `Thread.id`; subagent has no DB, so the registry owns the `AtomicU64`):

```rust
#[derive(Debug, Default)]
pub struct SubagentRegistry {
    states: dashmap::DashMap<u64, SubagentState>,
    next_id: std::sync::atomic::AtomicU64,
}

impl SubagentRegistry {
    pub fn new() -> Self { Self::default() }

    /// Allocate a new agent_id and insert a Running entry. Returns the id.
    /// Called by spawn_subagent.
    pub fn allocate_running(&self) -> u64 { /* fetch_add + insert */ }

    pub fn run_state(&self, agent_id: u64) -> SubagentRunState { /* default Idle */ }
    pub fn set_running(&self, agent_id: u64, handle: JoinHandle<()>) { /* ... */ }
    pub fn set_finished(&self, agent_id: u64, result: SubagentRunResult) { /* store + notify_waiters */ }
    pub fn set_error(&self, agent_id: u64, msg: String) { /* notify_waiters */ }
    pub fn abort(&self, agent_id: u64) { /* abort handle + remove */ }
    pub fn abort_all_and_clear(&self) -> Vec<u64> { /* abort running, clear, return aborted ids */ }
    pub fn contains(&self, agent_id: u64) -> bool { /* ... */ }
    pub fn notify(&self, agent_id: u64) -> Option<Arc<Notify>> { /* ... */ }
    pub fn all_agent_ids(&self) -> Vec<u64> { /* ... */ }
    pub fn result(&self, agent_id: u64) -> Option<...> { /* read guard on result */ }
}
```

### `SubagentContext`

Cheaply cloneable (all fields Arc/Clone), shared by all 6 tools:

```rust
#[derive(Clone)]
pub struct SubagentContext {
    pub registry: Arc<SubagentRegistry>,
    pub specs: Vec<MiddlewareSpec>,                    // parent's enabled, captured at new
    pub factory: Arc<dyn SubagentMiddlewareFactory>,   // API-bound
    pub parent_provider: Arc<dyn Provider>,            // inherited (no profile.model)
    pub parent_config: Arc<RwLock<Config>>,
    pub parent_working_directory: String,
    pub catalog: Arc<ProfileCatalog>,                  // loaded at parent new
    pub depth: u32,                                    // nesting depth; 0 = top-level parent
    pub max_depth: u32,                                // from the root parent's SubagentConfig
    pub allow_nested: bool,                            // from the parent's profile (Gate A)
}
```

Nesting fields: `max_depth` is taken from the **root** parent's `SubagentConfig` and propagated unchanged down the chain (a single bound for the whole tree). `allow_nested` is taken from each agent's **own profile** and re-derived at each level (the child's `SubagentMiddleware::new` reads the child profile's `allow_nested`), so each level's profile independently gates whether its children may nest. `depth` increments by 1 each level.
```

Ownership is implicit: the registry is per-parent and lives in `Agent.extensions["subagent"]`, so every tool validates `registry.contains(agent_id)` directly — **no DB query** (contrast subthread's `require_owned_subthread` DB query).

### Extension Key

```rust
// nekocode-subagent/src/lib.rs
pub const SUBAGENT_EXTENSION_KEY: &str = "subagent";
```

## Profile Format & Layered Config

### File locations

```
global:    <config_dir>/nekocode/agents.toml           (same dir as config.toml)
workspace: <working_directory>/.nekocode/agents.toml   (looked up under working_directory only)
```

`config_dir` is the `dirs::config_dir()` already resolved in `main.rs:28`. The workspace path is resolved relative to the parent agent's `working_directory` (already on `Agent.working_directory` / `Thread.working_directory`).

### File format (`agents.toml`)

```toml
# agents.toml — top-level is a profile array. No other tables.
[[agents]]
name = "explorer"
system_prompt = "You are a focused exploration subagent..."  # optional
working_directory = "/abs/path"                               # optional; omit = inherit parent
allow_nested = false                                          # optional, default false
middlewares = ["shell", "tool"]                               # must be ⊆ parent's enabled

[[agents]]
name = "reviewer"
middlewares = ["tool"]   # read-only: file tools only, no shell
```

Each `[[agents]]` maps to a `SubagentProfile`. The file contains **only** `[[agents]]` entries — no structural coupling to the main `config.toml`.

### `SubagentProfile`

```rust
pub struct SubagentProfile {
    pub name: String,
    pub system_prompt: Option<String>,    // prepended
    pub working_directory: Option<String>, // None = inherit parent
    pub allow_nested: bool,                // default false
    pub middlewares: Vec<String>,          // names; must be ⊆ parent's enabled specs at spawn
}
```

No `model` field — subagent always inherits the parent's provider (decided during design: subagent is the same model with a different system-prompt/toolset, not a separate model node).

### Load & merge (`ProfileCatalog`)

```rust
pub struct ProfileCatalog {
    profiles: std::collections::HashMap<String, SubagentProfile>,  // keyed by name
}

impl ProfileCatalog {
    pub fn load(global_path: &Path, workspace_path: Option<&Path>) -> Result<Self>;
    pub fn get(&self, name: &str) -> Result<&SubagentProfile>;
}
```

**Merge rule (workspace wins):**
1. Load global `agents.toml` (missing → empty catalog, no error).
2. Load workspace `.nekocode/agents.toml` (missing → skip).
3. **Merge by profile name:** a workspace profile with the same `name` **wholly replaces** the global one (whole-object replacement, not field-level merge — simpler, predictable). Distinct names → accumulate.
4. Result: a flat `HashMap<String, SubagentProfile>`.

**Reload:** the catalog is loaded once at parent `SubagentMiddleware::new` and cached in the middleware. Editing a profile requires re-activating the parent to take effect — consistent with the main `config.toml`'s load-at-startup semantics. Hot-reload is explicitly out of scope.

### Validation

At load (`ProfileCatalog::load`):
- Each profile has a non-empty `name`.
- No duplicate `name` after merge.
- `middlewares` entries are non-empty strings (syntactic only).

Validation of the valid-middleware set is **deferred to spawn**, because it depends on what the parent actually enabled (design constraint: `profile.middlewares` must be ⊆ parent's enabled specs).

## Run Loop

The runner calls the existing `Agent::run_loop` directly — no parallel loop, no `MessageStore`.

```rust
// nekocode-subagent/src/runner.rs
pub async fn run_subagent(
    agent_id: u64,
    child: nekocode_core::agent::Agent,
    prompt: String,
    registry: Arc<SubagentRegistry>,
    sender: tokio::sync::mpsc::UnboundedSender<AgentEvent>,  // drained
) {
    let result = child
        .run_loop(
            vec![MessageContent::Text { content: prompt }],
            Vec::new(),   // single-turn: old_turns always empty
            sender,
        )
        .await;
    match result {
        Ok(turn) => registry.set_finished(agent_id, SubagentRunResult {
            usage: turn.usage,
            messages: turn.messages,
            finished: turn.finished,
        }),
        Err(_partial) => {
            // run_loop already emitted a MessageEnd(Error) stream event;
            // store the error so waiters wake and inspect/read can see it.
            registry.set_error(agent_id, "subagent run_loop failed".into());
        }
    }
}
```

`old_turns` is always `Vec::new()` (single-turn). The sender is drained by a companion `tokio::spawn` task so `run_loop`'s `send()` never blocks (mirroring `StartSubthreadTool`'s drained-channel pattern); the drainer is `.abort()`ed on completion.

## Tools

All tools implement `nekocode_types::tool::Tool` (`spec()` + `async call(params)`). Each holds a `SubagentContext` (cheap clone). The middleware registers all 6 in `before_generate`.

### `spawn_subagent`

**Params:**
```jsonc
{ "profile": "explorer", "prompt": "Find all call sites…" }  // both required
```
**Behavior:**
1. `catalog.get(profile)?` → `SubagentProfile`. Missing → `InvalidParameters("profile '<name>' not found")`.
2. **Nesting check (two gates, both must pass):**
   - **Gate A — profile gate:** the spawning parent's profile must have `allow_nested = true`. A parent spawned from a profile with `allow_nested = false` cannot spawn subagents at all. (The parent's `allow_nested` is carried on `ctx` from its own profile.) If false → `ExecutionError("parent profile does not allow nested subagents")`.
   - **Gate B — depth gate:** if `ctx.depth + 1 > ctx.max_depth` → `ExecutionError("max subagent nesting depth exceeded")`.
   The child's `SubagentMiddleware` is constructed at `depth + 1` with the child profile's `allow_nested`, so a depth-nested subagent's ability to spawn further is governed by its own profile's `allow_nested` plus the shared `max_depth` bound.
3. `registry.allocate_running()` → `agent_id`.
4. **Middleware intersection:** intersect `profile.middlewares` with `ctx.specs` names. A profile middleware ⊄ parent's enabled set → `ExecutionError("profile '<n>' requests middleware '<x>', not enabled by parent")`. For each retained spec: `factory.build(spec, agent_id, fresh_child_extensions)` → isolated instance.
5. Resolve child `working_directory` (profile override or inherit parent), build `system_prompt` prefix from profile, construct child `Agent` (with `SubagentMiddleware::new` at `depth+1` + the selected middlewares).
6. Spawn background `run_subagent` → `registry.set_running(agent_id, handle)`.
7. Return `{ "agent_id": <id>, "status": "running" }`.

### `inspect_subagent`

**Params:** `{ "agent_id": <u64> }` (required)
**Behavior:** `registry.contains` check → `ExecutionError` if absent. Returns:
```jsonc
{ "agent_id": <id>, "status": "running" | "finished" | "error", "error": "<msg>"? }
```
(`error` included only when state is `Error`.)

### `read_subagent`

**Params:**
```jsonc
{ "agent_id": <u64>, "text_only": true }  // agent_id required; text_only optional, default true
```
**Behavior:**
1. `registry.contains` check; state must be `is_ready()` → else `ExecutionError("agent <id> is not ready (state: <s>)")`.
2. Read `registry.result(agent_id)` (`RwLock<Option<SubagentRunResult>>`).
3. `text_only=true` (default): extract the **last assistant message's text blocks** from `result.messages` (concatenate `Text` blocks, drop reasoning/tool-call blocks) — the concise answer the parent model wants. `text_only=false`: return the full `messages` array.
4. Returns:
```jsonc
{ "agent_id": <id>, "status": "finished"|"error", "text": "<last assistant text>" }
// or, text_only=false and finished:
{ "agent_id": <id>, "status": "finished", "messages": [...] }
```

### `wait_any_subagent`

**Params:** `{ "agent_ids": [<u64>...], "timeout": <f64> }` (both required; `agent_ids` non-empty)
**Behavior:** Structurally identical to `WaitAnySubthreadTool` — ready-check loop + `Notify`+`timeout` select. Ownership = `registry.contains` (no DB). Returns:
- ready: `{ "status": "ready", "agent_id": <id>, "run_state": "<name>" }`
- timeout: `{ "status": "timeout", "pending": [<ids>] }`

### `wait_all_subagents`

**Params:** `{ "agent_ids": [<u64>?], "timeout": <f64> }` (`timeout` required; `agent_ids` optional, defaults to all **Running** subagents)
**Behavior:** Identical to `WaitAllSubthreadsTool` — loop until all ready or timeout. Returns:
- all ready: `{ "status": "ready", "results": [{ "agent_id", "run_state" }, ...] }`
- timeout: `{ "status": "timeout", "ready": [<ids>], "pending": [<ids>] }`

### `abort_subagent`

**Params:** `{ "agent_id": <u64> }` (required)
**Behavior:** `registry.abort(agent_id)` — aborts the `task_handle` if still running, removes the entry. Absent → `ExecutionError`. Returns `{ "agent_id": <id>, "aborted": true }`.

### Helpers (in `tool/mod.rs`)

`parse_agent_id` / `parse_agent_ids` / `parse_timeout` / `run_state_name` / `notify_any` — duplicated from subthread (per the no-cross-crate-sharing guideline; each crate owns its helpers).

## API Layer Integration

### `ApiSubagentMiddlewareFactory` (new file)

`crates/nekocode/src/api/thread/subagent_factory.rs`. A struct capturing `db / config / working_directory`, implementing `SubagentMiddlewareFactory` — the `build_middlewares` match arms, but building isolated child instances:

```rust
#[derive(Clone)]
pub struct ApiSubagentMiddlewareFactory {
    pub db: toasty::Db,
    pub config: Arc<RwLock<Config>>,
    pub working_directory: String,
}

#[async_trait]
impl SubagentMiddlewareFactory for ApiSubagentMiddlewareFactory {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
        match spec.name.as_str() {
            "shell" => Box::new(nekocode_shell::Shell::new(
                extensions.clone(),
                ShellConfig::from_value(&spec.config),
            )),
            "tool" => Box::new(nekocode_file::ToolMiddleware::new(
                FileConfig::from_value(&spec.config),
                self.db.clone(),
                subagent_id,        // ← child id, not parent thread_id
            )),
            "mcp" => Box::new(nekocode_mcp::McpMiddleware::new(
                McpConfig::from_value(&spec.config),
            )),
            "skills" => { /* resolve skills_dir from self.config, as build_middlewares does */ }
            _ => /* skip or panic on unknown — log + skip is safer */,
        }
    }
}
```

**Isolation fixes vs the parent build:**
- **shell:** `Shell::new(extensions.clone(), …)` inserts a fresh `shell_states` into the **child's** extensions → independent shell session (no shared parent session).
- **tool/file:** `ToolMiddleware::new(cfg, db, subagent_id)` — `set_title` writes target `subagent_id`. Since a subagent has no DB row, the `set_title` write matches zero rows → harmless no-op at runtime (Toasty update matches 0 rows), or errors if it must. Accepted: explore/review profiles typically don't need `set_title`, and profiles can exclude `tool` if it bothers. Recorded as acceptable for v1; a future iteration can add a "subagent mode" flag to the file middleware to no-op `set_title` when `thread_id` is synthetic.

### `build_middlewares`: new `"subagent"` arm

```rust
// crates/nekocode/src/api/thread/mod.rs — inside build_middlewares
"subagent" => {
    let cfg = nekocode_subagent::SubagentConfig::from_value(&i.config);
    // Build specs from the parent's enabled middleware rows, excluding
    // "subagent" itself (prevents recursive self-registration; nesting is
    // governed by depth+max_depth, not by omission here).
    let specs: Vec<MiddlewareSpec> = middleware_rows
        .iter()
        .filter(|r| r.enabled && r.name != "subagent")
        .map(|r| MiddlewareSpec { name: r.name.clone(), config: r.config.clone() })
        .collect();
    let factory = Arc::new(ApiSubagentMiddlewareFactory {
        db: ctx.db.clone(),
        config: ctx.config.clone(),
        working_directory: ctx.working_directory.clone(),
    });
    middlewares.push(Box::new(nekocode_subagent::SubagentMiddleware::new(
        specs,
        factory,
        ctx.provider.clone(),         // ← needs provider added to MiddlewareBuildContext
        ctx.config.clone(),
        ctx.extensions.clone(),
        cfg,                          // SubagentConfig: the root max_depth bound
        0,                            // depth = 0 for the top-level parent
        true,                         // allow_nested = true at the root: the top-level
                                       // thread is not itself a subagent, so it is always
                                       // permitted to spawn its first-level subagents.
                                       // The depth gate + each child's own profile
                                       // allow_nested bound further nesting.
    )));
}
```

### `MiddlewareBuildContext`: re-add `provider`

```rust
pub(crate) struct MiddlewareBuildContext {
    pub db: toasty::Db,
    pub config: Arc<RwLock<Config>>,
    pub extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    pub thread_id: u64,
    pub working_directory: String,
    pub subthread_activator: Arc<dyn ThreadActivator>,
    pub provider: Arc<dyn Provider>,   // ← re-added; only the subagent arm uses it
}
```

Both `MiddlewareBuildContext { … }` construction sites (`activate.rs:57` and `subthread_activator.rs`) add `provider: provider.clone()` (the provider is already built at `activate.rs:44` and `subthread_activator.rs:62`). The subthread arm doesn't use it — consistent with the existing asymmetry where subthread resolves its own model via its activator.

### Stale comment cleanup

`activate.rs:42` and `subthread_activator.rs:47` both have a stale comment referencing "the subagent middleware" in the provider-sharing context. Rewrite to reference "the subthread middleware" (and note the subagent arm now also uses the re-added provider field).

### Cascade cleanup (`delete.rs`)

Add `abort_subagent_tasks`, mirroring `abort_subthread_tasks`:

```rust
async fn abort_subagent_tasks(
    active_threads: &DashMap<u64, Arc<RwLock<Agent>>>,
    thread_id: u64,
) {
    let registry: Option<Arc<nekocode_subagent::SubagentRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent.extensions
                .get(nekocode_subagent::SUBAGENT_EXTENSION_KEY)
                .and_then(|b| b.downcast_ref::<Arc<nekocode_subagent::SubagentRegistry>>().cloned())
        } else { None };

    if let Some(registry) = registry {
        let _aborted = registry.abort_all_and_clear();
    }
}
```

Called in `delete_threads_cascade` alongside the existing `abort_subthread_tasks`. **No DB cascade** — subagent is purely in-memory, so aborting running tasks + dropping the registry is the complete cleanup.

## Boundary with Subthread

| Concern | Subthread | Subagent |
|---|---|---|
| Crate | `nekocode-subthread` | `nekocode-subagent` |
| Config struct | `SubthreadConfig` | `SubagentConfig` |
| Registry | `SubthreadRegistry` | `SubagentRegistry` |
| Run state enum | `SubthreadRunState` | `SubagentRunState` |
| Context struct | `SubthreadContext` | `SubagentContext` |
| Middleware | `SubthreadMiddleware` | `SubagentMiddleware` |
| Extension key | `"subthread"` | `"subagent"` |
| Tool prefix | `spawn_subthread`, etc. | `spawn_subagent`, etc. |
| ID source | DB `Thread.id` (u64) | `AtomicU64` (u64) |
| Store | DB (Toasty ORM) | `RwLock<Option<SubagentRunResult>>` |
| Multi-turn | yes (start → send → …) | **no** (single-turn) |
| Activator trait | `ThreadActivator` (DB thread activation) | `SubagentMiddlewareFactory` (instance building) |
| Provider | resolves own model via activator | inherits parent provider |
| Tool count | 10 | 6 |
| Extra deps | `nekocode-entities` | none beyond core+types+`toasty`(type only) |

## Error Handling

Errors flow through two existing mechanisms; no new mechanism is introduced.

**1. Tool errors** — a tool returns `Err(ToolError)`, which `run_loop` wraps as `ToolCallResultInner::Error` and appends to the conversation so the model can react. Subagent tool error cases:

| Tool | Error case | Error type |
|---|---|---|
| `spawn_subagent` | profile not found | `InvalidParameters("profile '<name>' not found")` |
| `spawn_subagent` | profile middleware ⊄ parent enabled | `ExecutionError("profile '<n>' requests middleware '<x>', not enabled by parent")` |
| `spawn_subagent` | nesting depth exceeded | `ExecutionError("max subagent nesting depth exceeded")` |
| `inspect/read/wait/abort` | `agent_id` not in registry | `ExecutionError("agent <id> not found")` |
| `read_subagent` | state not `is_ready()` | `ExecutionError("agent <id> is not ready (state: <s>)")` |
| `wait_*` | `agent_ids` empty / `timeout` ≤ 0 | `InvalidParameters(...)` |

**2. Run errors** — `run_loop` returns `Err(partial_turn)` and emits a `MessageEnd(Error)` stream event (verified in `new_agent.rs:228-247`). The runner translates this into `registry.set_error(agent_id, msg)`. The error is then visible to the model via `inspect_subagent` (state `Error` + carried message), `read_subagent` (state-aware, returns error status), and `wait_*` (`is_ready()` includes `Error`). So a failed subagent is "ready" to its waiters — the parent wakes and can see the failure via inspect/read.

**3. Abort cleanup** — `abort_subagent` and cascade `abort_all_and_clear` call `JoinHandle::abort()` at runtime; the aborted task drops its `run_loop` future (no resource leak — subagent has no DB rows to clean, only the in-memory `RwLock<Option<SubagentRunResult>>` which drops with the registry entry).

## Testing Strategy

Three tiers mirroring the codebase's existing tiers.

**Tier 1 — Registry unit tests** (`nekocode-subagent/src/registry.rs`, `#[cfg(test)] mod tests`)
- Structurally mirror subthread's `registry.rs` tests: `allocate_running` → `run_state` Running; `set_finished` stores `result` + wakes waiters; `set_error`; `abort` removes + aborts; `abort_all_and_clear` returns aborted ids; `all_agent_ids`; ID monotonicity.
- Pure in-memory, no provider, no DB.

**Tier 2 — Runner tests via `run_loop` reuse** (`nekocode-subagent/src/runner.rs`, `#[cfg(test)]`)
- A `MockSubagentFactory` (returns small in-memory middlewares like `EchoMiddleware`) + `MockProvider` from `nekocode-core::agent::test_mocks`.
- `run_subagent` success: `MockProvider::new(vec![text_msg("result")])` → runner captures `Turn` → `registry.set_finished` → `read_subagent` returns `"result"`.
- `run_subagent` error: `MockProvider::new(vec![])` (exhausted → error) → runner calls `set_error` → state `Error`, `is_ready()` true.
- Proves the `run_loop` reuse works: the runner has no logic beyond mapping `run_loop`'s `Ok(Turn)`/`Err(partial)` to registry state.

**Tier 3 — Tool integration tests** (`nekocode-subagent/tests/integration.rs`)
- Mirror subthread's `tests/integration.rs` shape: build `SubagentMiddleware` directly with a `MockSubagentFactory` (returns `EchoMiddleware`-based middlewares) — no `NoopActivator` needed here.
- Lifecycle: `spawn` → `wait_any` (ready) → `read` (last assistant text) → `inspect` → `abort`.
- Profile resolution: load a catalog from a TOML string, `spawn` selects by name; unknown profile → `InvalidParameters`.
- Middleware intersection: a profile requesting a middleware not in the parent's specs → `ExecutionError`.
- Nesting enforcement (both gates): (a) `spawn` from a parent whose profile has `allow_nested=false` → `ExecutionError("parent profile does not allow nested subagents")`; (b) with `allow_nested=true` and `depth` at `max_depth` → a deeper `spawn` hits the depth gate and errors.
- Wait timeout: `wait_any` with `timeout=0.01` against a never-completing subagent (`MockProvider` returning a pending future) → `{ status: "timeout", pending: [...] }`.
- **Not covered (manual smoke):** the full API-layer wiring (`build_middlewares` `"subagent"` arm, `MiddlewareBuildContext.provider`, cascade `abort_subagent_tasks`) — flagged for manual smoke, matching how subthread treats `start_subthread`/wait (covered by an API-layer smoke test, not here).

**Tier 4 — API layer (manual smoke, per subthread precedent)**
- `ApiSubagentMiddlewareFactory::build` produces isolated instances (assert shell middleware gets a fresh extensions map — via a test hook or extension-key observation).
- Cascade delete aborts running subagents.

### Out of scope (YAGNI)
- Hot-reloading profiles (explicitly deferred).
- Cross-parent isolation (registry is parent-keyed — isolated by construction, nothing to test).
- Concurrent-spawn races (`AtomicU64` allocator is monotonic; single `fetch_add` in `allocate_running` is atomic).

## Implementation Phases

Each phase compiles and tests independently.

### Phase 0 — `nekocode-core` addition (minimal)
- Add `MiddlewareSpec` data struct to `crates/nekocode-core/src/middleware.rs`. No behavior, no new deps. Compiles, existing core tests pass.

### Phase 1 — Crate skeleton + registry
- Create `crates/nekocode-subagent/Cargo.toml` (deps per above). Add `nekocode-subagent = { path = "crates/nekocode-subagent" }` to root `[workspace.dependencies]`.
- `src/lib.rs` (re-exports + `SUBAGENT_EXTENSION_KEY`), `src/config.rs` (`SubagentConfig`), `src/registry.rs` (`SubagentRegistry`, `SubagentState`, `SubagentRunState`, `SubagentRunResult`).
- Registry unit tests (Tier 1). Compiles, tests pass.

### Phase 2 — Profile + loader
- `src/profile.rs` (`SubagentProfile`, `ProfileCatalog`, `load` + merge, load-time validation).
- Tests: global+workspace merge (workspace replaces), missing file → empty, duplicate name → error. Compiles, tests pass.

### Phase 3 — Factory trait + runner
- `src/factory.rs` (`SubagentMiddlewareFactory` trait).
- `src/runner.rs` (`run_subagent`: build child `Agent`, call `run_loop`, capture `Turn`, update registry; drained-sender pattern).
- Runner tests with `MockSubagentFactory` + `MockProvider` (Tier 2). Compiles, tests pass.

### Phase 4 — Tools + middleware
- `src/tool/` (6 tools + helpers in `mod.rs`).
- `src/middleware.rs` (`SubagentMiddleware`: `new`, `before_generate` registers 6 tools + publishes registry).
- `tests/integration.rs` (Tier 3). Compiles, tests pass.

### Phase 5 — API crate integration
- `crates/nekocode/src/api/thread/subagent_factory.rs` (`ApiSubagentMiddlewareFactory`).
- `crates/nekocode/src/api/thread/mod.rs`: add `"subagent"` arm to `build_middlewares`; add `provider: Arc<dyn Provider>` to `MiddlewareBuildContext`.
- `crates/nekocode/src/api/thread/activate.rs` + `subthread_activator.rs`: pass `provider: provider.clone()` into `MiddlewareBuildContext`; rewrite stale comments.
- `crates/nekocode/src/api/thread/delete.rs`: add `abort_subagent_tasks`, call in `delete_threads_cascade`.
- `crates/nekocode/Cargo.toml`: `nekocode-subagent.workspace = true`.
- Compiles; `cargo test -p nekocode` (existing tests must still pass — the `MiddlewareBuildContext` field addition is the only change touching existing code, and both construction sites are updated).

## File-level Change Summary

**New files:**
```
crates/nekocode-subagent/Cargo.toml
crates/nekocode-subagent/src/lib.rs
crates/nekocode-subagent/src/config.rs
crates/nekocode-subagent/src/profile.rs
crates/nekocode-subagent/src/registry.rs
crates/nekocode-subagent/src/factory.rs
crates/nekocode-subagent/src/runner.rs
crates/nekocode-subagent/src/middleware.rs
crates/nekocode-subagent/src/tool/mod.rs
crates/nekocode-subagent/src/tool/spawn_subagent.rs
crates/nekocode-subagent/src/tool/inspect_subagent.rs
crates/nekocode-subagent/src/tool/read_subagent.rs
crates/nekocode-subagent/src/tool/wait_any_subagent.rs
crates/nekocode-subagent/src/tool/wait_all_subagents.rs
crates/nekocode-subagent/src/tool/abort_subagent.rs
crates/nekocode-subagent/tests/integration.rs
crates/nekocode/src/api/thread/subagent_factory.rs
```

**Modified files:**
```
Cargo.toml                                              # +workspace dep
crates/nekocode/Cargo.toml                              # +nekocode-subagent dep
crates/nekocode-core/src/middleware.rs                  # +MiddlewareSpec
crates/nekocode/src/api/thread/mod.rs                   # +subagent arm, +provider field
crates/nekocode/src/api/thread/activate.rs              # +provider.clone(), comment fix
crates/nekocode/src/api/thread/subthread_activator.rs   # +provider.clone(), comment fix
crates/nekocode/src/api/thread/delete.rs                # +abort_subagent_tasks
```

## Definition of Done

Each phase: `cargo build -p <crate>` succeeds + `cargo test -p <crate>` passes. Final: `cargo build` (whole workspace) + `cargo test` (workspace) pass, with nekocode's existing tests still green (the `MiddlewareBuildContext` field addition is the only change touching existing code, and both construction sites are updated).
