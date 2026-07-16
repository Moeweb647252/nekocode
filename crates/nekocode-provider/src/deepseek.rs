use nekocode_core::{
    provider::{Provider, ProviderError, ProviderEvent, ProviderResponse},
    types::GenerateRequest,
};
use nekocode_types::generate::Usage;
use nekocode_types::{
    config::{DeepSeekConfig, DeepSeekEndpoint},
    generate::{AssistantContentBlock, AssistantMessage, MessageContent, MessageType},
    tool::{ToolCall, ToolCallResult, ToolCallResultInner, ToolSpec},
};
use tokio::sync::mpsc::UnboundedSender;
use tracing::debug;

use crate::{
    parser::{
        anthropic::{
            AnthropicStream,
            types::{
                ContentBlockParam, CreateMessageRequest, MessageContentParam, MessageParam,
                Role as AnthropicRole, TextBlockParam,
            },
        },
        openaiv1::{
            OpenAIV1Stream,
            types::{
                ChatCompletionAssistantMessageParam, ChatCompletionMessageFunctionToolCall,
                ChatCompletionMessageParam, ChatCompletionMessageToolCall, ChatCompletionRequest,
                ChatCompletionSystemMessageParam, ChatCompletionTool, ChatCompletionToolFunction,
                ChatCompletionUserMessageParam,
            },
        },
    },
    sse::EventSource,
};

/// DeepSeek LLM backend.
///
/// Despite the name, `DeepSeek` is the workspace's concrete [`Provider`]
/// implementation and dispatches to either the Anthropic or OpenAI-compatible
/// wire format depending on the configured [`DeepSeekEndpoint`]. All request
/// shaping and stream parsing is delegated to the [`parser`](crate::parser)
/// modules.
pub(crate) struct DeepSeek {
    client: reqwest::Client,
    config: DeepSeekConfig,
}

impl DeepSeek {
    /// Construct a backend from a [`DeepSeekConfig`].
    pub(crate) fn from_config(config: DeepSeekConfig) -> Self {
        DeepSeek {
            client: reqwest::Client::new(),
            config,
        }
    }
}

#[async_trait::async_trait]
impl Provider for DeepSeek {
    async fn stream_generate(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        match self
            .config
            .endpoint
            .as_ref()
            .unwrap_or(&DeepSeekEndpoint::OpenAI)
        {
            DeepSeekEndpoint::Anthropic => self.stream_generate_anthropic(request, sender).await,
            DeepSeekEndpoint::OpenAI => self.stream_generate_openai(request, sender).await,
        }
    }
}

impl DeepSeek {
    // ── OpenAI-compatible endpoint ──

    async fn stream_generate_openai(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        let body = self.build_openai_request(&request);
        #[cfg(debug_assertions)]
        debug!("OpenAI request body: {:?}", serde_json::to_string(&body));
        let url = format!("{}/v1/chat/completions", self.config.api_base);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::HttpError(e.to_string()))?;
        if resp.status() != 200 {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderError::Other(anyhow::anyhow!(
                "OpenAI API error: status {}, body {}",
                status,
                text
            )));
        }
        let sse = resp
            .events()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;
        let mut stream = OpenAIV1Stream::new(sse);
        let mut acc = ResponseAccumulator::new();
        while let Some(event) = stream
            .next_event()
            .await
            .map_err(|e| ProviderError::HttpError(e.to_string()))?
        {
            sender.send(event.clone()).ok();
            acc.ingest(&event);
        }
        let message = acc.finish();
        let usage = stream.take_usage().unwrap_or(Usage {
            total_input: 0,
            total_output: 0,
            cache_hit: false,
            cache_miss: 0,
        });
        Ok(ProviderResponse { message, usage })
    }

    fn build_openai_request(&self, request: &GenerateRequest) -> ChatCompletionRequest {
        let mut messages: Vec<ChatCompletionMessageParam> = Vec::new();
        if let Some(system) = &request.system_prompt {
            messages.push(
                ChatCompletionMessageParam::ChatCompletionSystemMessageParam(
                    ChatCompletionSystemMessageParam {
                        content: system.clone(),
                        name: None,
                    },
                ),
            );
        }
        for msg in &request.messages {
            let param = convert_to_openai_message(msg);
            messages.push(param);
        }
        let tools = build_openai_tools(request.tool_specs());

        ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            stream: true,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            max_tokens: self.config.max_tokens,
            stop: None,
            tools,
        }
    }

    // ── Anthropic-compatible endpoint ──

    async fn stream_generate_anthropic(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        let body = self.build_anthropic_body(&request);
        #[cfg(debug_assertions)]
        debug!("Anthropic request body: {:?}", serde_json::to_string(&body));
        let url = format!("{}/v1/messages", self.config.api_base);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::HttpError(e.to_string()))?;
        let sse = resp
            .events()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;
        let mut stream = AnthropicStream::new(sse);
        let mut acc = ResponseAccumulator::new();
        while let Some(event) = stream
            .next_event()
            .await
            .map_err(|e| ProviderError::HttpError(e.to_string()))?
        {
            sender.send(event.clone()).ok();
            acc.ingest(&event);
        }
        let message = acc.finish();
        let usage = stream.take_usage().unwrap_or(Usage {
            total_input: 0,
            total_output: 0,
            cache_hit: false,
            cache_miss: 0,
        });
        Ok(ProviderResponse { message, usage })
    }

    fn build_anthropic_body(&self, request: &GenerateRequest) -> CreateMessageRequest {
        let content: Vec<MessageParam> =
            request.messages.iter().map(to_anthropic_message).collect();

        let tools = build_anthropic_tools(request.tool_specs());

        CreateMessageRequest {
            content,
            model: self.config.model.clone(),
            metadata: None,
            output_config: None,
            stop_sequences: None,
            stream: true,
            system: request.system_prompt.clone(),
            temperature: self.config.temperature,
            thinking: None,
            tool_choice: None,
            tools,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
        }
    }
}

