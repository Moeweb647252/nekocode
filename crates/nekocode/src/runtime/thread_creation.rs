use nekocode_entities::{middleware::Middleware, thread::Thread};

use super::{RuntimeError, ThreadRuntime};

impl ThreadRuntime {
    pub(crate) async fn create_root(
        &self,
        working_directory: String,
    ) -> Result<Thread, RuntimeError> {
        let model = self.config.read().await.default_model.clone();
        let mut db = self.db.clone();
        // Workspace creation intentionally precedes the Thread transaction. A
        // failed Thread insert may leave an unreferenced workspace, but can
        // never leave a partially-created Thread or middleware chain.
        let workspace =
            nekocode_entities::workspace::find_or_create(&mut db, &working_directory).await?;
        self.persist_thread(working_directory, model, Some(workspace.id), None, false)
            .await
    }

    pub(crate) async fn create_child(
        &self,
        parent_thread_id: u64,
        working_directory: String,
        allow_subthread: bool,
    ) -> Result<Thread, RuntimeError> {
        let mut db = self.db.clone();
        let parent = toasty::query!(Thread FILTER .id == #parent_thread_id)
            .first()
            .exec(&mut db)
            .await?
            .ok_or_else(|| {
                RuntimeError::ItemNotFound(format!("Thread not found: {parent_thread_id}"))
            })?;

        let workspace_id = match parent.workspace_id {
            Some(workspace_id) => workspace_id,
            None => {
                let workspace = nekocode_entities::workspace::find_or_create(
                    &mut db,
                    &parent.working_directory,
                )
                .await?;
                let mut update = toasty::query!(Thread FILTER .id == #(parent.id)).update();
                update.set_workspace_id(Some(workspace.id));
                update.exec(&mut db).await?;
                workspace.id
            }
        };

        self.persist_thread(
            working_directory,
            parent.model,
            Some(workspace_id),
            Some(parent_thread_id),
            allow_subthread,
        )
        .await
    }

    async fn persist_thread(
        &self,
        working_directory: String,
        model: String,
        workspace_id: Option<u64>,
        own_by_id: Option<u64>,
        allow_subthread: bool,
    ) -> Result<Thread, RuntimeError> {
        let mut db = self.db.clone();
        let mut transaction = db.transaction().await?;
        let thread = toasty::create!(Thread {
            working_directory: working_directory.clone(),
            model,
            workspace_id,
            own_by_id,
        })
        .exec(&mut transaction)
        .await?;

        let shell_config = nekocode_shell::config::ShellConfig {
            working_directory: Some(working_directory.clone()),
            ..Default::default()
        }
        .to_value();
        toasty::create!(Middleware {
            thread_id: thread.id,
            order_index: 100,
            name: "shell".to_string(),
            config: toasty::Json(shell_config),
        })
        .exec(&mut transaction)
        .await?;

        let file_config = nekocode_file::config::FileConfig {
            working_directory: Some(working_directory),
        }
        .to_value();
        toasty::create!(Middleware {
            thread_id: thread.id,
            order_index: 200,
            name: "tool".to_string(),
            config: toasty::Json(file_config),
        })
        .exec(&mut transaction)
        .await?;

        if allow_subthread {
            let subthread_config = nekocode_subthread::SubthreadConfig {
                allow_subthread: false,
            }
            .to_value();
            toasty::create!(Middleware {
                thread_id: thread.id,
                order_index: 300,
                name: "subthread".to_string(),
                config: toasty::Json(subthread_config),
            })
            .exec(&mut transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(thread)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nekocode_types::config::Config;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    async fn runtime() -> std::sync::Arc<ThreadRuntime> {
        let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
        let db = nekocode_entities::prepare_db(std::env::temp_dir().join(format!(
            "nekocode_runtime_creation_{}_{}.db",
            std::process::id(),
            sequence
        )))
        .await
        .unwrap();
        ThreadRuntime::new(
            db,
            Config {
                default_model: "default-model".to_string(),
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn root_uses_default_model_and_default_middleware_order() {
        let runtime = runtime().await;
        let thread = runtime.create_root("/tmp/root".to_string()).await.unwrap();
        assert_eq!(thread.model, "default-model");
        assert!(thread.workspace_id.is_some());

        let mut db = runtime.db();
        let middlewares =
            toasty::query!(Middleware FILTER .thread_id == #(thread.id) ORDER BY .order_index ASC)
                .exec(&mut db)
                .await
                .unwrap();
        assert_eq!(
            middlewares
                .iter()
                .map(|middleware| (middleware.name.as_str(), middleware.order_index))
                .collect::<Vec<_>>(),
            vec![("shell", 100), ("tool", 200)]
        );
    }

    #[tokio::test]
    async fn child_inherits_model_workspace_and_optional_subthread_middleware() {
        let runtime = runtime().await;
        let parent = runtime
            .create_root("/tmp/parent".to_string())
            .await
            .unwrap();
        let child = runtime
            .create_child(parent.id, "/tmp/parent/child".to_string(), true)
            .await
            .unwrap();
        assert_eq!(child.model, parent.model);
        assert_eq!(child.workspace_id, parent.workspace_id);
        assert_eq!(child.own_by_id, Some(parent.id));

        let mut db = runtime.db();
        let middlewares =
            toasty::query!(Middleware FILTER .thread_id == #(child.id) ORDER BY .order_index ASC)
                .exec(&mut db)
                .await
                .unwrap();
        assert_eq!(
            middlewares
                .iter()
                .map(|middleware| (middleware.name.as_str(), middleware.order_index))
                .collect::<Vec<_>>(),
            vec![("shell", 100), ("tool", 200), ("subthread", 300)]
        );
    }
}
