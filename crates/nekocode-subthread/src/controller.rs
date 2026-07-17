/// A canonicalized child-thread creation request. Path containment validation
/// remains in the subthread tool adapter; implementations receive a trusted
/// working directory and own the persistence details.
#[derive(Debug, Clone)]
pub struct CreateSubthreadRequest {
    pub parent_thread_id: u64,
    pub working_directory: String,
    pub allow_subthread: bool,
}

#[derive(Debug, Clone)]
pub struct CreatedSubthread {
    pub subthread_id: u64,
    pub working_directory: String,
    pub allow_subthread: bool,
}

/// High-level control interface supplied by the server runtime.
///
/// The subthread crate owns the parent-local registry and tool protocol, while
/// this trait owns all persisted thread lifecycle work. Keeping that split
/// prevents tools from assembling a partial activation or generation state.
#[async_trait::async_trait]
pub trait SubthreadController: Send + Sync {
    async fn create(
        &self,
        request: CreateSubthreadRequest,
    ) -> Result<CreatedSubthread, anyhow::Error>;

    async fn run(
        &self,
        subthread_id: u64,
        prompt: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<(), anyhow::Error>;

    async fn invalidate(&self, subthread_id: u64) -> Result<(), anyhow::Error>;

    async fn delete(&self, subthread_id: u64) -> Result<(), anyhow::Error>;
}
