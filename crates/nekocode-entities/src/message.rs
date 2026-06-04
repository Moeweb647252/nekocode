use serde::Serialize;
use toasty::{BelongsTo, Model};

#[derive(Debug, Clone, Model, Serialize)]
pub struct Message {
    #[key]
    #[auto]
    pub id: u64,

    pub turn_id: u64,
    pub message_index: u64,
    #[serialize(json)]
    pub content: nekocode_types::generate::Message,
    #[serialize(json)]
    pub usage: Option<nekocode_types::generate::Usage>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[belongs_to(key=turn_id, references=id)]
    pub turn: BelongsTo<crate::turn::Turn>,
}
