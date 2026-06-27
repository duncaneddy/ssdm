//! R2 (S3-compatible) object store. Builds signed requests with rusty-s3 and
//! sends them with reqwest. Write-only from the daemon's perspective.

use std::time::Duration;

use anyhow::{anyhow, Result};
use rusty_s3::actions::PutObject;
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

use crate::config::Config;
use crate::sync::Store;

const CACHE_CONTROL: &str = "public, max-age=3600";
const SIGN_TTL: Duration = Duration::from_secs(300);

pub struct R2Store {
    bucket: Bucket,
    creds: Credentials,
    client: reqwest::Client,
}

impl R2Store {
    pub fn new(cfg: &Config) -> Result<Self> {
        let endpoint = cfg
            .r2_endpoint
            .parse()
            .map_err(|e| anyhow!("invalid R2_ENDPOINT {}: {e}", cfg.r2_endpoint))?;
        // R2 ignores region but the signer requires one; "auto" is conventional.
        let bucket = Bucket::new(endpoint, UrlStyle::Path, cfg.r2_bucket.clone(), "auto")
            .map_err(|e| anyhow!("bucket init: {e}"))?;
        let creds = Credentials::new(cfg.r2_access_key_id.clone(), cfg.r2_secret_access_key.clone());
        Ok(Self { bucket, creds, client: reqwest::Client::new() })
    }
}

impl Store for R2Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> Result<()> {
        let mut action: PutObject = self.bucket.put_object(Some(&self.creds), key);
        action.headers_mut().insert("content-type", content_type);
        action.headers_mut().insert("cache-control", CACHE_CONTROL);
        let url = action.sign(SIGN_TTL);

        let resp = self
            .client
            .put(url)
            .header("content-type", content_type)
            .header("cache-control", CACHE_CONTROL)
            .body(bytes)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!("R2 PUT {key} failed: {status} {body}"));
        }
        Ok(())
    }
}
