//! Lightweight, in-memory subagent machinery for `nekocode-core`.
//!
//! A [`SubAgent`] is a stripped-down counterpart of [`crate::agent::Agent`]:
//! it owns its own [`Provider`], middleware chain and tool registry, but it
//! **never touches the database**. Its conversation history lives entirely
//! in an in-memory `Vec<Message>`. This makes a `SubAgent` ideal for short
//! lived, program-driven tasks — a compact middleware spinning up a
//! summarisation model, a tool delegating a side conversation, or a parent
//! agent fanning out a few independent reads.
//!
//! Contrast with `nekocode-subthread`, which models long-lived, DB-persisted
//! background threads reachable through the HTTP API. `SubAgent` is the
//! ephemeral, code-driven sibling.
//!
//! # Two execution modes
//!
//! - [`SubAgent::run`] — synchronous `await`; returns a [`SubAgentRunSummary`]
//!   directly. Stream events can optionally be forwarded.
//! - [`SubAgent::spawn`] — background `tokio` task; returns a `JoinHandle`
//!   immediately. Completion is observed via a [`SubAgentRegistry`].
//!
//! # Why parallel to `Agent::run_loop` rather than shared
//!
//! `Agent::run_loop` weaves DB persistence into every step of its body.
//! Sharing the loop between persistent and in-memory variants would push every
//! `toasty::create!` / `query!` through an indirection and obscure both
//! versions. [`SubAgent::run`] re-implements the same shape (outer middleware
//! loop + inner tool-call loop, identical hook ordering) with comments
//! pointing at corresponding lines in `agent::mod`. The two MUST stay in
//! lockstep when the agent protocol evolves — that is the deliberate trade.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::anyhow;
use dashmap::DashMap;
use nekocode_types::generate::{
    AssistantContentBlock, Message, MessageContent, MessageMetadata, Role, StreamEvent,
    StreamEventData, Usage,
};
use nekocode_types::tool::{ToolCallResult, ToolCallResultInner, ToolRegistry};
use serde::Serialize;
use tokio::sync::mpsc::UnboundedSender;
use tokio::sync::{Notify, RwLock};
use tokio::task::JoinHandle;

use crate::agent::error::AgentError;
use crate::agent::store::MessageStore;
use crate::agent::{AgentEvent, AgentEventType};
use crate::middleware::{AgentControlFlow, Middleware};
use crate::provider::{Provider, ProviderEvent};
use crate::types::{GenerateRequest, GenerateResponse};

/// Extension key under which a parent agent may publish its
/// [`SubAgentRegistry`] into `Agent.extensions`, mirroring the `subthread`
/// convention. Per-parent (NOT a process-global singleton).
pub const SUBAGENT_EXTENSION_KEY: &str = "subagents";

// ===========================================================================
// SubAgentRunSummary
// ===========================================================================

/// In-memory conversation snapshot returned by a completed subagent run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SubAgentRunSummary {
    /// Every message produced during the run, in chronological order.
    pub messages: Vec<Message>,
    /// Aggregate token usage across every provider generation in this run.
    pub usage: Usage,
}

impl SubAgentRunSummary {
    /// Concatenate the text blocks of the **last** assistant message. Returns
    /// `None` when there is no assistant message, or its blocks contain no
    /// text (e.g. only tool calls). Compact-style middlewares use this as the
    /// convenience entry point to extract summary text.
    pub fn last_assistant_text(&self) -> Option<String> {
        let last_assistant = self
            .messages
            .iter()
            .rev()
            .find(|m| matches!(m, Message::Assistant(_)))?;
        if let Message::Assistant(a) = last_assistant {
            let texts: Vec<&str> = a
                .blocks
                .iter()
                .filter_map(|b| match b {
                    AssistantContentBlock::Text { content, .. } => Some(content.as_str()),
                    _ => None,
                })
                .collect();
            if texts.is_empty() { None } else { Some(texts.join("\n")) }
        } else {
            None
        }
    }
}

// ===========================================================================
// SubAgent
// ===========================================================================

