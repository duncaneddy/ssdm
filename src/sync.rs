//! One sync pass: render the index, then fetch/compare/upload each requested
//! product, persisting status incrementally so a later failure cannot undo an
//! earlier product's progress.

use std::path::Path;

use anyhow::Context;
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

/// Long cache for data files — they change rarely and the CDN should hold them.
const DATA_CACHE: &str = "public, max-age=3600";
/// Short cache for index.html: it is re-rendered on every restart and sync pass
/// to reflect registry/schedule changes, so a long TTL would leave returning
/// visitors on a stale page for up to an hour after a deploy.
const INDEX_CACHE: &str = "public, max-age=60";
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
    site_domain: &str,
    now_ms: u64,
) -> SyncSummary {
    let mut summary = SyncSummary::default();
    let mut status = load_status(data_dir);

    // Index page (from the full registry), best-effort.
    let html = render_index_html(site_domain, all);
    if let Err(e) = store.put(INDEX_KEY, html.into_bytes(), INDEX_CT, INDEX_CACHE).await {
        warn!("index.html upload failed: {e}");
    }

    for p in process {
        let key = object_key(p);
        // A corrupt archive, or any one part of a multi-part product, failing is
        // treated exactly like a failed download: the prior object and its status
        // stay intact, and the product's interval is the retry.
        match fetch_product(p, fetcher, rate).await {
            Ok(bytes) => {
                summary.checked += 1;
                let hash = content_hash(&bytes);
                let size = bytes.len() as u64;
                let is_current = status.get(&key).map(|e| e.hash == hash).unwrap_or(false);
                if is_current {
                    // Bytes already in the bucket: record the successful check only.
                    apply_update(&mut status, &key, &hash, size, now_ms);
                    info!("unchanged {key}");
                } else {
                    // Commit the hash to status ONLY after the bytes are stored.
                    // Recording it first would make the next pass hash the same
                    // content, conclude "unchanged", and never retry — leaving the
                    // object missing while the daemon reports success. For a static
                    // catalog, nothing upstream would ever dislodge that.
                    match persist_bytes(store, data_dir, p, &key, bytes).await {
                        Ok(()) => {
                            apply_update(&mut status, &key, &hash, size, now_ms);
                            summary.changed += 1;
                            info!("updated {key} ({size} bytes)");
                        }
                        Err(e) => {
                            summary.failed += 1;
                            record_attempt(&mut status, &key, now_ms);
                            warn!("upload failed for {key}: {e}");
                        }
                    }
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

/// Fetch every part of a product, in order, and concatenate them into the single
/// object we serve.
///
/// Ordering is the correctness property for a split catalog: Tycho-2's rows are
/// ordered across `tyc2.dat.00` … `.19`, so the parts must be joined in registry
/// order, and any part failing must fail the whole product rather than yield a
/// silently short catalog.
///
/// Each part is decoded as it arrives so only one compressed part is held at a
/// time on top of the accumulated output.
async fn fetch_product<F: Fetcher>(
    p: &Product,
    fetcher: &F,
    rate: &mut RateLimiter,
) -> anyhow::Result<Vec<u8>> {
    let mut out: Vec<u8> = Vec::new();
    for (i, url) in p.urls.iter().enumerate() {
        rate.throttle(url).await;
        let raw = fetcher
            .fetch(url)
            .await
            .with_context(|| format!("part {}/{}: {url}", i + 1, p.urls.len()))?;
        let decoded = decode_body(p, raw, url)?;
        check_declared_format(p, &decoded, url)?;
        if out.is_empty() {
            // Adopt the first part rather than copying it — this is the whole
            // buffer for a single-part product. Concatenating onto an empty
            // accumulator is the identity, so adopting is equivalent to extending
            // even if a part is legitimately empty.
            out = decoded;
            // Parts of a split catalog are near-uniform in size, so the first one
            // predicts the rest. Reserving up front keeps peak memory bounded and
            // predictable instead of letting repeated growth hold an old
            // allocation and its larger replacement at ~500 MB scale.
            let remaining = p.urls.len().saturating_sub(1);
            if remaining > 0 {
                out.reserve(out.len().saturating_mul(remaining));
            }
        } else {
            out.extend_from_slice(&decoded);
        }
    }
    Ok(out)
}

/// Decompress a gzip-archived part so it is stored, hashed, and served in the
/// plain-text form its ReadMe documents. Change detection therefore keys off the
/// bytes we actually serve, not the archive envelope.
///
/// `MultiGzDecoder`, not `GzDecoder`: the latter decodes only the first member
/// and silently ignores whatever follows, so a concatenated archive would be
/// truncated to its first member and trailing junk would pass as valid. Both
/// failure modes would serve a plausible-looking partial catalog. (This is not a
/// hypothetical shape for CDS — VizieR's own txt.gz endpoint returns a gzip
/// member followed by an uncompressed copy of the same table.)
fn decode_body(p: &Product, bytes: Vec<u8>, url: &str) -> anyhow::Result<Vec<u8>> {
    use std::io::Read;

    if !p.gunzip {
        return Ok(bytes);
    }
    let mut out = Vec::new();
    flate2::read::MultiGzDecoder::new(&bytes[..])
        .read_to_end(&mut out)
        .with_context(|| format!("gunzip {url} ({} bytes)", bytes.len()))?;
    Ok(out)
}

/// Reject a body that is not the format the product declares.
///
/// A download endpoint that answers 200 with an HTML interstitial or a CDN error
/// page is otherwise invisible to this pipeline: the bytes hash stably, upload
/// cleanly under `image/jpeg`, and compare "unchanged" on every later pass, so
/// the mirror would serve HTML from an image URL indefinitely with the landing
/// page reporting success. `decode_body` catches this for the gzipped catalogs
/// because a non-archive fails to decode; a raw binary product has no such
/// check, so the format signature is it.
///
/// Only formats with a signature we actually mirror are checked — text products
/// have no magic bytes to test and pass through.
fn check_declared_format(p: &Product, bytes: &[u8], url: &str) -> anyhow::Result<()> {
    let expected: &[u8] = match p.content_type {
        "image/jpeg" => &[0xFF, 0xD8, 0xFF],
        "image/png" => b"\x89PNG\r\n\x1a\n",
        _ => return Ok(()),
    };
    anyhow::ensure!(
        bytes.starts_with(expected),
        "{url} returned {} bytes that are not {} data",
        bytes.len(),
        p.content_type
    );
    Ok(())
}

/// Write the product locally and to the bucket.
///
/// Takes ownership so the primary upload can move the buffer instead of copying
/// it. At the scale of the largest product (Tycho-2, ~500 MB) a `to_vec()` here
/// doubles the daemon's peak memory for no benefit. Only an aliased product —
/// which is uploaded twice by definition — pays for a clone.
async fn persist_bytes<S: Store>(
    store: &S,
    data_dir: &Path,
    p: &Product,
    key: &str,
    bytes: Vec<u8>,
) -> anyhow::Result<()> {
    write_mirror(data_dir, key, &bytes)?;
    if let Some(akey) = alias_key(p) {
        write_mirror(data_dir, &akey, &bytes)?;
        store.put(&akey, bytes.clone(), p.content_type, DATA_CACHE).await?;
    }
    store.put(key, bytes, p.content_type, DATA_CACHE).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::products::Product;
    use crate::schedule::Schedule;
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
        // (key, body) of every successful put, so tests can assert on the bytes
        // that actually reached the bucket rather than inferring them.
        puts: Mutex<Vec<(String, Vec<u8>)>>,
        get_body: Option<Vec<u8>>,   // bytes returned by get(), None => 404
        fail_put_for: Option<String>, // key whose put fails, to model an upload error
    }
    impl FakeStore {
        fn put_keys(&self) -> Vec<String> {
            self.puts.lock().unwrap().iter().map(|(k, _)| k.clone()).collect()
        }
        fn body_of(&self, key: &str) -> Option<Vec<u8>> {
            self.puts.lock().unwrap().iter().rev().find(|(k, _)| k == key).map(|(_, b)| b.clone())
        }
    }
    impl Store for FakeStore {
        async fn put(&self, key: &str, bytes: Vec<u8>, _ct: &str, _cc: &str) -> anyhow::Result<()> {
            if self.fail_put_for.as_deref() == Some(key) {
                return Err(anyhow::anyhow!("simulated upload failure for {key}"));
            }
            self.puts.lock().unwrap().push((key.to_string(), bytes));
            Ok(())
        }
        async fn get(&self, _key: &str) -> anyhow::Result<Option<Vec<u8>>> {
            Ok(self.get_body.clone())
        }
    }

    fn product(name: &str, url: &str) -> Product {
        Product {
            category: "catalog", source: "celestrak", name: Box::leak(name.to_string().into_boxed_str()),
            urls: vec![url.into()], filename: format!("{name}.json"),
            content_type: "application/json", gunzip: false, availability: crate::products::Availability::Active, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(3600)),
        }
    }

    fn gzipped(plain: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(plain).unwrap();
        e.finish().unwrap()
    }

    fn gz_product(name: &str, url: &str, gunzip: bool) -> Product {
        Product {
            gunzip,
            ..product(name, url)
        }
    }

    #[tokio::test]
    async fn gzip_product_is_stored_and_hashed_decompressed() {
        let dir = tempfile::tempdir().unwrap();
        let plain = b"FK5 fixed-width rows".to_vec();
        let p = gz_product("fk5", "https://h/catalog.gz", true);
        let all = vec![gz_product("fk5", "https://h/catalog.gz", true)];
        let mut out = HashMap::new();
        out.insert("https://h/catalog.gz".to_string(), Some(gzipped(&plain)));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 0));

        // Assert on the bytes that actually reached the bucket, and separately on
        // the local mirror — inferring one from the other would hide a divergence.
        let key = crate::keys::object_key(&p);
        assert_eq!(
            store.body_of(&key).expect("product uploaded"),
            plain,
            "the gzip envelope must not be served"
        );
        let written = std::fs::read(dir.path().join(&key)).unwrap();
        assert_eq!(written, plain, "the local mirror holds the same decoded bytes");

        // Status records the served bytes: size and hash are the decompressed ones.
        let st = crate::local::load_status(dir.path());
        assert_eq!(st[&key].size, plain.len() as u64);
        assert_eq!(st[&key].hash, crate::status::content_hash(&plain));
    }

    #[tokio::test]
    async fn corrupt_gzip_is_treated_as_a_failed_fetch() {
        let dir = tempfile::tempdir().unwrap();
        let p = gz_product("fk5", "https://h/catalog.gz", true);
        let all = vec![gz_product("fk5", "https://h/catalog.gz", true)];
        let key = crate::keys::object_key(&p);

        // Seed a prior success so we can prove undecodable bytes do not clobber it.
        let mut seed = crate::status::Status::new();
        crate::status::apply_update(&mut seed, &key, "goodhash", 42, 500);
        crate::local::save_status(dir.path(), &seed).unwrap();

        let mut out = HashMap::new();
        out.insert("https://h/catalog.gz".to_string(), Some(b"not gzip at all".to_vec()));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (0, 0, 1), "undecodable => failed");
        assert!(
            !store.put_keys().iter().any(|k| k == &key),
            "a product that failed to decode must not be uploaded"
        );
        let st = crate::local::load_status(dir.path());
        assert_eq!(st[&key].hash, "goodhash", "prior good content preserved");
        assert_eq!(st[&key].last_attempt, 1000, "attempt still recorded");
    }

    #[test]
    fn gzip_decode_rejects_trailing_junk_and_keeps_every_member() {
        let p = gz_product("fk5", "https://h/catalog.gz", true);

        // A gzip member followed by non-gzip bytes must fail, not silently yield
        // the first member. VizieR's txt.gz endpoint returns exactly this shape.
        let mut junk = gzipped(b"REAL");
        junk.extend_from_slice(b"TRAILING JUNK");
        assert!(
            decode_body(&p, junk, "https://h/catalog.gz").is_err(),
            "trailing junk must not be silently discarded"
        );

        // A concatenated (multi-member) archive must decode in full, not be
        // truncated to its first member — that would serve a partial catalog.
        let mut two = gzipped(b"FIRST");
        two.extend_from_slice(&gzipped(b"SECOND"));
        assert_eq!(decode_body(&p, two, "https://h/catalog.gz").unwrap(), b"FIRSTSECOND");
    }

    #[tokio::test]
    async fn non_gzip_product_bytes_pass_through_untouched() {
        let dir = tempfile::tempdir().unwrap();
        // Bytes that happen to be gzip are still served verbatim when gunzip is false.
        let body = gzipped(b"payload");
        let p = gz_product("plain", "https://h/plain", false);
        let all = vec![gz_product("plain", "https://h/plain", false)];
        let mut out = HashMap::new();
        out.insert("https://h/plain".to_string(), Some(body.clone()));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        let written = std::fs::read(dir.path().join(crate::keys::object_key(&p))).unwrap();
        assert_eq!(written, body, "gunzip: false must not decode anything");
    }

    fn multipart_product(urls: &[&str], gunzip: bool) -> Product {
        Product {
            category: "star_catalog", source: "cds", name: "tycho2",
            urls: urls.iter().map(|u| u.to_string()).collect(),
            filename: "Tycho2_Catalog.txt".into(),
            content_type: "text/plain", gunzip,
            availability: crate::products::Availability::Active, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(3600)),
        }
    }

    #[tokio::test]
    async fn multipart_product_is_joined_in_registry_order() {
        // Tycho-2's rows run in sequence across tyc2.dat.00 … .19, so the served
        // file is only correct if the parts are concatenated in that exact order.
        let dir = tempfile::tempdir().unwrap();
        let p = multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], true);
        let all = vec![multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], true)];
        let mut out = HashMap::new();
        out.insert("https://h/p0".to_string(), Some(gzipped(b"AAA\n")));
        out.insert("https://h/p1".to_string(), Some(gzipped(b"BBB\n")));
        out.insert("https://h/p2".to_string(), Some(gzipped(b"CCC\n")));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 0));

        let key = crate::keys::object_key(&p);
        assert_eq!(
            store.body_of(&key).expect("uploaded"),
            b"AAA\nBBB\nCCC\n",
            "parts must be joined in order, each decompressed"
        );
        // 3 upstream parts collapse to exactly ONE served object.
        assert_eq!(
            store.put_keys().iter().filter(|k| k.contains("tycho2/latest")).count(),
            1,
            "a multi-part product is served as a single file"
        );
        let st = crate::local::load_status(dir.path());
        assert_eq!(st[&key].size, 12, "status size is the joined length");
    }

    #[tokio::test]
    async fn empty_parts_do_not_disturb_the_join() {
        // fetch_product adopts the first part instead of copying it. That is only
        // sound because concatenating onto an empty buffer is the identity — so an
        // empty leading (or interior) part must not shift or drop anything.
        let dir = tempfile::tempdir().unwrap();
        let p = multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], false);
        let all = vec![multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], false)];
        let mut out = HashMap::new();
        out.insert("https://h/p0".to_string(), Some(Vec::new())); // empty first part
        out.insert("https://h/p1".to_string(), Some(b"BBB\n".to_vec()));
        out.insert("https://h/p2".to_string(), Some(Vec::new())); // empty interior part
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!(
            store.body_of(&crate::keys::object_key(&p)).expect("uploaded"),
            b"BBB\n",
            "empty parts contribute nothing and shift nothing"
        );
    }

    #[tokio::test]
    async fn one_bad_part_fails_the_whole_product() {
        // A short catalog is worse than no update: it looks valid and parses fine.
        // If any part fails, nothing may be uploaded.
        let dir = tempfile::tempdir().unwrap();
        let p = multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], true);
        let all = vec![multipart_product(&["https://h/p0", "https://h/p1", "https://h/p2"], true)];
        let key = crate::keys::object_key(&p);

        let mut out = HashMap::new();
        out.insert("https://h/p0".to_string(), Some(gzipped(b"AAA\n")));
        out.insert("https://h/p1".to_string(), None); // middle part unavailable
        out.insert("https://h/p2".to_string(), Some(gzipped(b"CCC\n")));
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (0, 0, 1));
        assert!(
            !store.put_keys().iter().any(|k| k == &key),
            "a partial join must never be uploaded"
        );
        let st = crate::local::load_status(dir.path());
        assert_eq!(st[&key].hash, "", "no content recorded for a failed join");
        assert_eq!(st[&key].last_attempt, 1000);
    }

    fn image_product(url: &str, content_type: &'static str, filename: &str) -> Product {
        Product {
            category: "texture", source: "solarsystemscope", name: "moon",
            urls: vec![url.into()], filename: filename.into(),
            content_type, gunzip: false,
            availability: crate::products::Availability::Active, alias_name: None,
            info_url: None, cadence_label: None,
            schedule: Schedule::Every(Duration::from_secs(3600)),
        }
    }

    #[tokio::test]
    async fn html_served_as_an_image_is_rejected_not_mirrored() {
        // The dangerous case is a 200 response carrying an HTML interstitial. It
        // fetches fine, hashes stably, and would upload under image/jpeg — and
        // because the hash never changes, every later pass would call it
        // "unchanged" and never repair it. It must fail like a bad download.
        let dir = tempfile::tempdir().unwrap();
        let p = image_product("https://h/2k_moon.jpg", "image/jpeg", "2k_moon.jpg");
        let all = vec![image_product("https://h/2k_moon.jpg", "image/jpeg", "2k_moon.jpg")];
        let key = crate::keys::object_key(&p);

        let mut out = HashMap::new();
        out.insert(
            "https://h/2k_moon.jpg".to_string(),
            Some(b"<!DOCTYPE html><html>nope</html>".to_vec()),
        );
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (0, 0, 1), "not an image => failed");
        assert!(
            !store.put_keys().iter().any(|k| k == &key),
            "HTML must never be uploaded under an image content type"
        );
        let st = crate::local::load_status(dir.path());
        assert_eq!(st[&key].hash, "", "no content recorded, so the next pass retries");
    }

    #[tokio::test]
    async fn genuine_image_bytes_pass_the_format_check() {
        let dir = tempfile::tempdir().unwrap();
        // Minimal valid signatures for the two formats the registry mirrors.
        for (ct, filename, magic) in [
            ("image/jpeg", "2k_moon.jpg", vec![0xFFu8, 0xD8, 0xFF, 0xE0, 0x00]),
            ("image/png", "2k_ring.png", b"\x89PNG\r\n\x1a\n\x00".to_vec()),
        ] {
            let url = format!("https://h/{filename}");
            let p = image_product(&url, ct, filename);
            let all = vec![image_product(&url, ct, filename)];
            let mut out = HashMap::new();
            out.insert(url.clone(), Some(magic.clone()));
            let fetcher = FakeFetcher { out };
            let store = FakeStore::default();
            let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

            let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
            assert_eq!(sum.failed, 0, "{ct} with a valid signature must be accepted");
            assert_eq!(store.body_of(&crate::keys::object_key(&p)).unwrap(), magic);
        }
    }

    #[test]
    fn format_check_ignores_products_without_a_signature() {
        // Text products have no magic bytes; the check must not invent a rule for
        // them or every EOP file would start failing.
        let text = product("active", "https://h/active");
        assert!(check_declared_format(&text, b"1973 01 01 ...", "https://h/active").is_ok());
        assert!(check_declared_format(&text, b"", "https://h/active").is_ok());
    }

    #[tokio::test]
    async fn failed_upload_does_not_mark_content_current() {
        // If a product's bytes never reached the bucket, status must NOT record
        // their hash as current. Recording it makes the next pass compute the
        // same hash, conclude "unchanged", and skip the upload forever — leaving
        // the object permanently missing while the daemon reports success. For a
        // static catalog whose upstream content never changes again, "forever" is
        // literal: nothing would ever dislodge the wrong status.
        let dir = tempfile::tempdir().unwrap();
        let p = product("active", "https://h/active");
        let all = vec![product("active", "https://h/active")];
        let key = crate::keys::object_key(&p);
        let mut out = HashMap::new();
        out.insert("https://h/active".to_string(), Some(b"data".to_vec()));

        // Pass 1: fetch succeeds, upload fails.
        let fetcher = FakeFetcher { out: out.clone() };
        let store = FakeStore { fail_put_for: Some(key.clone()), ..Default::default() };
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);
        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!(sum.failed, 1, "an upload failure is a failure, not a success");

        let st = crate::local::load_status(dir.path());
        assert_ne!(
            st.get(&key).map(|e| e.hash.as_str()).unwrap_or(""),
            crate::status::content_hash(b"data"),
            "content that was never stored must not be recorded as current"
        );

        // Pass 2: same bytes, working store. The product MUST be uploaded now.
        let fetcher = FakeFetcher { out };
        let store = FakeStore::default();
        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 2000).await;
        assert_eq!(sum.changed, 1, "the retry must re-upload");
        assert!(
            store.put_keys().contains(&key),
            "a previously-failed upload must be retried on the next pass"
        );
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

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 0));
        let puts = store.put_keys();
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

        run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        let data_key = |puts: &[String]| {
            puts.iter()
                .filter(|k| k.contains("active/latest/active.json"))
                .count()
        };
        assert_eq!(
            data_key(&store.put_keys()),
            1,
            "first run uploads the product once"
        );

        let sum = run_sync(&all, &[&p], &fetcher, &store, &mut rate, dir.path(), "example.org", 2000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 0, 0));
        assert_eq!(
            data_key(&store.put_keys()),
            1,
            "unchanged content must not re-upload the product data"
        );
        // status.json must still be uploaded even when content is unchanged
        assert!(
            store.put_keys().iter().filter(|k| k.as_str() == "status.json").count() >= 2,
            "status.json must be uploaded to R2 after each run"
        );
    }

    #[tokio::test]
    async fn empty_process_still_refreshes_index() {
        // The daemon's startup pass relies on this: with nothing due to fetch,
        // run_sync must still re-render and upload index.html so a restart picks
        // up registry/schedule changes without fetching any product.
        let dir = tempfile::tempdir().unwrap();
        let all = vec![product("active", "https://h/active")];
        let fetcher = FakeFetcher { out: HashMap::new() };
        let store = FakeStore::default();
        let mut rate = RateLimiter::new(Duration::ZERO, Duration::ZERO);

        let sum = run_sync(&all, &[], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (0, 0, 0), "nothing fetched");
        let puts = store.put_keys();
        assert!(puts.contains(&"index.html".to_string()), "index.html refreshed with empty process list");
        assert!(!puts.iter().any(|k| k.contains("active/latest")), "no product data uploaded");
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
        let sum = run_sync(&all, &[&bad, &good], &fetcher, &store, &mut rate, dir.path(), "example.org", 1000).await;
        assert_eq!((sum.checked, sum.changed, sum.failed), (1, 1, 1));

        // good was still processed despite bad failing first.
        let puts = store.put_keys();
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
