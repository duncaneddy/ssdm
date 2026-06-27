//! Daemon configuration, loaded from environment variables.
//!
//! Storage is any S3-compatible provider (Cloudflare R2, AWS S3, MinIO, …)
//! configured via the `BUCKET_*` variables. We run on R2 for its zero egress
//! fees, but nothing here is R2-specific.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub bucket_name: String,
    pub bucket_endpoint: String,
    pub bucket_region: String,
    pub bucket_access_key_id: String,
    pub bucket_secret_access_key: String,
    /// Public serving domain (the S3 bucket's CDN/custom domain), used to build
    /// the download links on the landing page.
    pub site_domain: String,
    pub data_dir: PathBuf,
    pub run_on_start: bool,
    pub host_min_interval: Duration,
    pub stagger_jitter: Duration,
}

pub fn from_env() -> Result<Config> {
    from_map(&std::env::vars().collect())
}

pub fn from_map(m: &HashMap<String, String>) -> Result<Config> {
    let req = |k: &str| m.get(k).cloned().ok_or_else(|| anyhow!("missing required env var {k}"));
    let bucket_access_key_id = req("BUCKET_ACCESS_KEY_ID")?;
    let bucket_secret_access_key = req("BUCKET_SECRET_ACCESS_KEY")?;
    let bucket_endpoint = req("BUCKET_ENDPOINT")?;
    let site_domain = req("SITE_DOMAIN")?;

    let bucket_name = m.get("BUCKET_NAME").cloned().unwrap_or_else(|| "ssdm-data".into());
    let bucket_region = m.get("BUCKET_REGION").cloned().unwrap_or_else(|| "auto".into());
    let data_dir = PathBuf::from(m.get("DATA_DIR").cloned().unwrap_or_else(|| "/data".into()));
    let run_on_start = m.get("RUN_ON_START").map(|v| v == "true" || v == "1").unwrap_or(false);

    let secs = |k: &str, d: u64| -> Result<Duration> {
        match m.get(k) {
            Some(v) => Ok(Duration::from_secs(v.parse().map_err(|_| anyhow!("{k} must be an integer"))?)),
            None => Ok(Duration::from_secs(d)),
        }
    };
    let host_min_interval = secs("HOST_MIN_INTERVAL_SECS", 2)?;
    let stagger_jitter = secs("STAGGER_JITTER_SECS", 1)?;

    Ok(Config {
        bucket_name,
        bucket_endpoint,
        bucket_region,
        bucket_access_key_id,
        bucket_secret_access_key,
        site_domain,
        data_dir,
        run_on_start,
        host_min_interval,
        stagger_jitter,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn base() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("BUCKET_ACCESS_KEY_ID".into(), "ak".into());
        m.insert("BUCKET_SECRET_ACCESS_KEY".into(), "sk".into());
        m.insert("BUCKET_ENDPOINT".into(), "https://example.test".into());
        m.insert("SITE_DOMAIN".into(), "example.org".into());
        m
    }

    #[test]
    fn applies_defaults() {
        let c = from_map(&base()).unwrap();
        assert_eq!(c.bucket_name, "ssdm-data");
        assert_eq!(c.bucket_region, "auto");
        assert_eq!(c.bucket_endpoint, "https://example.test");
        assert_eq!(c.site_domain, "example.org");
        assert_eq!(c.data_dir.to_str().unwrap(), "/data");
        assert!(!c.run_on_start);
        assert_eq!(c.host_min_interval, Duration::from_secs(2));
        assert_eq!(c.stagger_jitter, Duration::from_secs(1));
    }

    #[test]
    fn missing_required_is_error() {
        for key in ["BUCKET_ACCESS_KEY_ID", "BUCKET_SECRET_ACCESS_KEY", "BUCKET_ENDPOINT", "SITE_DOMAIN"] {
            let mut m = base();
            m.remove(key);
            assert!(from_map(&m).is_err(), "{key} should be required");
        }
    }

    #[test]
    fn overrides_are_honored() {
        let mut m = base();
        m.insert("BUCKET_NAME".into(), "other".into());
        m.insert("BUCKET_REGION".into(), "us-east-1".into());
        m.insert("RUN_ON_START".into(), "true".into());
        m.insert("HOST_MIN_INTERVAL_SECS".into(), "5".into());
        let c = from_map(&m).unwrap();
        assert_eq!(c.bucket_name, "other");
        assert_eq!(c.bucket_region, "us-east-1");
        assert!(c.run_on_start);
        assert_eq!(c.host_min_interval, Duration::from_secs(5));
    }
}
