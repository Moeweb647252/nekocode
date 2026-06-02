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
