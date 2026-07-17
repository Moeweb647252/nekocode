use std::sync::{Arc, Weak};

use nekocode_subthread::controller::{
    CreateSubthreadRequest, CreatedSubthread, SubthreadController,
};

use super::ThreadRuntime;

#[derive(Clone)]
pub(crate) struct RuntimeSubthreadController {
    runtime: Weak<ThreadRuntime>,
}

impl RuntimeSubthreadController {
    pub(crate) fn new(runtime: Weak<ThreadRuntime>) -> Self {
        Self { runtime }
    }

    fn runtime(&self) -> Result<Arc<ThreadRuntime>, anyhow::Error> {
        self.runtime
            .upgrade()
            .ok_or_else(|| anyhow::anyhow!("thread runtime has shut down"))
    }
}

#[async_trait::async_trait]
impl SubthreadController for RuntimeSubthreadController {
    async fn create(
        &self,
        request: CreateSubthreadRequest,
    ) -> Result<CreatedSubthread, anyhow::Error> {
        let thread = self
            .runtime()?
            .create_child(
                request.parent_thread_id,
                request.working_directory,
                request.allow_subthread,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        Ok(CreatedSubthread {
            subthread_id: thread.id,
            working_directory: thread.working_directory,
            allow_subthread: request.allow_subthread,
        })
    }

    async fn run(
        &self,
        subthread_id: u64,
        prompt: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<(), anyhow::Error> {
        self.runtime()?
            .run_subthread(subthread_id, prompt, cancellation)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    async fn invalidate(&self, subthread_id: u64) -> Result<(), anyhow::Error> {
        self.runtime()?
            .invalidate_agent(subthread_id)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }

    async fn delete(&self, subthread_id: u64) -> Result<(), anyhow::Error> {
        self.runtime()?
            .delete_threads_cascade(subthread_id)
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
    }
}
