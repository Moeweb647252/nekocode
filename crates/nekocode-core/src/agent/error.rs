#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Db error: {0}")]
    DbError(#[from] toasty::Error),
    #[error("Provider error: {0}")]
    ProviderError(#[from] crate::provider::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