/// Lightweight, DB-free agent. Build via [`SubAgentBuilder`].
pub struct SubAgent {
    /// In-memory message history shared via `Arc<RwLock<>>` so a
    /// [`SubAgentRegistry`] (or any external observer holding the `Arc`) can
    /// read the final conversation after a spawned run completes — the
    /// `SubAgent` itself is moved into the background task.
    messages: Arc<RwLock<Vec<Message>>>,
    /// System prompt prepended to every provider generation. Captured up
    /// front so it survives both inner and outer regeneration loops, matching
    /// `Agent::run_loop`'s `base_system_prompt`.
    system_prompt: Option<String>,
    /// Middleware chain. Hooks (`before_generate`/`after_generate`) run in
    /// list order, same as `Agent`.
    middlewares: Arc<Vec<Box<dyn Middleware>>>,
    /// LLM backend. Owned by this subagent so it may target a different model
    /// than its parent.
    provider: Arc<dyn Provider>,
    /// Per-subagent extensions map. Independent of any parent
    /// `Agent.extensions` so middleware state stays scoped here.
    extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
}

// ===========================================================================
// SubAgentBuilder
// ===========================================================================

/// Builder for [`SubAgent`]. Required field: `provider`. Everything else
/// defaults to empty / `None`.
pub struct SubAgentBuilder {
    system_prompt: Option<String>,
    middlewares: Vec<Box<dyn Middleware>>,
    provider: Option<Arc<dyn Provider>>,
    seed_messages: Vec<Message>,
    extensions: Arc<DashMap<String, Box<dyn Any + Send + Sync>>>,
}

impl SubAgentBuilder {
    /// Start building a subagent that uses `provider` as its LLM backend.
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            system_prompt: None,
            middlewares: Vec::new(),
            provider: Some(provider),
            seed_messages: Vec::new(),
            extensions: Arc::new(DashMap::new()),
        }
    }

    /// Set the system prompt injected before every provider generation.
    pub fn system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    /// Append a single middleware to the chain.
    pub fn middleware(mut self, m: Box<dyn Middleware>) -> Self {
        self.middlewares.push(m);
        self
    }

    /// Replace the middleware chain with `middlewares`.
    pub fn middlewares(mut self, middlewares: Vec<Box<dyn Middleware>>) -> Self {
        self.middlewares = middlewares;
        self
    }

    /// Pre-populate the conversation history with a message (e.g. context
    /// from a compacted summary). Seed messages appear before the user
    /// input in the final message list.
    pub fn seed_message(mut self, m: Message) -> Self {
        self.seed_messages.push(m);
        self
    }

    /// Insert an entry into the per-subagent extensions map.
    pub fn extension(self, key: &str, value: Box<dyn Any + Send + Sync>) -> Self {
        self.extensions.insert(key.to_string(), value);
        self
    }

    /// Consume the builder and produce a [`SubAgent`]. Panics if `provider`
    /// was never set (it is required).
    pub fn build(self) -> SubAgent {
        let provider = self
            .provider
            .expect("SubAgentBuilder: provider is required");
        SubAgent {
            messages: Arc::new(RwLock::new(self.seed_messages)),
            system_prompt: self.system_prompt,
            middlewares: Arc::new(self.middlewares),
            provider,
            extensions: self.extensions,
        }
    }
}

impl SubAgent {
    /// Read-only access to the shared extensions map.
    pub fn extensions(&self) -> &Arc<DashMap<String, Box<dyn Any + Send + Sync>>> {
        &self.extensions
    }

    /// Read-only handle to the shared message history. Useful for inspecting
    /// a spawned subagent's in-flight conversation from outside the task.
    pub fn messages(&self) -> &Arc<RwLock<Vec<Message>>> {
        &self.messages
    }

    /// Snapshot the message history.
    pub async fn messages_snapshot(&self) -> Vec<Message> {
        self.messages.read().await.clone()
    }
}

impl SubAgent {
    /// Synchronously run the subagent to completion, returning a snapshot of
    /// the final conversation plus aggregate usage.
    ///
    /// Structurally mirrors `Agent::run_loop` with every DB
    /// `create!`/`query!` replaced by an in-memory `Vec<Message>`
    /// push/clone. Comments tag the corresponding step in `agent::mod`.
    ///
    /// `sender`, when `Some`, receives the same stream events a client of
    /// `Agent::run_loop` would see. Pass `None` to discard them.
    pub async fn run(
        &self,
        input: String,
        sender: Option<UnboundedSender<AgentEvent>>,
    ) -> Result<SubAgentRunSummary, AgentError> {
        // Build an in-memory store wrapping this SubAgent's message history.
        // We clone the messages into the store; after the loop we write them
        // back so that `self.messages` stays in sync (important for `spawn`).
        let store = super::store::InMemoryMessageStore::new(self.messages.read().await.clone());

        // Append the user input.
        store
            .push_user_message(MessageContent::Text { content: input })
            .await?;

        let base_system_prompt = self.system_prompt.clone();
        let request = GenerateRequest {
            messages: store.current_messages().await?,
            system_prompt: base_system_prompt.clone(),
            ..Default::default()
        };

        let (_index, total_usage) = super::store::run_loop_core(
            &store,
            self.provider.clone(),
            &self.middlewares,
            &sender,
            base_system_prompt,
            request,
            0,
        )
        .await?;

        // Finalize: in-memory store is a no-op, but call it for consistency.
        store.finalize(&total_usage).await?;

        // Collect final messages from the store and write them back to
        // self.messages so that subsequent calls or the registry see the
        // updated history.
        let final_messages = store.current_messages().await?;
        {
            let mut guard = self.messages.write().await;
            *guard = final_messages.clone();
        }

        Ok(SubAgentRunSummary {
            messages: final_messages,
            usage: total_usage,
        })
    }

