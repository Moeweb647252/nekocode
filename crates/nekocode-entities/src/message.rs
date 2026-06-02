use serde::Serialize;
use toasty::{BelongsTo, Model};

use crate::thread::Thread;

#[derive(Debug, Clone, Model, Serialize)]
pub struct Message {
    #[key]
    #[auto]
    pub id: u64,

    #[serialize(json)]
    pub content: nekocode_types::generate::Message,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    pub thread_id: u64,
    #[belongs_to(key=thread_id, references=id)]
    pub thread: BelongsTo<Thread>,
}
