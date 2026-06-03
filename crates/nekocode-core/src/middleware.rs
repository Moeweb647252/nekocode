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
        _: &mut GenerateRequest,
        _: &ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }

    async fn after_generate(
        &self,
        _: &GenerateRequest,
        _: &GenerateResponse,
        _: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
