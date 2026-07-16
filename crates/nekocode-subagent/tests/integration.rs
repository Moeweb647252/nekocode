//! Integration tests for nekocode-subagent tools. Builds a SubagentContext
//! directly (fields are pub) and invokes tools, bypassing
//! SubagentMiddleware::new (which loads agents.toml from the real config dir,
//! not controllable from tests). The provider/factory are local mocks.

use std::sync::{Arc, Mutex};

use nekocode_core::extensions::Extensions;
use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
use nekocode_subagent::{
    ProfileCatalog, SubagentContext, SubagentMiddlewareFactory, SubagentProfile, SubagentRegistry,
    tool::{
        AbortSubagentTool, InspectSubagentTool, ReadSubagentTool, SpawnSubagentTool,
        WaitAnySubagentTool,
    },
};
use nekocode_types::generate::{AssistantContentBlock, AssistantMessage, StopReason, Usage};
use nekocode_types::tool::Tool;
use tokio::sync::mpsc;

struct MockFactory;
#[async_trait::async_trait]
impl SubagentMiddlewareFactory for MockFactory {
    fn build(
        &self,
        _spec: MiddlewareSpec,
        _subagent_id: u64,
        _extensions: Extensions,
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
        _: &tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
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
        Self {
            responses: Mutex::new(r),
        }
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
                sender
                    .send(ProviderEvent::Content(content.clone()))
                    .unwrap();
            }
        }
        sender
            .send(ProviderEvent::MessageEnd(StopReason::Stop))
            .unwrap();
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

/// A provider that never resolves — used to exercise `wait_any`'s timeout
/// path against a subagent that never completes.
struct PendingProvider;
#[async_trait::async_trait]
impl Provider for PendingProvider {
    async fn stream_generate(
        &self,
        _: nekocode_core::types::GenerateRequest,
        _: mpsc::UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        std::future::pending().await
    }
}

async fn temp_db() -> toasty::Db {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "nekocode_subagent_it_{}_{}.db",
        std::process::id(),
        SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    nekocode_entities::prepare_db(path).await.unwrap()
}

fn catalog_with_explorer() -> Arc<ProfileCatalog> {
    // Build a catalog in-memory by parsing a TOML string. The `agents.toml`
    // shape is a top-level table with an `agents` array (mirrors `AgentsFile`
    // in profile.rs, which is private); pull the array out of that key.
    let toml = r#"
[[agents]]
name = "explorer"
middlewares = []
"#;
    #[derive(serde::Deserialize)]
    struct AgentsFile {
        #[serde(default)]
        agents: Vec<SubagentProfile>,
    }
    let parsed: AgentsFile = toml::from_str(toml).unwrap();
    let mut profiles = std::collections::HashMap::new();
    for p in parsed.agents {
        profiles.insert(p.name.clone(), p);
    }
    Arc::new(ProfileCatalog { profiles })
}

fn catalog_with_explorer_and_heavy() -> Arc<ProfileCatalog> {
    // Like `catalog_with_explorer`, but adds a "heavy" profile that requests
    // the "shell" middleware — used to exercise the intersection-rejection
    // branch in spawn_subagent (profile.middlewares must be a subset of the
    // parent's enabled specs).
    let toml = r#"
[[agents]]
name = "explorer"
middlewares = []

[[agents]]
name = "heavy"
middlewares = ["shell"]
"#;
    #[derive(serde::Deserialize)]
    struct AgentsFile {
        #[serde(default)]
        agents: Vec<SubagentProfile>,
    }
    let parsed: AgentsFile = toml::from_str(toml).unwrap();
    let mut profiles = std::collections::HashMap::new();
    for p in parsed.agents {
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
        parent_working_directory: "/tmp".into(),
        parent_db: db,
        catalog: catalog_with_explorer(),
        depth: 0,
        max_depth,
        allow_nested,
        run_cancel: Arc::new(std::sync::RwLock::new(
            tokio_util::sync::CancellationToken::new(),
        )),
    }
}

