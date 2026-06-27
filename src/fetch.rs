//! Worker-runtime HTTP fetch with a hard per-request timeout.
//! Compiles only for wasm; host `cargo test` skips it.

use std::time::Duration;

use futures_util::future::{select, Either};
use worker::*;

const USER_AGENT: &str = "ssdm-mirror/1.0 (+https://simplespacedata.org)";

/// GET `url`, returning the body bytes. Fails with a `RustError` if the request
/// does not complete within `timeout_secs` or returns a non-2xx status.
pub async fn fetch_bytes(url: &str, timeout_secs: u64) -> Result<Vec<u8>> {
    let headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)?;

    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);
    let request = Request::new_with_init(url, &init)?;

    let fetcher = Fetch::Request(request);
    let send = fetcher.send();
    let timeout = Delay::from(Duration::from_secs(timeout_secs));

    let mut response = match select(Box::pin(send), Box::pin(timeout)).await {
        Either::Left((result, _)) => result?,
        Either::Right((_, _)) => {
            return Err(Error::RustError(format!("timeout after {timeout_secs}s: {url}")))
        }
    };

    let status = response.status_code();
    if !(200..300).contains(&status) {
        return Err(Error::RustError(format!("HTTP {status} from {url}")));
    }
    response.bytes().await
}
