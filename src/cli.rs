//! Command-line interface: the daemon plus manual one-shot sync triggers.

use clap::{Parser, Subcommand};
use log::info;

use crate::config::from_env;
use crate::products::{products, Product};
use crate::ratelimit::RateLimiter;

#[derive(Parser)]
#[command(name = "ssdm", about = "Simple Space Data Mirror daemon")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run the scheduler loop forever (default).
    Daemon,
    /// Run one sync pass and exit.
    Sync {
        /// Force all active products regardless of due-ness.
        #[arg(long)]
        all: bool,
        /// Force a specific product by registry name (repeatable).
        #[arg(long)]
        product: Vec<String>,
    },
}

pub async fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let cfg = from_env()?;
    match cli.command.unwrap_or(Command::Daemon) {
        Command::Daemon => crate::scheduler::run_daemon(&cfg).await,
        Command::Sync { all, product } => {
            let items = products();
            let fetcher = crate::fetch::HttpFetcher::new(std::time::Duration::from_secs(20))?;
            let store = crate::store::R2Store::new(&cfg)?;
            let mut rate = RateLimiter::new(cfg.host_min_interval, cfg.stagger_jitter);
            let now = crate::scheduler::now_ms();

            let process: Vec<&Product> = if !product.is_empty() {
                let matched: Vec<&Product> = items
                    .iter()
                    .filter(|p| p.active && product.iter().any(|n| n == p.name))
                    .collect();
                let unknown: Vec<&str> = product
                    .iter()
                    .filter(|n| !items.iter().any(|p| p.active && &p.name == n))
                    .map(|s| s.as_str())
                    .collect();
                if !unknown.is_empty() {
                    anyhow::bail!("unknown product name(s): {}", unknown.join(", "));
                }
                matched
            } else if all {
                items.iter().filter(|p| p.active).collect()
            } else {
                let status = crate::local::load_status(&cfg.data_dir);
                crate::scheduler::due_indices(&items, &status, now)
                    .iter()
                    .map(|&i| &items[i])
                    .collect()
            };

            info!("manual sync over {} product(s)", process.len());
            let summary = crate::sync::run_sync(&items, &process, &fetcher, &store, &mut rate, &cfg.data_dir, now).await;
            if summary.failed > 0 {
                anyhow::bail!("{} fetch(es) failed (checked={}, changed={})", summary.failed, summary.checked, summary.changed);
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn parses_sync_with_products() {
        let cli = Cli::parse_from(["ssdm", "sync", "--product", "starlink", "--product", "geo"]);
        match cli.command.unwrap() {
            Command::Sync { all, product } => {
                assert!(!all);
                assert_eq!(product, vec!["starlink", "geo"]);
            }
            _ => panic!("expected sync"),
        }
    }

    #[test]
    fn defaults_to_daemon() {
        let cli = Cli::parse_from(["ssdm"]);
        assert!(cli.command.is_none());
    }
}