// ── Response accumulator ──

struct ResponseAccumulator {
    blocks: Vec<AssistantContentBlock>,
}

impl ResponseAccumulator {
    fn new() -> Self {
        Self { blocks: Vec::new() }
    }

    fn ingest(&mut self, event: &ProviderEvent) {
        match event {
            ProviderEvent::Content(content) => match self.blocks.last_mut() {
                Some(AssistantContentBlock::Text {
                    content: existing, ..
                }) => {
                    existing.push_str(content);
                }
                _ => self.blocks.push(AssistantContentBlock::Text {
                    content: content.clone(),
                    reasoning_content: None,
                }),
            },
            ProviderEvent::ReasoningContent(reasoning) => match self.blocks.last_mut() {
                Some(AssistantContentBlock::Text {
                    reasoning_content, ..
                }) => {
                    reasoning_content
                        .get_or_insert_with(String::new)
                        .push_str(reasoning);
                }
                _ => self.blocks.push(AssistantContentBlock::Text {
                    content: String::new(),
                    reasoning_content: Some(reasoning.clone()),
                }),
            },
            ProviderEvent::ToolCall(tool_call) => {
                self.blocks
                    .push(AssistantContentBlock::ToolCall(tool_call.clone()));
            }
            ProviderEvent::MessageStart | ProviderEvent::MessageEnd(_) => {}
        }
    }

    fn finish(self) -> AssistantMessage {
        AssistantMessage {
            blocks: self.blocks,
        }
    }
}

// ── Tool spec conversion helpers ──

fn build_openai_tools(specs: &[ToolSpec]) -> Option<Vec<ChatCompletionTool>> {
    if specs.is_empty() {
        return None;
    }
    Some(
        specs
            .iter()
            .map(|s| ChatCompletionTool {
                tool_type: "function".into(),
                function: ChatCompletionToolFunction {
                    name: s.name.clone(),
                    description: s.description.clone(),
                    parameters: s.parameter_schema.clone(),
                },
            })
            .collect(),
    )
}

fn build_anthropic_tools(specs: &[ToolSpec]) -> Option<Vec<crate::parser::anthropic::types::Tool>> {
    if specs.is_empty() {
        return None;
    }
    Some(
        specs
            .iter()
            .map(|s| crate::parser::anthropic::types::Tool {
                name: s.name.clone(),
                description: Some(s.description.clone()),
                input_schema: s.parameter_schema.clone(),
            })
            .collect(),
    )
}

// ── Message conversion helpers ──

fn message_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text { content } => content.clone(),
    }
}

fn convert_to_openai_message(msg: &MessageType) -> ChatCompletionMessageParam {
    match msg {
        MessageType::User(blocks) => ChatCompletionMessageParam::ChatCompletionUserMessageParam(
            ChatCompletionUserMessageParam {
                content: blocks
                    .iter()
                    .map(message_text)
                    .collect::<Vec<_>>()
                    .join("\n"),
                name: None,
            },
        ),
        MessageType::Assistant(assistant) => {
            let mut content_parts = Vec::new();
            let mut reasoning_parts = Vec::new();
            let mut tool_calls = Vec::new();
            for block in &assistant.blocks {
                match block {
                    AssistantContentBlock::Text {
                        content: text,
                        reasoning_content,
                    } => {
                        content_parts.push(text.clone());
                        if let Some(r) = reasoning_content {
                            reasoning_parts.push(r.clone());
                        }
                    }
                    AssistantContentBlock::ToolCall(tc) => {
                        tool_calls.push(tc.clone());
                    }
                }
            }
            let has_tool_calls = !tool_calls.is_empty();
            ChatCompletionMessageParam::ChatCompletionAssistantMessageParam(
                ChatCompletionAssistantMessageParam {
                    content: content_parts.join(""),
                    name: None,
                    refusal: None,
                    tool_calls: convert_tool_calls(&tool_calls),
                    prefix: None,
                    // Reasoning must only be passed back when the model
                    // performed a tool call; otherwise the API ignores it.
                    reasoning_content: if has_tool_calls && !reasoning_parts.is_empty() {
                        Some(reasoning_parts.join(""))
                    } else {
                        None
                    },
                },
            )
        }
        MessageType::MiddlewareMessage(content) => {
            ChatCompletionMessageParam::ChatCompletionUserMessageParam(
                ChatCompletionUserMessageParam {
                    content: message_text(content),
                    name: None,
                },
            )
        }
        MessageType::ToolCallResult(result) => {
            ChatCompletionMessageParam::ChatCompletionToolMessageParam(
                crate::parser::openaiv1::types::ChatCompletionToolMessageParam {
                    content: tool_result_text(result),
                    tool_call_id: result.id.clone(),
                },
            )
        }
    }
}

