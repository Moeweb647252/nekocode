//! Toasty ORM (SQLite) models for the persisted conversation state.
//!
//! The schema is: [`Workspace`] 1—* [`Thread`] 1—* [`Turn`] 1—* [`Message`];
//! each [`Thread`] also has zero-or-many [`Middleware`] rows, and [`Token`]
//! holds auth tokens. Models are declared with `#[derive(toasty::Model)]` and
//! pushed to SQLite on first run via [`prepare_db`].

use std::path::PathBuf;

use serde::Serializer;
use toasty::{Db, Json, query};

use crate::{
    message::Message, middleware::Middleware, thread::Thread, token::Token, turn::Turn,
    workspace::Workspace,
};

pub mod message;
pub mod middleware;
pub mod thread;
pub mod token;
pub mod turn;
pub mod workspace;

/// Open (or create) the SQLite database at `db_path` and ensure its schema is
/// initialized. Idempotent: if a probe query succeeds the schema is already
/// present; otherwise `push_schema` creates it.
pub async fn prepare_db(db_path: PathBuf) -> toasty::Result<Db> {
    if !db_path.exists() {
        std::fs::File::create(&db_path)?;
    }
    let mut db = Db::builder()
        .models(toasty::models!(
            Message, Thread, Turn, Token, Middleware, Workspace
        ))
        .connect(&format!("turso://{}", db_path.to_string_lossy()))
        .await?;
    if query!(Message LIMIT 1).exec(&mut db).await.is_err() {
        tracing::info!("Initializing database schema");
        db.push_schema().await?;
    } else {
        deduplicate_workspaces(&mut db).await?;
        // Apply model changes (including new indexes) to existing databases.
        db.push_schema().await?;
    }
    backfill_thread_workspaces(&mut db).await?;
    Ok(db)
}

async fn deduplicate_workspaces(db: &mut Db) -> toasty::Result<()> {
    let workspaces = match query!(Workspace ORDER BY .id ASC).exec(db).await {
        Ok(workspaces) => workspaces,
        Err(_) => return Ok(()),
    };
    let mut owners = std::collections::HashMap::<String, u64>::new();
    for workspace in workspaces {
        if let Some(owner_id) = owners.get(&workspace.working_directory).copied() {
            let duplicate_id = workspace.id;
            let mut update = query!(Thread FILTER .workspace_id == #duplicate_id).update();
            update.set_workspace_id(Some(owner_id));
            update.exec(db).await?;
            query!(Workspace FILTER .id == #duplicate_id)
                .delete()
                .exec(db)
                .await?;
        } else {
            owners.insert(workspace.working_directory, workspace.id);
        }
    }
    Ok(())
}

async fn backfill_thread_workspaces(db: &mut Db) -> toasty::Result<()> {
    let missing_workspace: Option<u64> = None;
    let threads = query!(Thread FILTER .workspace_id == #missing_workspace)
        .exec(db)
        .await?;
    for thread in threads {
        let workspace = crate::workspace::find_or_create(db, &thread.working_directory).await?;
        let mut update = query!(Thread FILTER .id == #(thread.id)).update();
        update.set_workspace_id(Some(workspace.id));
        update.exec(db).await?;
    }
    Ok(())
}

/// Serialize a Toasty [`Json<T>`] wrapper by forwarding to the inner `T`,
/// used as the `serialize_with` for JSON-typed DB columns so the wire shape is
/// the contained value rather than the `Json` wrapper.
pub fn serialize_json<T: serde::Serialize, S: Serializer>(
    value: &Json<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.0.serialize(serializer)
}
