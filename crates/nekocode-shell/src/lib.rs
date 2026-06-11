use nekocode_core::middleware::Middleware;

pub mod config;
pub mod tool;

pub struct Shell {}

#[async_trait::async_trait]
impl Middleware for Shell {
    async fn before_generate(
        &self,
        _: &mut nekocode_core::types::GenerateRequest,
        registry: &mut nekocode_types::tool::ToolRegistry,
    ) -> Result<(), anyhow::Error> {
        registry.insert("shell".into(), std::sync::Arc::new(tool::ShellTool {}));
        Ok(())
    }
}
