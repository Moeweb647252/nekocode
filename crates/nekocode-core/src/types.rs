use nekocode_types::generate::Message;

use crate::provider::ProviderResponse;

#[derive(Debug, Clone, Default)]
pub struct GenerateRequest {
    pub messages: Vec<Message>,
    pub system_prompt: Option<String>,
}

pub struct GenerateResponse {}

impl From<ProviderResponse> for GenerateResponse {
    fn from(_value: ProviderResponse) -> Self {
        todo!()
    }
}
