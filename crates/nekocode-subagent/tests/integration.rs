//! Integration tests for nekocode-subagent tools. Builds a SubagentContext
//! directly (fields are pub) and invokes tools, bypassing
//! SubagentMiddleware::new (which loads agents.toml from the real config dir,
//! not controllable from tests). The provider/factory are local mocks.

use std::sync::{Arc, Mutex};

use nekocode_core::middleware::{Middleware, MiddlewareSpec};
use nekocode_core::provider::{Provider, ProviderError, ProviderEvent, ProviderResponse};
use nekocode_subagent::{
    ProfileCatalog, SubagentContext, SubagentMiddlewareFactory, SubagentProfile, SubagentRegistry,
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
    }
}

#[tokio::test]
async fn spawn_wait_read_lifecycle() {
    let db = temp_db().await;
    let ctx = make_ctx(true, 1, db); // max_depth=1 → depth 0+1 <= 1, spawn allowed
    let spawn = SpawnSubagentTool::new(ctx.clone());
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
