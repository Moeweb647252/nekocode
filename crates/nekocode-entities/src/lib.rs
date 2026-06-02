use nekocode_types::config::Config;
use toasty::Db;

pub mod message;
pub mod thread;
pub mod token;

pub async fn prepare_db(config: &Config) -> toasty::Result<Db> {
    Db::builder()
        .connect(&format!("sqlite://{}?mode=rwc", config.app.db_path))
        .await
}
