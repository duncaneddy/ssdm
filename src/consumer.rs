//! Queue consumer: fetch each product, store on change, maintain status.json,
//! and ack/retry/drop per the backoff policy. Compiles only for wasm.

use worker::*;

use crate::fetch::fetch_bytes;
use crate::message::IngestMessage;
use crate::retry::{next_delay_secs, should_drop};
use crate::status::{apply_update, content_hash, parse_status, serialize_status, Status};

const STATUS_KEY: &str = "status.json";
const FETCH_TIMEOUT_SECS: u64 = 20;
const CACHE_CONTROL: &str = "public, max-age=3600";
const CAS_MAX_ATTEMPTS: u32 = 5;

fn data_meta(content_type: &str) -> HttpMetadata {
    HttpMetadata {
        content_type: Some(content_type.to_string()),
        cache_control: Some(CACHE_CONTROL.to_string()),
        ..Default::default()
    }
}

/// One product's downloaded result, pending the status update.
struct Fetched {
    body: IngestMessage,
    bytes: Vec<u8>,
    hash: String,
}

pub async fn run_consumer(batch: MessageBatch<IngestMessage>, env: &Env) -> Result<()> {
    let bucket = env.bucket("BUCKET")?;
    let messages = batch.messages()?;

    // 1. Fetch each message's product (sequentially; batch is small).
    //    Collect successes for a single status update; ack/retry failures.
    let now = Date::now().as_millis();
    let mut fetched: Vec<Fetched> = Vec::new();

    for msg in &messages {
        let body = msg.body().clone();
        match fetch_bytes(&body.url, FETCH_TIMEOUT_SECS).await {
            Ok(bytes) => {
                let hash = content_hash(&bytes);
                fetched.push(Fetched { body, bytes, hash });
                msg.ack();
            }
            Err(e) => {
                let elapsed = now.saturating_sub(body.enqueued_at) / 1000;
                if should_drop(elapsed) {
                    console_error!("dropping {} after {}s: {}", body.key, elapsed, e);
                    msg.ack();
                } else {
                    let delay = next_delay_secs(elapsed);
                    let opts = QueueRetryOptionsBuilder::new()
                        .with_delay_seconds(delay)
                        .build();
                    msg.retry_with_options(&opts);
                }
            }
        }
    }

    if fetched.is_empty() {
        return Ok(());
    }

    // 2. Read status.json once, decide per product whether content changed,
    //    write changed bytes (+alias) to R2.
    let (mut status, etag) = read_status(&bucket).await?;
    for f in &fetched {
        let changed = apply_update(&mut status, &f.body.key, &f.hash, f.bytes.len() as u64, now);
        if changed {
            bucket
                .put(&f.body.key, f.bytes.clone())
                .http_metadata(data_meta(&f.body.content_type))
                .execute()
                .await?;
            if let Some(akey) = &f.body.alias_key {
                bucket
                    .put(akey, f.bytes.clone())
                    .http_metadata(data_meta(&f.body.content_type))
                    .execute()
                    .await?;
            }
        }
    }

    // 3. Write status.json back with a conditional (CAS) loop.
    write_status_cas(&bucket, status, etag, &fetched, now).await
}

/// Read `status.json`, returning the parsed map and its etag (None if absent).
async fn read_status(bucket: &Bucket) -> Result<(Status, Option<String>)> {
    match bucket.get(STATUS_KEY).execute().await? {
        Some(obj) => {
            let etag = obj.etag();
            let bytes = match obj.body() {
                Some(b) => b.bytes().await?,
                None => Vec::new(),
            };
            Ok((parse_status(&bytes), Some(etag)))
        }
        None => Ok((Status::new(), None)),
    }
}

/// Conditionally write `status`. On etag conflict, re-read, re-apply this
/// batch's entries onto the fresh map, and retry.
async fn write_status_cas(
    bucket: &Bucket,
    mut status: Status,
    mut etag: Option<String>,
    fetched: &[Fetched],
    now_ms: u64,
) -> Result<()> {
    for attempt in 0..CAS_MAX_ATTEMPTS {
        let cond = match &etag {
            Some(t) => Conditional { etag_matches: Some(t.clone()), etag_does_not_match: None, uploaded_before: None, uploaded_after: None },
            None => Conditional { etag_matches: None, etag_does_not_match: Some("*".to_string()), uploaded_before: None, uploaded_after: None },
        };
        let result = bucket
            .put(STATUS_KEY, serialize_status(&status))
            .only_if(cond)
            .http_metadata(data_meta("application/json"))
            .execute()
            .await;

        match result {
            Ok(_) => return Ok(()),
            Err(e) => {
                if attempt + 1 == CAS_MAX_ATTEMPTS {
                    console_error!("status.json CAS failed after {CAS_MAX_ATTEMPTS} tries: {e}");
                    return Ok(()); // data already written; status updates next run
                }
                // Conflict (or other) — re-read and re-apply our entries.
                let (fresh, fresh_etag) = read_status(bucket).await?;
                status = fresh;
                etag = fresh_etag;
                for f in fetched {
                    apply_update(&mut status, &f.body.key, &f.hash, f.bytes.len() as u64, now_ms);
                }
            }
        }
    }
    Ok(())
}
