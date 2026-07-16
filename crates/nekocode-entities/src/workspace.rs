use serde::Serialize;
use toasty::{Deferred, Model};

/// A workspace: a working directory that groups one or more threads. The
/// working directory is the grouping key (threads in the same project share a
/// workspace), while `name` is an optional display label. A thread references
/// its workspace via `Thread.workspace_id`.
#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    #[key]
    #[auto]
    pub id: u64,

    #[unique]
    pub working_directory: String,
    pub name: Option<String>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[has_many]
    pub threads: Deferred<Vec<crate::thread::Thread>>,
}

/// Find the workspace owning `working_directory`, creating one if none exists.
/// A workspace is unique per directory, so this is idempotent. Used when
/// creating a thread to ensure its owning workspace exists.
pub async fn find_or_create(
    db: &mut toasty::Db,
    working_directory: &str,
) -> toasty::Result<Workspace> {
    let key = working_directory.to_string();
    if let Some(ws) = toasty::query!(Workspace FILTER .working_directory == #(key))
        .first()
        .exec(db)
        .await?
    {
        return Ok(ws);
    }
    let created = toasty::create!(Workspace {
        working_directory: working_directory.to_string(),
        name: None,
    })
    .exec(db)
    .await;
    match created {
        Ok(workspace) => Ok(workspace),
        Err(create_error) => {
            // Another request/process may have won the unique insert race.
            let retry_key = working_directory.to_string();
            if let Some(workspace) =
                toasty::query!(Workspace FILTER .working_directory == #(retry_key))
                    .first()
                    .exec(db)
                    .await?
            {
                Ok(workspace)
            } else {
                Err(create_error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    #[tokio::test]
    async fn concurrent_find_or_create_returns_one_workspace() {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let db_path = std::env::temp_dir().join(format!(
            "nekocode_workspace_unique_{}_{}.db",
            std::process::id(),
            n
        ));
        let db = crate::prepare_db(db_path).await.unwrap();
        let directory = format!("/tmp/nekocode-workspace-{n}");
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let mut db = db.clone();
            let directory = directory.clone();
            tasks.push(tokio::spawn(async move {
                find_or_create(&mut db, &directory).await.unwrap().id
            }));
        }
        let mut ids = Vec::new();
        for task in tasks {
            ids.push(task.await.unwrap());
        }
        assert!(ids.iter().all(|id| *id == ids[0]));

        let mut query_db = db.clone();
        let rows = toasty::query!(Workspace FILTER .working_directory == #directory)
            .exec(&mut query_db)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }
}
