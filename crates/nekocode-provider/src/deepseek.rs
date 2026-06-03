use nekocode_core::{
    provider::{
        GenerateOption, Message, MessageContent, Provider, ProviderError, ProviderEvent,
        ProviderResponse, Role,
    },
    types::GenerateRequest,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    parser::{
        anthropic::AnthropicStream,
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

pub struct Config {
    pub api_base: String,
    pub api_key: String,
    pub model: String,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub max_tokens: Option<usize>,
    pub context_window_size: Option<usize>,
    pub endpoint: Option<DeepSeekEndpoint>,
}

#[derive(Serialize, Deserialize)]
pub enum DeepSeekEndpoint {
    #[serde(rename = "anthropic")]
    Anthropic,
    #[serde(rename = "openai")]
    OpenAI,
}

pub struct DeepSeek {
    client: reqwest::Client,
    config: Config,
}

impl DeepSeek {
    pub fn from_config(config: Config) -> Self {
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
        match self.config.endpoint.as_ref().unwrap_or(&DeepSeekEndpoint::OpenAI) {
            DeepSeekEndpoint::Anthropic => {
                self.stream_generate_anthropic(request, sender).await
            }
            DeepSeekEndpoint::OpenAI => {
                self.stream_generate_openai(request, sender).await
            }
        }
    }

    async fn generate(&self, request: GenerateRequest) -> Result<ProviderResponse, ProviderError> {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        self.stream_generate(request, tx).await
    }
}

impl DeepSeek {
    /// Build the effective options by merging config defaults with per-request options.
    fn effective_options(&self, options: &GenerateOption) -> GenerateOption {
        GenerateOption {
            model: if options.model.is_empty() {
                self.config.model.clone()
            } else {
                options.model.clone()
            },
            max_tokens: options.max_tokens.or(self.config.max_tokens),
            top_p: options.top_p.or(self.config.top_p),
            top_k: options.top_k.or(self.config.top_k),
            temperature: options.temperature.or(self.config.temperature),
        }
    }

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
        Ok(ProviderResponse {})
    }

    fn build_openai_request(&self, request: &GenerateRequest) -> ChatCompletionRequest {
        let mut messages: Vec<ChatCompletionMessageParam> = Vec::new();
        if let Some(system) = &request.system_prompt {
            messages.push(ChatCompletionMessageParam::ChatCompletionSystemMessageParam(
                ChatCompletionSystemMessageParam {
                    content: system.clone(),
                    name: None,
                },
            ));
        }
        for msg in &request.messages {
            let param = convert_to_openai_message(msg);
            messages.push(param);
        }
        let opts = self.effective_options(&request.options);
        ChatCompletionRequest {
            model: opts.model,
            messages,
            stream: true,
            temperature: opts.temperature,
            top_p: opts.top_p,
            max_tokens: opts.max_tokens,
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
        Ok(ProviderResponse {})
    }

    fn build_anthropic_body(&self, request: &GenerateRequest) -> serde_json::Value {
        let opts = self.effective_options(&request.options);
        let messages: Vec<serde_json::Value> = request
            .messages
            .iter()
            .map(|msg| {
                let role_str = match msg.role {
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Custom(ref s) => s.as_str(),
                };
                let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                match &msg.content {
                    MessageContent::Text(text) => {
                        content_blocks.push(serde_json::json!({
                            "type": "text",
                            "text": text,
                        }));
                    }
                }
                if let Some(reasoning) = &msg.reasoning_content {
                    content_blocks.push(serde_json::json!({
                        "type": "thinking",
                        "thinking": reasoning,
                        "signature": "",
                    }));
                }
                serde_json::json!({
                    "role": role_str,
                    "content": content_blocks,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": opts.model,
            "messages": messages,
            "stream": true,
        });
        if let Some(system) = &request.system_prompt {
            body["system"] = serde_json::json!(system);
        }
        if let Some(max_tokens) = opts.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tokens);
        }
        if let Some(temperature) = opts.temperature {
            body["temperature"] = serde_json::json!(temperature);
        }
        if let Some(top_p) = opts.top_p {
            body["top_p"] = serde_json::json!(top_p);
        }
        if let Some(top_k) = opts.top_k {
            body["top_k"] = serde_json::json!(top_k);
        }
        body
    }
}

fn convert_to_openai_message(msg: &Message) -> ChatCompletionMessageParam {
    let content = match &msg.content {
        MessageContent::Text(text) => text.clone(),
    };
    match msg.role {
        Role::User => {
            ChatCompletionMessageParam::ChatCompletionUserMessageParam(
                ChatCompletionUserMessageParam {
                    content,
                    name: None,
                },
            )
        }
        Role::Assistant => {
            ChatCompletionMessageParam::ChatCompletionAssistantMessageParam(
                ChatCompletionAssistantMessageParam {
                    content,
                    name: None,
                    refusal: None,
                    tool_calls: None,
                    prefix: None,
                    reasoning_content: msg.reasoning_content.clone(),
                },
            )
        }
        Role::Custom(_) => {
            ChatCompletionMessageParam::ChatCompletionSystemMessageParam(
                ChatCompletionSystemMessageParam {
                    content,
                    name: None,
                },
            )
        }
    }
}
