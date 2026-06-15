use serde::Serialize;
use toasty::{Deferred, Model};

#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    #[key]
    #[auto]
    pub id: u64,

    pub title: Option<String>,
    pub working_directory: String,
    pub model: String,
    pub generate_start_turn_id: Option<u64>,

    /// Owning workspace. Nullable so an `ALTER TABLE … ADD COLUMN` migration
    /// succeeds against an existing DB with rows; a startup backfill links
    /// legacy threads to a workspace by `working_directory`.
    #[index]
    pub workspace_id: Option<u64>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key = workspace_id, references = id)]
    pub workspace: Deferred<crate::workspace::Workspace>,

    #[has_many]
    pub turns: Deferred<Vec<crate::turn::Turn>>,
    #[has_many]
    pub middlewares: Deferred<Vec<crate::middleware::Middleware>>,
}
