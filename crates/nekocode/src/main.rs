use std::{path::PathBuf, sync::Arc};

use axum::Router;
use clap::Parser;
use nekocode_core::agent::Agent;
use nekocode_types::config::Config;
use tokio::sync::RwLock;
use tracing::info;

mod api;

#[derive(clap::Parser)]
struct Args {
    #[arg(short, long)]
    config_path: Option<String>,
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
    let config_path = if let Some(path) = args.config_path {
        PathBuf::from(path)
    } else {
        dirs::config_dir()
            .map(|p| p.join("nekocode"))
            .unwrap_or(PathBuf::new())
    };
    let config_file_path = config_path.join("config.toml");
    let config_content = std::fs::read_to_string(&config_file_path).unwrap_or_else(|_| {
        panic!(
            "Failed to read config file at {}",
            config_file_path.to_string_lossy()
        )
    });
    let config: Config = toml::from_str(&config_content).expect("Failed to load config");
    let db = nekocode_entities::prepare_db(config_path.join("nekocode.db"))
        .await
        .expect("Failed to prepare database");
    info!(
        "Start listening at: {}:{}",
        config.server.host, config.server.port
    );
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
        .nest(
            "/api",
            api::public_router().merge(
                api::protected_router()
                    .layer(axum::middleware::from_fn_with_state(
                        app_state.clone(),
                        api::auth_middleware,
                    )),
            ),
        )
        .with_state(app_state);
    axum::serve(listener, router)
        .await
        .expect("Failed to serve application");
}
