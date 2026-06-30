# Subagent Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Re-introduce subagent as a lighter, in-memory, single-turn sibling to `nekocode-subthread` that reuses `Agent::run_loop` directly and is driven by named profiles from `agents.toml`.

**Architecture:** A new leaf crate `nekocode-subagent` (depends only on `nekocode-core` + `nekocode-types`) holds a per-parent `SubagentRegistry` in `Agent.extensions["subagent"]`, exposes 6 tools via a `SubagentMiddleware`, and runs each subagent by calling the existing `Agent::run_loop` with empty history and capturing the returned `Turn`. Middleware instances are built in isolation at spawn time through a `SubagentMiddlewareFactory` trait defined in the subagent crate and implemented by the API crate (the only layer that sees `shell/file/mcp/skills`).

**Tech Stack:** Rust 2024 edition, tokio (async runtime + `Notify`/`mpsc`/`JoinHandle`), `dashmap` (per-parent registry), `toasty` (type only — never queried), serde/TOML (profiles), `async-trait`.

**Reference spec:** `docs/superpowers/specs/2026-06-30-subagent-redesign.md`

---

## File Structure

**New crate `crates/nekocode-subagent/`:**
- `Cargo.toml` — manifest (deps: core, types, toasty, async-trait, anyhow, serde, serde_json, tokio, tokio-util, tracing, dashmap, jiff, futures-util).
- `src/lib.rs` — re-exports + `SUBAGENT_EXTENSION_KEY`.
- `src/config.rs` — `SubagentConfig` (root `max_depth` bound).
- `src/profile.rs` — `SubagentProfile`, `ProfileCatalog` (load + global/workspace merge + validation).
- `src/registry.rs` — `SubagentRegistry`, `SubagentState`, `SubagentRunState`, `SubagentRunResult`.
- `src/factory.rs` — `SubagentMiddlewareFactory` trait.
- `src/runner.rs` — `run_subagent` (build child `Agent`, call `run_loop`, capture `Turn`, update registry; drained-sender pattern).
- `src/middleware.rs` — `SubagentMiddleware` (`new`, `before_generate` registers 6 tools + publishes registry).
- `src/tool/mod.rs` — parse helpers (`parse_agent_id`, `parse_agent_ids`, `parse_timeout`), `run_state_name`, `notify_any`.
- `src/tool/spawn_subagent.rs`, `inspect_subagent.rs`, `read_subagent.rs`, `wait_any_subagent.rs`, `wait_all_subagents.rs`, `abort_subagent.rs` — the 6 tools.
- `tests/integration.rs` — Tier 3 tool integration tests.

**New file in API crate:**
- `crates/nekocode/src/api/thread/subagent_factory.rs` — `ApiSubagentMiddlewareFactory` (impl of the factory trait).

**Modified files:**
- `Cargo.toml` (root) — add `nekocode-subagent` workspace dep.
- `crates/nekocode-core/src/middleware.rs` — add `MiddlewareSpec` struct.
- `crates/nekocode/Cargo.toml` — add `nekocode-subagent` dep.
- `crates/nekocode/src/api/thread/mod.rs` — add `provider` field to `MiddlewareBuildContext`; add `"subagent"` arm in `build_middlewares`.
- `crates/nekocode/src/api/thread/activate.rs` — pass `provider.clone()` into `MiddlewareBuildContext`; fix stale comment.
- `crates/nekocode/src/api/thread/subthread_activator.rs` — pass `provider.clone()` into `MiddlewareBuildContext`; fix stale comment.
- `crates/nekocode/src/api/thread/delete.rs` — add `abort_subagent_tasks`, call in `delete_threads_cascade`.

---

## Task 1: Add `MiddlewareSpec` to `nekocode-core`

**Files:**
- Modify: `crates/nekocode-core/src/middleware.rs`

- [ ] **Step 1: Add the `MiddlewareSpec` struct**

Open `crates/nekocode-core/src/middleware.rs`. After the `AgentControlFlow` enum definition (and before the `Middleware` trait), add:

```rust
/// Name + raw config — enough for the API layer to rebuild an isolated
/// middleware instance for a subagent. Defined here (in nekocode-core) so
/// `nekocode-subagent`, which depends only on core + types, can refer to it
/// by name without seeing the individual middleware crates.
#[derive(Debug, Clone)]
pub struct MiddlewareSpec {
    pub name: String,
    pub config: serde_json::Value,
}
```

- [ ] **Step 2: Verify the crate still builds and tests pass**

Run: `cargo build -p nekocode-core && cargo test -p nekocode-core`
Expected: build succeeds, all existing tests pass (no behavior change — pure data struct addition).

- [ ] **Step 3: Commit**

```bash
git add crates/nekocode-core/src/middleware.rs
git commit -m "feat(core): add MiddlewareSpec data struct for subagent instance building"
```

---

## Task 2: Create the `nekocode-subagent` crate skeleton + `SubagentConfig`

**Files:**
- Create: `crates/nekocode-subagent/Cargo.toml`
- Create: `crates/nekocode-subagent/src/lib.rs`
- Create: `crates/nekocode-subagent/src/config.rs`
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Add the workspace dependency to the root manifest**

In `Cargo.toml` (root), under `[workspace.dependencies]`, after the `nekocode-subthread` line, add:

```toml
nekocode-subagent = { path = "crates/nekocode-subagent" }
```

- [ ] **Step 2: Create the crate manifest**

Create `crates/nekocode-subagent/Cargo.toml`:

```toml
[package]
name = "nekocode-subagent"
version = "0.1.0"
edition = "2024"

[dependencies]
nekocode-core.workspace = true
nekocode-types.workspace = true
toasty.workspace = true
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

- [ ] **Step 3: Create `src/lib.rs`**

Create `crates/nekocode-subagent/src/lib.rs`:

```rust
//! Lightweight, in-memory, single-turn subagent machinery.
//!
//! A subagent is spawned with a prompt, runs `Agent::run_loop` once, stores
//! the captured `Turn` in memory, and is done. It is purely in-memory (no DB
//! rows, no `ThreadActivator`), lighter than `nekocode-subthread` (which is
//! DB-persisted and multi-turn). Per-parent state lives in
//! `Agent.extensions["subagent"]` as an `Arc<SubagentRegistry>`.
//!
//! Middleware instances for a child are built in isolation at spawn time via
//! the `SubagentMiddlewareFactory` trait (implemented by the API crate, the
//! only layer that can see the shell/file/mcp/skills constructors).

pub mod config;
pub mod factory;
pub mod middleware;
pub mod profile;
pub mod registry;
pub mod runner;
pub mod tool;

pub use config::SubagentConfig;
pub use factory::SubagentMiddlewareFactory;
pub use middleware::{SubagentContext, SubagentMiddleware, SUBAGENT_EXTENSION_KEY};
pub use profile::{ProfileCatalog, SubagentProfile};
pub use registry::{SubagentRegistry, SubagentRunResult, SubagentRunState, SubagentState};

/// Extension key under which a parent agent publishes its
/// `Arc<SubagentRegistry>` into `Agent.extensions`. Per-parent (NOT a
/// process-global singleton).
pub const SUBAGENT_EXTENSION_KEY: &str = "subagent";
```

Note: `SUBAGENT_EXTENSION_KEY` is defined as a `pub const` at the crate root and re-exported via `pub use middleware::SUBAGENT_EXTENSION_KEY` — but to avoid a duplicate-definition conflict, define it **only** at the crate root and have `middleware.rs` reference it as `crate::SUBAGENT_EXTENSION_KEY`. Remove the `SUBAGENT_EXTENSION_KEY` from the `pub use middleware::{…}` line. The corrected `lib.rs` re-export line is:

```rust
pub use middleware::{SubagentContext, SubagentMiddleware};
```

And keep `pub const SUBAGENT_EXTENSION_KEY: &str = "subagent";` at the crate root (already shown above).

- [ ] **Step 4: Create `src/config.rs`**

Create `crates/nekocode-subagent/src/config.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Per-parent middleware config for the subagent middleware. Stored as a
/// `Middleware` row's `config` JSON in the parent thread's DB row.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SubagentConfig {
    /// Maximum *nesting* depth — how many levels of subagents may spawn
    /// further subagents. The top-level thread spawns level-1 subagents
    /// (depth 0 → child at depth 1); `max_depth` bounds how deep those may
    /// nest. `max_depth = 0` (the default) means level-1 subagents cannot
    /// themselves spawn (depth 1 + 1 > 0). `max_depth = 1` allows one level
    /// of nesting. Propagated unchanged down the chain as the single
    /// tree-wide bound.
    #[serde(default)]
    pub max_depth: u32,
}

impl SubagentConfig {
    /// Deserialize from a `serde_json::Value` (the middleware row's `config`
    /// column), falling back to the default on any error.
    pub fn from_value(v: &serde_json::Value) -> Self {
        serde_json::from_value(v.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_max_depth_is_zero() {
        assert_eq!(SubagentConfig::default().max_depth, 0);
    }

    #[test]
    fn from_value_parses_max_depth() {
        let v = serde_json::json!({ "max_depth": 2 });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 2);
    }

    #[test]
    fn from_value_falls_back_on_invalid() {
        let v = serde_json::json!({ "max_depth": "not a number" });
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }

    #[test]
    fn from_value_omits_missing_field() {
        let v = serde_json::json!({});
        assert_eq!(SubagentConfig::from_value(&v).max_depth, 0);
    }
}
```

- [ ] **Step 5: Create stub modules so the crate compiles**

For the crate to compile, the modules referenced in `lib.rs` must exist. Create minimal stubs that will be filled in later tasks:

`crates/nekocode-subagent/src/factory.rs`:
```rust
use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};

/// Builds an isolated child middleware instance from a spec. Implemented by
/// the API crate (the only layer that can see the shell/file/mcp/skills
/// constructors). `subagent_id` is the AtomicU64-allocated ID (not a DB id);
/// `extensions` is the child's fresh DashMap so middleware like shell gets
/// its own session map.
#[async_trait::async_trait]
pub trait SubagentMiddlewareFactory: Send + Sync {
    fn build(
        &self,
        spec: MiddlewareSpec,
        subagent_id: u64,
        extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
    ) -> Box<dyn Middleware>;
}
```

`crates/nekocode-subagent/src/profile.rs`:
```rust
use serde::{Deserialize, Serialize};

/// A named subagent profile loaded from `agents.toml`. A *filter* over the
/// parent's enabled middlewares plus overrides for system-prompt/workdir/
/// nesting. No `model` field — subagent always inherits the parent provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProfile {
    pub name: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub allow_nested: bool,
    #[serde(default)]
    pub middlewares: Vec<String>,
}

