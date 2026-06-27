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

/// Select products to force-sync by name, matching against the FULL registry
/// (active or inactive — forcing a frozen product by name is a valid dev action).
/// Errors if any name matches no product at all.
fn select_products<'a>(items: &'a [Product], names: &[String]) -> anyhow::Result<Vec<&'a Product>> {
    let unknown: Vec<&str> = names
        .iter()
        .filter(|n| !items.iter().any(|p| &p.name == n))
        .map(|s| s.as_str())
        .collect();
    if !unknown.is_empty() {
        anyhow::bail!("unknown product name(s): {}", unknown.join(", "));
    }
    Ok(items
        .iter()
        .filter(|p| names.iter().any(|n| n == p.name))
        .collect())
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
                select_products(&items, &product)?
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
    use std::time::Duration;

    fn test_product(name: &'static str, active: bool) -> Product {
        Product {
            category: "catalog", source: "celestrak", name,
            url: format!("https://h/{name}"), filename: format!("{name}.json"),
            content_type: "application/json", active, alias_name: None,
            interval: Duration::from_secs(3600),
        }
    }

    #[test]
    fn select_products_includes_inactive_when_named() {
        let items = vec![test_product("starlink", true), test_product("frozen", false)];
        let names = vec!["frozen".to_string()];
        let selected = select_products(&items, &names).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "frozen");
        assert!(!selected[0].active, "inactive product is force-selected by name");
    }

    #[test]
    fn select_products_rejects_unknown_name() {
        let items = vec![test_product("starlink", true), test_product("frozen", false)];
        let names = vec!["frozen".to_string(), "nope".to_string()];
        let err = match select_products(&items, &names) {
            Ok(_) => panic!("expected error for unknown name"),
            Err(e) => e,
        };
        assert!(err.to_string().contains("nope"), "unknown name reported: {err}");
    }

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
