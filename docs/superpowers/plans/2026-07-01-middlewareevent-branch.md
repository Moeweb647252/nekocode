# MiddlewareEvent Branch Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `MiddlewareEvent` branch to `AgentEventType` and relay a subagent's `AgentEvent` stream out through it, with hard cascade abort so subagents never outlive their parent turn.

**Architecture:** New `AgentEventSink` (tx + shared `Arc<AtomicUsize>`) replaces the bare `UnboundedSender` in `run_loop`, giving all producers unique monotonic indices. `Middleware::before_generate` gains a narrow `&UnboundedSender<MiddlewareEvent>`; `run_loop` runs a merge relay that wraps each `MiddlewareEvent` into a uniquely-indexed `AgentEvent`. `spawn_subagent`'s drained channel becomes a relay that serializes each child `AgentEvent` into a `MiddlewareEvent`. A new `on_turn_end` trait hook + `CancellationToken` cascade abort subagents when the parent turn ends.

**Tech Stack:** Rust workspace (`nekocode-core`, `nekocode-subagent`, `nekocode-subthread`, `nekocode` API crate), `tokio` mpsc, `tokio-util::sync::CancellationToken`, `serde_json`, TypeScript webui mirror.

**Spec:** `docs/superpowers/specs/2026-07-01-middlewareevent-branch-design.md`

---

## File Structure

**New:**
- `crates/nekocode-core/src/agent/sink.rs` — `AgentEventSink` (tx + shared atomic index). One responsibility: unique-index send onto the parent stream.

**Modified (core):**
- `crates/nekocode-core/src/agent/mod.rs` — `AgentEventType::MiddlewareEvent` variant + `MiddlewareEvent` struct; re-export `AgentEventSink`.
- `crates/nekocode-core/src/agent/new_agent.rs` — `run_loop` takes `AgentEventSink`; `Agent::send` via sink; build `mev_tx` + merge relay; pass `&mev_tx` to `before_generate`; call `on_turn_end` before return.
- `crates/nekocode-core/src/middleware.rs` — `before_generate` gains `&UnboundedSender<MiddlewareEvent>`; add default `on_turn_end`.
- `crates/nekocode-core/src/agent/test_mocks.rs` — mock middlewares accept new param; add a `RelayMiddleware` test mock that emits a `MiddlewareEvent`.

**Modified (subagent):**
- `crates/nekocode-subagent/src/middleware.rs` — `before_generate` passes `mev_tx.clone()` into `SpawnSubagentTool::new`; impl `on_turn_end` → `abort_all_and_clear`.
- `crates/nekocode-subagent/src/tool/spawn_subagent.rs` — `SpawnSubagentTool` gains `mev_tx` field; drained → relay pattern; clone `cancel_token` into child.
- `crates/nekocode-subagent/src/registry.rs` — `SubagentState.cancel`; `abort_all_and_clear` cancels first.
- `crates/nekocode-subagent/src/runner.rs` — `run_subagent` takes `AgentEventSink` (+ cancel token) instead of bare sender.
- `crates/nekocode-subagent/tests/integration.rs` — update 6 `SpawnSubagentTool::new` sites; add relay test + cascade test.

**Modified (other middleware impls — mechanical):**
- `crates/nekocode-shell/src/lib.rs`, `crates/nekocode-file/src/lib.rs`, `crates/nekocode-mcp/src/lib.rs`, `crates/nekocode-skills/src/lib.rs`, `crates/nekocode-subthread/src/middleware.rs`, `crates/nekocode/src/api/thread/subagent_factory.rs` (`NoopMiddleware`), `crates/nekocode-subagent/tests/integration.rs` (`NoopMiddleware`) — accept new `before_generate` param (ignore).

**Modified (callers):**
- `crates/nekocode/src/api/generate/stream_generate.rs` — wrap `tx` in `AgentEventSink::new(tx)`.
- `crates/nekocode/src/api/thread/subthread_controller.rs` + `crates/nekocode-subthread/src/controller.rs` — `ThreadController::run` takes `AgentEventSink`; wrap sender.

**Modified (webui):**
- `webui/src/api/types.ts` — add `middlewareEvent` branch to `AgentEventType`.

---

## Task 1: Add `MiddlewareEvent` type + TS mirror

**Files:**
- Modify: `crates/nekocode-core/src/agent/mod.rs:19-24`
- Modify: `webui/src/api/types.ts:148`

- [ ] **Step 1: Add the type in nekocode-core**

Edit `crates/nekocode-core/src/agent/mod.rs`. Replace the `AgentEventType` enum and add the `MiddlewareEvent` struct. Also add `use std::borrow::Cow;` to the imports at the top of the file (the existing `use` block around lines 5-10).

```rust
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum AgentEventType {
    StreamEvent(StreamEvent),
    /// An event relayed out of a child generation by a middleware
    /// (subagent today; reusable by subthread / others later). The
    /// payload is an opaque JSON value + a source-published type tag,
    /// so this enum never has to know the internal shape of each
    /// source's events.
    MiddlewareEvent(MiddlewareEvent),
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MiddlewareEvent {
    /// Originating middleware kind, e.g. "subagent".
    pub source: Cow<'static, str>,
    /// Stable id of the originating child (subagent agent_id).
    pub source_id: u64,
    /// Source-published type tag for `data`, e.g. "agentEvent".
    pub event_type: String,
    /// Opaque payload. For subagent: the serialized child `AgentEvent`.
    pub data: serde_json::Value,
}
```

- [ ] **Step 2: Add a serde round-trip test**

Append to `crates/nekocode-core/src/agent/mod.rs` (a `#[cfg(test)]` module at the end of the file):

