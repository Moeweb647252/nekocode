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

pub async fn prepare_db(db_path: PathBuf) -> toasty::Result<Db> {
    if !db_path.exists() {
        std::fs::File::create(&db_path)?;
    }
    let mut db = Db::builder()
        .models(toasty::models!(Message, Thread, Turn, Token, Middleware, Workspace))
        .connect(&format!("turso://{}", db_path.to_string_lossy()))
        .await?;
    if query!(Message LIMIT 1).exec(&mut db).await.is_err() {
        tracing::info!("Initializing database schema");
        db.push_schema().await?;
    }
    Ok(db)
}

pub fn serialize_json<T: serde::Serialize, S: Serializer>(
    value: &Json<T>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.0.serialize(serializer)
}
