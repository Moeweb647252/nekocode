use serde::Serialize;
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize)]
pub struct Turn {
    #[key]
    #[auto]
    pub id: u64,

    pub thread_id: u64,
    pub turn_index: u64,
    #[serialize(json)]
    pub usage: nekocode_types::generate::Usage,
    pub finished: bool,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=thread_id, references=id)]
    pub thread: toasty::BelongsTo<crate::thread::Thread>,
    #[has_many]
    pub messages: toasty::HasMany<crate::message::Message>,
}
