use serde::Serialize;
use toasty::Json;

#[derive(toasty::Model, Clone, Debug, Serialize)]
pub struct Middleware {
    #[key]
    #[auto]
    pub id: u64,
    #[index]
    pub thread_id: u64,
    pub name: String,
    pub config: Json<serde_json::Value>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=thread_id, references=id)]
    pub thread: toasty::Deferred<crate::thread::Thread>,
}