/// Catalog of profiles keyed by name, loaded from global + workspace
/// `agents.toml` files. Filled in by Task 4.
pub struct ProfileCatalog {
    pub profiles: std::collections::HashMap<String, SubagentProfile>,
}
```

`crates/nekocode-subagent/src/registry.rs`:
```rust
// Filled in by Task 3.
```

`crates/nekocode-subagent/src/runner.rs`:
```rust
// Filled in by Task 5.
```

`crates/nekocode-subagent/src/middleware.rs`:
```rust
// Filled in by Task 6.
```

`crates/nekocode-subagent/src/tool/mod.rs`:
```rust
// Filled in by Tasks 6-7.
```

- [ ] **Step 6: Verify the crate builds and config tests pass**

Run: `cargo build -p nekocode-subagent && cargo test -p nekocode-subagent`
Expected: build succeeds; the 4 `config.rs` tests pass.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml crates/nekocode-subagent/
git commit -m "feat(subagent): scaffold nekocode-subagent crate + SubagentConfig"
```

---

## Task 3: Implement the `SubagentRegistry` + state types

**Files:**
- Modify: `crates/nekocode-subagent/src/registry.rs`
- Test: inline `#[cfg(test)] mod tests` in the same file

- [ ] **Step 1: Write the failing tests**

Replace the contents of `crates/nekocode-subagent/src/registry.rs` with the test module first (TDD). The tests define the expected API surface:

```rust
use std::sync::atomic::AtomicU64;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_types::generate::{Message, Usage};
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;

#[derive(Debug, Clone)]
pub enum SubagentRunState {
    Idle,
    Running,
    Finished,
    Error(String),
}

impl SubagentRunState {
    pub fn is_ready(&self) -> bool {
        matches!(self, SubagentRunState::Finished | SubagentRunState::Error(_))
    }
}

fn run_state_name(s: &SubagentRunState) -> &'static str {
    match s {
        SubagentRunState::Idle => "idle",
        SubagentRunState::Running => "running",
        SubagentRunState::Finished => "finished",
        SubagentRunState::Error(_) => "error",
    }
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubagentRunResult {
    pub usage: Usage,
    pub messages: Vec<Message>,
    pub finished: bool,
}

#[derive(Debug)]
pub struct SubagentState {
    pub agent_id: u64,
    pub run_state: SubagentRunState,
    pub task_handle: Option<JoinHandle<()>>,
    pub notify: Arc<Notify>,
    pub result: RwLock<Option<SubagentRunResult>>,
}

impl SubagentState {
    pub fn new(agent_id: u64) -> Self {
        Self {
            agent_id,
            run_state: SubagentRunState::Running,
            task_handle: None,
            notify: Arc::new(Notify::new()),
            result: RwLock::new(None),
        }
    }
}

#[derive(Debug, Default)]
pub struct SubagentRegistry {
    states: DashMap<u64, SubagentState>,
    next_id: AtomicU64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allocate_running_returns_monotonic_ids_and_running_state() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        assert!(id2 > id1, "ids must be monotonic");
        assert!(matches!(reg.run_state(id1), SubagentRunState::Running));
    }

    #[test]
    fn run_state_absent_defaults_to_idle() {
        let reg = SubagentRegistry::new();
        assert!(matches!(reg.run_state(999), SubagentRunState::Idle));
    }

    #[test]
    fn set_finished_stores_result_and_wakes() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        let result = SubagentRunResult {
            usage: Usage::default(),
            messages: Vec::new(),
            finished: true,
        };
        reg.set_finished(id, result.clone());
        assert!(matches!(reg.run_state(id), SubagentRunState::Finished));
        let got = reg.result(id);
        assert!(got.is_some(), "result should be stored");
    }

    #[test]
    fn set_error_marks_ready() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        reg.set_error(id, "boom".into());
        assert!(matches!(reg.run_state(id), SubagentRunState::Error(_)));
        assert!(reg.run_state(id).is_ready());
    }

    #[test]
    fn abort_removes_entry() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        reg.abort(id);
        assert!(!reg.contains(id));
        assert!(matches!(reg.run_state(id), SubagentRunState::Idle));
    }

    #[test]
    fn abort_all_and_clear_empties_and_returns_ids() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        let aborted = reg.abort_all_and_clear();
        assert_eq!(aborted.len(), 2, "both running entries aborted");
        assert!(!reg.contains(id1));
        assert!(!reg.contains(id2));
    }

    #[test]
    fn all_agent_ids_lists_tracked() {
        let reg = SubagentRegistry::new();
        let id1 = reg.allocate_running();
        let id2 = reg.allocate_running();
        let mut ids = reg.all_agent_ids();
        ids.sort();
        assert_eq!(ids, vec![id1, id2]);
    }

    #[test]
    fn notify_returns_handle_for_tracked() {
        let reg = SubagentRegistry::new();
        let id = reg.allocate_running();
        assert!(reg.notify(id).is_some());
        assert!(reg.notify(999).is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nekocode-subagent registry`
Expected: FAIL — the `impl SubagentRegistry` block with `new`/`allocate_running`/`run_state`/`set_finished`/`set_error`/`abort`/`abort_all_and_clear`/`contains`/`all_agent_ids`/`notify`/`result` methods does not exist yet (compile errors: no such methods).

- [ ] **Step 3: Implement the registry methods**

Append the `impl SubagentRegistry` block to `crates/nekocode-subagent/src/registry.rs` (after the struct definition, before `#[cfg(test)]`):

```rust
impl SubagentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new monotonic agent_id and insert a Running entry.
    /// Returns the allocated id. Called by spawn_subagent.
    pub fn allocate_running(&self) -> u64 {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        self.states.insert(id, SubagentState::new(id));
        id
    }

    /// Snapshot the run state of a subagent, defaulting to Idle if absent.
    pub fn run_state(&self, agent_id: u64) -> SubagentRunState {
        self.states
            .get(&agent_id)
            .map(|s| s.run_state.clone())
            .unwrap_or(SubagentRunState::Idle)
    }

    /// Mark a subagent as Running and store its task handle.
    pub fn set_running(&self, agent_id: u64, handle: JoinHandle<()>) {
        if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Running;
            s.task_handle = Some(handle);
        }
    }

    /// Mark a subagent as Finished, store its result, and wake waiters.
    pub fn set_finished(&self, agent_id: u64, result: SubagentRunResult) {
        if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Finished;
            s.task_handle = None;
            // Write the result outside the DashMap guard to avoid holding
            // the guard across an await. Clone is cheap (Arc-ish fields).
            let result_slot = s.result.clone();
            drop(s);
            // result is RwLock<Option<..>>; blocking write is fine — no await.
            *result_slot.blocking_write() = Some(result);
            // Re-acquire to notify.
            if let Some(s) = self.states.get(&agent_id) {
                s.notify.notify_waiters();
            }
        }
    }

    /// Mark a subagent as Error and wake waiters.
    pub fn set_error(&self, agent_id: u64, msg: String) {
        if let Some(mut s) = self.states.get_mut(&agent_id) {
            s.run_state = SubagentRunState::Error(msg);
            s.task_handle = None;
            s.notify.notify_waiters();
        }
    }

    /// Abort a subagent's background task (if running) and remove its entry.
    pub fn abort(&self, agent_id: u64) {
        if let Some((_, state)) = self.states.remove(&agent_id) {
            if let Some(handle) = state.task_handle {
                handle.abort();
            }
        }
    }

    /// Abort every running subagent's background task and clear the registry.
    /// Returns the ids that had in-flight tasks aborted. Used by cascade
    /// delete.
    pub fn abort_all_and_clear(&self) -> Vec<u64> {
        let mut aborted = Vec::new();
        for entry in self.states.iter() {
            if entry.task_handle.is_some() {
                aborted.push(entry.agent_id);
            }
        }
        // Abort handles, then clear.
        for (_, state) in self.states.iter() {
            if let Some(handle) = &state.task_handle {
                handle.abort();
            }
        }
        self.states.clear();
        aborted
    }

    /// Whether the registry currently tracks `agent_id`.
    pub fn contains(&self, agent_id: u64) -> bool {
        self.states.contains_key(&agent_id)
    }

    /// Clone of the Notify for a subagent, so waiters can subscribe without
    /// holding a DashMap guard. Returns None if not tracked.
    pub fn notify(&self, agent_id: u64) -> Option<Arc<Notify>> {
        self.states.get(&agent_id).map(|s| s.notify.clone())
    }

    /// All agent ids tracked by this (per-parent) registry.
    pub fn all_agent_ids(&self) -> Vec<u64> {
        self.states.iter().map(|s| s.agent_id).collect()
    }

    /// Snapshot of a finished subagent's result (clone of the stored
    /// SubagentRunResult). Returns None if absent or not yet finished.
    pub fn result(&self, agent_id: u64) -> Option<SubagentRunResult> {
        let s = self.states.get(&agent_id)?;
        // blocking_read avoids holding the DashMap guard across an await.
        s.result.blocking_read().clone()
    }
}

impl SubagentRunState {
    /// Lowercase name for JSON serialization in tool results.
    pub fn name(&self) -> &'static str {
        run_state_name(self)
    }
}
```

Note on `set_finished`: the `result` field is a `RwLock`, and holding a `DashMap` write guard (`get_mut`) across `.write().await` would risk a deadlock with any concurrent `get` on the same shard. The implementation uses `blocking_write()` (safe because the registry methods are not async and there is no `await` in the path), cloning the `RwLock` Arc out of the guard before writing. This mirrors the lock-discipline care in `nekocode-subthread`'s `SubthreadRegistry`.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nekocode-subagent registry`
Expected: all 8 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nekocode-subagent/src/registry.rs
git commit -m "feat(subagent): implement SubagentRegistry with AtomicU64 id allocation"
```

---

## Task 4: Implement `ProfileCatalog` (load + merge + validation)

**Files:**
- Modify: `crates/nekocode-subagent/src/profile.rs`
- Test: inline `#[cfg(test)] mod tests`

- [ ] **Step 1: Write the failing tests**

Replace `crates/nekocode-subagent/src/profile.rs` with the full implementation including tests. Tests first define the expected behavior:

