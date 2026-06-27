//! Daemon configuration, loaded from environment variables.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};

#[derive(Clone, Debug)]
pub struct Config {
    pub r2_account_id: String,
    pub r2_access_key_id: String,
    pub r2_secret_access_key: String,
    pub r2_bucket: String,
    pub r2_endpoint: String,
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
    let r2_account_id = req("R2_ACCOUNT_ID")?;
    let r2_access_key_id = req("R2_ACCESS_KEY_ID")?;
    let r2_secret_access_key = req("R2_SECRET_ACCESS_KEY")?;

    let r2_bucket = m.get("R2_BUCKET").cloned().unwrap_or_else(|| "ssdm-data".into());
    let r2_endpoint = m
        .get("R2_ENDPOINT")
        .cloned()
        .unwrap_or_else(|| format!("https://{r2_account_id}.r2.cloudflarestorage.com"));
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
        r2_account_id,
        r2_access_key_id,
        r2_secret_access_key,
        r2_bucket,
        r2_endpoint,
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
        m.insert("R2_ACCOUNT_ID".into(), "acct".into());
        m.insert("R2_ACCESS_KEY_ID".into(), "ak".into());
        m.insert("R2_SECRET_ACCESS_KEY".into(), "sk".into());
        m
    }

    #[test]
    fn applies_defaults() {
        let c = from_map(&base()).unwrap();
        assert_eq!(c.r2_bucket, "ssdm-data");
        assert_eq!(c.r2_endpoint, "https://acct.r2.cloudflarestorage.com");
        assert_eq!(c.data_dir.to_str().unwrap(), "/data");
        assert!(!c.run_on_start);
        assert_eq!(c.host_min_interval, std::time::Duration::from_secs(2));
        assert_eq!(c.stagger_jitter, std::time::Duration::from_secs(1));
    }

    #[test]
    fn missing_required_is_error() {
        let mut m = base();
        m.remove("R2_SECRET_ACCESS_KEY");
        assert!(from_map(&m).is_err());
    }

    #[test]
    fn overrides_are_honored() {
        let mut m = base();
        m.insert("R2_BUCKET".into(), "other".into());
        m.insert("R2_ENDPOINT".into(), "https://example.test".into());
        m.insert("RUN_ON_START".into(), "true".into());
        m.insert("HOST_MIN_INTERVAL_SECS".into(), "5".into());
        let c = from_map(&m).unwrap();
        assert_eq!(c.r2_bucket, "other");
        assert_eq!(c.r2_endpoint, "https://example.test");
        assert!(c.run_on_start);
        assert_eq!(c.host_min_interval, std::time::Duration::from_secs(5));
    }
}
