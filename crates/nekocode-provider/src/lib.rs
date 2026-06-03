use nekocode_core::provider::Provider;

mod deepseek;
pub mod parser;
mod sse;

pub fn build_from_config(config: &nekocode_types::config::ModelConfigType) -> Box<dyn Provider> {
    match config {
        nekocode_types::config::ModelConfigType::DeepSeek(deepseek_config) => {
            Box::new(deepseek::DeepSeek::from_config(deepseek_config.clone()))
        }
    }
}