/// Like `make_ctx` but backs the spawned subagent with a `PendingProvider` so
/// the subagent's run future never completes. Used to exercise `wait_any`'s
/// timeout path without killing the still-running subagent.
fn make_pending_ctx(allow_nested: bool, max_depth: u32, db: toasty::Db) -> SubagentContext {
    SubagentContext {
        registry: Arc::new(SubagentRegistry::new()),
        specs: Vec::new(),
        factory: Arc::new(MockFactory),
        parent_provider: Arc::new(PendingProvider),
        parent_working_directory: "/tmp".into(),
        parent_db: db,
        catalog: catalog_with_explorer(),
        depth: 0,
        max_depth,
        allow_nested,
        run_cancel: Arc::new(std::sync::RwLock::new(
            tokio_util::sync::CancellationToken::new(),
        )),
    }
}

/// A mev_tx whose receiver is kept alive so sends succeed; for tests that
/// don't assert on relayed events. The caller binds the receiver (e.g. to
/// `_mev_rx`) so it stays alive for the test's lifetime — fine for a test.
fn dummy_mev_tx() -> (
    tokio::sync::mpsc::UnboundedSender<nekocode_core::agent::MiddlewareEvent>,
    tokio::sync::mpsc::UnboundedReceiver<nekocode_core::agent::MiddlewareEvent>,
) {
    tokio::sync::mpsc::unbounded_channel()
}

#[tokio::test]
async fn spawn_wait_read_lifecycle() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 1, db); // max_depth=1 → depth 0+1 <= 1, spawn allowed
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx.clone(), mev_tx);
    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();
    assert_eq!(res.get("status").unwrap().as_str(), Some("running"));

    // Wait for it to finish (small timeout; the mock resolves immediately).
    let wait = WaitAnySubagentTool::new(ctx.clone());
    let wres = wait
        .call(serde_json::json!({ "agent_ids": [agent_id], "timeout": 5.0 }))
        .await
        .unwrap();
    assert_eq!(wres.get("status").unwrap().as_str(), Some("ready"));

    let read = ReadSubagentTool::new(ctx.clone());
    let rres = read
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(rres.get("text").unwrap().as_str(), Some("done"));

    let inspect = InspectSubagentTool::new(ctx.clone());
    let ires = inspect
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(ires.get("status").unwrap().as_str(), Some("finished"));

    let abort = AbortSubagentTool::new(ctx.clone());
    let ares = abort
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(ares.get("aborted").unwrap().as_bool(), Some(true));
}

#[tokio::test]
async fn spawn_unknown_profile_errors() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 0, db);
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx, mev_tx);
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
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx, mev_tx);
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
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx, mev_tx);
    let err = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .expect_err("depth exceeded");
    assert!(err.to_string().contains("max subagent nesting depth"));
}

#[tokio::test]
async fn spawn_requests_unenabled_middleware_errors() {
    let db = temp_db().await;
    // Parent has NO enabled middlewares (empty specs); the "heavy" profile
    // requests "shell", which is not in the parent's enabled set.
    let mut ctx = make_ctx(true, 1, db);
    ctx.catalog = catalog_with_explorer_and_heavy();
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx, mev_tx);
    let err = spawn
        .call(serde_json::json!({ "profile": "heavy", "prompt": "hi" }))
        .await
        .expect_err("unenabled middleware should be rejected");
    let msg = err.to_string();
    assert!(msg.contains("requests middleware"), "msg: {msg}");
    assert!(msg.contains("shell"), "msg: {msg}");
    assert!(msg.contains("not enabled by parent"), "msg: {msg}");
}

