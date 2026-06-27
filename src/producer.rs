//! Cron producer: write the landing page, then enqueue one fetch per product.
//! Compiles only for wasm.

use worker::*;

use crate::keys::{alias_key, object_key};
use crate::message::IngestMessage;
use crate::page::render_index_html;
use crate::products::products;

const CACHE_CONTROL: &str = "public, max-age=3600";

fn html_meta() -> HttpMetadata {
    HttpMetadata {
        content_type: Some("text/html; charset=utf-8".to_string()),
        cache_control: Some(CACHE_CONTROL.to_string()),
        ..Default::default()
    }
}

pub async fn run_producer(env: &Env) -> Result<()> {
    let items = products();
    let bucket = env.bucket("BUCKET")?;

    // 1. Landing page first — never blocked by a fetch outage.
    bucket
        .put("index.html", render_index_html(&items))
        .http_metadata(html_meta())
        .execute()
        .await?;

    // 2. Enqueue one descriptor per active product.
    let queue = env.queue("QUEUE")?;
    let now = Date::now().as_millis();
    for p in items.iter().filter(|p| p.active) {
        let msg = IngestMessage {
            key: object_key(p),
            url: p.url.clone(),
            content_type: p.content_type.to_string(),
            alias_key: alias_key(p),
            enqueued_at: now,
        };
        queue.send(&msg).await?;
    }
    Ok(())
}