    /// Spawn the subagent in a background tokio task. Returns a `JoinHandle`
    /// immediately. Completion lands in `registry` under `subagent_id`,
    /// observable via [`SubAgentRegistry::run_state`] /
    /// [`SubAgentRegistry::wait_for`].
    ///
    /// `self` must be wrapped in `Arc` because the background task takes
    /// ownership. The subagent's `messages` `Arc<RwLock<>>` ensures the
    /// final conversation remains readable through the registry after the
    /// task completes (the registry stores it inside the
    /// [`SubAgentRunState::Finished`] variant).
    pub fn spawn(
        self: Arc<Self>,
        subagent_id: u64,
        registry: Arc<SubAgentRegistry>,
        input: String,
    ) -> JoinHandle<()> {
        registry.set_running(subagent_id);
        tokio::spawn(async move {
            let result = self.run(input, None).await;
            match result {
                Ok(summary) => registry.set_finished(subagent_id, summary),
                Err(e) => registry.set_error(subagent_id, e.to_string()),
            }
        })
    }
}

// ===========================================================================
// SubAgentRunState
// ===========================================================================

/// Lifecycle of a subagent tracked by a [`SubAgentRegistry`]. In-memory only:
/// NOT persisted across server restarts (the registry is per-parent and lives
/// in `Agent.extensions`).
#[derive(Debug, Clone)]
pub enum SubAgentRunState {
    /// Registered with the registry but no background task is running yet.
    Idle,
    /// A `spawn`ed background `run` task is in flight.
    Running,
    /// The background task completed successfully. Carries the final
    /// conversation snapshot + aggregate usage — the result is kept here
    /// because, unlike a subthread, a subagent never writes to the DB, so the
    /// registry IS the durable surface for its outcome.
    Finished { summary: SubAgentRunSummary },
    /// The background task errored; carries the error message.
    Error(String),
}

impl SubAgentRunState {
    /// "Ready" means the subagent has settled and its outcome is final.
    /// `Idle` and `Running` are NOT ready.
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Finished { .. } | Self::Error(_))
    }
}

// ===========================================================================
// SubAgentRegistry
// ===========================================================================

/// Per-parent in-memory map of subagent run state. Owned by the parent
/// thread's `Agent.extensions` (key [`SUBAGENT_EXTENSION_KEY`]), shared via
/// `Arc` with any tool/middleware that spawns subagents. NOT a process-global
/// singleton — each parent has its own.
///
/// Mirrors `nekocode_subthread::SubthreadRegistry` in shape, but differs in
/// two ways: (1) ids are minted by an internal `AtomicU64` rather than the
/// DB's `Thread` primary key; (2) `Finished` carries the full
/// [`SubAgentRunSummary`] because results are not DB-backed.
#[derive(Debug, Default)]
pub struct SubAgentRegistry {
    states: DashMap<u64, SubAgentRunState>,
    notifies: DashMap<u64, Arc<Notify>>,
    next_id: AtomicU64,
}

