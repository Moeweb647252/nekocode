# MiddlewareEvent branch of AgentEvent — design

Date: 2026-07-01
Status: Approved (brainstormed)
Scope: introduce a generic `MiddlewareEvent` branch on `AgentEvent` and wire the
subagent's `AgentEvent` stream out through it. Per-turn hard-cascade abort so a
subagent never survives its parent turn.

## Background & the gap

- `AgentEvent { index, data: AgentEventType }` has exactly one variant today:
  `StreamEvent(StreamEvent)` (`nekocode-core/src/agent/mod.rs:12-24`).
- A subagent's `AgentEvent` stream is currently **discarded**. `spawn_subagent`
  (`nekocode-subagent/src/tool/spawn_subagent.rs:147-155`) creates a fresh
  `mpsc::unbounded_channel::<AgentEvent>()` and runs a drain task
  `while rx.recv().await.is_some() {}`. Only the final `SubagentRunResult`
  survives, surfaced later via `read_subagent`.
- Subagent is fire-and-forget: a parent turn can finish while spawned children
  keep running in detached tokio tasks.
- `MiddlewareEvent` does not exist anywhere yet.

## Decisions (locked during brainstorm)

1. **Scope**: a *generic* `MiddlewareEvent` branch; subagent consumes it first.
   subthread and others reuse the same branch later (via a `source` tag) without
   re-branching the enum.
2. **Sender bridge**: extend the `Middleware` trait (`before_generate`) to be
   handed a per-turn outbound handle, rather than threading a sender via
   `Agent.extensions`.
3. **Capability narrowing**: the handle passed to `before_generate` is a
   `UnboundedSender<MiddlewareEvent>` (not a full `AgentEventSink`). A middleware
   therefore cannot forge a `StreamEvent`; `StreamEvent` production stays the
   sole privilege of `Agent::send`.
4. **Stream lifecycle**: a subagent must NOT continue running after its parent
   turn ends — hard cascade abort, not the "keep open until children drain"
   option.

## §1 — New type `MiddlewareEvent` (nekocode-core)

```rust
// crates/nekocode-core/src/agent/mod.rs

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[serde(tag = "type")]
pub enum AgentEventType {
    StreamEvent(StreamEvent),
    /// An event relayed out of a child generation by a middleware
    /// (subagent today; reusable by subthread / others later). Payload is an
    /// opaque JSON value + a source-published type tag, so this enum never
    /// has to know the internal shape of each source's events.
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

Lives in `nekocode-core` (like `MiddlewareSpec`) so `nekocode-subagent`, which
depends on core+types only, can name it without depending on the API crate.

TS mirror (`webui/src/api/types.ts`):
```ts
export type AgentEventType =
  | { type: 'streamEvent'; data: RawStreamEventData; createdAt: string }
  | { type: 'middlewareEvent'; source: string; sourceId: number; eventType: string; data: unknown }
```

## §2 — `AgentEventSink`: shared index + parent `tx`

Once a relay task also writes the parent stream, multiple producers each holding
a private counter would collide indices and break the `boxcar::Vec<AgentEvent>`
replay (indexed by `AgentEvent.index`). A shared atomic index is mandatory.

```rust
// crates/nekocode-core/src/agent/sink.rs (new)

#[derive(Clone)]
pub struct AgentEventSink {
    tx: tokio::sync::mpsc::UnboundedSender<AgentEvent>,
    index: Arc<std::sync::atomic::AtomicUsize>,
}

impl AgentEventSink {
    pub fn new(tx: UnboundedSender<AgentEvent>) -> Self { … }
    /// Allocate the next unique index and send. Fails only on client disconnect.
    pub fn send(&self, data: AgentEventType) -> Result<(), AgentError> {
        let idx = self.index.fetch_add(1, Ordering::Relaxed);
        self.tx.send(AgentEvent { index: idx, data })
            .map_err(|e| AgentError::Other(anyhow::anyhow!("error sending agent event {e}")))
    }
}
```

`Agent::run_loop`'s `UnboundedSender` parameter becomes `AgentEventSink`.
`Agent::send` is refactored to allocate from the shared atomic (the `&mut index`
parameter goes away). The error branch in `run_loop`'s outer match
(`new_agent.rs:234-240`) and the final `TurnEnd` send use the sink too.

`stream_generate.rs` wraps `tx` once with `AgentEventSink::new(tx)` before
passing it to `run_loop`; the receiver loop, the `boxcar::Vec`, the
`broadcast::channel`, and `WebSocketEvent::Delta` all stay unchanged — the sink
is purely a send-side replacement.

## §3 — Extend `Middleware` trait with a narrow sender

```rust
// crates/nekocode-core/src/middleware.rs

