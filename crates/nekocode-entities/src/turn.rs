use serde::Serialize;
use toasty::{Json, Model};

#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Turn {
    #[key]
    #[auto]
    pub id: u64,

    #[index]
    pub thread_id: u64,
    pub turn_index: u64,
    pub usage: Json<nekocode_types::generate::Usage>,
    pub finished: bool,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=thread_id, references=id)]
    pub thread: toasty::Deferred<crate::thread::Thread>,
    #[has_many]
    pub messages: toasty::Deferred<Vec<crate::message::Message>>,
}