```rust
#[cfg(test)]
mod middleware_event_tests {
    use super::*;
    use nekocode_types::generate::{StreamEvent, StreamEventData};

    #[test]
    fn middleware_event_serde_round_trip() {
        let child = AgentEvent {
            index: 7,
            data: AgentEventType::StreamEvent(StreamEvent {
                data: StreamEventData::TurnEnd,
                created_at: jiff::Timestamp::now(),
            }),
        };
        let mev = MiddlewareEvent {
            source: Cow::Borrowed("subagent"),
            source_id: 42,
            event_type: "agentEvent".into(),
            data: serde_json::to_value(&child).unwrap(),
        };
        let wrapped = AgentEvent {
            index: 3,
            data: AgentEventType::MiddlewareEvent(mev),
        };
        let json = serde_json::to_value(&wrapped).unwrap();
        assert_eq!(json["index"], 3);
        assert_eq!(json["data"]["type"], "middlewareEvent");
        assert_eq!(json["data"]["source"], "subagent");
        assert_eq!(json["data"]["sourceId"], 42);
        assert_eq!(json["data"]["eventType"], "agentEvent");
        assert_eq!(json["data"]["data"]["index"], 7);
    }
}
```

- [ ] **Step 3: Run the test**

Run: `cargo test -p nekocode-core agent::middleware_event_tests -- --nocapture`
Expected: PASS (1 test).

- [ ] **Step 4: Add the TS mirror**

Edit `webui/src/api/types.ts:148`. Replace the single-variant union:

```ts
export type AgentEventType =
  | { type: 'streamEvent'; data: RawStreamEventData; createdAt: string }
  | { type: 'middlewareEvent'; source: string; sourceId: number; eventType: string; data: unknown }
```

- [ ] **Step 5: Verify webui type-checks**

