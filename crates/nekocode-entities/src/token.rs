use serde::Serialize;
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize)]
pub struct Token {
    #[key]
    #[auto]
    id: u64,

    pub token: String,
    pub expires_at: jiff::Timestamp,

    #[update(jiff::Timestamp::now())]
    updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    created_at: jiff::Timestamp,
}
