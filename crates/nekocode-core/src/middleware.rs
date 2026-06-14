use async_trait::async_trait;
use nekocode_types::{
    generate::MessageContent,
    tool::ToolRegistry,
};

use crate::types::{GenerateRequest, GenerateResponse};

/// How the agent run loop should proceed after `after_generate` runs.
pub enum AgentControlFlow {
    /// Stop the run loop and return the current response to the client.
    Output,
    /// Inject `content` as a middleware message and run another outer-loop
    /// generation with the same system prompt and tools.
    GenerateWith(MessageContent),
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
