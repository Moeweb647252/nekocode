use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallResult {
    pub id: String,
    pub result: ToolCallResultInner,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ToolCallResultInner {
    #[serde(rename = "success")]
    Success(serde_json::Value),
    #[serde(rename = "error")]
    Error(String),
}

impl From<Result<serde_json::Value, ToolError>> for ToolCallResultInner {
    fn from(value: Result<serde_json::Value, ToolError>) -> Self {
        match value {
            Ok(result) => ToolCallResultInner::Success(result),
            Err(err) => ToolCallResultInner::Error(err.to_string()),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameter_schema: serde_json::Value,
}

use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;

#[derive(Debug, thiserror::Error)]
pub enum ToolError {}

#[async_trait]
pub trait Tool {
    async fn call(&self, params: serde_json::Value) -> Result<serde_json::Value, ToolError>;
}

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool + Send + Sync>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn insert(&mut self, name: String, tool: Arc<dyn Tool + Send + Sync>) {
        self.tools.insert(name, tool);
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool + Send + Sync>> {
        self.tools.get(name).cloned()
    }
}
