use async_trait::async_trait;
use nekocode_types::{
    generate::MessageContent,
    tool::ToolRegistry,
};

use crate::types::{GenerateRequest, GenerateResponse};

/// How the agent run loop should proceed after `after_generate` runs.
#[derive(Debug, Clone)]
pub enum AgentControlFlow {
    /// Stop the run loop and return the current response to the client.
    Output,
    /// Inject `content` as a middleware message and run another outer-loop
    /// generation with the same system prompt and tools.
    GenerateWith(MessageContent),
}

/// Name + raw config — enough for the API layer to rebuild an isolated
/// middleware instance for a subagent. Defined here (in nekocode-core) so
/// `nekocode-subagent`, which depends only on core + types, can refer to it
/// by name without seeing the individual middleware crates.
#[derive(Debug, Clone)]
pub struct MiddlewareSpec {
    pub name: String,
    pub config: serde_json::Value,
}

#[async_trait]
pub trait Middleware: Send + Sync {
    async fn before_generate(
        &self,
        _: &mut GenerateRequest,
        _: &mut ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(
        &self,
        _: &GenerateResponse,
        _: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
