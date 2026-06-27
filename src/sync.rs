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

#[allow(async_fn_in_trait)]
pub trait Fetcher {
    async fn fetch(&self, url: &str) -> anyhow::Result<Vec<u8>>;
}

#[allow(async_fn_in_trait)]
pub trait Store {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: &str, cache_control: &str) -> anyhow::Result<()>;
    /// Fetch an object's bytes, or `None` if it does not exist (404).
    async fn get(&self, key: &str) -> anyhow::Result<Option<Vec<u8>>>;
}

const STATUS_KEY: &str = "status.json";
const STATUS_CT: &str = "application/json";

/// Long cache for data files and the page shell — they change rarely and the
/// CDN should hold them.
const DATA_CACHE: &str = "public, max-age=3600";
/// Short cache for status.json: it changes every pass and the landing page reads
/// it on each load for freshness, so a long TTL would show stale "last updated".
const STATUS_CACHE: &str = "public, max-age=60";

/// Seed the local `status.json` from R2 when the local copy is missing/empty.
///
/// The daemon treats the local volume as the source of truth and uploads the
/// whole map on every pass. On a fresh volume that would clobber an existing
/// remote `status.json` (e.g. during migration, or a single-product test) down
/// to only the just-synced products. Seeding the local map from R2 first makes
/// partial syncs merge into the existing set instead of truncating it.
///
/// Best-effort: any R2 error is logged and ignored (the daemon still works, it
/// just falls back to re-populating via a full pass).
pub async fn bootstrap_status<S: Store>(store: &S, data_dir: &Path) {
    if !load_status(data_dir).is_empty() {
        return; // local already has state — nothing to seed
    }
    match store.get(STATUS_KEY).await {
        Ok(Some(bytes)) => {
            let remote = crate::status::parse_status(&bytes);
            if remote.is_empty() {
                return;
            }
            match save_status(data_dir, &remote) {
                Ok(()) => info!("bootstrapped local status.json from R2 ({} entries)", remote.len()),
                Err(e) => warn!("failed to write bootstrapped status.json: {e}"),
            }
        }
        Ok(None) => info!("no remote status.json to bootstrap from; starting fresh"),
        Err(e) => warn!("status.json bootstrap from R2 failed (continuing): {e}"),
    }
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
    if let Err(e) = store.put(INDEX_KEY, html.into_bytes(), INDEX_CT, DATA_CACHE).await {
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
        let status_body = crate::status::serialize_status(&status).into_bytes();
        if let Err(e) = store.put(STATUS_KEY, status_body, STATUS_CT, STATUS_CACHE).await {
            warn!("status.json upload failed after {key}: {e}");
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
    store.put(key, bytes.to_vec(), p.content_type, DATA_CACHE).await?;
    if let Some(akey) = alias_key(p) {
        write_mirror(data_dir, &akey, bytes)?;
        store.put(&akey, bytes.to_vec(), p.content_type, DATA_CACHE).await?;
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
        puts: Mutex<Vec<String>>,    // keys put
        get_body: Option<Vec<u8>>,   // bytes returned by get(), None => 404
    }
    impl Store for FakeStore {
        async fn put(&self, key: &str, _bytes: Vec<u8>, _ct: &str, _cc: &str) -> anyhow::Result<()> {
            self.puts.lock().unwrap().push(key.to_string());
            Ok(())
        }
        async fn get(&self, _key: &str) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self.get_body.clone())
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
        assert!(puts.contains(&"status.json".to_string()), "status.json must be uploaded to R2 after each product");
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
        let data_key = |puts: &[String]| {
            puts.iter()
                .filter(|k| k.contains("active/latest/active.json"))
                .count()
        };
        assert_eq!(
            data_key(&store.puts.lock().unwrap()),
            1,
            "first run uploads the product once"
        );

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), 2000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 0, 0));
        assert_eq!(
            data_key(&store.puts.lock().unwrap()),
            1,
            "unchanged content must not re-upload the product data"
        );
        // status.json must still be uploaded even when content is unchanged
        assert!(
            store.puts.lock().unwrap().iter().filter(|k| k.as_str() == "status.json").count() >= 2,
            "status.json must be uploaded to R2 after each run"
        );
    }

    #[tokio::test]
    async fn failure_preserves_prior_and_continues() {
        let dir = tempfile::tempdir().unwrap();
        let good = product("active", "https://h/active");
        let bad = product("bad", "https://h/bad");
        let all = vec![product("active", "https://h/active"), product("bad", "https://h/bad")];
        let bad_key = crate::keys::object_key(&bad);
        let good_key = crate::keys::object_key(&good);

        // Seed a prior successful status for the failing key so we can prove it
        // is preserved across a later failed fetch.
        let mut seed = crate::status::Status::new();
        crate::status::apply_update(&mut seed, &bad_key, "seedhash", 7, 500);
        crate::local::save_status(dir.path(), &seed).unwrap();

        let mut out = HashMap::new();
        out.insert("https://h/active".to_string(), Some(b"data".to_vec()));
        out.insert("https://h/bad".to_string(), None);
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        // Failing product FIRST, succeeding product second: proves the loop
        // continues past the failure.
        let sum = run_sync(&all, &[&bad, &good], &fetcher, &store, &mut rate, dir.path(), 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 1));

        // good was still processed despite bad failing first.
        let puts = store.puts.lock().unwrap();
        assert!(
            puts.iter().any(|k| k == &good_key),
            "succeeding product after the failure must still be uploaded"
        );

        // bad's prior success fields are untouched; only last_attempt advanced.
        let st = crate::local::load_status(dir.path());
        let bad_entry = &st[&bad_key];
        assert_eq!(bad_entry.last_checked, 500, "prior last_checked preserved");
        assert_eq!(bad_entry.hash, "seedhash", "prior hash preserved");
        assert_eq!(bad_entry.size, 7, "prior size preserved");
        assert_eq!(bad_entry.last_attempt, 1000, "failed attempt advances last_attempt");
        // good product persisted.
        assert!(st.keys().any(|k| k.contains("active/latest")));
    }

    #[tokio::test]
    async fn bootstrap_seeds_local_status_from_r2_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        // Remote status.json with two entries.
        let mut remote = crate::status::Status::new();
        crate::status::apply_update(&mut remote, "a/latest/a.json", "h1", 1, 100);
        crate::status::apply_update(&mut remote, "b/latest/b.json", "h2", 2, 200);
        let store = FakeStore {
            get_body: Some(crate::status::serialize_status(&remote).into_bytes()),
            ..Default::default()
        };

        // Local is empty → bootstrap pulls the remote map down.
        assert!(crate::local::load_status(dir.path()).is_empty());
        bootstrap_status(&store, dir.path()).await;
        assert_eq!(crate::local::load_status(dir.path()), remote);
    }

    #[tokio::test]
    async fn bootstrap_is_noop_when_local_already_has_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut local = crate::status::Status::new();
        crate::status::apply_update(&mut local, "local/latest/x.json", "lh", 9, 1);
        crate::local::save_status(dir.path(), &local).unwrap();

        // Remote has different state; bootstrap must NOT overwrite a non-empty local.
        let mut remote = crate::status::Status::new();
        crate::status::apply_update(&mut remote, "remote/latest/y.json", "rh", 5, 2);
        let store = FakeStore {
            get_body: Some(crate::status::serialize_status(&remote).into_bytes()),
            ..Default::default()
        };

        bootstrap_status(&store, dir.path()).await;
        assert_eq!(crate::local::load_status(dir.path()), local, "non-empty local untouched");
    }

    #[tokio::test]
    async fn bootstrap_handles_missing_remote() {
        let dir = tempfile::tempdir().unwrap();
        let store = FakeStore::default(); // get_body None => 404
        bootstrap_status(&store, dir.path()).await;
        assert!(crate::local::load_status(dir.path()).is_empty(), "stays empty when no remote");
    }
}
