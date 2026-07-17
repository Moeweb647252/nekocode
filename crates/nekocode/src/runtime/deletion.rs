use nekocode_entities::{
    message::Message, middleware::Middleware, thread::Thread, turn::Turn, workspace::Workspace,
};

use super::{RuntimeError, ThreadRuntime};

impl ThreadRuntime {
    pub(crate) async fn delete_threads_cascade(&self, root: u64) -> Result<(), RuntimeError> {
        let _lifecycle = self.lifecycle.lock().await;
        let thread_ids = self.collect_descendants(root).await?;
        for thread_id in &thread_ids {
            if self.generations.contains(*thread_id) {
                return Err(RuntimeError::ThreadGenerating);
            }
        }
        for thread_id in thread_ids.iter().rev() {
            self.agents.remove_and_shutdown(*thread_id).await;
        }
        self.delete_thread_rows(&thread_ids).await
    }

    pub(crate) async fn delete_workspace(&self, workspace_id: u64) -> Result<(), RuntimeError> {
        let _lifecycle = self.lifecycle.lock().await;
        let mut db = self.db.clone();
        let threads = toasty::query!(Thread FILTER .workspace_id == #workspace_id)
            .exec(&mut db)
            .await?;
        let thread_ids: Vec<u64> = threads.iter().map(|thread| thread.id).collect();
        for thread_id in &thread_ids {
            if self.generations.contains(*thread_id) {
                return Err(RuntimeError::ThreadGenerating);
            }
        }
        for thread_id in &thread_ids {
            self.agents.remove_and_shutdown(*thread_id).await;
        }
        self.delete_thread_rows(&thread_ids).await?;
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        toasty::query!(Workspace FILTER .id == #workspace_id)
            .delete()
            .exec(&mut transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn collect_descendants(&self, root: u64) -> Result<Vec<u64>, RuntimeError> {
        let mut ids = vec![root];
        let mut frontier = vec![root];
        while let Some(parent) = frontier.pop() {
            let mut db = self.db.clone();
            let children = toasty::query!(Thread FILTER .own_by_id == #parent)
                .exec(&mut db)
                .await?;
            for child in children {
                if !ids.contains(&child.id) {
                    ids.push(child.id);
                    frontier.push(child.id);
                }
            }
        }
        Ok(ids)
    }

    async fn delete_thread_rows(&self, thread_ids: &[u64]) -> Result<(), RuntimeError> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        for thread_id in thread_ids {
            let turns = toasty::query!(Turn FILTER .thread_id == #thread_id)
                .exec(&mut transaction)
                .await?;
            for turn in turns {
                toasty::query!(Message FILTER .turn_id == #(turn.id))
                    .delete()
                    .exec(&mut transaction)
                    .await?;
            }
            toasty::query!(Turn FILTER .thread_id == #thread_id)
                .delete()
                .exec(&mut transaction)
                .await?;
            toasty::query!(Middleware FILTER .thread_id == #thread_id)
                .delete()
                .exec(&mut transaction)
                .await?;
            toasty::query!(Thread FILTER .id == #thread_id)
                .delete()
                .exec(&mut transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
}
