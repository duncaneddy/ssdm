pub mod keys;
pub mod message;
pub mod page;
pub mod products;
pub mod retry;
pub mod status;

#[cfg(target_arch = "wasm32")]
mod fetch;

#[cfg(target_arch = "wasm32")]
mod ingest;

#[cfg(target_arch = "wasm32")]
mod producer;

#[cfg(target_arch = "wasm32")]
use worker::{console_error, event, Env, Result, ScheduleContext, ScheduledEvent};

/// Cron entrypoint: runs the full ingest once per scheduled trigger.
#[cfg(target_arch = "wasm32")]
#[event(scheduled)]
pub async fn scheduled(_event: ScheduledEvent, env: Env, _ctx: ScheduleContext) -> Result<()> {
    if let Err(e) = ingest::ingest_all(&env).await {
        console_error!("ingest_all failed: {}", e);
        return Err(e);
    }
    Ok(())
}
