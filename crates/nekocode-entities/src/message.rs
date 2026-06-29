use crate::serialize_json;
use serde::Serialize;
use toasty::{Deferred, Json, Model};

#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    #[key]
    #[auto]
    pub id: u64,

    #[index]
    pub turn_id: u64,
    pub message_index: u64,
    #[serde(serialize_with = "serialize_json")]
    pub content: Json<nekocode_types::generate::MessageType>,
    pub usage: Option<Json<nekocode_types::generate::Usage>>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=turn_id, references=id)]
    pub turn: Deferred<crate::turn::Turn>,
}