Run: `cd webui && npx tsc --noEmit` (or the repo's existing type-check command). Expected: no new errors.

- [ ] **Step 6: Commit**

```bash
git add crates/nekocode-core/src/agent/mod.rs webui/src/api/types.ts
git commit -m "feat(core): add MiddlewareEvent branch to AgentEventType"
```

---

## Task 2: Add `AgentEventSink`

**Files:**
- Create: `crates/nekocode-core/src/agent/sink.rs`
- Modify: `crates/nekocode-core/src/agent/mod.rs` (add `pub mod sink;` + re-export)

- [ ] **Step 1: Write the failing test**

Create `crates/nekocode-core/src/agent/sink.rs` with the test first (implementation will be added in Step 3 so the test fails to compile, which counts as failing):

```rust
//! Cheaply cloneable handle onto the parent's outbound stream. `index`
//! is a shared atomic so every producer (the parent's own `Agent::send`
//! and any merge relay forwarding `MiddlewareEvent`s) allocates a
//! unique, contiguous monotonic index — required once the stream has
//! more than one producer.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc::UnboundedSender;

use crate::agent::error::AgentError;
use crate::agent::{AgentEvent, AgentEventType};

#[derive(Clone)]
pub struct AgentEventSink {
    tx: UnboundedSender<AgentEvent>,
    index: Arc<AtomicUsize>,
}

impl AgentEventSink {
    pub fn new(tx: UnboundedSender<AgentEvent>) -> Self {
        Self { tx, index: Arc::new(AtomicUsize::new(0)) }
    }

    /// Allocate the next unique index and send. Fails only on client
    /// disconnect (the receiver gone).
    pub fn send(&self, data: AgentEventType) -> Result<(), AgentError> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(AgentEvent { index: idx, data })
            .map_err(|e| AgentError::Other(anyhow::anyhow!("error sending agent event {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekocode_types::generate::{StreamEvent, StreamEventData};
    use tokio::sync::mpsc;

    fn turn_end() -> AgentEventType {
        AgentEventType::StreamEvent(StreamEvent {
            data: StreamEventData::TurnEnd,
            created_at: jiff::Timestamp::now(),
        })
    }

    #[tokio::test]
    async fn send_allocates_unique_contiguous_indices_across_clones() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let sink = AgentEventSink::new(tx);
        // Two producers sharing one sink via clone.
        let sink_b = sink.clone();

        sink.send(turn_end()).unwrap();        // index 0
        sink_b.send(turn_end()).unwrap();      // index 1
        sink.send(turn_end()).unwrap();        // index 2

        let mut got = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            got.push(ev.index);
        }
        assert_eq!(got, vec![0, 1, 2]);
    }
}
```

- [ ] **Step 2: Register the module**

Edit `crates/nekocode-core/src/agent/mod.rs`. Add after `pub mod new_agent;` (line 2):

```rust
pub mod sink;
```

And add to the re-exports near the `Agent` struct (after the `use crate::middleware::Middleware;` line, add a `pub use sink::AgentEventSink;`).

- [ ] **Step 3: Run the test to verify it passes**

Run: `cargo test -p nekocode-core agent::sink -- --nocapture`
Expected: PASS (1 test). (Since Step 1 wrote impl + test together, this verifies both compile and behave.)

- [ ] **Step 4: Commit**

```bash
git add crates/nekocode-core/src/agent/sink.rs crates/nekocode-core/src/agent/mod.rs
git commit -m "feat(core): add AgentEventSink with shared atomic index"
```

---

## Task 3: Extend `Middleware` trait signature + update all impls

This is a signature-level change that breaks all `Middleware` impls at once; the trait change + every impl update land in one commit so the workspace stays buildable.

**Files:**
- Modify: `crates/nekocode-core/src/middleware.rs:29-46`
- Modify (mechanical — accept new param, ignore): `crates/nekocode-shell/src/lib.rs:64`, `crates/nekocode-file/src/lib.rs:33`, `crates/nekocode-mcp/src/lib.rs:94`, `crates/nekocode-skills/src/lib.rs:66`, `crates/nekocode-subthread/src/middleware.rs:65`, `crates/nekocode/src/api/thread/subagent_factory.rs:54`, `crates/nekocode-subagent/tests/integration.rs:35`, `crates/nekocode-core/src/agent/test_mocks.rs:119,143,172`
- Modify: `crates/nekocode-core/src/agent/new_agent.rs:81` (call site — add `&mev_tx`; `mev_tx` created in Task 5, so here pass a placeholder channel — see Step 3)

- [ ] **Step 1: Extend the trait**

Edit `crates/nekocode-core/src/middleware.rs`. Replace the trait block:

```rust
#[async_trait]
pub trait Middleware: Send + Sync {
    async fn before_generate(
        &self,
        _: &mut GenerateRequest,
        _: &mut ToolRegistry,
        // Narrow capability: a middleware can only emit MiddlewareEvent,
        // never a forged StreamEvent. Index allocation / wrapping is done
        // by a merge relay inside run_loop, not by the middleware.
        _: &tokio::sync::mpsc::UnboundedSender<crate::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(&self, _: &GenerateResponse, _: &mut AgentControlFlow)
        -> Result<(), anyhow::Error> { Ok(()) }

    /// Called once at the end of the turn (both Ok and Err paths) before
    /// `run_loop` returns. Default is a no-op; middlewares that spawn
    /// detached work (e.g. subagent) override this to cascade-abort it.
    async fn on_turn_end(&self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
```

- [ ] **Step 2: Update every `impl Middleware` to accept the new param**

For each implementor below, add a third ignored parameter `_: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>` (or `crate::agent::MiddlewareEvent` when inside nekocode-core) to its `before_generate` signature. Bodies are unchanged. The default `on_turn_end` covers all of them, so do NOT add `on_turn_end` here.

Implementors & their `before_generate` signature lines:
- `crates/nekocode-shell/src/lib.rs` (`impl Middleware for Shell`, ~line 64)
- `crates/nekocode-file/src/lib.rs` (`impl Middleware for ToolMiddleware`, ~line 33)
- `crates/nekocode-mcp/src/lib.rs` (`impl Middleware for McpMiddleware`, ~line 94)
- `crates/nekocode-skills/src/lib.rs` (`impl Middleware for SkillsMiddleware`, ~line 66)
- `crates/nekocode-subthread/src/middleware.rs` (`impl Middleware for SubthreadMiddleware`, ~line 65)
- `crates/nekocode/src/api/thread/subagent_factory.rs` (`impl Middleware for NoopMiddleware`, ~line 54)
- `crates/nekocode-subagent/tests/integration.rs` (`impl Middleware for NoopMiddleware`, ~line 35)
- `crates/nekocode-core/src/agent/test_mocks.rs` — `EchoMiddleware` (~119), `InjectMiddleware` (~143), `OneShotRegenerateMiddleware` (~172)

Example shape (apply to each):

```rust
async fn before_generate(
    &self,
    _: &mut nekocode_core::types::GenerateRequest,
    _: &mut nekocode_types::tool::ToolRegistry,
    _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
) -> Result<(), anyhow::Error> {
    Ok(())
}
```

(Inside `nekocode-core`/`test_mocks.rs` use `crate::agent::MiddlewareEvent` instead of `nekocode_core::agent::MiddlewareEvent`.)

- [ ] **Step 3: Make the call site compile (temporary)**

The `before_generate` call at `crates/nekocode-core/src/agent/new_agent.rs:81` now needs a third arg. Task 5 builds the real `mev_tx`; for now create a throwaway channel at the top of `run_loop` so the workspace compiles. Add near the top of `run_loop` (after `let mut index = 0usize;` at line 61):

```rust
// TEMPORARY: replaced by the real mev_tx channel + merge relay in Task 5.
let (mev_tx, _mev_rx) = tokio::sync::mpsc::unbounded_channel();
```

And update the call site (line 81):

```rust
middleware
    .before_generate(&mut request, &mut tool_registry, &mev_tx)
    .await?;
```

- [ ] **Step 4: Build the whole workspace**

Run: `cargo build --workspace`
Expected: compiles cleanly (the `on_turn_end` calls don't exist yet; that's Task 5).

- [ ] **Step 5: Run the full test suite to confirm no regression**

Run: `cargo test --workspace`
Expected: all existing tests PASS (signature change is behavior-neutral so far).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor(core): extend Middleware trait with mev_tx + on_turn_end"
```

---

## Task 4: Refactor `run_loop` + `Agent::send` to `AgentEventSink`

**Files:**
- Modify: `crates/nekocode-core/src/agent/new_agent.rs:28-33, 81-83, 91-97, 219, 234-240, 252-268, 324, 357, 386, 431`

- [ ] **Step 1: Change `run_loop` signature**

Edit `crates/nekocode-core/src/agent/new_agent.rs`. Change the signature at line 28-33:

```rust
pub async fn run_loop(
    &self,
    input: Vec<MessageContent>,
    old_turns: Vec<Turn>,
    sink: crate::agent::AgentEventSink,
) -> Result<Turn, Turn> {
```

Remove the `let mut index = 0usize;` line (line 61) — the sink holds the shared atomic. Remove the temporary `mev_tx`/`_mev_rx` from Task 3 Step 3 (Task 5 adds the real one).

- [ ] **Step 2: Refactor `Agent::send` to use the sink**

Replace the `send` helper (lines 250-268):

```rust
/// Send a stream event as an [`AgentEvent`], allocating the index from
/// the shared sink. A closed client channel fails the run.
fn send(
    sink: &crate::agent::AgentEventSink,
    data: StreamEventData,
) -> Result<(), AgentError> {
    sink.send(AgentEventType::StreamEvent(StreamEvent {
        data,
        created_at: jiff::Timestamp::now(),
    }))
}
```

- [ ] **Step 3: Update all `Self::send` / `sender.send` call sites inside `run_loop`**

- Lines 91-97 (`Self::send(&sender, &mut index, StreamEventData::MessageStart { ... })`) → `Self::send(&sink, StreamEventData::MessageStart { ... })?`
- Every other `Self::send(&sender, &mut index, …)` inside `run_loop` (search the function body): drop the `&mut index` argument.
- Line 219 (`Self::send(&sender, &mut index, StreamEventData::TurnEnd)?`) → `Self::send(&sink, StreamEventData::TurnEnd)?`
- Lines 234-240 (error path `sender.send(AgentEvent { index, data: AgentEventType::StreamEvent(...) })`) → replace with `let _ = sink.send(AgentEventType::StreamEvent(StreamEvent { data: StreamEventData::MessageEnd(StopReason::Error(e.to_string())), created_at: jiff::Timestamp::now() }));`

- [ ] **Step 4: Update the `before_generate` call site**

Line 81 still references `&mev_tx` from Task 3. Since Task 3's temporary channel was removed in Step 1, re-add the temporary channel at the top of `run_loop` (same as Task 3 Step 3) so it compiles; Task 5 replaces it with the real channel + relay. Keep the call as `before_generate(&mut request, &mut tool_registry, &mev_tx)`.

- [ ] **Step 5: Update core-internal `run_loop` test call sites**

The four test call sites (lines 324, 357, 386, 431) pass `tx` (a bare sender). Wrap each: replace `tx` with `crate::agent::AgentEventSink::new(tx)`. Example (line 324):

```rust
.run_loop(text_input("hi"), Vec::new(), crate::agent::AgentEventSink::new(tx))
```

Apply to all four.

- [ ] **Step 6: Update external `run_loop` callers (temporary — proper wrap happens in Task 6)**

- `crates/nekocode/src/api/generate/stream_generate.rs:72`: change `tx` → `nekocode_core::agent::AgentEventSink::new(tx)`.
- `crates/nekocode-subagent/src/runner.rs:21`: the `run_subagent` signature still takes a bare sender (Task 6 changes it); for now wrap at the call site inside `run_subagent`: `.run_loop(vec![MessageContent::Text { content: prompt }], Vec::new(), nekocode_core::agent::AgentEventSink::new(sender))`.
- `crates/nekocode/src/api/thread/subthread_controller.rs:112`: wrap `sender` → `nekocode_core::agent::AgentEventSink::new(sender)` at the call site (Task 6 cleans up the trait).

- [ ] **Step 7: Build + test core**

Run: `cargo test -p nekocode-core`
Expected: PASS (all existing agent tests still pass — behavior unchanged).

- [ ] **Step 8: Build the whole workspace**

Run: `cargo build --workspace`
Expected: compiles.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor(core): run_loop + Agent::send go through AgentEventSink"
```

---

## Task 5: `mev_tx` channel + merge relay + `on_turn_end` dispatch in `run_loop`

**Files:**
- Modify: `crates/nekocode-core/src/agent/new_agent.rs` (run_loop top + exit path + `before_generate` call)
- Modify: `crates/nekocode-core/src/agent/test_mocks.rs` (add `RelayMiddleware`)

- [ ] **Step 1: Write the failing test**

Add a mock middleware that emits a `MiddlewareEvent` during `before_generate`, plus a `run_loop` test asserting the wrapped event reaches the parent stream with a unique index coexisting with `StreamEvent`s. Append to `crates/nekocode-core/src/agent/test_mocks.rs`:

```rust
/// Emits one MiddlewareEvent into the mev_tx it receives in
/// before_generate. Used to test run_loop's merge relay.
pub struct RelayMiddleware;

#[async_trait]
impl Middleware for RelayMiddleware {
    async fn before_generate(
        &self,
        _: &mut GenerateRequest,
        _: &mut ToolRegistry,
        mev_tx: &tokio::sync::mpsc::UnboundedSender<crate::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        let _ = mev_tx.send(crate::agent::MiddlewareEvent {
            source: std::borrow::Cow::Borrowed("test"),
            source_id: 1,
            event_type: "ping".into(),
            data: serde_json::json!({ "hello": "world" }),
        });
        Ok(())
    }
}
```

Then add the run_loop test in `crates/nekocode-core/src/agent/new_agent.rs`'s `tests` module. It uses the existing `make_agent(provider, middlewares)`, `text_input`, `MockProvider`, and `text_msg` helpers already defined in that module:

```rust
#[tokio::test]
async fn merge_relay_forwards_middleware_event_with_unique_index() {
    use crate::agent::test_mocks::RelayMiddleware;
    use nekocode_types::generate::StreamEventData;

    let agent = make_agent(
        Arc::new(MockProvider::new(vec![text_msg("ok")])),
        vec![Box::new(RelayMiddleware)],
    )
    .await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let sink = crate::agent::AgentEventSink::new(tx);
    let _turn = agent
        .run_loop(text_input("hi"), Vec::new(), sink)
        .await
        .expect("turn ok");

    let mut events = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        events.push(ev);
    }
    // The merge-relayed MiddlewareEvent must be present.
    let mev = events
        .iter()
        .find(|e| matches!(e.data, crate::agent::AgentEventType::MiddlewareEvent(_)))
        .expect("middleware event relayed");
    let _ = mev;
    // Indices across all events must be unique and contiguous 0..n.
    let mut idx: Vec<usize> = events.iter().map(|e| e.index).collect();
    idx.sort();
    let expected: Vec<usize> = (0..events.len()).collect();
    assert_eq!(idx, expected, "indices unique & contiguous");
    // And a StreamEvent (MessageEnd) coexists.
    assert!(events.iter().any(|e| matches!(
        e.data,
        crate::agent::AgentEventType::StreamEvent(se)
            if matches!(se.data, StreamEventData::MessageEnd(_))
    )));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nekocode-core merge_relay_forwards_middleware_event -- --nocapture`
Expected: FAIL — no `MiddlewareEvent` is relayed yet (the temporary `mev_tx` from Task 3/4 is a dead-end channel). The `find` panics "middleware event relayed".

- [ ] **Step 3: Implement the mev_tx channel + merge relay**

At the top of `run_loop` (replacing the temporary `mev_tx` from Task 4 Step 4), add:

```rust
// Channel for middlewares to enqueue MiddlewareEvents; a merge relay
// wraps each into a uniquely-indexed AgentEvent on the parent stream.
let (mev_tx, mev_rx) = tokio::sync::mpsc::unbounded_channel::<crate::agent::MiddlewareEvent>();
let relay_sink = sink.clone();
let merge_relay = tokio::spawn(async move {
    let mut mev_rx = mev_rx;
    while let Some(mev) = mev_rx.recv().await {
        // send failure (client gone) just stops relaying
        let _ = relay_sink.send(crate::agent::AgentEventType::MiddlewareEvent(mev));
    }
});
```

The `before_generate` call stays `before_generate(&mut request, &mut tool_registry, &mev_tx)`.

- [ ] **Step 4: Call `on_turn_end` + abort the merge relay on every exit path**

The run_loop body is wrapped in `let result: Result<Turn, AgentError> = async { … }.await;` followed by a `match result`. After the `match` (so it runs on both Ok and Err), before the function returns, add:

```rust
// Cascade-abort any detached work middlewares spawned this turn
// (subagent aborts its children here), then stop the merge relay.
for middleware in self.middlewares.iter() {
    let _ = middleware.on_turn_end().await;
}
merge_relay.abort();
```

(Placed after the `match result { … }` block, immediately before the final `Ok`/`Err` returns — i.e. both branches flow through it. If control flow makes a single trailing spot awkward, duplicate the two lines at the end of each `match` arm instead.)

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo test -p nekocode-core merge_relay_forwards_middleware_event -- --nocapture`
Expected: PASS.

- [ ] **Step 6: Run the whole core suite**

Run: `cargo test -p nekocode-core`
Expected: PASS (no regression; `on_turn_end` is a no-op for existing mocks).

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "feat(core): mev_tx merge relay + on_turn_end dispatch in run_loop"
```

---

## Task 6: Update `run_loop` callers to pass `AgentEventSink` cleanly

Task 4 wrapped callers inline; this task moves the wrap to the right boundary (`run_subagent`, `ThreadController::run`) so callers pass a sink, not a bare sender.

**Files:**
- Modify: `crates/nekocode-subagent/src/runner.rs:13-26`
- Modify: `crates/nekocode-subthread/src/controller.rs:38-58` (trait)
- Modify: `crates/nekocode/src/api/thread/subthread_controller.rs:101-130` (impl + call site)
- Modify: `crates/nekocode/src/api/generate/stream_generate.rs:72` (already wrapped; verify)

- [ ] **Step 1: Change `run_subagent` to take `AgentEventSink`**

Edit `crates/nekocode-subagent/src/runner.rs`. Change the signature (lines 13-19) and the `run_loop` call (lines 20-26):

```rust
pub async fn run_subagent(
    agent_id: u64,
    child: Agent,
    prompt: String,
    registry: Arc<SubagentRegistry>,
    sink: nekocode_core::agent::AgentEventSink,
) {
    let result = child
        .run_loop(
            vec![MessageContent::Text { content: prompt }],
            Vec::new(),
            sink,
        )
        .await;
    // …unchanged match…
}
```

Update the import on line 3: `use nekocode_core::agent::{Agent, AgentEvent, AgentEventSink};` (drop `AgentEvent` if now unused). Update the two test call sites in this file (lines 137, 152): they do `run_subagent(id, child, "do thing".into(), registry.clone(), tx)` — change `tx` to `nekocode_core::agent::AgentEventSink::new(tx)`.

- [ ] **Step 2: Change `ThreadController::run` trait to take `AgentEventSink`**

Edit `crates/nekocode-subthread/src/controller.rs`. Change the trait method signature (around line 46):

```rust
async fn run(
    &self,
    agent: Agent,
    prompt: String,
    sink: nekocode_core::agent::AgentEventSink,
) -> Result<Turn, Turn>;
```

Update imports as needed (`nekocode_core::agent::AgentEventSink`).

- [ ] **Step 3: Update `ApiThreadController::run` impl + call site**

Edit `crates/nekocode/src/api/thread/subthread_controller.rs`. The impl signature (line 105) takes `sink: nekocode_core::agent::AgentEventSink` instead of `sender: UnboundedSender<AgentEvent>`, and the `run_loop` call (line 112) passes `sink` directly (drop the inline `AgentEventSink::new` wrapper added in Task 4 Step 6).

- [ ] **Step 4: Verify `stream_generate.rs` already passes a sink**

`crates/nekocode/src/api/generate/stream_generate.rs:72` should read `.run_loop(vec![…], old_turns, nekocode_core::agent::AgentEventSink::new(tx))` (from Task 4). Confirm; no change.

- [ ] **Step 5: Build + test the workspace**

Run: `cargo build --workspace && cargo test --workspace`
Expected: compiles; all tests PASS.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: callers pass AgentEventSink to run_loop/run_subagent"
```

---

## Task 7: `SubagentMiddleware` wires `mev_tx` into `SpawnSubagentTool` + impls `on_turn_end`

**Files:**
- Modify: `crates/nekocode-subagent/src/tool/spawn_subagent.rs:12-20` (struct + `new`)
- Modify: `crates/nekocode-subagent/src/middleware.rs:82-98` (before_generate + on_turn_end)
- Modify: `crates/nekocode-subagent/tests/integration.rs` (6 `SpawnSubagentTool::new` sites)

- [ ] **Step 1: Give `SpawnSubagentTool` a `mev_tx` field**

Edit `crates/nekocode-subagent/src/tool/spawn_subagent.rs`. Replace the struct + constructor (lines 12-20):

```rust
pub struct SpawnSubagentTool {
    ctx: SubagentContext,
    mev_tx: tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
}

impl SpawnSubagentTool {
    pub fn new(
        ctx: SubagentContext,
        mev_tx: tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Self {
        Self { ctx, mev_tx }
    }
}
```

- [ ] **Step 2: Pass `mev_tx` from `SubagentMiddleware::before_generate` + impl `on_turn_end`**

Edit `crates/nekocode-subagent/src/middleware.rs`. Update `before_generate` (line 84-97) to pass `mev_tx.clone()` into the spawn tool, and add `on_turn_end`:

```rust
#[async_trait]
impl Middleware for SubagentMiddleware {
    async fn before_generate(
        &self,
        _request: &mut nekocode_core::types::GenerateRequest,
        registry: &mut ToolRegistry,
        mev_tx: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        let ctx = &self.ctx;
        registry.insert("spawn_subagent".into(), Arc::new(SpawnSubagentTool::new(ctx.clone(), mev_tx.clone())));
        registry.insert("inspect_subagent".into(), Arc::new(InspectSubagentTool::new(ctx.clone())));
        registry.insert("read_subagent".into(), Arc::new(ReadSubagentTool::new(ctx.clone())));
        registry.insert("wait_any_subagent".into(), Arc::new(WaitAnySubagentTool::new(ctx.clone())));
        registry.insert("wait_all_subagents".into(), Arc::new(WaitAllSubagentsTool::new(ctx.clone())));
        registry.insert("abort_subagent".into(), Arc::new(AbortSubagentTool::new(ctx.clone())));
        Ok(())
    }

    async fn on_turn_end(&self) -> Result<(), anyhow::Error> {
        // Parent turn is over: no subagent may outlive it.
        self.ctx.registry.abort_all_and_clear();
        Ok(())
    }
}
```

- [ ] **Step 3: Add a `dummy_mev_tx` helper + update the 6 test call sites**

Edit `crates/nekocode-subagent/tests/integration.rs`. Add a helper near the other helpers (after `make_ctx`):

```rust
/// A mev_tx whose receiver is kept alive so sends succeed; for tests
/// that don't assert on relayed events. (Receiver is leaked for the
/// test's lifetime — fine for a test.)
fn dummy_mev_tx() -> (
    tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    tokio::sync::mpsc::UnboundedReceiver<nekocode_core::agent::MiddlewareEvent>,
) {
    tokio::sync::mpsc::unbounded_channel()
}
```

Update each of the 6 `SpawnSubagentTool::new(ctx…)` call sites (lines 199, 241, 253, 265, 280, 295) to pass a mev_tx. For the five that don't test relay, use a dummy kept alive in a local:

```rust
let (mev_tx, _mev_rx) = dummy_mev_tx();
let spawn = SpawnSubagentTool::new(ctx.clone(), mev_tx);
```

(For `spawn_wait_read_lifecycle` at line 199 and `wait_any_timeout_against_pending_subagent` at line 295, use the same dummy pattern — Task 8 adds a dedicated relay test with a real reader.)

- [ ] **Step 4: Build + test subagent**

Run: `cargo test -p nekocode-subagent`
Expected: all existing tests PASS (signature updated, behavior unchanged).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(subagent): wire mev_tx into SpawnSubagentTool; on_turn_end aborts"
```

---

## Task 8: `spawn_subagent` relay: child `AgentEvent` → `MiddlewareEvent`

**Files:**
- Modify: `crates/nekocode-subagent/src/tool/spawn_subagent.rs:145-157`
- Modify: `crates/nekocode-subagent/src/runner.rs` (run_subagent now takes sink — done in Task 6; spawn_subagent must build a sink from `child_tx`)
- Test: `crates/nekocode-subagent/tests/integration.rs` (new test)

- [ ] **Step 1: Write the failing test**

Append to `crates/nekocode-subagent/tests/integration.rs`:

```rust
#[tokio::test]
async fn spawn_subagent_relays_child_events_as_middleware_event() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 1, db);
    let (mev_tx, mut mev_rx) = tokio::sync::mpsc::unbounded_channel();
    let spawn = SpawnSubagentTool::new(ctx.clone(), mev_tx);

    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();

    // Wait for the subagent to finish so the relay has flushed everything.
    let wait = WaitAnySubagentTool::new(ctx.clone());
    let _ = wait
        .call(serde_json::json!({ "agent_ids": [agent_id], "timeout": 5.0 }))
        .await
        .unwrap();

    let mut relayed = Vec::new();
    while let Ok(mev) = mev_rx.try_recv() {
        relayed.push(mev);
    }
    assert!(!relayed.is_empty(), "at least one child event relayed");
    for mev in &relayed {
        assert_eq!(mev.source, "subagent");
        assert_eq!(mev.source_id, agent_id);
        assert_eq!(mev.event_type, "agentEvent");
        assert!(mev.data.is_object(), "data is the serialized child AgentEvent");
    }
    // The child emits at least a MessageEnd; ensure it's in the relayed set.
    assert!(relayed.iter().any(|mev| {
        mev.data.get("data").and_then(|d| d.get("type")).and_then(|t| t.as_str())
            == Some("streamEvent")
    }));
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nekocode-subagent spawn_subagent_relays_child_events -- --nocapture`
Expected: FAIL — `relayed` is empty (the drain task still discards child events).

- [ ] **Step 3: Replace the drained-sender pattern with the relay**

Edit `crates/nekocode-subagent/src/tool/spawn_subagent.rs` lines 145-157. Replace:

```rust
// Relay pattern: a companion task wraps each child AgentEvent as a
// MiddlewareEvent and forwards it to the parent's mev_tx (which
// run_loop's merge relay turns into a uniquely-indexed AgentEvent on
// the parent stream). Replaces the old drain-and-discard task.
let (child_tx, child_rx) = mpsc::unbounded_channel();
let mev_tx = self.mev_tx.clone();
let relay_target_agent_id = agent_id;
let registry = self.ctx.registry.clone();

let handle = tokio::spawn(async move {
    let relay = tokio::spawn(async move {
        while let Some(child_event) = child_rx.recv().await {
            let mev = nekocode_core::agent::MiddlewareEvent {
                source: std::borrow::Cow::Borrowed("subagent"),
                source_id: relay_target_agent_id,
                event_type: "agentEvent".into(),
                data: serde_json::to_value(&child_event)
                    .unwrap_or(serde_json::Value::Null),
            };
            // Parent stream may have closed: send failure just stops relaying.
            let _ = mev_tx.send(mev);
        }
    });
    run_subagent(
        agent_id,
        child,
        prompt,
        registry,
        nekocode_core::agent::AgentEventSink::new(child_tx),
    )
    .await;
    // run_subagent returns → child run_loop dropped child_tx → relay ends.
    relay.await.ok();
});
```

Note: `run_subagent` now takes an `AgentEventSink` (Task 6), so wrap `child_tx` with `AgentEventSink::new`. The `mev_tx` import: `use tokio::sync::mpsc;` is already at the top of the file (line 6).

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p nekocode-subagent spawn_subagent_relays_child_events -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Run the full subagent suite**

Run: `cargo test -p nekocode-subagent`
Expected: all tests PASS (including the cascade test added in Task 9 — if not yet added, just this task's tests).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(subagent): relay child AgentEvent stream as MiddlewareEvent"
```

---

## Task 9: `CancellationToken` cascade abort

**Files:**
- Modify: `crates/nekocode-subagent/src/registry.rs:41-62, 142-155`
- Modify: `crates/nekocode-subagent/src/tool/spawn_subagent.rs` (clone cancel token; pass to `run_subagent`)
- Modify: `crates/nekocode-subagent/src/runner.rs` (accept cancel token; child run_loop respects it)
- Test: `crates/nekocode-subagent/tests/integration.rs`

> **Coupled edits:** Steps 5 and 6 change `run_subagent`'s signature and its caller together — the workspace won't compile between them. That's expected; both land in the Task 9 commit and the test run is Step 8 (after both).

- [ ] **Step 1: Write the failing test**

Append to `crates/nekocode-subagent/tests/integration.rs`:

```rust
#[tokio::test]
async fn abort_all_and_clear_cancels_child_token() {
    let registry = Arc::new(SubagentRegistry::new());
    let id = registry.allocate_running();
    let cancel = registry.cancel_token(id).expect("token present while running");
    assert!(!cancel.is_cancelled());
    let aborted = registry.abort_all_and_clear();
    assert_eq!(aborted, vec![id]);
    assert!(cancel.is_cancelled(), "cancel token fired by abort_all_and_clear");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p nekocode-subagent abort_all_and_clear_cancels_child_token -- --nocapture`
Expected: FAIL — `cancel_token` method doesn't exist yet (compile error).

- [ ] **Step 3: Add `cancel` to `SubagentState` + accessor + cancel-first `abort_all_and_clear`**

Edit `crates/nekocode-subagent/src/registry.rs`. Add the field to `SubagentState` (lines 41-50):

```rust
#[derive(Debug)]
pub struct SubagentState {
    pub agent_id: u64,
    pub run_state: SubagentRunState,
    pub task_handle: Option<JoinHandle<()>>,
    pub notify: Arc<Notify>,
    pub result: Arc<RwLock<Option<SubagentRunResult>>>,
    /// Cancelled by `abort_all_and_clear` so the child run_loop can bail
    /// at its next await and run its own `on_turn_end` (real recursion).
    pub cancel: Arc<tokio_util::sync::CancellationToken>,
}
```

Update `SubagentState::new` (lines 52-61) to initialize `cancel: Arc::new(tokio_util::sync::CancellationToken::new())`.

Add an accessor on `SubagentRegistry` (near `run_state`):

```rust
/// Snapshot the cancel token for a running subagent (for tests / cascade).
pub fn cancel_token(&self, agent_id: u64) -> Option<Arc<tokio_util::sync::CancellationToken>> {
    self.states.get(&agent_id).map(|s| s.cancel.clone())
}
```

Update `abort_all_and_clear` (lines 142-155) to cancel first. The complete new body:

```rust
pub fn abort_all_and_clear(&self) -> Vec<u64> {
    let mut aborted = Vec::new();
    // Cancel every child's run_loop first — gives each layer a chance to
    // bail at its next await and run its own on_turn_end →
    // abort_all_and_clear (real recursion across nesting depth) — then
    // fall back to JoinHandle::abort() as the hard guarantee.
    for entry in self.states.iter() {
        entry.cancel.cancel();
        aborted.push(entry.agent_id);
    }
    // Abort handles, then clear.
    for entry in self.states.iter() {
        if let Some(handle) = &entry.task_handle {
            handle.abort();
        }
    }
    self.states.clear();
    aborted
}
```

(This is the existing body plus the `entry.cancel.cancel()` line inside the first loop; the abort-then-clear mechanics are unchanged.)

- [ ] **Step 4: Run the unit test to verify it passes**

Run: `cargo test -p nekocode-subagent abort_all_and_clear_cancels_child_token -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Wire the cancel token from spawn into the child run loop**

Edit `crates/nekocode-subagent/src/tool/spawn_subagent.rs`. After `let agent_id = self.ctx.registry.allocate_running();` (line 94), capture the token:

```rust
let child_cancel = self
    .ctx
    .registry
    .cancel_token(agent_id)
    .expect("token present right after allocate_running");
```

Pass `child_cancel` into the spawned task and through to `run_subagent`. Update the `tokio::spawn` block from Task 8 Step 3 to also move `child_cancel` and call:

```rust
run_subagent(
    agent_id,
    child,
    prompt,
    registry,
    nekocode_core::agent::AgentEventSink::new(child_tx),
    child_cancel,
)
.await;
```

- [ ] **Step 6: Make `run_subagent` accept + honor the cancel token**

Edit `crates/nekocode-subagent/src/runner.rs`. Add a `cancel: tokio_util::sync::CancellationToken` parameter, and race `child.run_loop(...)` against cancellation so a cancelled child bails promptly (its own `on_turn_end` then recurses):

```rust
pub async fn run_subagent(
    agent_id: u64,
    child: Agent,
    prompt: String,
    registry: Arc<SubagentRegistry>,
    sink: nekocode_core::agent::AgentEventSink,
    cancel: tokio_util::sync::CancellationToken,
) {
    let run = child.run_loop(
        vec![MessageContent::Text { content: prompt }],
        Vec::new(),
        sink,
    );
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            // Parent aborted: record error so waiters wake and inspect/read
            // can see it. The child run_loop future is dropped here.
            registry.set_error(agent_id, "subagent cancelled by parent turn end".into());
            return;
        }
        r = run => r,
    };
    match result {
        Ok(turn) => registry.set_finished(agent_id, SubagentRunResult {
            usage: turn.usage, messages: turn.messages, finished: turn.finished,
        }),
        Err(_partial) => registry.set_error(agent_id, "subagent run_loop failed".into()),
    }
}
```

Update the two test call sites in `runner.rs` (lines 137, 152): pass `tokio_util::sync::CancellationToken::new()` (a fresh, never-cancelled token) so the unit tests behave as before.

- [ ] **Step 7: Add a `from_context` constructor + an end-to-end cascade test**

First, expose a constructor that builds a `SubagentMiddleware` from an already-built `SubagentContext` (the test builds ctx directly, bypassing `SubagentMiddleware::new` which loads `agents.toml` from the real config dir). In `crates/nekocode-subagent/src/middleware.rs`, add:

```rust
/// Build a middleware from an already-constructed context (tests that
/// build `SubagentContext` directly use this; `new` builds the ctx
/// then delegates here).
pub fn from_context(ctx: SubagentContext) -> Self {
    Self { ctx }
}
```

Refactor the tail of `new` (the `let ctx = SubagentContext { … }; Self { ctx }` part, ~lines 66-78) to end with `Self::from_context(ctx)` so both paths share one construction site.

Then append this e2e test to `crates/nekocode-subagent/tests/integration.rs`. It backs the spawned subagent with `PendingProvider` (already in the test file) so the subagent would hang forever without cancellation — only `on_turn_end` → `abort_all_and_clear` can end it:

```rust
#[tokio::test]
async fn on_turn_end_aborts_running_subagent() {
    use nekocode_subagent::SubagentMiddleware;
    let db = temp_db().await;
    let ctx = make_pending_ctx(true, 1, db);
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let mw = SubagentMiddleware::from_context(ctx.clone());
    let mut reg = nekocode_core::types::GenerateRequest::default();
    let mut tools = nekocode_types::tool::ToolRegistry::new();
    mw.before_generate(&mut reg, &mut tools, &mev_tx).await.unwrap();
    let spawn = tools.get("spawn_subagent").unwrap().clone();
    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();
    assert_eq!(ctx.registry.run_state(agent_id), nekocode_subagent::SubagentRunState::Running);

    // Parent turn ends:
    mw.on_turn_end().await.unwrap();

    // The never-completing subagent must be gone from the registry.
    assert_eq!(
        ctx.registry.run_state(agent_id),
        nekocode_subagent::SubagentRunState::Idle,
        "subagent evicted by on_turn_end"
    );
}
```

(`ToolRegistry::get(&self, name: &str) -> Option<Arc<dyn Tool + Send + Sync>>` exists at `crates/nekocode-types/src/tool.rs:85`; `use nekocode_types::tool::Tool;` is already imported in the test file so `.call(...)` resolves on the `Arc`.)

- [ ] **Step 8: Run the full subagent suite**

Run: `cargo test -p nekocode-subagent`
Expected: all tests PASS, including the new cascade tests.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "feat(subagent): CancellationToken cascade abort on parent turn end"
```

---

## Task 10: Full workspace verification + clippy

**Files:** none (verification only)

- [ ] **Step 1: Build the whole workspace**

Run: `cargo build --workspace`
Expected: compiles cleanly.

- [ ] **Step 2: Run the full test suite**

Run: `cargo test --workspace`
Expected: all tests PASS.

- [ ] **Step 3: Run clippy**

Run: `cargo clippy --workspace --all-targets -- -D warnings`
Expected: no warnings. (Fix any new clippy lints introduced by the new code — e.g. unused `mev_tx` imports, needless clones.)

- [ ] **Step 4: Verify the spec's index-uniqueness property holds end-to-end**

The Task 5 test already proves parent `Agent::send` + merge relay share one atomic. Manually confirm in `stream_generate.rs` that only ONE `AgentEventSink::new(tx)` wraps the top-level `tx` (no second wrapping that would create a divergent index space).

- [ ] **Step 5: Commit any clippy fixes**

```bash
git add -A
git commit -m "chore: clippy fixes for MiddlewareEvent relay" --allow-empty
```

(Only commit if there were fixes; skip the commit otherwise.)

---

## Notes for the implementer

- **One `AgentEventSink::new` per stream.** The shared `Arc<AtomicUsize>` lives on the sink; never wrap a sender twice or you'll get two divergent index spaces. `stream_generate` is the single top-level wrap; child `run_loop`s each wrap their own `child_tx` (a different sender).
- **Merge relay termination.** The merge relay spawned in `run_loop` exits naturally when every `mev_tx` clone is dropped (i.e. when the tools holding them drop at `run_loop` return). `on_turn_end` (which aborts subagents, the only `mev_tx` producers) runs first, so no new events arrive; `merge_relay.abort()` in Step 4 is a belt-and-suspenders guarantee.
- **Child index is preserved, not reindexed.** The child `AgentEvent` (with its own child-space `index`) is embedded verbatim as `data`; the parent assigns only the *enclosing* `AgentEvent.index`. Don't reindex `data`.
- **`on_turn_end` runs on both Ok and Err exit paths** of `run_loop` — that's where the "no subagent outlives the parent turn" guarantee lives. Don't gate it on success.
- **Nested subagents double-wrap.** A grandchild event becomes a child `MiddlewareEvent`, which the child spawn relay then wraps again as a parent `MiddlewareEvent` whose `data` is that child `AgentEvent` (itself a `MiddlewareEvent`). This recursive nesting is by design — `data` is opaque JSON; clients unwrap recursively.
