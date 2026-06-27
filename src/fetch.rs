//! Native HTTP fetch with a per-request timeout and short bounded retry.

use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use log::warn;

use crate::retry::{backoff_delay, MAX_ATTEMPTS};
use crate::sync::Fetcher;

const USER_AGENT: &str = "ssdm-mirror/1.0 (+https://simplespacedata.org)";

/// Outcome of a single fetch attempt.
///
/// `Retryable` is a transient failure (network error, timeout, 5xx) worth a
/// quick in-run retry. `Fatal` is a client error (4xx, e.g. 403/429 rate-limit)
/// where retrying within seconds is pointless and can prolong a throttle — we
/// give up immediately and let the product's interval be the real backoff.
pub enum FetchError {
    Retryable(anyhow::Error),
    Fatal(anyhow::Error),
}

/// Call `make()` up to MAX_ATTEMPTS times, sleeping `backoff_delay` between tries.
/// A `Fatal` error short-circuits immediately (no retry).
pub async fn fetch_with_retry<Fut, MakeReq>(mut make: MakeReq) -> Result<Vec<u8>>
where
    MakeReq: FnMut() -> Fut,
    Fut: Future<Output = std::result::Result<Vec<u8>, FetchError>>,
{
    let mut last_err = anyhow!("no attempts made");
    for attempt in 1..=MAX_ATTEMPTS {
        match make().await {
            Ok(bytes) => return Ok(bytes),
            Err(FetchError::Fatal(e)) => {
                warn!("fetch attempt {attempt} failed (not retryable): {e}");
                return Err(e);
            }
            Err(FetchError::Retryable(e)) => {
                last_err = e;
                if attempt < MAX_ATTEMPTS {
                    warn!("fetch attempt {attempt} failed: {last_err}; retrying");
                    tokio::time::sleep(backoff_delay(attempt)).await;
                }
            }
        }
    }
    Err(last_err)
}

pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new(timeout: Duration) -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .timeout(timeout)
            .build()?;
        Ok(Self { client })
    }

    async fn get_once(&self, url: &str) -> std::result::Result<Vec<u8>, FetchError> {
        let resp = self
            .client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Retryable(anyhow!(e)))?;
        let status = resp.status();
        if !status.is_success() {
            let err = anyhow!("HTTP {status} from {url}");
            // 4xx (incl. 403 forbidden / 429 too-many-requests) won't clear on a
            // quick retry; 5xx and the rest are transient.
            return Err(if status.is_client_error() {
                FetchError::Fatal(err)
            } else {
                FetchError::Retryable(err)
            });
        }
        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| FetchError::Retryable(anyhow!(e)))
    }
}

impl Fetcher for HttpFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>> {
        fetch_with_retry(|| self.get_once(url)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[tokio::test(start_paused = true)]
    async fn retries_then_succeeds() {
        let calls = Cell::new(0u32);
        let res = fetch_with_retry(|| {
            calls.set(calls.get() + 1);
            let n = calls.get();
            async move {
                if n < 3 {
                    Err(FetchError::Retryable(anyhow!("transient")))
                } else {
                    Ok(vec![1u8, 2, 3])
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(res, vec![1, 2, 3]);
        assert_eq!(calls.get(), 3);
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_after_max_attempts() {
        let calls = Cell::new(0u32);
        let res = fetch_with_retry(|| {
            calls.set(calls.get() + 1);
            async move { Err::<Vec<u8>, _>(FetchError::Retryable(anyhow!("always"))) }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.get(), crate::retry::MAX_ATTEMPTS);
    }

    #[tokio::test(start_paused = true)]
    async fn fatal_error_is_not_retried() {
        let calls = Cell::new(0u32);
        let res = fetch_with_retry(|| {
            calls.set(calls.get() + 1);
            async move { Err::<Vec<u8>, _>(FetchError::Fatal(anyhow!("403 Forbidden"))) }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.get(), 1, "a fatal (4xx) error must not be retried");
    }
}
