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

    /// ID of the parent thread that owns this subthread. `None` for top-level
    /// threads. Used to express the subthread relationship in the database.
    /// Nullable so an `ALTER TABLE … ADD COLUMN` migration succeeds against an
    /// existing DB with rows.
    #[index]
    pub own_by_id: Option<u64>,

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prepare_db;

    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn test_db_path() -> std::path::PathBuf {
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "nekocode_thread_own_by_test_{}_{}.db",
            std::process::id(),
            n
        ))
    }

    /// `own_by_id` must round-trip through the DB. Validates the new column
    /// was added to the schema and is queryable.
    #[tokio::test]
    async fn own_by_id_roundtrips() {
        let mut db = prepare_db(test_db_path()).await.expect("prepare_db");
        let parent = toasty::create!(Thread {
            working_directory: "/tmp".to_string(),
            model: "default".to_string(),
        })
        .exec(&mut db)
        .await
        .expect("create parent");

        let child = toasty::create!(Thread {
            working_directory: "/tmp/sub".to_string(),
            model: "default".to_string(),
            own_by_id: Some(parent.id),
        })
        .exec(&mut db)
        .await
        .expect("create child");

        assert_eq!(child.own_by_id, Some(parent.id));

        // Index-backed query: find children of parent.
        let children = toasty::query!(Thread FILTER .own_by_id == #(parent.id))
            .exec(&mut db)
            .await
            .expect("query children");
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child.id);

        // Top-level threads have None.
        let top = toasty::query!(Thread FILTER .id == #(parent.id))
            .first()
            .exec(&mut db)
            .await
            .expect("query parent")
            .expect("parent exists");
        assert!(top.own_by_id.is_none());
    }
}
