/// Errors the agent run loop can surface: missing items, DB failures
/// (from Toasty), provider failures, and `Other` for anything else wrapped
/// in anyhow.
#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Generation cancelled")]
    Cancelled,

    #[error("Item not found: {0}")]
    ItemNotFound(String),

    #[error("Db error: {0}")]
    DbError(#[from] toasty::Error),
    #[error("Provider error: {0}")]
    ProviderError(#[from] crate::provider::ProviderError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