```rust
use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentProfile {
    pub name: String,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub working_directory: Option<String>,
    #[serde(default)]
    pub allow_nested: bool,
    #[serde(default)]
    pub middlewares: Vec<String>,
}

/// Catalog of profiles keyed by name, loaded from global + workspace
/// `agents.toml` files (workspace wholly replaces same-named global entries).
pub struct ProfileCatalog {
    pub profiles: HashMap<String, SubagentProfile>,
}

impl ProfileCatalog {
    pub fn empty() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Load global then workspace, merging by name (workspace replaces).
    pub fn load(global_path: &Path, workspace_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        todo!("Task 4 Step 3")
    }

    pub fn get(&self, name: &str) -> Result<&SubagentProfile, anyhow::Error> {
        self.profiles
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("profile '{}' not found", name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_tmp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nekocode_subagent_profile_{}_{}_{}",
            std::process::id(),
            name,
            std::sync::atomic::AtomicU64::new(0).fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("agents.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn load_missing_global_returns_empty_catalog() {
        let cat = ProfileCatalog::load(
            Path::new("/nonexistent/agents.toml"),
            None,
        )
        .expect("missing global is ok");
        assert!(cat.profiles.is_empty());
    }

    #[test]
    fn load_global_only() {
        let g = write_tmp(
            "global_only",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell", "tool"]
"#,
        );
        let cat = ProfileCatalog::load(&g, None).unwrap();
        let p = cat.get("explorer").unwrap();
        assert_eq!(p.middlewares, vec!["shell".to_string(), "tool".to_string()]);
        assert!(!p.allow_nested);
    }

    #[test]
    fn workspace_wholly_replaces_same_named_global() {
        let g = write_tmp(
            "replace_global",
            r#"
[[agents]]
name = "explorer"
system_prompt = "global prompt"
middlewares = ["shell", "tool"]
"#,
        );
        let w = write_tmp(
            "replace_ws",
            r#"
[[agents]]
name = "explorer"
middlewares = ["tool"]
allow_nested = true
"#,
        );
        let cat = ProfileCatalog::load(&g, Some(&w)).unwrap();
        let p = cat.get("explorer").unwrap();
        // Replaced wholesale: global system_prompt gone, workspace fields used.
        assert_eq!(p.system_prompt, None);
        assert_eq!(p.middlewares, vec!["tool".to_string()]);
        assert!(p.allow_nested);
    }

    #[test]
    fn workspace_adds_distinct_names() {
        let g = write_tmp(
            "add_global",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell"]
"#,
        );
        let w = write_tmp(
            "add_ws",
            r#"
[[agents]]
name = "reviewer"
middlewares = ["tool"]
"#,
        );
        let cat = ProfileCatalog::load(&g, Some(&w)).unwrap();
        assert!(cat.get("explorer").is_ok());
        assert!(cat.get("reviewer").is_ok());
    }

    #[test]
    fn duplicate_name_in_single_file_is_error() {
        let g = write_tmp(
            "dup",
            r#"
[[agents]]
name = "explorer"
middlewares = ["shell"]

[[agents]]
name = "explorer"
middlewares = ["tool"]
"#,
        );
        let err = ProfileCatalog::load(&g, None).expect_err("duplicate should error");
        assert!(err.to_string().contains("duplicate"));
    }

    #[test]
    fn empty_name_is_error() {
        let g = write_tmp(
            "emptyname",
            r#"
[[agents]]
name = ""
middlewares = ["shell"]
"#,
        );
        let err = ProfileCatalog::load(&g, None).expect_err("empty name should error");
        assert!(err.to_string().contains("empty"));
    }

    #[test]
    fn get_unknown_returns_error() {
        let cat = ProfileCatalog::empty();
        let err = cat.get("nope").expect_err("unknown profile");
        assert!(err.to_string().contains("not found"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nekocode-subagent profile`
Expected: FAIL — `load` is a `todo!()` and panics; the non-load tests (`get_unknown`) pass already. The load-dependent tests panic with "not yet implemented".

- [ ] **Step 3: Implement `load`**

Replace the `todo!()` body of `ProfileCatalog::load` with:

```rust
    pub fn load(global_path: &Path, workspace_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let mut profiles: HashMap<String, SubagentProfile> = HashMap::new();
        // Global first (missing is OK → empty).
        if let Ok(content) = std::fs::read_to_string(global_path) {
            let parsed: Vec<SubagentProfile> = toml::from_str(&content)?;
            for p in parsed {
                if p.name.is_empty() {
                    anyhow::bail!("profile with empty name in {}", global_path.display());
                }
                if profiles.insert(p.name.clone(), p).is_some() {
                    anyhow::bail!("duplicate profile name in {}: see above", global_path.display());
                }
            }
        }
        // Workspace second (missing is OK → skip). Replaces same-named entries.
        if let Some(ws) = workspace_path {
            if let Ok(content) = std::fs::read_to_string(ws) {
                let parsed: Vec<SubagentProfile> = toml::from_str(&content)?;
                for p in parsed {
                    if p.name.is_empty() {
                        anyhow::bail!("profile with empty name in {}", ws.display());
                    }
                    // Workspace wholly replaces (whole-object, not field merge).
                    // A duplicate within the workspace file itself is an error.
                    if profiles.contains_key(&p.name) {
                        // Replacing a global entry is allowed; but a duplicate
                        // *within this workspace file* would surface as a second
                        // iter with the same name. Detect by checking a local set.
                    }
                    profiles.insert(p.name.clone(), p);
                }
                // Detect intra-workspace duplicates by re-parsing keys.
                let names: Vec<&str> = parsed.iter().map(|p| p.name.as_str()).collect();
                let mut seen = std::collections::HashSet::new();
                for n in names {
                    if !seen.insert(n) {
                        anyhow::bail!("duplicate profile name '{}' in {}", n, ws.display());
                    }
                }
            }
        }
        Ok(Self { profiles })
    }
```

Note: `toml` is a transitive dependency via `nekocode-types`? No — add it explicitly. **Correction:** the `toml` crate is NOT in this crate's deps. Add `toml.workspace = true` to `[dependencies]` in `crates/nekocode-subagent/Cargo.toml` (the workspace already declares `toml = "1.1"`). Do this before running the tests.

Edit `crates/nekocode-subagent/Cargo.toml` `[dependencies]` to add the line:
```toml
toml.workspace = true
```

Also fix the duplicate-detection logic: the intra-workspace duplicate check is done after the insert loop, which is too late. Move the `seen` set into the insert loop:

```rust
        if let Some(ws) = workspace_path {
            if let Ok(content) = std::fs::read_to_string(ws) {
                let parsed: Vec<SubagentProfile> = toml::from_str(&content)?;
                let mut seen = std::collections::HashSet::new();
                for p in parsed {
                    if p.name.is_empty() {
                        anyhow::bail!("profile with empty name in {}", ws.display());
                    }
                    if !seen.insert(p.name.clone()) {
                        anyhow::bail!("duplicate profile name '{}' in {}", p.name, ws.display());
                    }
                    // Workspace wholly replaces any same-named global entry.
                    profiles.insert(p.name.clone(), p);
                }
            }
        }
```

Apply the same `seen`-set pattern to the global block for intra-global duplicate detection. Final `load`:

```rust
    pub fn load(global_path: &Path, workspace_path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let mut profiles: HashMap<String, SubagentProfile> = HashMap::new();
        // Global first (missing file is OK → empty catalog).
        if let Ok(content) = std::fs::read_to_string(global_path) {
            let parsed: Vec<SubagentProfile> = toml::from_str(&content)?;
            let mut seen = std::collections::HashSet::new();
            for p in parsed {
                if p.name.is_empty() {
                    anyhow::bail!("profile with empty name in {}", global_path.display());
                }
                if !seen.insert(p.name.clone()) {
                    anyhow::bail!("duplicate profile name '{}' in {}", p.name, global_path.display());
                }
                profiles.insert(p.name.clone(), p);
            }
        }
        // Workspace second (missing is OK → skip). Wholly replaces same-named
        // global entries; intra-workspace duplicates are an error.
        if let Some(ws) = workspace_path {
            if let Ok(content) = std::fs::read_to_string(ws) {
                let parsed: Vec<SubagentProfile> = toml::from_str(&content)?;
                let mut seen = std::collections::HashSet::new();
                for p in parsed {
                    if p.name.is_empty() {
                        anyhow::bail!("profile with empty name in {}", ws.display());
                    }
                    if !seen.insert(p.name.clone()) {
                        anyhow::bail!("duplicate profile name '{}' in {}", p.name, ws.display());
                    }
                    profiles.insert(p.name.clone(), p);
                }
            }
        }
        Ok(Self { profiles })
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nekocode-subagent profile`
Expected: all 7 tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nekocode-subagent/src/profile.rs crates/nekocode-subagent/Cargo.toml
git commit -m "feat(subagent): implement ProfileCatalog with global+workspace merge"
```

---

## Task 5: Implement the runner (`run_subagent`)

**Files:**
- Modify: `crates/nekocode-subagent/src/runner.rs`
- Test: inline `#[cfg(test)] mod tests` (with a local mock provider)

- [ ] **Step 1: Write the failing tests with a local mock provider**

The `nekocode-core` test mocks (`MockProvider`, `EchoMiddleware`) are `pub(crate)` — not accessible from this crate. Per the no-cross-crate-sharing guideline, define a local mock in the test module.

Replace `crates/nekocode-subagent/src/runner.rs` with:

```rust
use std::sync::Arc;

use nekocode_core::agent::{Agent, AgentEvent, AgentEventType};
use nekocode_types::generate::{MessageContent, StreamEvent};
use tokio::sync::mpsc::UnboundedSender;

use crate::registry::{SubagentRegistry, SubagentRunResult};

/// Run a child agent's `run_loop` once with the given prompt and capture the
/// resulting `Turn` into the registry. The `sender` is provided by the
/// caller (the spawn tool sets up a drained channel so `run_loop`'s `send()`
/// never blocks). `old_turns` is always empty (single-turn).
pub async fn run_subagent(
    agent_id: u64,
    child: Agent,
    prompt: String,
    registry: Arc<SubagentRegistry>,
    sender: UnboundedSender<AgentEvent>,
) {
    let result = child
        .run_loop(
            vec![MessageContent::Text { content: prompt }],
            Vec::new(),
            sender,
        )
        .await;
    match result {
        Ok(turn) => registry.set_finished(
            agent_id,
            SubagentRunResult {
                usage: turn.usage,
                messages: turn.messages,
                finished: turn.finished,
            },
        ),
        Err(_partial) => {
            // run_loop already emitted a MessageEnd(Error) stream event;
            // record the error so waiters wake and inspect/read can see it.
            registry.set_error(agent_id, "subagent run_loop failed".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
    use nekocode_types::generate::{
        AssistantContentBlock, AssistantMessage, StopReason, Usage,
    };
    use tokio::sync::mpsc;

    /// A local mock provider returning a scripted sequence of assistant
    /// messages (FIFO). Exhausting the list yields an error — mirrors
    /// nekocode-core's MockProvider shape without crossing crate visibility.
    struct MockProvider {
        responses: Mutex<Vec<AssistantMessage>>,
    }

    impl MockProvider {
        fn new(responses: Vec<AssistantMessage>) -> Self {
            let mut r = responses;
            r.reverse(); // pop() is LIFO; reverse once for FIFO
            Self { responses: Mutex::new(r) }
        }
    }

    fn text_msg(s: &str) -> AssistantMessage {
        AssistantMessage {
            blocks: vec![AssistantContentBlock::Text {
                content: s.to_string(),
                reasoning_content: None,
            }],
        }
    }

    #[async_trait::async_trait]
    impl Provider for MockProvider {
        async fn stream_generate(
            &self,
            _request: nekocode_core::types::GenerateRequest,
            sender: UnboundedSender<ProviderEvent>,
        ) -> Result<ProviderResponse, ProviderError> {
            let msg = self
                .responses
                .lock()
                .unwrap()
                .pop()
                .ok_or_else(|| ProviderError::Other(anyhow::anyhow!("mock exhausted")))?;
            for block in &msg.blocks {
                if let AssistantContentBlock::Text { content, .. } = block {
                    sender.send(ProviderEvent::Content(content.clone())).unwrap();
                }
            }
            sender.send(ProviderEvent::MessageEnd(StopReason::Stop)).unwrap();
            Ok(ProviderResponse {
                message: msg,
                usage: Usage {
                    total_input: 10,
                    total_output: 5,
                    cache_hit: false,
                    cache_miss: 10,
                },
            })
        }
    }

    async fn make_child(provider: Arc<dyn Provider>) -> Agent {
        let path = std::env::temp_dir().join(format!(
            "nekocode_subagent_runner_{}_{}.db",
            std::process::id(),
            std::sync::atomic::AtomicU64::new(0)
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let db = nekocode_entities::prepare_db(path).await.expect("prepare_db");
        Agent {
            thread_id: 0,
            working_directory: "/tmp".into(),
            db,
            middlewares: Arc::new(Vec::new()),
            provider,
            extensions: Arc::new(dashmap::DashMap::new()),
        }
    }

    #[tokio::test]
    async fn run_subagent_success_stores_result() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry.allocate_running();
        let child = make_child(Arc::new(MockProvider::new(vec![text_msg("result")]))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        run_subagent(id, child, "do thing".into(), registry.clone(), tx).await;
        assert!(matches!(registry.run_state(id), crate::registry::SubagentRunState::Finished));
        let result = registry.result(id).expect("result stored");
        assert!(result.finished);
        // The captured turn has user + assistant messages.
        assert_eq!(result.messages.len(), 2);
    }

    #[tokio::test]
    async fn run_subagent_error_marks_error_state() {
        let registry = Arc::new(SubagentRegistry::new());
        let id = registry.allocate_running();
        // Empty responses → first stream_generate errors ("mock exhausted").
        let child = make_child(Arc::new(MockProvider::new(Vec::new()))).await;
        let (tx, _rx) = mpsc::unbounded_channel();
        run_subagent(id, child, "do thing".into(), registry.clone(), tx).await;
        assert!(matches!(
            registry.run_state(id),
            crate::registry::SubagentRunState::Error(_)
        ));
        assert!(registry.run_state(id).is_ready());
    }
}
```

