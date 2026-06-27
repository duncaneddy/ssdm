use ssdm::cli;

#[tokio::main]
async fn main() {
    // Load .env (if present) into the process environment for local runs.
    // No-op in Docker, where docker-compose injects vars via env_file.
    let _ = dotenvy::dotenv();
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = cli::run().await {
        log::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}
