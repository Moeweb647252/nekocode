use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use nekocode_core::agent::subagent::SubAgentRegistry;
use nekocode_core::middleware::Middleware;
use nekocode_core::provider::{Provider, ProviderError, ProviderResponse};
use nekocode_core::types::GenerateRequest;
use nekocode_subagent::SubagentMiddleware;
use nekocode_subagent::config::SubagentConfig;
use nekocode_types::generate::{AssistantContentBlock, AssistantMessage, StopReason, Usage};
use nekocode_types::tool::ToolRegistry;

/// A mock provider that returns exactly one text response, then errors.
#[derive(Clone)]
struct FixedResponseProvider {
    response: &'static str,
}

#[async_trait]
impl Provider for FixedResponseProvider {
    async fn stream_generate(
        &self,
        _request: GenerateRequest,
        sender: tokio::sync::mpsc::UnboundedSender<nekocode_core::provider::ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        // Emit Content then MessageEnd
        let _ = sender.send(nekocode_core::provider::ProviderEvent::Content(
            self.response.to_string(),
        ));
        let _ = sender.send(nekocode_core::provider::ProviderEvent::MessageEnd(
            StopReason::Stop,
        ));
        Ok(ProviderResponse {
            message: AssistantMessage {
                blocks: vec![AssistantContentBlock::Text {
                    content: self.response.to_string(),
                    reasoning_content: None,
                }],
            },
            usage: nekocode_types::generate::Usage {
                total_input: 5,
                total_output: 10,
                cache_hit: false,
                cache_miss: 5,
            },
        })
    }
}

fn setup_middleware() -> (Arc<dashmap::DashMap<String, Box<dyn std::any::Any + Send + Sync>>>, SubagentMiddleware) {
    let extensions = Arc::new(dashmap::DashMap::new());
    let provider = Arc::new(FixedResponseProvider { response: "hello from subagent" });
    let middleware = SubagentMiddleware::new(
        extensions.clone(),
        provider,
        SubagentConfig { allow_subagent: true },
    );
    (extensions, middleware)
}

#[tokio::test]
async fn spawn_and_status() {
    let (_extensions, middleware) = setup_middleware();
    let mut registry = ToolRegistry::new();

    // Register tools via before_generate
    middleware
        .before_generate(&mut GenerateRequest::default(), &mut registry)
        .await
        .unwrap();

    // Call spawn_subagent
    let spawn_tool = registry.get("spawn_subagent").unwrap();
    let result = spawn_tool
        .call(serde_json::json!({"user_prompt": "test"}))
        .await
        .unwrap();
    let subagent_id = result["subagent_id"].as_u64().expect("got subagent_id");
    assert_eq!(result["status"], "started");

    // Poll status until finished (with timeout)
    let status_tool = registry.get("subagent_status").unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut final_status = None;
    while tokio::time::Instant::now() < deadline {
        let status = status_tool
            .call(serde_json::json!({"subagent_id": subagent_id}))
            .await
            .unwrap();
        let s = status["status"].as_str().unwrap();
        if s == "finished" {
            final_status = Some(status);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let status = final_status.expect("subagent should finish within timeout");
    assert_eq!(status["summary_text"], "hello from subagent");
    assert!(status["message_count"].as_u64().unwrap_or(0) >= 2); // user msg + assistant
}

#[tokio::test]
async fn wait_one_subagent() {
    let (_extensions, middleware) = setup_middleware();
    let mut registry = ToolRegistry::new();
    middleware
        .before_generate(&mut GenerateRequest::default(), &mut registry)
        .await
        .unwrap();

    let spawn_tool = registry.get("spawn_subagent").unwrap();
    let result = spawn_tool
        .call(serde_json::json!({"user_prompt": "test"}))
        .await
        .unwrap();
    let subagent_id = result["subagent_id"].as_u64().unwrap();

    let wait_one_tool = registry.get("wait_one_subagent").unwrap();
    let result = wait_one_tool
        .call(serde_json::json!({
            "subagent_id": subagent_id,
            "timeout_secs": 5,
        }))
        .await
        .unwrap();
    assert_eq!(result["status"], "ready");
    assert_eq!(result["summary_text"], "hello from subagent");
}

#[tokio::test]
async fn wait_one_subagent_timeout() {
    // Create a subagent with an Idle entry (never spawned) -> times out.
    let (extensions, middleware) = setup_middleware();
    let mut registry = ToolRegistry::new();
    middleware
        .before_generate(&mut GenerateRequest::default(), &mut registry)
        .await
        .unwrap();
    let registry_inner: Arc<SubAgentRegistry> = {
        let ext = extensions.get("subagents").unwrap();
        // The registry was inserted as Box<Arc<SubAgentRegistry>>, so
        // we need to downcast the box to get it out.
        let boxed: &Box<dyn std::any::Any + Send + Sync> = ext.value();
        boxed.downcast_ref::<Arc<SubAgentRegistry>>()
            .expect("subagents extension should be Arc<SubAgentRegistry>")
            .clone()
    };
    let id = registry_inner.allocate(); // Idle, never spawned

    let wait_one_tool = registry.get("wait_one_subagent").unwrap();
    let result = wait_one_tool
        .call(serde_json::json!({
            "subagent_id": id,
            "timeout_secs": 0.1,
        }))
        .await
        .unwrap();
    assert_eq!(result["status"], "timeout");
}

#[tokio::test]
async fn wait_all_subagent() {
    let (_extensions, middleware) = setup_middleware();
    let mut registry = ToolRegistry::new();
    middleware
        .before_generate(&mut GenerateRequest::default(), &mut registry)
        .await
        .unwrap();

    let spawn_tool = registry.get("spawn_subagent").unwrap();

    // Spawn two subagents
    let id1 = spawn_tool
        .call(serde_json::json!({"user_prompt": "task1"}))
        .await
        .unwrap()["subagent_id"]
        .as_u64()
        .unwrap();
    let id2 = spawn_tool
        .call(serde_json::json!({"user_prompt": "task2"}))
        .await
        .unwrap()["subagent_id"]
        .as_u64()
        .unwrap();

    let wait_all_tool = registry.get("wait_all_subagent").unwrap();
    let result = wait_all_tool
        .call(serde_json::json!({
            "subagent_ids": [id1, id2],
            "timeout_secs": 5,
        }))
        .await
        .unwrap();
    assert_eq!(result["status"], "ready");
    let results = result["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
}

#[tokio::test]
async fn wait_all_defaults_to_running() {
    let (_extensions, middleware) = setup_middleware();
    let mut registry = ToolRegistry::new();
    middleware
        .before_generate(&mut GenerateRequest::default(), &mut registry)
        .await
        .unwrap();

    let spawn_tool = registry.get("spawn_subagent").unwrap();
    spawn_tool
        .call(serde_json::json!({"user_prompt": "task"}))
        .await
        .unwrap();

    // wait_all without specifying ids should wait for all running subagents
    let wait_all_tool = registry.get("wait_all_subagent").unwrap();
    let result = wait_all_tool
        .call(serde_json::json!({"timeout_secs": 5}))
        .await
        .unwrap();
    assert_eq!(result["status"], "ready");
    assert!(!result["results"].as_array().unwrap().is_empty());
}