impl SubAgentRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh subagent id and register it as `Idle`. The id is
    /// monotonically increasing and unique within this registry.
    pub fn allocate(&self) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        // Start from 1 so 0 can never be a valid id (acts as a sentinel).
        let id = id + 1;
        self.states.insert(id, SubAgentRunState::Idle);
        self.notifies.insert(id, Arc::new(Notify::new()));
        id
    }

    /// Snapshot the run state of a subagent, defaulting to `Idle` if absent
    /// (matching the subthread registry's forgiving default).
    pub fn run_state(&self, id: u64) -> SubAgentRunState {
        self.states
            .get(&id)
            .map(|s| s.clone())
            .unwrap_or(SubAgentRunState::Idle)
    }

    /// Mark a subagent as `Running`. Called by `SubAgent::spawn` right before
    /// the background task is launched.
    pub fn set_running(&self, id: u64) {
        if let Some(mut s) = self.states.get_mut(&id) {
            *s = SubAgentRunState::Running;
        } else {
            self.states.insert(id, SubAgentRunState::Running);
            self.notifies.entry(id).or_insert_with(|| Arc::new(Notify::new()));
        }
    }

    /// Mark a subagent as `Finished` and wake any waiters.
    pub fn set_finished(&self, id: u64, summary: SubAgentRunSummary) {
        if let Some(mut s) = self.states.get_mut(&id) {
            *s = SubAgentRunState::Finished { summary };
        } else {
            self.states
                .insert(id, SubAgentRunState::Finished { summary });
            self.notifies.entry(id).or_insert_with(|| Arc::new(Notify::new()));
        }
        self.notify_waiters(id);
    }

    /// Mark a subagent as `Error` and wake any waiters.
    pub fn set_error(&self, id: u64, msg: String) {
        if let Some(mut s) = self.states.get_mut(&id) {
            *s = SubAgentRunState::Error(msg);
        } else {
            self.states.insert(id, SubAgentRunState::Error(msg));
            self.notifies.entry(id).or_insert_with(|| Arc::new(Notify::new()));
        }
        self.notify_waiters(id);
    }

    /// Remove a subagent's entry. No-op if absent.
    pub fn remove(&self, id: u64) {
        self.states.remove(&id);
        self.notifies.remove(&id);
    }

    /// Clone of the `Notify` for a subthread, so waiters can subscribe without
    /// holding a DashMap guard. Returns `None` if the subagent isn't tracked.
    pub fn notify(&self, id: u64) -> Option<Arc<Notify>> {
        self.notifies.get(&id).map(|n| n.clone())
    }

    /// All subagent ids currently tracked by this registry.
    pub fn all_ids(&self) -> Vec<u64> {
        self.states.iter().map(|s| *s.key()).collect()
    }

    fn notify_waiters(&self, id: u64) {
        if let Some(n) = self.notifies.get(&id) {
            n.notify_waiters();
        }
    }

    /// Wait until `id` becomes ready (`Finished` or `Error`), or until
    /// `timeout` elapses. Returns the final state, or `Idle`/`Running` on
    /// timeout (use [`SubAgentRunState::is_ready`] to tell). Does NOT affect
    /// a still-running task on timeout.
    pub async fn wait_for(&self, id: u64, timeout: Duration) -> Result<SubAgentRunState, AgentError> {
        self.wait_any(vec![id], timeout)
            .await
            .map(|o| o.state)
    }

    /// Wait until any one of `ids` becomes ready, or until `timeout` elapses.
    /// Returns the first ready subagent's state on success; on timeout returns
    /// `Err` carrying the list of still-pending ids.
    pub async fn wait_any(
        &self,
        ids: Vec<u64>,
        timeout: Duration,
    ) -> Result<WaitAnyOutcome, AgentError> {
        if ids.is_empty() {
            return Err(AgentError::Other(anyhow!("wait_any: ids is empty")));
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            // First scan for an already-ready subagent.
            for id in &ids {
                let state = self.run_state(*id);
                if state.is_ready() {
                    return Ok(WaitAnyOutcome { id: *id, state });
                }
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                let pending: Vec<u64> = ids
                    .iter()
                    .filter(|id| !self.run_state(**id).is_ready())
                    .copied()
                    .collect();
                return Err(AgentError::Other(anyhow!("timeout; pending: {pending:?}")));
            }
            // Re-collect notify handles each iteration (entries may be removed).
            let notifies: Vec<_> = ids.iter().filter_map(|id| self.notify(*id)).collect();
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

    /// Wait until ALL of `ids` are ready, or until `timeout` elapses. With an
    /// empty `ids`, defaults to all currently-`Running` subagents. Returns the
    /// ready/pending split on timeout.
    pub async fn wait_all(
        &self,
        ids: Vec<u64>,
        timeout: Duration,
    ) -> Result<Vec<WaitAllEntry>, WaitAllTimeout> {
        let ids: Vec<u64> = if ids.is_empty() {
            self.all_ids()
                .into_iter()
                .filter(|id| matches!(self.run_state(*id), SubAgentRunState::Running))
                .collect()
        } else {
            ids
        };
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let (mut ready, mut pending) = (Vec::new(), Vec::new());
            for id in &ids {
                let state = self.run_state(*id);
                if state.is_ready() {
                    ready.push(WaitAllEntry { id: *id, state });
                } else {
                    pending.push(*id);
                }
            }
            if pending.is_empty() {
                return Ok(ready);
            }
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(WaitAllTimeout { ready, pending });
            }
            let notifies: Vec<_> = ids.iter().filter_map(|id| self.notify(*id)).collect();
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

/// Outcome of a successful [`SubAgentRegistry::wait_any`].
#[derive(Debug, Clone)]
pub struct WaitAnyOutcome {
    pub id: u64,
    pub state: SubAgentRunState,
}

/// One entry in a [`SubAgentRegistry::wait_all`] result.
#[derive(Debug, Clone)]
pub struct WaitAllEntry {
    pub id: u64,
    pub state: SubAgentRunState,
}

/// Timeout outcome of [`SubAgentRegistry::wait_all`]: ready entries plus the
/// list of still-pending ids.
#[derive(Debug, Clone)]
pub struct WaitAllTimeout {
    pub ready: Vec<WaitAllEntry>,
    pub pending: Vec<u64>,
}

/// Race a slice of `Notify` handles; resolves when the first fires. Mirrors
/// the `nekocode-subthread` crate's `notify_any` helper, using
/// `futures_util::future::select_all`.
async fn notify_any(notifies: &[Arc<Notify>]) {
    if notifies.is_empty() {
        return;
    }
    let futs: Vec<_> = notifies
        .iter()
        .map(|n| {
            let n = n.clone();
            Box::pin(async move { n.notified().await })
        })
        .collect();
    let (_res, _idx, _rest) = futures_util::future::select_all(futs).await;
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::test_mocks::{text_msg, toolcall_msg, EchoMiddleware, InjectMiddleware, MockProvider, OneShotRegenerateMiddleware};
    use crate::agent::store::MessageStore;
    use crate::middleware::AgentControlFlow;
    use std::sync::Mutex;

    fn build(provider: Arc<dyn Provider>) -> SubAgent {
        SubAgentBuilder::new(provider).build()
    }

    #[tokio::test]
    async fn run_plain_text_no_middleware() {
        // Single text response, no tools, no middleware. The summary should
        // contain [user, assistant] and last_assistant_text returns the text.
        let provider = Arc::new(MockProvider::new(vec![text_msg("hello world")]));
        let sub = build(provider);
        let summary = sub.run("hi".to_string(), None).await.unwrap();
        assert_eq!(summary.messages.len(), 2);
        assert_eq!(summary.last_assistant_text().as_deref(), Some("hello world"));
        assert_eq!(summary.usage.total_input, 10);
        assert_eq!(summary.usage.total_output, 5);
    }

    #[tokio::test]
    async fn run_tool_call_loop() {
        // First generation requests a tool call; the mock tool echoes it; the
        // second generation is a plain text answer. We expect:
        // [user, assistant(toolcall), toolcallresult, assistant(text)].
        let provider = Arc::new(MockProvider::new(vec![
            toolcall_msg("c1", "echo", serde_json::json!({"value": "ping"})),
            text_msg("done after echo"),
        ]));
        let sub = SubAgentBuilder::new(provider)
            .middleware(Box::new(EchoMiddleware))
            .build();
        let summary = sub.run("go".to_string(), None).await.unwrap();
        // user, assistant(toolcall), toolresult, assistant(text)
        assert_eq!(summary.messages.len(), 4);
        let last = summary.last_assistant_text();
        assert_eq!(last.as_deref(), Some("done after echo"));
        // The tool result must be the echoed value.
        let has_echo = summary.messages.iter().any(|m| match m {
            Message::ToolCallResult(tcr) => match &tcr.result {
                ToolCallResultInner::Success { value } => {
                    *value == serde_json::json!({"value": "ping"})
                }
                _ => false,
            },
            _ => false,
        });
        assert!(has_echo, "expected an echo tool result in the message list");
    }

    #[tokio::test]
    async fn run_generate_with_outer_loop() {
        // The regenerate middleware fires once, injecting a middleware
        // message, which causes a second generation. Provider supplies two
        // text messages.
        let provider = Arc::new(MockProvider::new(vec![
            text_msg("first"),
            text_msg("second"),
        ]));
        let sub = SubAgentBuilder::new(provider)
            .middleware(Box::new(OneShotRegenerateMiddleware {
                fired: Mutex::new(false),
                inject: "please continue".to_string(),
            }))
            .build();
        let summary = sub.run("start".to_string(), None).await.unwrap();
        // user, assistant(first), middlewaremsg, assistant(second)
        assert_eq!(summary.messages.len(), 4);
        assert_eq!(summary.last_assistant_text().as_deref(), Some("second"));
        // The middleware message must be present.
        let has_mw = summary.messages.iter().any(|m| {
            matches!(m, Message::MiddlewareMessage(MessageContent::Text { content }) if content == "please continue")
        });
        assert!(has_mw, "expected the injected middleware message");
    }

    #[tokio::test]
    async fn spawn_and_wait_for() {
        // Spawn a subagent in the background; the registry should flip to
        // Finished carrying the summary.
        let provider = Arc::new(MockProvider::new(vec![text_msg("bg result")]));
        let sub = Arc::new(build(provider));
        let registry = Arc::new(SubAgentRegistry::new());
        let id = registry.allocate();
        assert!(matches!(registry.run_state(id), SubAgentRunState::Idle));

        let handle = sub.spawn(id, registry.clone(), "hi".to_string());
        let state = registry
            .wait_for(id, Duration::from_secs(2))
            .await
            .unwrap();
        assert!(state.is_ready());
        match state {
            SubAgentRunState::Finished { summary } => {
                assert_eq!(summary.last_assistant_text().as_deref(), Some("bg result"));
            }
            other => panic!("expected Finished, got {other:?}"),
        }
        // JoinHandle completes.
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn wait_any_picks_first_ready() {
        // Two subagents; the first finishes quickly, the second slowly.
        let fast = Arc::new(MockProvider::new(vec![text_msg("fast")]));
        let slow = Arc::new(MockProvider::new(vec![text_msg("slow")]));
        let registry = Arc::new(SubAgentRegistry::new());
        let id_fast = registry.allocate();
        let id_slow = registry.allocate();

        // Spawn both subagents in the background. We hold the JoinHandles
        // so they aren't dropped (which clippy flags as a detached future);
        // both get joined at the end of the test.
        let h_fast =
            Arc::new(build(fast)).spawn(id_fast, registry.clone(), "go".to_string());
        let h_slow =
            Arc::new(build(slow)).spawn(id_slow, registry.clone(), "go".to_string());

        let outcome = registry
            .wait_any(vec![id_fast, id_slow], Duration::from_secs(2))
            .await
            .unwrap();
        assert!(outcome.state.is_ready());
        assert!(outcome.id == id_fast || outcome.id == id_slow);
        // Drain both background tasks before the test exits.
        h_fast.await.unwrap();
        h_slow.await.unwrap();
    }

    #[tokio::test]
    async fn wait_for_timeout_returns_err() {
        // A subagent that is Idle (never spawned) should time out.
        let registry = Arc::new(SubAgentRegistry::new());
        let id = registry.allocate();
        let res = registry.wait_for(id, Duration::from_millis(50)).await;
        assert!(res.is_err(), "expected timeout error for an idle subagent");
    }

    #[tokio::test]
    async fn last_assistant_text_none_when_no_assistant() {
        let summary = SubAgentRunSummary {
            messages: vec![Message::User(MessageContent::Text {
                content: "only user".to_string(),
            })],
            usage: Usage::default(),
        };
        assert!(summary.last_assistant_text().is_none());
    }

    #[tokio::test]
    async fn seed_messages_are_present_in_summary() {
        let provider = Arc::new(MockProvider::new(vec![text_msg("ok")]));
        let sub = SubAgentBuilder::new(provider)
            .seed_message(Message::User(MessageContent::Text {
                content: "preamble".to_string(),
            }))
            .build();
        let summary = sub.run("actual input".to_string(), None).await.unwrap();
        // preamble(user), actual input(user), assistant(ok)
        assert_eq!(summary.messages.len(), 3);
        let first = &summary.messages[0];
        assert!(matches!(first, Message::User(MessageContent::Text { content }) if content == "preamble"));
    }

    #[tokio::test]
    async fn registry_allocate_ids_are_unique_and_start_at_one() {
        let reg = SubAgentRegistry::new();
        let a = reg.allocate();
        let b = reg.allocate();
        assert_ne!(a, b);
        assert!(a >= 1 && b >= 1);
    }
}
