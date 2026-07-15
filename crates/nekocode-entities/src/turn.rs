use serde::Serialize;
use toasty::{Json, Model};

/// One user turn in a [`Thread`](crate::thread::Thread): the result of a
/// full agent run (possibly multiple provider generations for tool rounds).
///
/// Stores the aggregated [`nekocode_types::generate::Usage`], whether the turn
/// `finished` normally, and owns the turn's
/// [`Message`](crate::message::Message)s via the `messages` relation.
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