Note: the test uses `nekocode_entities::prepare_db` — so `nekocode-entities` must be a **dev-dependency** of this crate (not a regular dep, keeping the production dep graph lean). Add to `crates/nekocode-subagent/Cargo.toml`:

```toml
[dev-dependencies]
nekocode-entities.workspace = true
tokio = { workspace = true, features = ["macros", "rt-multi-thread"] }
```

Also confirm `AssistantMessage`'s field name is `blocks` (it is, per `nekocode-types/src/generate.rs` and the `MockProvider` template). The `reasoning_content: None` field on `AssistantContentBlock::Text` must match — verify against `nekocode-types/src/generate.rs` before running; if the variant has only `content`, drop `reasoning_content`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p nekocode-subagent runner`
Expected: FAIL with compile errors if the `AssistantContentBlock::Text` shape doesn't match, otherwise the tests should PASS (the runner is already implemented above). If they pass immediately, that's correct — the runner logic is minimal and the mock validates it. (Per TDD we wrote the test first; the impl is in the same file because the runner is small. The "fail" step here is verifying the test compiles and exercises the path.)

- [ ] **Step 3: Verify the AssistantContentBlock shape**

Run: `grep -nA4 "Text {" crates/nekocode-types/src/generate.rs`
If the `Text` variant is `Text { content: String }` (no `reasoning_content`), edit the test's `text_msg` helper to drop `reasoning_content: None`. Expected: the `grep` shows the exact fields.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p nekocode-subagent runner`
Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/nekocode-subagent/src/runner.rs crates/nekocode-subagent/Cargo.toml
git commit -m "feat(subagent): implement run_subagent runner reusing Agent::run_loop"
```

---

## Task 6: Implement the 6 tools + `SubagentMiddleware`

This is the largest task. It's split into sub-steps but committed as one unit since the tools and middleware are interdependent (the middleware registers the tools; the tools share `SubagentContext`).

**Files:**
- Create: `crates/nekocode-subagent/src/tool/mod.rs`
- Create: `crates/nekocode-subagent/src/tool/spawn_subagent.rs`
- Create: `crates/nekocode-subagent/src/tool/inspect_subagent.rs`
- Create: `crates/nekocode-subagent/src/tool/read_subagent.rs`
- Create: `crates/nekocode-subagent/src/tool/wait_any_subagent.rs`
- Create: `crates/nekocode-subagent/src/tool/wait_all_subagents.rs`
- Create: `crates/nekocode-subagent/src/tool/abort_subagent.rs`
- Modify: `crates/nekocode-subagent/src/middleware.rs`

- [ ] **Step 1: Implement `tool/mod.rs` (helpers + parse functions)**

Create `crates/nekocode-subagent/src/tool/mod.rs`:

```rust
use std::sync::Arc;
use std::time::Duration;

use nekocode_types::tool::ToolError;

use crate::registry::SubagentRunState;

pub mod abort_subagent;
pub mod inspect_subagent;
pub mod read_subagent;
pub mod spawn_subagent;
pub mod wait_all_subagents;
pub mod wait_any_subagent;

pub use abort_subagent::AbortSubagentTool;
pub use inspect_subagent::InspectSubagentTool;
pub use read_subagent::ReadSubagentTool;
pub use spawn_subagent::SpawnSubagentTool;
pub use wait_all_subagents::WaitAllSubagentsTool;
pub use wait_any_subagent::WaitAnySubagentTool;

/// Parse a single `agent_id` (u64) parameter.
pub(crate) fn parse_agent_id(params: &serde_json::Value) -> Result<u64, ToolError> {
    params
        .get("agent_id")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| ToolError::InvalidParameters("Missing or invalid 'agent_id' parameter".into()))
}

/// Parse a non-empty `agent_ids` array parameter.
pub(crate) fn parse_agent_ids(params: &serde_json::Value) -> Result<Vec<u64>, ToolError> {
    let arr = params
        .get("agent_ids")
        .and_then(|v| v.as_array())
        .ok_or_else(|| ToolError::InvalidParameters("Missing 'agent_ids' array parameter".into()))?;
    if arr.is_empty() {
        return Err(ToolError::InvalidParameters(
            "'agent_ids' must be a non-empty array".into(),
        ));
    }
    arr.iter()
        .map(|v| {
            v.as_u64().ok_or_else(|| {
                ToolError::InvalidParameters("'agent_ids' must contain integers".into())
            })
        })
        .collect()
}

/// Parse a positive `timeout` (seconds, f64) parameter.
pub(crate) fn parse_timeout(params: &serde_json::Value) -> Result<f64, ToolError> {
    let t = params
        .get("timeout")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| ToolError::InvalidParameters("Missing or invalid 'timeout' parameter".into()))?;
    if t <= 0.0 {
        return Err(ToolError::InvalidParameters("'timeout' must be positive".into()));
    }
    Ok(t)
}

/// Lowercase state name for JSON results.
pub(crate) fn run_state_name(s: &SubagentRunState) -> &'static str {
    s.name()
}

/// Await any one of the given Notify handles. Mirrors nekocode-subthread's
/// notify_any helper (duplicated per the no-cross-crate-sharing guideline).
pub(crate) async fn notify_any(notifies: &[Arc<tokio::sync::Notify>]) {
    use futures_util::future::select_all;
    use std::future::Future;
    use std::pin::Pin;
    if notifies.is_empty() {
        std::future::pending::<()>().await;
        return;
    }
    let futures: Vec<Pin<Box<dyn Future<Output = ()> + Send>>> = notifies
        .iter()
        .map(|n| {
            let n = n.clone();
            Box::pin(async move { n.notified().await })
        })
        .collect();
    let _ = select_all(futures).await;
}

// Silence unused-import warning for Duration until wait tools are wired.
#[allow(dead_code)]
fn _ensure_duration_used() -> Duration {
    Duration::from_secs(0)
}
```

- [ ] **Step 2: Implement `spawn_subagent.rs`**

Create `crates/nekocode-subagent/src/tool/spawn_subagent.rs`:

```rust
use std::sync::Arc;

use nekocode_core::agent::Agent;
use nekocode_core::middleware::MiddlewareSpec;
use nekocode_types::generate::MessageContent;
use nekocode_types::tool::{Tool, ToolError, ToolSpec};
use tokio::sync::mpsc;

use crate::middleware::SubagentMiddleware;
use crate::runner::run_subagent;
use crate::SubagentContext;

pub struct SpawnSubagentTool {
    ctx: SubagentContext,
}

impl SpawnSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for SpawnSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "spawn_subagent".to_string(),
            description: "Spawn a single-turn subagent that runs a given prompt to completion under a named profile. Returns immediately with status 'running'. Poll completion via inspect_subagent, wait_any_subagent, or wait_all_subagents; read the result via read_subagent. Refuses if the profile is unknown, if the profile requests middlewares the parent did not enable, or if the nesting depth limit is exceeded.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "profile": {
                        "type": "string",
                        "description": "The profile name to load from agents.toml."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "The user message to run as the subagent's single turn."
                    }
                },
                "required": ["profile", "prompt"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let profile_name = params
            .get("profile")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'profile' parameter".into()))?;
        let prompt = params
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidParameters("Missing 'prompt' parameter".into()))?
            .to_string();

        let profile = self
            .ctx
            .catalog
            .get(profile_name)
            .map_err(|e| ToolError::InvalidParameters(e.to_string()))?;

        // Gate A: the parent's profile must allow nesting.
        if !self.ctx.allow_nested {
            return Err(ToolError::ExecutionError(
                "parent profile does not allow nested subagents".into(),
            ));
        }
        // Gate B: depth bound.
        if self.ctx.depth + 1 > self.ctx.max_depth {
            return Err(ToolError::ExecutionError(
                "max subagent nesting depth exceeded".into(),
            ));
        }

        // Middleware intersection: profile.middlewares must be ⊆ parent specs.
        let spec_names: std::collections::HashSet<&str> =
            self.ctx.specs.iter().map(|s| s.name.as_str()).collect();
        for m in &profile.middlewares {
            if !spec_names.contains(m.as_str()) {
                return Err(ToolError::ExecutionError(format!(
                    "profile '{}' requests middleware '{}', not enabled by parent",
                    profile_name, m
                )));
            }
        }
        let selected_specs: Vec<MiddlewareSpec> = self
            .ctx
            .specs
            .iter()
            .filter(|s| profile.middlewares.contains(&s.name))
            .cloned()
            .collect();

        let agent_id = self.ctx.registry.allocate_running();
        let child_extensions = Arc::new(dashmap::DashMap::new());

        // Build isolated middleware instances via the factory.
        let mut child_middlewares: Vec<Box<dyn nekocode_core::middleware::Middleware>> = Vec::new();
        for spec in &selected_specs {
            child_middlewares.push(self.ctx.factory.build(
                spec.clone(),
                agent_id,
                child_extensions.clone(),
            ));
        }

        // Construct the child's own SubagentMiddleware (at depth+1, with the
        // child profile's allow_nested). It registers the subagent tools for
        // the child so it can itself spawn (subject to the gates above).
        let child_subagent_mw = SubagentMiddleware::new(
            self.ctx.specs.clone(),
            self.ctx.factory.clone(),
            self.ctx.parent_provider.clone(),
            self.ctx.parent_config.clone(),
            child_extensions.clone(),
            crate::SubagentConfig { max_depth: self.ctx.max_depth },
            self.ctx.depth + 1,
            profile.allow_nested,
        );
        child_middlewares.push(Box::new(child_subagent_mw));

        let working_directory = profile
            .working_directory
            .clone()
            .unwrap_or_else(|| self.ctx.parent_working_directory.clone());

        let child = Agent {
            thread_id: agent_id,
            working_directory: working_directory.clone(),
            db: self.ctx.parent_db.clone(),
            middlewares: Arc::new(child_middlewares),
            provider: self.ctx.parent_provider.clone(),
            extensions: child_extensions,
        };

        // Drained-sender pattern: a companion task drains the event channel so
        // run_loop's send() never blocks when no one consumes.
        let (tx, mut rx) = mpsc::unbounded_channel();
        let registry = self.ctx.registry.clone();
        let handle = tokio::spawn(async move {
            let drain = tokio::spawn(async move {
                while rx.recv().await.is_some() {}
            });
            run_subagent(agent_id, child, prompt, registry, tx).await;
            drain.abort();
        });

        self.ctx.registry.set_running(agent_id, handle);

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "status": "running",
        }))
    }
}
```

- [ ] **Step 3: Implement `inspect_subagent.rs`**

Create `crates/nekocode-subagent/src/tool/inspect_subagent.rs`:

```rust
use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::{run_state_name, parse_agent_id};
use crate::SubagentContext;

