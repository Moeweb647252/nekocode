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
        while let Some(event) = stream.next_event().await? {
            sender.send(event).ok();
        }
        Ok(ProviderResponse {
            message: Message::User(MessageContent::Text(String::new())),
            usage: ProviderUsage {
                total_input: 0,
                total_output: 0,
                cache_hit: false,
                cache_miss: 0,
            },
        })
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
        while let Some(event) = stream.next_event().await? {
            sender.send(event).ok();
        }
        Ok(ProviderResponse {
            message: Message::User(MessageContent::Text(String::new())),
            usage: ProviderUsage {
                total_input: 0,
                total_output: 0,
                cache_hit: false,
                cache_miss: 0,
            },
        })
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
        Message::Assistant(AssistantMessage {
            blocks,
            reasoning,
            tool_calls,
        }) => {
            let content = blocks
                .iter()
                .filter_map(|b| match b {
                    AssistantContentBlock::Text(text) => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");
            let extra_reasoning: Vec<_> = blocks
                .iter()
                .filter_map(|b| match b {
                    AssistantContentBlock::Reasoning(r) => Some(r.as_str()),
                    _ => None,
                })
                .collect();
            let combined_reasoning: Vec<&str> = reasoning
                .as_deref()
                .into_iter()
                .chain(extra_reasoning)
                .collect();
            let reasoning_content = if combined_reasoning.is_empty() {
                None
            } else {
                Some(combined_reasoning.join(""))
            };
            ChatCompletionMessageParam::ChatCompletionAssistantMessageParam(
                ChatCompletionAssistantMessageParam {
                    content,
                    name: None,
                    refusal: None,
                    tool_calls: convert_tool_calls(tool_calls),
                    prefix: None,
                    reasoning_content,
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
        Message::Assistant(AssistantMessage {
            blocks: assistant_blocks,
            reasoning,
            tool_calls,
        }) => {
            let mut blocks: Vec<ContentBlockParam> = Vec::new();
            for block in assistant_blocks {
                match block {
                    AssistantContentBlock::Text(text) => {
                        blocks.push(ContentBlockParam::TextBlockParam(TextBlockParam {
                            text: text.clone(),
                        }));
                    }
                    AssistantContentBlock::Reasoning(r) => {
                        blocks.push(ContentBlockParam::ThinkingBlockParam {
                            signature: String::new(),
                            thinking: r.clone(),
                        });
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
            if let Some(reasoning) = reasoning {
                blocks.push(ContentBlockParam::ThinkingBlockParam {
                    signature: String::new(),
                    thinking: reasoning.clone(),
                });
            }
            for tc in tool_calls {
                blocks.push(ContentBlockParam::ToolUseBlockParam(
                    crate::parser::anthropic::types::ToolUseBlockParam {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        input: tc.args.clone(),
                    },
                ));
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
