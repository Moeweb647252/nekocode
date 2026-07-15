use serde::Serialize;
use toasty::Model;

/// An auth token minted on password login, checked by the API auth middleware
/// against the `Token` header. `expires_at` enforces the 30-day lifetime.
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
