use ssdm::cli;

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    if let Err(e) = cli::run().await {
        log::error!("fatal: {e:#}");
        std::process::exit(1);
    }
}
