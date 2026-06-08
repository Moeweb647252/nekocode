use nekocode_core::{
    provider::{Provider, ProviderError, ProviderEvent, ProviderResponse, ProviderUsage},
    types::GenerateRequest,
};
use nekocode_types::{
    config::{DeepSeekConfig, DeepSeekEndpoint},
    generate::{AssistantContentBlock, AssistantMessage, Message, MessageContent},
    tool::{ToolCall, ToolCallResult, ToolCallResultInner},
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
                ChatCompletionAssistantMessageParam, ChatCompletionMessageParam,
                ChatCompletionRequest, ChatCompletionSystemMessageParam,
                ChatCompletionUserMessageParam,
            },
        },
    },
    sse::EventSource,
};

pub struct DeepSeek {
    client: reqwest::Client,
    config: DeepSeekConfig,
}

impl DeepSeek {
    pub fn from_config(config: DeepSeekConfig) -> Self {
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

    async fn generate(&self, request: GenerateRequest) -> Result<ProviderResponse, ProviderError> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        self.stream_generate(request, tx).await
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
        debug!("OpenAI request body: {:?}", body);
        let url = format!("{}/v1/chat/completions", self.config.api_base);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .json(&body)
            .send()
            .await?;
        let sse = resp
            .events()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;
        let mut stream = OpenAIV1Stream::new(sse);
        let mut acc = ResponseAccumulator::new();
        while let Some(event) = stream.next_event().await? {
            let event = event; // re-bind as immutable
            sender.send(event.clone()).ok();
            acc.ingest(&event);
        }
        let message = acc.finish();
        let usage = stream.take_usage().unwrap_or(ProviderUsage {
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
        ChatCompletionRequest {
            model: self.config.model.clone(),
            messages,
            stream: true,
            temperature: self.config.temperature,
            top_p: self.config.top_p,
            max_tokens: self.config.max_tokens,
            stop: None,
        }
    }

    // ── Anthropic-compatible endpoint ──

    async fn stream_generate_anthropic(
        &self,
        request: GenerateRequest,
        sender: UnboundedSender<ProviderEvent>,
    ) -> Result<ProviderResponse, ProviderError> {
        let body = self.build_anthropic_body(&request);
        debug!("Anthropic request body: {:?}", body);
        let url = format!("{}/v1/messages", self.config.api_base);
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?;
        let sse = resp
            .events()
            .await
            .map_err(|e| ProviderError::Other(e.into()))?;
        let mut stream = AnthropicStream::new(sse);
        let mut acc = ResponseAccumulator::new();
        while let Some(event) = stream.next_event().await? {
            sender.send(event.clone()).ok();
            acc.ingest(&event);
        }
        let message = acc.finish();
        let usage = stream.take_usage().unwrap_or(ProviderUsage {
            total_input: 0,
            total_output: 0,
            cache_hit: false,
            cache_miss: 0,
        });
        Ok(ProviderResponse { message, usage })
    }

    fn build_anthropic_body(&self, request: &GenerateRequest) -> CreateMessageRequest {
        let content: Vec<MessageParam> = request
            .messages
            .iter()
            .map(|msg| to_anthropic_message(msg))
            .collect();

        CreateMessageRequest {
            content,
            model: self.config.model.clone(),
            metadata: None,
            ouput_config: None,
            stop_sequences: None,
            stream: true,
            system: request.system_prompt.clone(),
            temperature: self.config.temperature,
            thinking: None,
            tool_choice: None,
            tools: None,
            top_p: self.config.top_p,
            top_k: self.config.top_k,
            max_tokens: self.config.max_tokens,
        }
    }
}

// ── Response accumulator ──

struct ResponseAccumulator {
    text: String,
    reasoning: String,
    tool_calls: Vec<ToolCall>,
}

impl ResponseAccumulator {
    fn new() -> Self {
        Self {
            text: String::new(),
            reasoning: String::new(),
            tool_calls: Vec::new(),
        }
    }

    fn ingest(&mut self, event: &ProviderEvent) {
        match event {
            ProviderEvent::Content(s) => self.text.push_str(s),
            ProviderEvent::ReasoningContent(s) => self.reasoning.push_str(s),
            ProviderEvent::ToolCall(tc) => self.tool_calls.push(tc.clone()),
            ProviderEvent::MessageStart | ProviderEvent::MessageEnd => {}
        }
    }

    fn finish(self) -> AssistantMessage {
        let mut blocks: Vec<AssistantContentBlock> = Vec::new();

        if !self.text.is_empty() {
            blocks.push(AssistantContentBlock::Text {
                content: self.text,
                reasoning_content: if self.reasoning.is_empty() {
                    None
                } else {
                    Some(self.reasoning)
                },
            });
        } else if !self.reasoning.is_empty() {
            blocks.push(AssistantContentBlock::Text {
                content: String::new(),
                reasoning_content: Some(self.reasoning),
            });
        }

        for tc in self.tool_calls {
            blocks.push(AssistantContentBlock::ToolCall(tc));
        }

        AssistantMessage { blocks }
    }
}

// ── Message conversion helpers ──

fn message_text(content: &MessageContent) -> String {
    match content {
        MessageContent::Text(text) => text.clone(),
    }
}

fn convert_to_openai_message(msg: &Message) -> ChatCompletionMessageParam {
    match msg {
        Message::User(content) => ChatCompletionMessageParam::ChatCompletionUserMessageParam(
            ChatCompletionUserMessageParam {
                content: message_text(content),
                name: None,
            },
        ),
        Message::Assistant(assistant) => {
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
        Message::MiddlewareMessage(content) => {
            ChatCompletionMessageParam::ChatCompletionUserMessageParam(
                ChatCompletionUserMessageParam {
                    content: message_text(content),
                    name: None,
                },
            )
        }
        Message::ToolCallResult(result) => {
            ChatCompletionMessageParam::ChatCompletionToolMessageParam(
                crate::parser::openaiv1::types::ChatCompletionToolMessageParam {
                    content: tool_result_text(result),
                    tool_call_id: result.id.clone(),
                },
            )
        }
    }
}

fn convert_tool_calls(
    tool_calls: &[ToolCall],
) -> Option<Vec<crate::parser::openaiv1::types::ChatCompletionMessageToolCall>> {
    if tool_calls.is_empty() {
        return None;
    }
    Some(
        tool_calls
            .iter()
            .map(|tc| {
                crate::parser::openaiv1::types::ChatCompletionMessageToolCall::ChatCompletionMessageFunctionToolCall(
                    crate::parser::openaiv1::types::ChatCompletionMessageFunctionToolCall {
                        name: tc.name.clone(),
                        arguments: tc.args.to_string(),
                    },
                )
            })
            .collect(),
    )
}

fn tool_result_text(result: &ToolCallResult) -> String {
    match &result.result {
        ToolCallResultInner::Success(val) => val.to_string(),
        ToolCallResultInner::Error(err) => err.clone(),
    }
}

fn to_anthropic_message(msg: &Message) -> MessageParam {
    match msg {
        Message::User(content) => MessageParam {
            role: AnthropicRole::User,
            content: MessageContentParam::Blocks(vec![ContentBlockParam::TextBlockParam(
                TextBlockParam {
                    text: message_text(content),
                },
            )]),
        },
        Message::Assistant(assistant) => {
            let has_tool_calls = assistant
                .blocks
                .iter()
                .any(|b| matches!(b, AssistantContentBlock::ToolCall(_)));
            let mut blocks: Vec<ContentBlockParam> = Vec::new();
            for block in &assistant.blocks {
                match block {
                    AssistantContentBlock::Text {
                        content: text,
                        reasoning_content,
                    } => {
                        blocks.push(ContentBlockParam::TextBlockParam(TextBlockParam {
                            text: text.clone(),
                        }));
                        // Thinking must only be passed back when the model
                        // performed a tool call; otherwise the API ignores it.
                        if has_tool_calls {
                            if let Some(r) = reasoning_content {
                                blocks.push(ContentBlockParam::ThinkingBlockParam {
                                    signature: String::new(),
                                    thinking: r.clone(),
                                });
                            }
                        }
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
        Message::MiddlewareMessage(content) => MessageParam {
            role: AnthropicRole::User,
            content: MessageContentParam::Blocks(vec![ContentBlockParam::TextBlockParam(
                TextBlockParam {
                    text: message_text(content),
                },
            )]),
        },
        Message::ToolCallResult(result) => MessageParam {
            role: AnthropicRole::User,
            content: MessageContentParam::Blocks(vec![ContentBlockParam::ToolResultBlockParam(
                crate::parser::anthropic::types::ToolResultBlockParam {
                    tool_use_id: result.id.clone(),
                    content: vec![],
                },
            )]),
        },
    }
}
