//! Native HTTP fetch with a per-request timeout and short bounded retry.

use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use log::warn;

use crate::retry::{backoff_delay, MAX_ATTEMPTS};
use crate::sync::Fetcher;

const USER_AGENT: &str = "ssdm-mirror/1.0 (+https://simplespacedata.org)";

/// Call `make()` up to MAX_ATTEMPTS times, sleeping `backoff_delay` between tries.
pub async fn fetch_with_retry<Fut, MakeReq>(mut make: MakeReq) -> Result<Vec<u8>>
where
    MakeReq: FnMut() -> Fut,
    Fut: Future<Output = Result<Vec<u8>>>,
{
    let mut last_err = anyhow!("no attempts made");
    for attempt in 1..=MAX_ATTEMPTS {
        match make().await {
            Ok(bytes) => return Ok(bytes),
            Err(e) => {
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

    async fn get_once(&self, url: &str) -> Result<Vec<u8>> {
        let resp = self.client.get(url).send().await?;
        let status = resp.status();
        if !status.is_success() {
            return Err(anyhow!("HTTP {status} from {url}"));
        }
        Ok(resp.bytes().await?.to_vec())
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
                if n < 3 { Err(anyhow::anyhow!("transient")) } else { Ok(vec![1u8, 2, 3]) }
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
            async move { Err::<Vec<u8>, _>(anyhow::anyhow!("always")) }
        })
        .await;
        assert!(res.is_err());
        assert_eq!(calls.get(), crate::retry::MAX_ATTEMPTS);
    }
}