#[tokio::test]
async fn wait_any_timeout_against_pending_subagent() {
    let db = temp_db().await;
    let ctx = make_pending_ctx(true, 1, db);
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let spawn = SpawnSubagentTool::new(ctx.clone(), mev_tx);
    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();

    // The subagent never completes (PendingProvider). wait_any with a tiny
    // timeout must return timeout + the pending list, WITHOUT killing the
    // running subagent.
    let wait = WaitAnySubagentTool::new(ctx.clone());
    let wres = wait
        .call(serde_json::json!({ "agent_ids": [agent_id], "timeout": 0.05 }))
        .await
        .unwrap();
    assert_eq!(wres.get("status").unwrap().as_str(), Some("timeout"));
    let pending = wres.get("pending").unwrap().as_array().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].as_u64(), Some(agent_id));

    // The subagent should still be running (timeout does NOT kill it).
    let inspect = InspectSubagentTool::new(ctx.clone());
    let ires = inspect
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
    assert_eq!(ires.get("status").unwrap().as_str(), Some("running"));

    // Clean up the never-completing task so the test doesn't leak a runtime task.
    let abort = AbortSubagentTool::new(ctx);
    abort
        .call(serde_json::json!({ "agent_id": agent_id }))
        .await
        .unwrap();
}

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
        assert!(
            mev.data.is_object(),
            "data is the serialized child AgentEvent"
        );
    }
    // The child emits at least a streamEvent; ensure it's in the relayed set.
    assert!(relayed.iter().any(|mev| {
        mev.data
            .get("data")
            .and_then(|d| d.get("type"))
            .and_then(|t| t.as_str())
            == Some("streamEvent")
    }));
}

#[tokio::test]
async fn abort_all_and_clear_cancels_child_token() {
    let registry = Arc::new(SubagentRegistry::new());
    let id = registry.allocate_running();
    let cancel = registry
        .cancel_token(id)
        .expect("token present while running");
    assert!(!cancel.is_cancelled());
    let aborted = registry.abort_all_and_clear().await;
    assert_eq!(aborted, vec![id]);
    assert!(
        cancel.is_cancelled(),
        "cancel token fired by abort_all_and_clear"
    );
}

#[tokio::test]
async fn on_turn_end_aborts_running_subagent() {
    use nekocode_subagent::SubagentMiddleware;
    let db = temp_db().await;
    let ctx = make_pending_ctx(true, 1, db);
    let (mev_tx, _mev_rx) = dummy_mev_tx();
    let mw = SubagentMiddleware::from_context(ctx.clone());
    let mut reg = nekocode_core::types::GenerateRequest::default();
    let mut tools = nekocode_types::tool::ToolRegistry::new();
    mw.before_generate(&mut reg, &mut tools, &mev_tx)
        .await
        .unwrap();
    let spawn = tools.get("spawn_subagent").unwrap().clone();
    let res = spawn
        .call(serde_json::json!({ "profile": "explorer", "prompt": "hi" }))
        .await
        .unwrap();
    let agent_id = res.get("agent_id").unwrap().as_u64().unwrap();
    assert_eq!(
        ctx.registry.run_state(agent_id),
        nekocode_subagent::SubagentRunState::Running
    );

    // Parent turn ends:
    mw.on_turn_end().await.unwrap();

    // The never-completing subagent must be gone from the registry.
    assert_eq!(
        ctx.registry.run_state(agent_id),
        nekocode_subagent::SubagentRunState::Idle,
        "subagent evicted by on_turn_end"
    );
}

#[tokio::test]
async fn turn_start_renews_tree_cancellation_token() {
    use nekocode_subagent::SubagentMiddleware;
    let ctx = make_pending_ctx(true, 1, temp_db().await);
    let middleware = SubagentMiddleware::from_context(ctx.clone());

    middleware.on_turn_start().await.unwrap();
    let first = ctx
        .run_cancel
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    middleware.on_turn_end().await.unwrap();
    assert!(first.is_cancelled());

    middleware.on_turn_start().await.unwrap();
    let second = ctx
        .run_cancel
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    assert!(!second.is_cancelled());
}
