use async_trait::async_trait;
use nekocode_types::tool::ToolRegistry;

use crate::types::{GenerateRequest, GenerateResponse};

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
        _: &GenerateResponse,
        _: &mut AgentControlFlow,
    ) -> Result<(), anyhow::Error> {
        Ok(())
    }
}
