//! R2 (S3-compatible) object store. Builds signed requests with rusty-s3 and
//! sends them with reqwest. Write-only from the daemon's perspective.

use std::time::Duration;

use anyhow::{anyhow, Result};
use rusty_s3::actions::{GetObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

use crate::config::Config;
use crate::sync::Store;

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
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self { bucket, creds, client })
    }
}

impl Store for R2Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str, cache_control: &str) -> Result<()> {
        // Single source of truth for the two object headers so the signed and
        // sent values cannot drift apart (a mismatch would be a 403 from R2).
        let ct = content_type;
        let cc = cache_control;

        let mut action: PutObject = self.bucket.put_object(Some(&self.creds), key);
        action.headers_mut().insert("content-type", ct);
        action.headers_mut().insert("cache-control", cc);
        let url = action.sign(SIGN_TTL);

        let resp = self
            .client
            .put(url)
            .header("content-type", ct)
            .header("cache-control", cc)
            .body(bytes)
            .send()
            .await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp
                .bytes()
                .await
                .map(|b| String::from_utf8_lossy(&b[..b.len().min(4096)]).into_owned())
                .unwrap_or_default();
            return Err(anyhow!("R2 PUT {key} failed: {status} {body}"));
        }
        Ok(())
    }

    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let action: GetObject = self.bucket.get_object(Some(&self.creds), key);
        let url = action.sign(SIGN_TTL);

        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let body = resp
                .bytes()
                .await
                .map(|b| String::from_utf8_lossy(&b[..b.len().min(4096)]).into_owned())
                .unwrap_or_default();
            return Err(anyhow!("R2 GET {key} failed: {status} {body}"));
        }
        Ok(Some(resp.bytes().await?.to_vec()))
    }
}
