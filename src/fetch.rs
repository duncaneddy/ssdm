//! Native HTTP fetch with a per-request timeout and short bounded retry.

use std::future::Future;
use std::time::Duration;

use anyhow::{anyhow, Result};
use log::warn;

use crate::retry::{backoff_delay, MAX_ATTEMPTS};
use crate::sync::Fetcher;


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

/// Fail an attempt that goes this long without delivering any bytes. This, not
/// the total deadline, is what bounds a dead or stalled upstream.
const READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Absolute ceiling for one attempt. Must exceed the slowest product's honest
/// transfer time: the Hipparcos main catalog is ~53 MB and takes ~82s from CDS,
/// so a total timeout sized for the small EOP files would fail it every time.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(600);

pub struct HttpFetcher {
    client: reqwest::Client,
}

impl HttpFetcher {
    pub fn new(site_domain: &str) -> Result<Self> {
        Self::with_timeouts(site_domain, READ_TIMEOUT, TOTAL_TIMEOUT)
    }

    /// Construct with explicit timeouts. Production uses `new`; tests use this to
    /// exercise the timeout policy on a sub-second scale.
    fn with_timeouts(site_domain: &str, read: Duration, total: Duration) -> Result<Self> {
        // Identify ourselves to upstreams with a contact URL (the public site).
        let user_agent = format!("ssdm-mirror/1.0 (+https://{site_domain})");
        let client = reqwest::Client::builder()
            .user_agent(user_agent)
            .read_timeout(read)
            .timeout(total)
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

    /// Serve one HTTP response whose body is dribbled out in `chunks` pieces,
    /// `gap` apart, then hold the connection open. Returns the bound URL.
    ///
    /// This models the two upstreams we care about: one that is slow but always
    /// making progress (CDS delivering 53 MB over ~82s), and one that accepts a
    /// connection and then stalls forever.
    async fn dribbling_server(chunks: usize, gap: Duration) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            use tokio::io::AsyncWriteExt;
            let _ = sock
                .write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {chunks}\r\n\r\n").as_bytes())
                .await;
            let _ = sock.flush().await;
            for _ in 0..chunks {
                tokio::time::sleep(gap).await;
                if sock.write_all(b"x").await.is_err() {
                    return;
                }
                let _ = sock.flush().await;
            }
            // Hold the socket open so a client that wants more bytes must time out.
            tokio::time::sleep(Duration::from_secs(60)).await;
        });
        format!("http://{addr}/slow")
    }

    // Real timers: reqwest's timeouts are driven by the real clock, so these
    // tests use short (sub-second) budgets rather than tokio's paused time.

    #[tokio::test]
    async fn slow_but_progressing_transfer_is_not_killed() {
        // 10 chunks × 50ms = ~500ms total, far beyond the 150ms inactivity
        // budget, but no single gap exceeds it. This is the property that makes
        // the 53 MB Hipparcos catalog fetchable; a total-deadline-only policy
        // (the old 20s `.timeout()`) fails exactly this case.
        let url = dribbling_server(10, Duration::from_millis(50)).await;
        let f = HttpFetcher::with_timeouts(
            "example.org",
            Duration::from_millis(150),
            Duration::from_secs(30),
        )
        .unwrap();
        let bytes = match f.get_once(&url).await {
            Ok(b) => b,
            Err(FetchError::Retryable(e)) | Err(FetchError::Fatal(e)) => {
                panic!("a steadily-progressing transfer must not time out: {e}")
            }
        };
        assert_eq!(bytes.len(), 10);
    }

    #[tokio::test]
    async fn stalled_transfer_fails_on_the_inactivity_budget() {
        // Headers arrive, then the body never does. The read timeout must fire
        // well before the total deadline — that is what keeps a dead upstream
        // from occupying the sync pass for the full ceiling.
        let url = dribbling_server(1, Duration::from_secs(60)).await;
        let f = HttpFetcher::with_timeouts(
            "example.org",
            Duration::from_millis(150),
            Duration::from_secs(30),
        )
        .unwrap();
        let start = std::time::Instant::now();
        let err = match f.get_once(&url).await {
            Ok(b) => panic!("expected a timeout, got {} bytes", b.len()),
            Err(FetchError::Retryable(e)) => e,
            Err(FetchError::Fatal(e)) => panic!("a stall is transient, not fatal: {e}"),
        };
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "must fail on the inactivity budget, not the 30s ceiling: took {:?} ({err})",
            start.elapsed()
        );
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