fn convert_tool_calls(tool_calls: &[ToolCall]) -> Option<Vec<ChatCompletionMessageToolCall>> {
    if tool_calls.is_empty() {
        return None;
    }
    Some(
        tool_calls
            .iter()
            .map(|tc| {
                ChatCompletionMessageToolCall::ChatCompletionMessageFunctionToolCall(
                    ChatCompletionMessageFunctionToolCall {
                        id: tc.id.clone(),
                        function: crate::parser::openaiv1::types::Function {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.args).unwrap_or_default(),
                        },
                    },
                )
            })
            .collect(),
    )
}

fn tool_result_text(result: &ToolCallResult) -> String {
    match &result.result {
        ToolCallResultInner::Success { value } => value.to_string(),
        ToolCallResultInner::Error { error } => error.clone(),
    }
}

fn to_anthropic_message(msg: &MessageType) -> MessageParam {
    match msg {
        MessageType::User(blocks) => MessageParam {
            role: AnthropicRole::User,
            content: MessageContentParam::Blocks(
                blocks
                    .iter()
                    .map(|b| {
                        ContentBlockParam::TextBlockParam(TextBlockParam {
                            text: message_text(b),
                        })
                    })
                    .collect(),
            ),
        },
        MessageType::Assistant(assistant) => {
            let mut blocks: Vec<ContentBlockParam> = Vec::new();
            for block in &assistant.blocks {
                match block {
                    AssistantContentBlock::Text {
                        content: text,
                        reasoning_content: _,
                    } => {
                        blocks.push(ContentBlockParam::TextBlockParam(TextBlockParam {
                            text: text.clone(),
                        }));
                    }
                    AssistantContentBlock::ToolCall(tc) => {
                        blocks.push(ContentBlockParam::ToolUseBlockParam(
                            crate::parser::anthropic::types::ToolUseBlockParam {
                                id: tc.id.clone(),
                                name: tc.name.clone(),
                                input: tc.args.clone(),
                            },
                        ));
                    }
                }
            }
            MessageParam {
                role: AnthropicRole::Assistant,
                content: MessageContentParam::Blocks(blocks),
            }
        }
        MessageType::MiddlewareMessage(content) => MessageParam {
            role: AnthropicRole::User,
            content: MessageContentParam::Blocks(vec![ContentBlockParam::TextBlockParam(
                TextBlockParam {
                    text: message_text(content),
                },
            )]),
        },
        MessageType::ToolCallResult(result) => {
            let (text, is_error) = match &result.result {
                ToolCallResultInner::Success { value } => (value.to_string(), None),
                ToolCallResultInner::Error { error } => (error.clone(), Some(true)),
            };
            MessageParam {
                role: AnthropicRole::User,
                content: MessageContentParam::Blocks(vec![
                    ContentBlockParam::ToolResultBlockParam(
                        crate::parser::anthropic::types::ToolResultBlockParam {
                            tool_use_id: result.id.clone(),
                            content: vec![
                                crate::parser::anthropic::types::ContentBlock::TextBlock { text },
                            ],
                            is_error,
                        },
                    ),
                ]),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulator_preserves_text_tool_text_order() {
        let mut accumulator = ResponseAccumulator::new();
        accumulator.ingest(&ProviderEvent::Content("before".into()));
        accumulator.ingest(&ProviderEvent::ToolCall(ToolCall {
            id: "call_1".into(),
            name: "echo".into(),
            args: serde_json::json!({}),
        }));
        accumulator.ingest(&ProviderEvent::Content("after".into()));

        let message = accumulator.finish();
        assert_eq!(message.blocks.len(), 3);
        assert!(matches!(
            &message.blocks[0],
            AssistantContentBlock::Text { content, .. } if content == "before"
        ));
        assert!(matches!(
            message.blocks[1],
            AssistantContentBlock::ToolCall(_)
        ));
        assert!(matches!(
            &message.blocks[2],
            AssistantContentBlock::Text { content, .. } if content == "after"
        ));
    }

    #[test]
    fn anthropic_tool_result_contains_payload_and_error_flag() {
        let result = MessageType::ToolCallResult(ToolCallResult {
            id: "call_1".into(),
            result: ToolCallResultInner::Error {
                error: "failed".into(),
            },
        });

        let encoded = serde_json::to_value(to_anthropic_message(&result)).unwrap();
        assert_eq!(encoded["content"][0]["tool_use_id"], "call_1");
        assert_eq!(encoded["content"][0]["is_error"], true);
        assert_eq!(encoded["content"][0]["content"][0]["text"], "failed");
    }
}
