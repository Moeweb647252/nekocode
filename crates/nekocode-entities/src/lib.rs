use nekocode_types::config::Config;
use toasty::Db;

use crate::{message::Message, thread::Thread, token::Token, turn::Turn};

pub mod message;
pub mod thread;
pub mod token;
pub mod turn;

pub async fn prepare_db(config: &Config) -> toasty::Result<Db> {
    Db::builder()
        .models(toasty::models!(Message, Thread, Turn, Token))
        .connect(&format!("sqlite://{}?mode=rwc", config.app.db_path))
        .await
}