pub struct InspectSubagentTool {
    ctx: SubagentContext,
}

impl InspectSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for InspectSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "inspect_subagent".to_string(),
            description: "Inspect a subagent's current run state. Returns the state ('running', 'finished', or 'error') and, when errored, the error message.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "integer", "description": "The agent id returned by spawn_subagent." }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let agent_id = parse_agent_id(&params)?;
        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        let state = self.ctx.registry.run_state(agent_id);
        let mut out = serde_json::json!({
            "agent_id": agent_id,
            "status": run_state_name(&state),
        });
        if let crate::registry::SubagentRunState::Error(msg) = &state {
            out["error"] = serde_json::Value::String(msg.clone());
        }
        Ok(out)
    }
}
```

- [ ] **Step 4: Implement `read_subagent.rs`**

Create `crates/nekocode-subagent/src/tool/read_subagent.rs`:

```rust
use nekocode_types::generate::{AssistantContentBlock, MessageType};
use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::parse_agent_id;
use crate::SubagentContext;

pub struct ReadSubagentTool {
    ctx: SubagentContext,
}

impl ReadSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for ReadSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read_subagent".to_string(),
            description: "Read a finished subagent's result. By default returns only the last assistant message's text (text_only=true); with text_only=false returns the full message list. Refuses if the subagent is not finished/errored.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "integer", "description": "The agent id returned by spawn_subagent." },
                    "text_only": { "type": "boolean", "description": "If true (default), return only the last assistant text. If false, return the full message list." }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let agent_id = parse_agent_id(&params)?;
        let text_only = params
            .get("text_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        let state = self.ctx.registry.run_state(agent_id);
        if !state.is_ready() {
            return Err(ToolError::ExecutionError(format!(
                "agent {} is not ready (state: {})",
                agent_id,
                state.name()
            )));
        }
        let result = self
            .ctx
            .registry
            .result(agent_id)
            .ok_or_else(|| ToolError::ExecutionError(format!("agent {} has no result", agent_id)))?;

        if text_only {
            let text = last_assistant_text(&result.messages).unwrap_or_default();
            Ok(serde_json::json!({
                "agent_id": agent_id,
                "status": state.name(),
                "text": text,
            }))
        } else {
            Ok(serde_json::json!({
                "agent_id": agent_id,
                "status": state.name(),
                "messages": result.messages,
            }))
        }
    }
}

/// Concatenate the text blocks of the last assistant message. Returns None
/// if there is no assistant message or it has no text blocks.
fn last_assistant_text(messages: &[nekocode_types::generate::Message]) -> Option<String> {
    let last = messages.iter().rev().find(|m| {
        matches!(m.data, MessageType::Assistant(_))
    })?;
    if let MessageType::Assistant(a) = &last.data {
        let texts: Vec<&str> = a
            .blocks
            .iter()
            .filter_map(|b| match b {
                AssistantContentBlock::Text { content, .. } => Some(content.as_str()),
                _ => None,
            })
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    } else {
        None
    }
}
```

- [ ] **Step 5: Implement `wait_any_subagent.rs`**

Create `crates/nekocode-subagent/src/tool/wait_any_subagent.rs`:

```rust
use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::{notify_any, parse_agent_ids, parse_timeout, run_state_name};
use crate::SubagentContext;

pub struct WaitAnySubagentTool {
    ctx: SubagentContext,
}

impl WaitAnySubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAnySubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_any_subagent".to_string(),
            description: "Wait until any one of the specified subagents becomes ready (finished or errored), or until the timeout elapses. Returns the first ready subagent on success, or the list of still-pending subagents on timeout. Does NOT kill or affect running subagents on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The agent ids to wait on."
                    },
                    "timeout": { "type": "number", "description": "Maximum seconds to wait. Must be positive." }
                },
                "required": ["agent_ids", "timeout"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let ids = parse_agent_ids(&params)?;
        let timeout_secs = parse_timeout(&params)?;
        for id in &ids {
            if !self.ctx.registry.contains(*id) {
                return Err(ToolError::ExecutionError(format!("agent {} not found", id)));
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            for id in &ids {
                let state = self.ctx.registry.run_state(*id);
                if state.is_ready() {
                    return Ok(serde_json::json!({
                        "status": "ready",
                        "agent_id": id,
                        "run_state": run_state_name(&state),
                    }));
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "pending": ids,
                }));
            }
            let notifies: Vec<_> = ids
                .iter()
                .filter_map(|id| self.ctx.registry.notify(*id))
                .collect();
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifies.is_empty() {
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(&notifies) => {}
                }
            }
        }
    }
}
```

- [ ] **Step 6: Implement `wait_all_subagents.rs`**

Create `crates/nekocode-subagent/src/tool/wait_all_subagents.rs`:

```rust
use std::time::Duration;

use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::{notify_any, parse_timeout, run_state_name};
use crate::SubagentContext;

pub struct WaitAllSubagentsTool {
    ctx: SubagentContext,
}

impl WaitAllSubagentsTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for WaitAllSubagentsTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "wait_all_subagents".to_string(),
            description: "Wait until all specified subagents are ready (finished or errored), or until the timeout elapses. With no agent_ids, defaults to all of the parent's currently-running subagents. On timeout, returns the ready and pending lists separately. Does NOT kill or affect running subagents on timeout.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_ids": {
                        "type": "array",
                        "items": { "type": "integer" },
                        "description": "The agent ids to wait on. If omitted, waits on all of the parent's currently-running subagents."
                    },
                    "timeout": { "type": "number", "description": "Maximum seconds to wait. Must be positive." }
                },
                "required": ["timeout"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let timeout_secs = parse_timeout(&params)?;
        // agent_ids optional: default to all currently-running.
        let ids: Vec<u64> = match params.get("agent_ids").and_then(|v| v.as_array()) {
            Some(arr) => {
                let v: Result<Vec<u64>, ToolError> = arr
                    .iter()
                    .map(|x| {
                        x.as_u64().ok_or_else(|| {
                            ToolError::InvalidParameters("'agent_ids' must contain integers".into())
                        })
                    })
                    .collect();
                v?
            }
            None => self
                .ctx
                .registry
                .all_agent_ids()
                .into_iter()
                .filter(|id| {
                    matches!(
                        self.ctx.registry.run_state(*id),
                        crate::registry::SubagentRunState::Running
                    )
                })
                .collect(),
        };
        if ids.is_empty() {
            return Ok(serde_json::json!({ "status": "ready", "results": [] }));
        }
        for id in &ids {
            if !self.ctx.registry.contains(*id) {
                return Err(ToolError::ExecutionError(format!("agent {} not found", id)));
            }
        }

        let deadline = tokio::time::Instant::now() + Duration::from_secs_f64(timeout_secs);
        loop {
            let (ready, pending): (Vec<u64>, Vec<u64>) = ids.iter().partition(|id| {
                self.ctx.registry.run_state(**id).is_ready()
            });
            if pending.is_empty() {
                let results: Vec<serde_json::Value> = ready
                    .iter()
                    .map(|id| {
                        serde_json::json!({
                            "agent_id": id,
                            "run_state": run_state_name(&self.ctx.registry.run_state(*id)),
                        })
                    })
                    .collect();
                return Ok(serde_json::json!({
                    "status": "ready",
                    "results": results,
                }));
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Ok(serde_json::json!({
                    "status": "timeout",
                    "ready": ready,
                    "pending": pending,
                }));
            }
            let notifies: Vec<_> = ids
                .iter()
                .filter_map(|id| self.ctx.registry.notify(*id))
                .collect();
            let sleep = tokio::time::sleep_until(deadline);
            tokio::pin!(sleep);
            if notifies.is_empty() {
                (&mut sleep).await;
            } else {
                tokio::select! {
                    _ = sleep => {}
                    _ = notify_any(&notifies) => {}
                }
            }
        }
    }
}
```

- [ ] **Step 7: Implement `abort_subagent.rs`**

Create `crates/nekocode-subagent/src/tool/abort_subagent.rs`:

```rust
use nekocode_types::tool::{Tool, ToolError, ToolSpec};

use crate::tool::parse_agent_id;
use crate::SubagentContext;

pub struct AbortSubagentTool {
    ctx: SubagentContext,
}

impl AbortSubagentTool {
    pub fn new(ctx: SubagentContext) -> Self {
        Self { ctx }
    }
}

#[async_trait::async_trait]
impl Tool for AbortSubagentTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "abort_subagent".to_string(),
            description: "Abort a subagent's background task (if running) and remove it from the registry. The subagent's in-memory result is discarded.".to_string(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "agent_id": { "type": "integer", "description": "The agent id returned by spawn_subagent." }
                },
                "required": ["agent_id"]
            }),
        }
    }

    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError> {
        let agent_id = parse_agent_id(&params)?;
        if !self.ctx.registry.contains(agent_id) {
            return Err(ToolError::ExecutionError(format!(
                "agent {} not found",
                agent_id
            )));
        }
        self.ctx.registry.abort(agent_id);
        Ok(serde_json::json!({
            "agent_id": agent_id,
            "aborted": true,
        }))
    }
}
```

- [ ] **Step 8: Implement `middleware.rs` (`SubagentContext` + `SubagentMiddleware`)**

Replace `crates/nekocode-subagent/src/middleware.rs` with:

```rust
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::Provider;
use nekocode_types::config::Config;
use nekocode_types::tool::ToolRegistry;
use tokio::sync::RwLock;