#[async_trait]
pub trait Middleware: Send + Sync {
    async fn before_generate(
        &self,
        _: &mut GenerateRequest,
        _: &mut ToolRegistry,
        // NEW: narrow capability — can only emit MiddlewareEvent, cannot
        // forge StreamEvent. index allocation / wrapping is NOT done on the
        // middleware side; a merge relay inside run_loop does it, so the
        // parent stream's indices stay globally contiguous and unique.
        _: &tokio::sync::mpsc::UnboundedSender<MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> { Ok(()) }

    async fn after_generate(&self, _: &GenerateResponse, _: &mut AgentControlFlow)
        -> Result<(), anyhow::Error> { Ok(()) }
}
```

`run_loop` wiring at the top:
1. Build `AgentEventSink` (wraps `tx` + the shared `Arc<AtomicUsize>`); used
   privately by `Agent::send`.
2. Build `mpsc::unbounded_channel::<MiddlewareEvent>()`.
3. Spawn a **merge relay** with a clone of the `AgentEventSink`: loop on
   `mev_rx.recv()`, then call `sink.send(AgentEventType::MiddlewareEvent(mev))`
   per `MiddlewareEvent`. `sink.send` allocates the unique index from the shared
   `Arc<AtomicUsize>` and forwards to `tx` in one step — the merge relay never
   touches `tx` directly (the sink owns it). Two producers (`Agent::send` and
   the merge relay) share the same `Arc<AtomicUsize>` → indices stay unique;
   both funnel through the same `tx`.
4. Pass `&mev_tx` into each `middleware.before_generate(...)`.

Per-layer boundary: the child `run_loop` builds its *own* `AgentEventSink` and
its *own* `mev_tx`; the parent's `mev_tx` is **not** handed down recursively.
The parent `mev_tx` is used only by the relay in `spawn_subagent` (§4). Each
layer owns its own stream.

`on_turn_end` hook (also in the trait, with default `Ok(())`):
```rust
async fn on_turn_end(&self) -> Result<(), anyhow::Error> { Ok(()) }
```
`run_loop` calls `on_turn_end` on every middleware before returning (both Ok and
Err paths). This is the clean, generic cascade-abort hook used by §5 without
making `core` depend on `nekocode-subagent` specifics.

Signature-level consequence: every `Middleware` impl gets the new `&mev_tx`
parameter. Impls that don't relay events accept it as `_` — default `Ok(())`
makes the source change mechanical; no logic change required.

## §4 — `spawn_subagent` relay: child AgentEvent → MiddlewareEvent

`SubagentContext` gains `mev_tx: UnboundedSender<MiddlewareEvent>`, populated by
`SubagentMiddleware::before_generate` from the sink it receives. `ctx.clone()`
carries it into every tool; only `spawn_subagent` reads it.

`spawn_subagent.rs` (replacing lines 145-157):
```rust
let (child_tx, child_rx) = mpsc::unbounded_channel();
let mev_tx = self.ctx.mev_tx.clone();
let relay_target_agent_id = agent_id;

let handle = tokio::spawn(async move {
    let relay = tokio::spawn(async move {
        while let Some(child_event) = child_rx.recv().await {
            let mev = MiddlewareEvent {
                source: Cow::Borrowed("subagent"),
                source_id: relay_target_agent_id,
                event_type: "agentEvent".into(),
                data: serde_json::to_value(&child_event)
                    .unwrap_or(serde_json::Value::Null),
            };
            // parent stream may have closed: send failure just stops relaying
            let _ = mev_tx.send(mev);
        }
    });
    run_subagent(agent_id, child, prompt, registry, child_tx).await;
    relay.await.ok();
});
```

Properties:
- Child `AgentEvent.index` is in the child's index space and is embedded verbatim
  in `data` — not re-indexed into the parent. The parent assigns the enclosing
  `AgentEvent.index` (unique, contiguous) at the merge relay.
- Subagent stream is now surfaced **live** to the client (previously dropped),
  in addition to the existing `read_subagent` final-result path.
- On parent-stream close, `mev_tx.send` returns `Err`; the relay's `let _ =`
  silences it. `run_subagent` finishing closes `child_tx`, so `child_rx.recv()`
  returns `None`, the relay exits, `relay.await` resolves. Lifecycle is clean.

Child-layer `SubagentMiddleware::new` is constructed with the **child's own**
`mev_tx` (produced by the child `run_loop` per §3) — not the parent's.

## §5 — Hard cascade abort via `CancellationToken` + `on_turn_end`

New behavioral requirement: a subagent must not survive the end of its parent
turn. `JoinHandle::abort()` alone does not recursively cancel tasks the child
spawned (grandchildren), so we use a `CancellationToken` cascade (the same
primitive already used for interrupt support in `stream_generate.rs:41`).

```rust
// nekocode-subagent/src/registry.rs
pub struct SubagentState {
    // …existing…
    pub cancel: Arc<tokio_util::sync::CancellationToken>,
}

impl SubagentRegistry {
    pub fn abort_all_and_clear(&self) -> Vec<u64> {
        // Cancel every child's run_loop first — gives each layer a chance to
        // run its own on_turn_end → abort_all_and_clear (real recursion) —
        // then fall back to JoinHandle::abort() as the hard guarantee
        // (covers the case where cleanup didn't get to run).
        for entry in self.states.iter() { entry.cancel.cancel(); }
        // …existing JoinHandle::abort() & remove…
    }
}
```

`spawn_subagent` clones its `cancel_token` into the child run loop. Each run
loop, in its main select, has `cancel_token.cancelled()` race against the body
so it can bail promptly and its own `on_turn_end` re-enters `abort_all_and_clear`
on its own registry.

Parent run_loop exit path:
```rust
// at the end of run_loop (Ok and Err branches), before returning:
for mw in self.middlewares.iter() {
    let _ = mw.on_turn_end().await;
}
```
`SubagentMiddleware::on_turn_end()` calls its registry's `abort_all_and_clear()`.
Generic hook → `core` never imports `nekocode_subagent` types; `nekocode-subthread`
or any later fire-and-forget producer can opt into the same hook.

## §6 — Change surface & tests

### Files

| Crate / file | Change |
|---|---|
| `nekocode-core/src/agent/mod.rs` | add `MiddlewareEvent` variant + struct |
| `nekocode-core/src/agent/sink.rs` (new) | `AgentEventSink` |
| `nekocode-core/src/middleware.rs` | `before_generate` gets `&UnboundedSender<MiddlewareEvent>`; default `on_turn_end` |
| `nekocode-core/src/agent/new_agent.rs` | `run_loop` takes `AgentEventSink`; build `mev_tx` + merge relay; `Agent::send` via sink; call `on_turn_end`; pass `&mev_tx` to `before_generate` |
| `nekocode-core/src/agent/test_mocks.rs` | mock middlewares accept the new param (ignore) |
| `nekocode-subagent/src/middleware.rs` | `SubagentContext` gains `mev_tx`; `before_generate` writes it into ctx; impl `on_turn_end` |
| `nekocode-subagent/src/tool/spawn_subagent.rs` | drained → relay (§4); clone child `cancel_token` |
| `nekocode-subagent/src/registry.rs` | `SubagentState.cancel`; `abort_all_and_clear` cancels first |
| `nekocode-subagent/src/runner.rs` + child `SubagentMiddleware::new` | child gets its *own* `mev_tx` from its own `run_loop` |
| `nekocode/src/api/generate/stream_generate.rs` | wrap `tx` in `AgentEventSink::new(tx)` before `run_loop` |
| `nekocode/src/api/thread/mod.rs::build_middlewares` | middlewares accept new param (ignore) |
| `nekocode/src/api/thread/subagent_factory.rs` | child factory arms accept new param |
| `nekocode-subthread` middleware impls | signature follow-up (ignore sink; opt into `on_turn_end` later) |
| `webui/src/api/types.ts` | add `middlewareEvent` branch to `AgentEventType` |
| `webui` rendering | out of scope this design — pass-through only for now |

### Tests

1. Subagent relay: parent holds `mev_rx`; spawn child → parent `mev_rx` receives
   `source="subagent"`, `source_id=agent_id`, `event_type="agentEvent"`, `data`
   is the child's serialized event. After child messageEnd, `child_rx` closes and
   the relay exits.
2. Index uniqueness: concurrent parent `send` + merge-relay drains `mev_tx`;
   collect all `AgentEvent`s on parent `tx`; assert index set is `{0..n}` with no
   duplicates and no gaps.
3. Cascade abort: parent `run_loop` finishes → child `SubagentState` evicted from
   parent registry, child `JoinHandle::is_finished()`. Two-level (parent → child
   → grandchild) both cancelled; grandchild handle observed cancelled.
4. Stream close: parent `tx` dropped early → merge relay exits, child relay
   `mev_tx.send` silently Errs, `run_subagent` still completes and
   `registry.set_finished` runs; no panic.
5. Trait signature compile: all `Middleware` impls compile (default `Ok(())`
   covers non-relaying implementations).

### Risks / boundaries

- One merge-relay task per turn — small overhead. Could be replaced by inline
  `try_recv` drains if profiling shows it matters; not done now.
- Nested `serde_json::Value` payload doubles parent-stream bandwidth when a
   child stream is dense. Design is "full relay" by default; filtering /
   sampling is a future addition, not in scope here.
- `on_turn_end` is a small trait surface addition, but it is far cleaner than
  making `nekocode-core` import `nekocode-subagent` types directly — worth the
  trade.