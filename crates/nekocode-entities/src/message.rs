use crate::serialize_json;
use serde::Serialize;
use toasty::{Deferred, Json, Model};

/// A single persisted message, child of a [`Turn`](crate::turn::Turn).
///
/// `content` stores the [`nekocode_types::generate::MessageType`] as JSON
/// (serialized via `crate::serialize_json`); `usage` is only set for assistant
/// messages. `message_index` is the message's position within its turn.
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
