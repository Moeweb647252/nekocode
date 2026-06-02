use std::{path::PathBuf, sync::Arc};

use axum::Router;
use clap::Parser;
use nekocode_core::agent::Agent;
use nekocode_types::config::Config;
use tokio::sync::RwLock;

mod api;

fn default_config_path() -> String {
    let config_dir = dirs::config_dir().unwrap_or(PathBuf::new());
    config_dir
        .join("nekocode")
        .join("config.toml")
        .to_string_lossy()
        .into_owned()
}

#[derive(clap::Parser)]
struct Args {
    #[arg(short, long, default_value_t = default_config_path())]
    config_path: String,
}

#[derive(Clone)]
pub struct AppState {
    db: toasty::Db,
    config: Arc<RwLock<Config>>,
    generate_states:
        Arc<dashmap::DashMap<api::generate::ThreadId, Arc<api::generate::GenerateState>>>,
    active_threads: Arc<dashmap::DashMap<api::generate::ThreadId, Arc<RwLock<Agent>>>>,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(tracing_subscriber::filter::LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();
    let args = Args::parse();
    let config_content = std::fs::read_to_string(&args.config_path)
        .unwrap_or_else(|_| panic!("Failed to read config file at {}", args.config_path));
    let config: Config = toml::from_str(&config_content).expect("Failed to load config");
    let db = nekocode_entities::prepare_db(&config)
        .await
        .expect("Failed to prepare database");

    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port))
            .await
            .expect("Failed to bind TCP listener");

    let app_state = AppState {
        db,
        config: Arc::new(RwLock::new(config)),
        generate_states: Arc::new(dashmap::DashMap::new()),
        active_threads: Arc::new(dashmap::DashMap::new()),
    };

    let router = Router::new()
        .nest("/api", api::router())
        .with_state(app_state);

    axum::serve(listener, router)
        .await
        .expect("Failed to serve application");
}
