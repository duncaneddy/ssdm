//! S3-compatible object store (Cloudflare R2, AWS S3, MinIO, …). Builds signed
//! requests with rusty-s3 and sends them with reqwest.

use std::time::Duration;

use anyhow::{anyhow, Result};
use rusty_s3::actions::{GetObject, PutObject};
use rusty_s3::{Bucket, Credentials, S3Action, UrlStyle};

use crate::config::Config;
use crate::sync::Store;

const SIGN_TTL: Duration = Duration::from_secs(300);

/// Bound a dead endpoint at connection setup rather than via the total deadline.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Fail if the store goes this long without sending any response bytes.
const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Fixed allowance for any request, independent of body size.
const REQUEST_BASE_TIMEOUT: Duration = Duration::from_secs(60);
/// Pessimistic floor throughput used to budget upload time for a body. Well below
/// any real uplink, so it never fails an upload that is genuinely progressing.
const MIN_UPLOAD_BYTES_PER_SEC: u64 = 625_000; // 5 Mbit/s

/// Ceiling for one request, sized to its body.
///
/// This is the only limit a stalled *upload* hits: `read_timeout` governs reading
/// the response, not sending the body. A single flat ceiling would have to
/// accommodate the largest product (Tycho-2, ~500 MB), which would then let a
/// 4 KB `status.json` PUT hang for that same span and stall the sequential sync
/// loop. Budgeting per body keeps small writes bounded in about a minute while
/// still giving the catalog the time it legitimately needs on a slow uplink.
fn request_timeout(body_len: usize) -> Duration {
    REQUEST_BASE_TIMEOUT + Duration::from_secs(body_len as u64 / MIN_UPLOAD_BYTES_PER_SEC)
}

pub struct R2Store {
    bucket: Bucket,
    creds: Credentials,
    client: reqwest::Client,
}

impl R2Store {
    pub fn new(cfg: &Config) -> Result<Self> {
        let endpoint = cfg
            .bucket_endpoint
            .parse()
            .map_err(|e| anyhow!("invalid BUCKET_ENDPOINT {}: {e}", cfg.bucket_endpoint))?;
        // Path-style works for R2/MinIO/B2/Wasabi; AWS S3 may require virtual-hosted.
        let url_style = match cfg.bucket_url_style.to_lowercase().as_str() {
            "path" => UrlStyle::Path,
            "virtual" | "vhost" | "virtual-host" | "virtual_host" => UrlStyle::VirtualHost,
            other => return Err(anyhow!("invalid BUCKET_URL_STYLE '{other}' (expected 'path' or 'virtual')")),
        };
        // R2 ignores region ("auto"); AWS S3 and some others verify it in the
        // SigV4 signature, so it's configurable via BUCKET_REGION.
        let bucket = Bucket::new(endpoint, url_style, cfg.bucket_name.clone(), cfg.bucket_region.clone())
            .map_err(|e| anyhow!("bucket init: {e}"))?;
        let creds = Credentials::new(cfg.bucket_access_key_id.clone(), cfg.bucket_secret_access_key.clone());
        // No client-wide `timeout`: each request sets its own, sized to its body.
        let client = reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(READ_TIMEOUT)
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

        let timeout = request_timeout(bytes.len());
        let resp = self
            .client
            .put(url)
            .header("content-type", ct)
            .header("cache-control", cc)
            .timeout(timeout)
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

        let resp = self.client.get(url).timeout(REQUEST_BASE_TIMEOUT).send().await?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_timeout_scales_with_body_but_stays_tight_for_small_writes() {
        // A small write (status.json, index.html) must not inherit the ceiling the
        // 500 MB catalog needs — it would stall the sequential sync loop.
        assert_eq!(request_timeout(4_000), Duration::from_secs(60));
        // Tycho-2 at ~500 MB gets a budget generous enough for a slow uplink.
        let tycho = request_timeout(525_761_991);
        assert!(
            tycho >= Duration::from_secs(15 * 60) && tycho <= Duration::from_secs(30 * 60),
            "500 MB budget was {tycho:?}"
        );
    }
}
