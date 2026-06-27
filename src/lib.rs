pub mod keys;
pub mod message;
pub mod page;
pub mod products;
pub mod retry;
pub mod status;

#[cfg(target_arch = "wasm32")]
mod consumer;
#[cfg(target_arch = "wasm32")]
mod fetch;
#[cfg(target_arch = "wasm32")]
mod producer;

#[cfg(target_arch = "wasm32")]
use worker::{console_error, event, Context, Env, MessageBatch, Result, ScheduleContext, ScheduledEvent};

#[cfg(target_arch = "wasm32")]
use crate::message::IngestMessage;

/// Cron entrypoint: write the landing page and enqueue per-product fetches.
#[cfg(target_arch = "wasm32")]
#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) -> Result<()> {
    if let Err(e) = producer::run_producer(&env).await {
        console_error!("producer failed: {}", e);
        return Err(e);
    }
    Ok(())
}

/// Queue entrypoint: fetch, store-on-change, update status, ack/retry/drop.
#[cfg(target_arch = "wasm32")]
#[event(queue)]
pub async fn queue(batch: MessageBatch<IngestMessage>, env: Env, _ctx: Context) -> Result<()> {
    if let Err(e) = consumer::run_consumer(batch, &env).await {
        console_error!("consumer failed: {}", e);
        return Err(e);
    }
    Ok(())
}