use crate::factory::SubagentMiddlewareFactory;
use crate::profile::ProfileCatalog;
use crate::registry::SubagentRegistry;
use crate::tool::{
    AbortSubagentTool, InspectSubagentTool, ReadSubagentTool, SpawnSubagentTool,
    WaitAllSubagentsTool, WaitAnySubagentTool,
};

/// Shared, cheaply-cloneable context for all subagent tools. All fields are
/// Arc/Clone, so this is safe to hand to every tool.
#[derive(Clone)]
pub struct SubagentContext {
    pub registry: Arc<SubagentRegistry>,
    pub specs: Vec<MiddlewareSpec>,
    pub factory: Arc<dyn SubagentMiddlewareFactory>,
    pub parent_provider: Arc<dyn Provider>,
    pub parent_config: Arc<RwLock<Config>>,
    pub parent_working_directory: String,
    pub parent_db: toasty::Db,
    pub catalog: Arc<ProfileCatalog>,
    pub depth: u32,
    pub max_depth: u32,
    pub allow_nested: bool,
}

/// The subagent middleware. Registered on a parent agent's middleware chain;
/// in `before_generate` it inserts the 6 subagent tools and publishes the
/// per-parent `SubagentRegistry` to `Agent.extensions["subagent"]`.
pub struct SubagentMiddleware {
    ctx: SubagentContext,
    registry: Arc<SubagentRegistry>,
}

impl SubagentMiddleware {
    pub fn new(
        specs: Vec<MiddlewareSpec>,
        factory: Arc<dyn SubagentMiddlewareFactory>,
        parent_provider: Arc<dyn Provider>,
        parent_config: Arc<RwLock<Config>>,
        parent_extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
        config: crate::SubagentConfig,
        depth: u32,
        allow_nested: bool,
    ) -> Self {
        let registry = Arc::new(SubagentRegistry::new());
        // Load the profile catalog once, here, from global + workspace paths.
        let global_path = global_agents_toml_path();
        let workspace_path = workspace_agents_toml_path(&parent_extensions);
        let catalog = Arc::new(
            ProfileCatalog::load(&global_path, workspace_path.as_deref())
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to load agents.toml: {e}; using empty catalog");
                    ProfileCatalog::empty()
                }),
        );
        // Publish the registry to the parent's extensions so the API layer can
        // reach it for cascade cleanup. (We need the working_directory for the
        // workspace path, which we resolve via the catalog load above using
        // parent_extensions — see workspace_agents_toml_path.)
        parent_extensions.insert(
            crate::SUBAGENT_EXTENSION_KEY.into(),
            Box::new(registry.clone()) as Box<dyn std::any::Any + Send + Sync>,
        );
        let ctx = SubagentContext {
            registry: registry.clone(),
            specs,
            factory,
            parent_provider,
            parent_config,
            parent_working_directory: String::new(), // set below via set_working_directory
            parent_db: /* see note */ panic!("parent_db must be set"),
            catalog,
            depth,
            max_depth: config.max_depth,
            allow_nested,
        };
        Self { ctx, registry }
    }
}

#[async_trait::async_trait]
impl Middleware for SubagentMiddleware {
    async fn before_generate(
        &self,
        _request: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        let ctx = &self.ctx;
        registry.insert("spawn_subagent".into(), Arc::new(SpawnSubagentTool::new(ctx.clone())));
        registry.insert("inspect_subagent".into(), Arc::new(InspectSubagentTool::new(ctx.clone())));
        registry.insert("read_subagent".into(), Arc::new(ReadSubagentTool::new(ctx.clone())));
        registry.insert("wait_any_subagent".into(), Arc::new(WaitAnySubagentTool::new(ctx.clone())));
        registry.insert("wait_all_subagents".into(), Arc::new(WaitAllSubagentsTool::new(ctx.clone())));
        registry.insert("abort_subagent".into(), Arc::new(AbortSubagentTool::new(ctx.clone())));
        Ok(())
    }
}

/// Resolve the global agents.toml path: same dir as config.toml, i.e.
/// `<config_dir>/nekocode/agents.toml`.
fn global_agents_toml_path() -> std::path::PathBuf {
    dirs::config_dir()
        .map(|p| p.join("nekocode").join("agents.toml"))
        .unwrap_or_else(|| std::path::PathBuf::from("agents.toml"))
}

/// Resolve the workspace agents.toml path: `<working_directory>/.nekocode/agents.toml`.
/// The working_directory is read from the parent agent's extensions under a
/// well-known key, OR (simpler) passed explicitly. Here we resolve from the
/// parent's Thread working_directory, which the API layer stores in extensions.
fn workspace_agents_toml_path(
    _parent_extensions: &DashMap<String, Box<dyn std::any::Any + Send + Sync>>,
) -> Option<std::path::PathBuf> {
    // The working_directory is most reliably passed via SubagentContext, but
    // it isn't available at new() time without an extra parameter. The API
    // build_middlewares arm has ctx.working_directory, so the cleanest fix is
    // to pass working_directory into SubagentMiddleware::new. See Task 8.
    None
}
```

**Important — the `parent_db` and `working_directory` issue:** The `SubagentContext` needs `parent_db` (to build the child `Agent.db` field) and `parent_working_directory` (for the workspace profile path and as the child's default workdir). These aren't cleanly available in `new()`'s current signature. **This must be fixed by adding `parent_db: toasty::Db` and `parent_working_directory: String` parameters to `SubagentMiddleware::new`.** This is corrected in Task 8 when the API integration wires the real call site; for now, leave the two `panic!`s and the broken `workspace_agents_toml_path` as TODOs that Task 8 will resolve. **Do NOT commit until Task 8 fixes the signature** — the crate will not compile with the panics. Instead, structure Task 6 to land a *compiling* version by fixing the signature now.

**Step 8 correction (apply now):** Update the `SubagentMiddleware::new` signature and body to take `parent_db` and `parent_working_directory`, and resolve the workspace path from the latter:

```rust
    pub fn new(
        specs: Vec<MiddlewareSpec>,
        factory: Arc<dyn SubagentMiddlewareFactory>,
        parent_provider: Arc<dyn Provider>,
        parent_config: Arc<RwLock<Config>>,
        parent_extensions: Arc<DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
        parent_db: toasty::Db,
        parent_working_directory: String,
        config: crate::SubagentConfig,
        depth: u32,
        allow_nested: bool,
    ) -> Self {
        let registry = Arc::new(SubagentRegistry::new());
        let global_path = global_agents_toml_path();
        let workspace_path = workspace_agents_toml_path(&parent_working_directory);
        let catalog = Arc::new(
            ProfileCatalog::load(&global_path, workspace_path.as_deref())
                .unwrap_or_else(|e| {
                    tracing::warn!("failed to load agents.toml: {e}; using empty catalog");
                    ProfileCatalog::empty()
                }),
        );
        parent_extensions.insert(
            crate::SUBAGENT_EXTENSION_KEY.into(),
            Box::new(registry.clone()) as Box<dyn std::any::Any + Send + Sync>,
        );
        let ctx = SubagentContext {
            registry: registry.clone(),
            specs,
            factory,
            parent_provider,
            parent_config,
            parent_working_directory,
            parent_db,
            catalog,
            depth,
            max_depth: config.max_depth,
            allow_nested,
        };
        Self { ctx, registry }
    }
```

And replace `workspace_agents_toml_path`:

```rust
fn workspace_agents_toml_path(working_directory: &str) -> Option<std::path::PathBuf> {
    let p = std::path::Path::new(working_directory).join(".nekocode").join("agents.toml");
    if p.exists() {
        Some(p)
    } else {
        None
    }
}
```

Add `dirs` to the crate deps (it's a workspace dep): add `dirs.workspace = true` to `[dependencies]` in `crates/nekocode-subagent/Cargo.toml`.

- [ ] **Step 9: Verify the crate builds**

Run: `cargo build -p nekocode-subagent`
Expected: build succeeds. (No tests added in this task — Tier 3 integration tests come in Task 7.)

- [ ] **Step 10: Commit**

```bash
git add crates/nekocode-subagent/src/tool/ crates/nekocode-subagent/src/middleware.rs crates/nekocode-subagent/Cargo.toml
git commit -m "feat(subagent): implement 6 tools + SubagentMiddleware"
```

---

## Task 7: Tier 3 integration tests

**Files:**
- Create: `crates/nekocode-subagent/tests/integration.rs`

- [ ] **Step 1: Write the integration tests**

Create `crates/nekocode-subagent/tests/integration.rs`:

```rust
//! Integration tests for nekocode-subagent tools. Exercises the
//! spawn→wait→read→inspect→abort lifecycle, profile resolution, middleware
//! intersection, nesting gates, and wait timeout — all against in-memory
//! state with a local mock provider (the nekocode-core test mocks are
//! pub(crate) and not accessible from here).

use std::sync::{Arc, Mutex};

use nekocode_core::agent::Agent;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
use nekocode_subagent::{
    SubagentConfig, SubagentContext, SubagentMiddleware, SubagentMiddlewareFactory,
};
use nekocode_types::generate::{
    AssistantContentBlock, AssistantMessage, MessageContent, StopReason, Usage,
};
use nekocode_types::tool::{Tool, ToolRegistry};
use tokio::sync::mpsc;

/// A factory that returns a no-op middleware (sufficient — the tools don't
/// exercise the child's middleware behavior, only the registry/runner path).
struct MockFactory;

#[async_trait::async_trait]
impl SubagentMiddlewareFactory for MockFactory {
    fn build(
        &self,
        _spec: MiddlewareSpec,
        _subagent_id: u64,
        _extensions: Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
        // A middleware that registers nothing — keeps the child's tool
        // registry empty so run_loop finishes after one assistant turn.
        Box::new(NoopMiddleware)
    }
}

struct NoopMiddleware;
#[async_trait::async_trait]
impl Middleware for NoopMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        _: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct MockProvider {
    responses: Mutex<Vec<AssistantMessage>>,
}

impl MockProvider {
    fn new(responses: Vec<AssistantMessage>) -> Self {
        let mut r = responses;
        r.reverse();
        Self { responses: Mutex::new(r) }
    }
}

fn text_msg(s: &str) -> AssistantMessage {
    AssistantMessage {
        blocks: vec![AssistantContentBlock::Text {
            content: s.to_string(),
            reasoning_content: None,
        }],
    }
}

