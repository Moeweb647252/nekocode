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
    let ws = toasty::create!(Workspace {
        working_directory: working_directory.to_string(),
        name: None,
    })
    .exec(db)
    .await?;
    Ok(ws)
}
