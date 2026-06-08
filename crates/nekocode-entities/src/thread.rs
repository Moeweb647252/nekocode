use serde::Serialize;
use toasty::{Deferred, Model};

#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    #[key]
    #[auto]
    pub id: u64,

    pub title: Option<String>,
    pub working_directory: String,
    pub model: String,
    pub generate_start_turn_id: Option<u64>,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,

    #[has_many]
    pub turns: Deferred<Vec<crate::turn::Turn>>,
}