#[async_trait::async_trait]
impl Provider for MockProvider {
    async fn stream_generate(
        &self,
        _request: nekocode_core::types::GenerateRequest,
        sender: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        let msg = self
            .responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| ProviderError::Other(anyhow::anyhow!("mock exhausted")))?;
        for block in &msg.blocks {
            if let AssistantContentBlock::Text { content, .. } = block {
                sender.send(ProviderEvent::Content(content.clone())).unwrap();
            }
        }
        sender.send(ProviderEvent::MessageEnd(StopReason::Stop)).unwrap();
        Ok(ProviderResponse {
            message: msg,
            usage: Usage {
                total_input: 10,
                total_output: 5,
                cache_hit: false,
                cache_miss: 10,
            },
        })
    }
}

async fn make_middleware(allow_nested: bool, max_depth: u32) -> SubagentMiddleware {
    let db = nekocode_entities::prepare_db(std::env::temp_dir().join(format!(
        "nekocode_subagent_it_{}_{}.db",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0)
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )))
    .await
    .unwrap();
    let config = Arc::new(tokio::sync::RwLock::new(nekocode_types::config::Config::default()));
    let extensions = Arc::new(dashmap::DashMap::new());
    // Build a catalog with one profile "explorer" that uses no middlewares.
    let catalog_dir = std::env::temp_dir().join(format!(
        "nekocode_subagent_it_cat_{}_{}",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0)
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&catalog_dir).unwrap();
    std::fs::write(
        catalog_dir.join("agents.toml"),
        r#"
[[agents]]
name = "explorer"
middlewares = []
"#,
    )
    .unwrap();
    // Point the global path at our temp catalog by overriding dirs is not
    // feasible; instead test the tools directly via the context. For these
    // tests we bypass ProfileCatalog::load and construct the context by hand
    // using a catalog built from the temp file.
    let global_path = catalog_dir.join("agents.toml");
    let catalog = Arc::new(
        nekocode_subagent::ProfileCatalog::load(&global_path, None).unwrap(),
    );
    // Construct the middleware the long way to inject our catalog/provider.
    // Since SubagentMiddleware::new loads its own catalog, we instead build a
    // SubagentContext directly and register tools ourselves for the test.
    let _ = (allow_nested, max_depth, config, extensions, db);
    unreachable!("see Step 2 — tests use a hand-built context, not SubagentMiddleware::new")
}
```

**The above `make_middleware` is a dead end** because `SubagentMiddleware::new` loads its own catalog from the real `dirs::config_dir()` path, which the test can't control. **Step 2 corrects this**: the tests must build a `SubagentContext` directly (the fields are all `pub`) and call the tools directly, bypassing `SubagentMiddleware::new`. Replace the entire `tests/integration.rs` with the corrected version:

```rust
//! Integration tests for nekocode-subagent tools. Builds a SubagentContext
//! directly (fields are pub) and invokes tools, bypassing
//! SubagentMiddleware::new (which loads agents.toml from the real config dir,
//! not controllable from tests). The provider/factory are local mocks.

use std::sync::{Arc, Mutex};

use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
use nekocode_subagent::{
    ProfileCatalog, SubagentConfig, SubagentContext, SubagentMiddlewareFactory, SubagentRegistry,
    tool::{AbortSubagentTool, InspectSubagentTool, ReadSubagentTool, SpawnSubagentTool, WaitAnySubagentTool},
};
use nekocode_types::generate::{
    AssistantContentBlock, AssistantMessage, StopReason, Usage,
};
use nekocode_types::tool::Tool;
use tokio::sync::mpsc;

struct MockFactory;
#[async_trait::async_trait]
impl SubagentMiddlewareFactory for MockFactory {
    fn build(
        &self,
        _spec: MiddlewareSpec,
        _subagent_id: u64,
        _extensions: Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>,
    ) -> Box<dyn Middleware> {
        Box::new(NoopMiddleware)
    }
}

struct NoopMiddleware;
#[async_trait::async_trait]
impl Middleware for NoopMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        _: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct MockProvider {
    responses: Mutex<Vec<AssistantMessage>>,
}
impl MockProvider {
    fn new(responses: Vec<AssistantMessage>) -> Self {
        let mut r = responses;
        r.reverse();
        Self { responses: Mutex::new(r) }
    }
}
fn text_msg(s: &str) -> AssistantMessage {
    AssistantMessage {
        blocks: vec![AssistantContentBlock::Text {
            content: s.to_string(),
            reasoning_content: None,
        }],
    }
}
#[async_trait::async_trait]
impl Provider for MockProvider {
    async fn stream_generate(
        &self,
        _request: nekocode_core::types::GenerateRequest,
        sender: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        let msg = self
            .responses
            .lock()
            .unwrap()
            .pop()
            .ok_or_else(|| ProviderError::Other(anyhow::anyhow!("mock exhausted")))?;
        for block in &msg.blocks {
            if let AssistantContentBlock::Text { content, .. } = block {
                sender.send(ProviderEvent::Content(content.clone())).unwrap();
            }
        }
        sender.send(ProviderEvent::MessageEnd(StopReason::Stop)).unwrap();
        Ok(ProviderResponse {
            message: msg,
            usage: Usage { total_input: 10, total_output: 5, cache_hit: false, cache_miss: 10 },
        })
    }
}

async fn temp_db() -> toasty::Db {
    let path = std::env::temp_dir().join(format!(
        "nekocode_subagent_it_{}_{}.db",
        std::process::id(),
        std::sync::atomic::AtomicU64::new(0)
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    nekocode_entities::prepare_db(path).await.unwrap()
}

fn catalog_with_explorer() -> Arc<ProfileCatalog> {
    // Build a catalog in-memory by parsing a TOML string.
    let toml = r#"
[[agents]]
name = "explorer"
middlewares = []
"#;
    let parsed: Vec<nekocode_subagent::SubagentProfile> = toml::from_str(toml).unwrap();
    let mut profiles = std::collections::HashMap::new();
    for p in parsed {
        profiles.insert(p.name.clone(), p);
    }
    Arc::new(ProfileCatalog { profiles })
}

fn make_ctx(allow_nested: bool, max_depth: u32, db: toasty::Db) -> SubagentContext {
    SubagentContext {
        registry: Arc::new(SubagentRegistry::new()),
        specs: Vec::new(),
        factory: Arc::new(MockFactory),
        parent_provider: Arc::new(MockProvider::new(vec![text_msg("done")])),
        parent_config: Arc::new(tokio::sync::RwLock::new(nekocode_types::config::Config::default())),
        parent_working_directory: "/tmp".into(),
        parent_db: db,
        catalog: catalog_with_explorer(),
        depth: 0,
        max_depth,
        allow_nested,
    }
}

#[tokio::test]
async fn spawn_wait_read_lifecycle() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 0, db);
    let spawn = SpawnSubagentTool::new(ctx.clone());
    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();
    assert_eq!(res.get("status").unwrap().as_str(), "running");

    // Wait for it to finish (small timeout; the mock resolves immediately).
    let wait = WaitAnySubagentTool::new(ctx.clone());
    let wres = wait
        .call(serde_json::json!({ "agent_ids": [agent_id], "timeout": 5.0 }))
        .await
        .unwrap();
    assert_eq!(wres.get("status").unwrap().as_str(), "ready");

    let read = ReadSubagentTool::new(ctx.clone());
    let rres = read
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(rres.get("text").unwrap().as_str(), "done");

    let inspect = InspectSubagentTool::new(ctx.clone());
    let ires = inspect
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(ires.get("status").unwrap().as_str(), "finished");

    let abort = AbortSubagentTool::new(ctx.clone());
    let ares = abort
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(ares.get("aborted").unwrap().as_bool(), true);
}

#[tokio::test]
async fn spawn_unknown_profile_errors() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 0, db);
    let spawn = SpawnSubagentTool::new(ctx);
    let err = spawn
        .call(serde_json::json!({ "profile": "nope", "prompt": "hi" }))
        .await
        .expect_err("unknown profile");
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn spawn_when_parent_disallows_nesting_errors() {
    let db = temp_db().await;
    let ctx = make_ctx(false, 5, db); // allow_nested=false
    let spawn = SpawnSubagentTool::new(ctx);
    let err = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .expect_err("nesting disallowed");
    assert!(err.to_string().contains("does not allow nested"));
}

