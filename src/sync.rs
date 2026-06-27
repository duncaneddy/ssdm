//! One sync pass: render the index, then fetch/compare/upload each requested
//! product, persisting status incrementally so a later failure cannot undo an
//! earlier product's progress.

use std::path::Path;

use log::{info, warn};

use crate::keys::{alias_key, object_key};
use crate::local::{load_status, save_status, write_mirror};
use crate::page::render_index_html;
use crate::products::Product;
use crate::ratelimit::RateLimiter;
use crate::status::{apply_update, content_hash, record_attempt};

const INDEX_KEY: &str = "index.html";
const INDEX_CT: &str = "text/html; charset=utf-8";

pub trait Fetcher {
    async fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

pub trait Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str) -> anyhow::Result<()>;
}

#[derive(Default, Debug, PartialEq)]
pub struct SyncSummary {
    pub checked: u32,
    pub changed: u32,
    pub failed: u32,
}

pub async fn run_sync<F: Fetcher, S: Store>(
    all: &[Product],
    process: &[&Product],
    fetcher: &F,
    store: &S,
    rate: &mut RateLimiter,
    data_dir: &Path,
    now_ms: u64,
) -> SyncSummary {
    let mut summary = SyncSummary::default();
    let mut status = load_status(data_dir);

    // Index page (from the full registry), best-effort.
    let html = render_index_html(all);
    if let Err(e) = store.put(INDEX_KEY, html.into_bytes(), INDEX_CT).await {
        warn!("index.html upload failed: {e}");
    }

    for p in process {
        let key = object_key(p);
        rate.throttle(&p.url).await;
        match fetcher.fetch(&p.url).await {
            Ok(bytes) => {
                summary.checked += 1;
                let hash = content_hash(&bytes);
                let changed = apply_update(&mut status, &key, &hash, bytes.len() as u64, now_ms);
                if changed {
                    summary.changed += 1;
                    if let Err(e) = persist_bytes(store, data_dir, p, &key, &bytes).await {
                        warn!("upload failed for {key}: {e}");
                    } else {
                        info!("updated {key} ({} bytes)", bytes.len());
                    }
                } else {
                    info!("unchanged {key}");
                }
            }
            Err(e) => {
                summary.failed += 1;
                record_attempt(&mut status, &key, now_ms);
                warn!("fetch failed for {key}: {e}");
            }
        }
        if let Err(e) = save_status(data_dir, &status) {
            warn!("status persist failed after {key}: {e}");
        }
    }

    info!(
        "sync done: checked={} changed={} failed={}",
        summary.checked, summary.changed, summary.failed
    );
    summary
}

async fn persist_bytes<S: Store>(
    store: &S,
    data_dir: &Path,
    p: &Product,
    key: &str,
    bytes: &[u8],
) -> anyhow::Result<()> {
    write_mirror(data_dir, key, bytes)?;
    store.put(key, bytes.to_vec(), p.content_type).await?;
    if let Some(akey) = alias_key(p) {
        write_mirror(data_dir, &akey, bytes)?;
        store.put(&akey, bytes.to_vec(), p.content_type).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::Product;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::Duration;

    struct FakeFetcher {
        // url -> Some(bytes) on success, None to simulate a hard failure
        out: HashMap<String, Option<Vec<u8>>>,
    }
    impl Fetcher for FakeFetcher {
        async fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>> {
            match self.out.get(url) {
                Some(Some(b)) => Ok(b.clone()),
                _ => Err(anyhow::anyhow!("fail")),
            }
        }
    }

    #[derive(Default)]
    struct FakeStore {
        puts: Mutex<Vec<String>>, // keys put
    }
    impl Store for FakeStore {
        async fn put(&self, key: &str, _bytes: Vec<u8>, _ct: &str) -> anyhow::Result<()> {
            self.puts.lock().unwrap().push(key.to_string());
            Ok(())
        }
    }

    fn product(name: &str, url: &str) -> Product {
        Product {
            category: "catalog", source: "celestrak", name: Box::leak(name.to_string().into_boxed_str()),
            url: url.into(), filename: format!("{name}.json"),
            content_type: "application/json", active: true, alias_name: None,
            interval: Duration::from_secs(3600),
        }
    }

    #[tokio::test]
    async fn changed_uploads_and_records_status() {
        let dir = tempfile::tempdir().unwrap();
        let p = product("active", "https://h/active");
        let all = vec![product("active", "https://h/active")];
        let mut out = HashMap::new();
        out.insert("https://h/active".to_string(), Some(b"data".to_vec()));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 0));
        let puts = store.puts.lock().unwrap();
        assert!(puts.contains(&"index.html".to_string()));
        assert!(puts.iter().any(|k| k.contains("active/latest/active.json")));
        let st = crate::local::load_status(dir.path());
        assert!(!st.is_empty());
    }

    #[tokio::test]
    async fn unchanged_skips_data_upload() {
        let dir = tempfile::tempdir().unwrap();
        let p = product("active", "https://h/active");
        let all = vec![product("active", "https://h/active")];
        let mut out = HashMap::new();
        out.insert("https://h/active".to_string(), Some(b"data".to_vec()));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), 1000).await;
        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), 2000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 0, 0));
    }

    #[tokio::test]
    async fn failure_preserves_prior_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let good = product("active", "https://h/active");
        let bad = product("bad", "https://h/bad");
        let all = vec![product("active", "https://h/active"), product("bad", "https://h/bad")];
        let mut out = HashMap::new();
        out.insert("https://h/active".to_string(), Some(b"data".to_vec()));
        out.insert("https://h/bad".to_string(), None);
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&good, &bad], &fetcher, &store, &mut rate, dir.path(), 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 1));
        // good product still persisted despite bad's failure
        let st = crate::local::load_status(dir.path());
        assert!(st.keys().any(|k| k.contains("active/latest")));
    }
}
