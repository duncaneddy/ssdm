//! Worker-runtime I/O: fetch each active product and write it (and any alias) to R2.
//! This module compiles only for wasm; host `cargo test` skips it.

use worker::*;

use crate::keys::{alias_key, object_key};
use crate::page::render_index_html;
use crate::products::{products, Product};

const USER_AGENT: &str = "ssdm-mirror/1.0 (+https://simplespacedata.org)";
const CACHE_CONTROL: &str = "public, max-age=3600";

/// Fetch all active products, write them to R2, regenerate the landing page,
/// and return an error summarizing any per-product failures.
pub async fn ingest_all(env: &Env) -> Result<()> {
    let bucket = env.bucket("BUCKET")?;
    let items = products();
    let mut failures: Vec<String> = Vec::new();

    for p in &items {
        if !p.active {
            continue;
        }
        if let Err(e) = fetch_and_store(&bucket, p).await {
            console_error!("ingest failed for {}: {}", p.name, e);
            failures.push(format!("{} ({})", p.name, e));
        }
    }

    let html = render_index_html(&items);
    bucket
        .put("index.html", html)
        .http_metadata(meta("text/html; charset=utf-8"))
        .execute()
        .await?;

    if !failures.is_empty() {
        return Err(Error::RustError(format!(
            "{} product(s) failed: {}",
            failures.len(),
            failures.join("; ")
        )));
    }
    Ok(())
}

async fn fetch_and_store(bucket: &Bucket, p: &Product) -> Result<()> {
    let bytes = fetch_bytes(&p.url).await?;
    bucket
        .put(object_key(p), bytes.clone())
        .http_metadata(meta(p.content_type))
        .execute()
        .await?;
    if let Some(akey) = alias_key(p) {
        bucket
            .put(akey, bytes)
            .http_metadata(meta(p.content_type))
            .execute()
            .await?;
    }
    Ok(())
}

async fn fetch_bytes(url: &str) -> Result<Vec<u8>> {
    let mut headers = Headers::new();
    headers.set("User-Agent", USER_AGENT)?;

    let mut init = RequestInit::new();
    init.with_method(Method::Get).with_headers(headers);

    let request = Request::new_with_init(url, &init)?;
    let mut response = Fetch::Request(request).send().await?;

    let status = response.status_code();
    if !(200..300).contains(&status) {
        return Err(Error::RustError(format!("HTTP {status} from {url}")));
    }
    response.bytes().await
}

fn meta(content_type: &str) -> HttpMetadata {
    HttpMetadata {
        content_type: Some(content_type.to_string()),
        cache_control: Some(CACHE_CONTROL.to_string()),
        ..Default::default()
    }
}