#[tokio::test]
async fn spawn_exceeding_max_depth_errors() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 0, db); // max_depth=0 → depth 0+1 > 0
    let spawn = SpawnSubagentTool::new(ctx);
    let err = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .expect_err("depth exceeded");
    assert!(err.to_string().contains("max subagent nesting depth"));
}
```

Note: `ProfileCatalog { profiles }` is constructed directly in the test — its `profiles` field must be `pub` (it is, per Task 4). `SubagentProfile` must also be `pub` and constructible via TOML parse (it is). `nekocode_subagent::SubagentProfile` must be re-exported from `lib.rs` (it is, per Task 2).

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test -p nekocode-subagent --test integration`
Expected: all 4 tests PASS. If `AssistantContentBlock::Text` has no `reasoning_content` field, drop it from `text_msg` (per Task 5 Step 3's verification).

- [ ] **Step 3: Commit**

```bash
git add crates/nekocode-subagent/tests/integration.rs
git commit -m "test(subagent): integration tests for spawn/wait/read/inspect/abort lifecycle"
```

---

## Task 8: API crate integration

**Files:**
- Create: `crates/nekocode/src/api/thread/subagent_factory.rs`
- Modify: `crates/nekocode/Cargo.toml`
- Modify: `crates/nekocode/src/api/thread/mod.rs`
- Modify: `crates/nekocode/src/api/thread/activate.rs`
- Modify: `crates/nekocode/src/api/thread/subthread_activator.rs`
- Modify: `crates/nekocode/src/api/thread/delete.rs`

- [ ] **Step 1: Add the dependency to the API crate**

In `crates/nekocode/Cargo.toml`, under `[dependencies]`, add:

```toml
nekocode-subagent.workspace = true
```

- [ ] **Step 2: Create `subagent_factory.rs`**

Create `crates/nekocode/src/api/thread/subagent_factory.rs`:

```rust
use std::any::Any;
use std::sync::Arc;

use dashmap::DashMap;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_subagent::SubagentMiddlewareFactory;
use nekocode_types::config::Config;
use tokio::sync::RwLock;

/// API-layer implementation of `SubagentMiddlewareFactory`. Builds isolated
/// child middleware instances by name + config — the match arms mirror
/// `build_middlewares`, but each instance is constructed with the child's
/// `subagent_id` and the child's fresh `extensions` (so shell gets its own
/// session map, file's thread_id is the synthetic subagent id).
#[derive(Clone)]
pub struct ApiSubagentMiddlewareFactory {
    pub db: toasty::Db,
    pub config: Arc<RwLock<Config>>,
    pub working_directory: String,
}

#[async_trait::async_trait]
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
                nekocode_shell::config::ShellConfig::from_value(&spec.config),
            )),
            "tool" => Box::new(nekocode_file::ToolMiddleware::new(
                nekocode_file::config::FileConfig::from_value(&spec.config),
                self.db.clone(),
                subagent_id,
            )),
            "mcp" => Box::new(nekocode_mcp::McpMiddleware::new(
                nekocode_mcp::config::McpConfig::from_value(&spec.config),
            )),
            "skills" => {
                // Resolve skills_dir from the shared config, mirroring
                // build_middlewares. We block-read the config since this fn
                // is sync (factory::build is not async).
                let skills_dir = self.config.blocking_read().skills.directory.clone();
                Box::new(nekocode_skills::SkillsMiddleware::new(
                    nekocode_skills::SkillsConfig::from_value(&spec.config),
                    std::path::PathBuf::from(skills_dir),
                ))
            }
            other => {
                tracing::warn!("unknown middleware in subagent spec: {other}; skipping");
                // A no-op middleware so the child still runs.
                Box::new(nekocode_subagent::tool::spawn_subagent::NoopMiddleware)
            }
        }
    }
}
```

Note: `factory::build` is **sync** (the trait declares `fn build`, not `async fn`). So `self.config.blocking_read()` is used. This is safe because `build` is called from within `spawn_subagent::call` (an async context), but `blocking_read` will panic if called from within an async runtime worker thread that holds the runtime. To avoid this, the cleaner approach is to **resolve `skills_dir` once at factory construction time** (the factory is built in `build_middlewares`, an async context) and store it on the struct. Apply that correction:

```rust
#[derive(Clone)]
pub struct ApiSubagentMiddlewareFactory {
    pub db: toasty::Db,
    pub skills_dir: std::path::PathBuf,
}

#[async_trait::async_trait]
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
                nekocode_shell::config::ShellConfig::from_value(&spec.config),
            )),
            "tool" => Box::new(nekocode_file::ToolMiddleware::new(
                nekocode_file::config::FileConfig::from_value(&spec.config),
                self.db.clone(),
                subagent_id,
            )),
            "mcp" => Box::new(nekocode_mcp::McpMiddleware::new(
                nekocode_mcp::config::McpConfig::from_value(&spec.config),
            )),
            "skills" => Box::new(nekocode_skills::SkillsMiddleware::new(
                nekocode_skills::SkillsConfig::from_value(&spec.config),
                self.skills_dir.clone(),
            )),
            other => {
                tracing::warn!("unknown middleware in subagent spec: {other}; skipping");
                Box::new(NoopMiddleware)
            }
        }
    }
}

struct NoopMiddleware;
#[async_trait::async_trait]
impl Middleware for NoopMiddleware {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        _: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
```

(The `NoopMiddleware` is defined here, not in the subagent crate. Remove the reference to `nekocode_subagent::tool::spawn_subagent::NoopMiddleware` from the first version.)

- [ ] **Step 3: Add `provider` field to `MiddlewareBuildContext`**

In `crates/nekocode/src/api/thread/mod.rs`, edit the `MiddlewareBuildContext` struct (around line 27-34) to add a `provider` field:

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

- [ ] **Step 4: Add the `"subagent"` arm to `build_middlewares`**

In `crates/nekocode/src/api/thread/mod.rs`, inside `build_middlewares`'s `match i.name.as_str()`, after the `"subthread"` arm (around line 93), add:

```rust
            "subagent" => {
                let cfg = nekocode_subagent::SubagentConfig::from_value(&i.config);
                // Build specs from the parent's enabled middleware rows,
                // excluding "subagent" itself (prevents recursive self-
                // registration; nesting is governed by depth+max_depth, not
                // by omission here).
                let specs: Vec<nekocode_core::middleware::MiddlewareSpec> = middleware_rows
                    .iter()
                    .filter(|r| r.enabled && r.name != "subagent")
                    .map(|r| nekocode_core::middleware::MiddlewareSpec {
                        name: r.name.clone(),
                        config: r.config.clone(),
                    })
                    .collect();
                let skills_dir = {
                    let config = ctx.config.read().await;
                    std::path::PathBuf::from(config.skills.directory.clone())
                };
                let factory = std::sync::Arc::new(
                    crate::api::thread::subagent_factory::ApiSubagentMiddlewareFactory {
                        db: ctx.db.clone(),
                        skills_dir,
                    },
                );
                middlewares.push(Box::new(nekocode_subagent::SubagentMiddleware::new(
                    specs,
                    factory,
                    ctx.provider.clone(),
                    ctx.config.clone(),
                    ctx.extensions.clone(),
                    ctx.db.clone(),
                    ctx.working_directory.clone(),
                    cfg,
                    0,    // depth = 0 for the top-level parent
                    true, // allow_nested = true at the root: the top-level
                          // thread is not itself a subagent, so it is always
                          // permitted to spawn its first-level subagents.
                          // The depth gate + each child's own profile
                          // allow_nested bound further nesting.
                )));
            }
```

- [ ] **Step 5: Pass `provider` into `MiddlewareBuildContext` at both construction sites**

In `crates/nekocode/src/api/thread/activate.rs`, the `MiddlewareBuildContext { … }` literal (around line 57-64) — add `provider: provider.clone(),` as the last field. The `provider` variable is already in scope (built at line 44).

In `crates/nekocode/src/api/thread/subthread_activator.rs`, the `MiddlewareBuildContext { … }` literal inside `activate` (around line 67-74) — add `provider: provider.clone(),`. The `provider` variable is in scope (built at line 62).

- [ ] **Step 6: Fix the stale comments**

In `crates/nekocode/src/api/thread/activate.rs` (around line 41-43), the comment reads:
```
// Build the provider once and share it via Arc — both the middleware-build
// context (for the subagent middleware) and the Agent struct itself need
// the same provider instance.
```
Rewrite to:
```
// Build the provider once and share it via Arc — both the middleware-build
// context (for the subthread and subagent middlewares) and the Agent struct
// itself need the same provider instance.
```

In `crates/nekocode/src/api/thread/subthread_activator.rs` (around line 45-48), the comment reads:
```
// Build the provider once and share it via Arc — both the
// middleware-build context (for the subagent middleware) and the
// Agent struct itself need the same provider instance.
```
Rewrite to:
```
// Build the provider once and share it via Arc — both the
// middleware-build context (for the subthread and subagent middlewares)
// and the Agent struct itself need the same provider instance.
```

- [ ] **Step 7: Add `abort_subagent_tasks` to `delete.rs`**

In `crates/nekocode/src/api/thread/delete.rs`, add a new function mirroring `abort_subthread_tasks` (which is already in the file). Place it right after `abort_subthread_tasks`:

```rust
/// Read a thread's per-parent `SubagentRegistry` from its activated `Agent`'s
/// extensions (if any) and abort every in-flight subagent background task it
/// owns. No-op when the thread isn't activated. No DB cascade — subagent
/// state is purely in-memory.
async fn abort_subagent_tasks(
    active_threads: &dashmap::DashMap<u64, Arc<tokio::sync::RwLock<nekocode_core::agent::Agent>>>,
    thread_id: u64,
) {
    let registry: Option<Arc<nekocode_subagent::SubagentRegistry>> =
        if let Some(agent_entry) = active_threads.get(&thread_id) {
            let agent = agent_entry.value().read().await;
            agent
                .extensions
                .get(nekocode_subagent::SUBAGENT_EXTENSION_KEY)
                .and_then(|b| {
                    b.downcast_ref::<Arc<nekocode_subagent::SubagentRegistry>>()
                        .cloned()
                })
        } else {
            None
        };

    if let Some(registry) = registry {
        let _aborted = registry.abort_all_and_clear();
    }
}
```

Then find the call site of `abort_subthread_tasks` inside `delete_threads_cascade` and add `abort_subagent_tasks(active_threads, thread_id).await;` right after it.

- [ ] **Step 8: Verify the whole workspace builds and existing tests pass**

Run: `cargo build && cargo test -p nekocode`
Expected: build succeeds; all existing nekocode tests pass (the `MiddlewareBuildContext` field addition is the only change touching existing code, and both construction sites were updated in Step 5).

- [ ] **Step 9: Commit**

```bash
git add crates/nekocode/Cargo.toml crates/nekocode/src/api/thread/
git commit -m "feat(api): integrate nekocode-subagent (factory, build_middlewares arm, cascade cleanup)"
```

---

## Task 9: Final full-workspace verification

**Files:** none (verification only)

- [ ] **Step 1: Build the whole workspace**

Run: `cargo build`
Expected: succeeds with no warnings related to subagent code.

- [ ] **Step 2: Run the whole workspace test suite**

Run: `cargo test`
Expected: all tests pass — nekocode-core (incl. `run_loop` tests), nekocode-subagent (config, registry, profile, runner, integration), and nekocode (existing API tests).

- [ ] **Step 3: Run clippy on the new crate**

Run: `cargo clippy -p nekocode-subagent -- -D warnings`
Expected: no warnings (fix any that appear — common ones: unused `Duration` import in `tool/mod.rs`, unused `_ensure_duration_used` helper — remove the helper and the `#[allow(dead_code)]` once wait tools use `Duration`).

- [ ] **Step 4: Final commit (if any cleanup was needed)**

```bash
git add -A
git commit -m "chore(subagent): clippy cleanup"
```

If no cleanup was needed, this step is a no-op.

---

## Self-Review Notes

**Spec coverage:**
- Architecture (crate, deps, factory trait): Tasks 1-2, 6, 8 ✓
- Profile format + layered load: Task 4 ✓
- Registry/state/run-state: Task 3 ✓
- 6 tools: Task 6 ✓
- Middleware + API integration: Tasks 6, 8 ✓
- Error handling: covered via `ToolError` returns in each tool (Task 6) + runner error path (Task 5) ✓
- Testing strategy (Tiers 1-3): Tasks 3 (Tier 1), 5 (Tier 2), 7 (Tier 3) ✓; Tier 4 (API manual smoke) is out of automated scope per spec ✓
- Implementation phases (0-5): Tasks 1 (Phase 0), 2-3 (Phase 1), 4 (Phase 2), 5 (Phase 3), 6-7 (Phase 4), 8 (Phase 5) ✓
- Cascade cleanup: Task 8 Step 7 ✓

**Placeholder scan:** No "TBD"/"TODO"/"implement later" in the actionable steps. The "see Step 2 correction" notes are inline corrections, not deferred work.

**Type consistency:**
- `SubagentContext` fields match across Tasks 6 (definition), 7 (test construction), 8 (API usage): `registry`, `specs`, `factory`, `parent_provider`, `parent_config`, `parent_working_directory`, `parent_db`, `catalog`, `depth`, `max_depth`, `allow_nested` ✓
- `SubagentMiddleware::new` signature is consistent after the Step 8 correction (adds `parent_db`, `parent_working_directory`); Task 8's call site matches ✓
- `SubagentMiddlewareFactory::build` is sync (`fn`, not `async fn`) in Tasks 2, 6, 7, 8 ✓
- `SubagentRunState::name()` added in Task 3, used in Tasks 6 (inspect/read/wait) ✓
- `ProfileCatalog { profiles }` field is `pub`, used in Task 7's direct construction ✓
