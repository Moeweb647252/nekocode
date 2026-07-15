use nekocode_core::provider::Provider;

mod deepseek;
pub(crate) mod parser;
mod sse;

/// Build a [`Provider`] from a [`nekocode_types::config::ModelConfigType`].
///
/// This is the single entry point the rest of the workspace uses to obtain a
/// concrete LLM backend; the returned `Provider` is a boxed trait object so
/// callers stay backend-agnostic.
pub fn build_from_config(config: &nekocode_types::config::ModelConfigType) -> Box<dyn Provider> {
    match config {
        nekocode_types::config::ModelConfigType::DeepSeek(deepseek_config) => {
            Box::new(deepseek::DeepSeek::from_config(deepseek_config.clone()))
        }
    }
}
