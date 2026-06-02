use async_trait::async_trait;

use crate::{
    agent::tool::ToolRegistry,
    types::{GenerateRequest, GenerateResponse},
};

pub enum AgentControlFlow {
    Output,
    GenerateWith(GenerateRequest),
}

#[async_trait]
pub trait Middleware: Send + Sync {
    async fn before_generate(
        &self,
        request: &mut GenerateRequest,
        tool_registry: &ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(
        &self,
        request: &GenerateRequest,
        response: &GenerateResponse,
        control_flow: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
