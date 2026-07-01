//! Shared test mocks for agent tests. Only compiled in `#[cfg(test)]`.

use std::sync::{Arc, Mutex};

use crate::{
    middleware::{AgentControlFlow, Middleware},
    provider::{Provider, ProviderError, ProviderEvent, ProviderResponse},
    types::{GenerateRequest, GenerateResponse},
};
use async_trait::async_trait;
use nekocode_types::{
    generate::{AssistantMessage, AssistantContentBlock, MessageContent, StopReason, Usage},
    tool::{Tool, ToolCall, ToolRegistry, ToolSpec},
};
use tokio::sync::mpsc::UnboundedSender;

// ── MockProvider ──

pub struct MockProvider {
    responses: Mutex<Vec<AssistantMessage>>,
}

impl MockProvider {
    pub fn new(responses: Vec<AssistantMessage>) -> Self {
        let mut r = responses;
        r.reverse(); // pop() is LIFO; reverse once for FIFO
        Self {
            responses: Mutex::new(r),
        }
    }
}

#[async_trait]
impl Provider for MockProvider {
    async fn stream_generate(
        &self,
        _request: GenerateRequest,
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
            if let AssistantContentBlock::ToolCall(tc) = block {
                sender.send(ProviderEvent::ToolCall(tc.clone())).unwrap();
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

pub fn text_msg(s: &str) -> AssistantMessage {
    AssistantMessage {
        blocks: vec![AssistantContentBlock::Text {
            content: s.into(),
            reasoning_content: None,
        }],
    }
}

pub fn toolcall_msg(id: &str, name: &str, args: serde_json::Value) -> AssistantMessage {
    AssistantMessage {
        blocks: vec![AssistantContentBlock::ToolCall(ToolCall {
            id: id.into(),
            name: name.into(),
            args,
        })],
    }
}

// ── Echo middleware ──

struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "echo".into(),
            parameter_schema: serde_json::json!({
                "type": "object",
                "properties": { "value": { "type": "string" } },
                "required": ["value"]
            }),
        }
    }

    async fn call(
        &self,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, nekocode_types::tool::ToolError> {
        Ok(params)
    }
}

pub struct EchoMiddleware;

#[async_trait]
impl Middleware for EchoMiddleware {
    async fn before_generate(
        &self,
        _req: &mut GenerateRequest,
        reg: &mut ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<crate::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        reg.insert("echo".into(), Arc::new(EchoTool));
        Ok(())
    }

    async fn after_generate(
        &self,
        _resp: &GenerateResponse,
        _cf: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

// ── InjectMiddleware ──

pub struct InjectMiddleware(pub AgentControlFlow);

#[async_trait]
impl Middleware for InjectMiddleware {
    async fn before_generate(
        &self,
        _req: &mut GenerateRequest,
        _reg: &mut ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<crate::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(
        &self,
        _resp: &GenerateResponse,
        cf: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        *cf = self.0.clone();
        Ok(())
    }
}

// ── OneShotRegenerateMiddleware ──

/// Middleware that fires `GenerateWith` exactly once on the first
/// `after_generate` call. Used to test the outer middleware loop.
pub struct OneShotRegenerateMiddleware {
    pub fired: std::sync::Mutex<bool>,
    pub inject: String,
}

#[async_trait]
impl Middleware for OneShotRegenerateMiddleware {
    async fn before_generate(
        &self,
        _req: &mut GenerateRequest,
        _reg: &mut ToolRegistry,
        _: &tokio::sync::mpsc::UnboundedSender<crate::agent::MiddlewareEvent>,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(
        &self,
        _resp: &GenerateResponse,
        flow: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        let mut g = self.fired.lock().unwrap();
        if !*g {
            *g = true;
            *flow = AgentControlFlow::GenerateWith(MessageContent::Text {
                content: self.inject.clone(),
            });
        }
        Ok(())
    }
}

// ── RelayMiddleware ──

/// Emits one MiddlewareEvent into the mev_tx it receives in
/// `before_generate`. Used to test run_loop's merge relay.
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