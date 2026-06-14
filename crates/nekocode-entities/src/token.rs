use serde::Serialize;
use toasty::Model;

#[derive(Debug, Clone, Model, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    #[key]
    #[auto]
    pub id: u64,

    pub token: String,
    pub expires_at: jiff::Timestamp,

    #[update(jiff::Timestamp::now())]
    pub updated_at: jiff::Timestamp,
    #[default(jiff::Timestamp::now())]
    pub created_at: jiff::Timestamp,
}
