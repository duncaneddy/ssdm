# SSDM — Simple Space Data Mirror

A self-hosted service that mirrors Earth Orientation Parameter (EOP) files and
star catalogs (FK5, Hipparcos) into a public Cloudflare R2 bucket served at
https://yourgreatdomain.com, for use with
[Brahe](https://github.com/duncaneddy/brahe).

## How it works

A long-running Docker daemon (`src/`) fetches each product in `src/products.rs`
on its own interval, compares the download against a locally persisted
`status.json` (the change-detection source of truth), and uploads changed bytes
to a public R2 bucket under `/<category>/<source>/<name>/latest/<filename>`. A
public R2 custom domain serves the bucket directly via Cloudflare's CDN; the
daemon is only ever in the write path.

Each product has its own cadence (daily EOP, weekly realizations, monthly
availability checks for the fixed star catalogs). The daemon sleeps until the
soonest product is due, syncs the due set sequentially, and persists
`status.json` after each product (locally and to R2). Per-host rate limiting and
a small stagger keep us polite to upstreams; failed downloads retry briefly
in-run and otherwise wait for the product's next interval.

## Product availability

Every product declares an `Availability` (`src/products.rs`):

| State | Fetched | Listed on the landing page | Object in the bucket |
|---|---|---|---|
| `Active` | yes | yes | kept current |
| `Frozen` | no | yes, greyed out | frozen at its last value |
| `Disabled` | no | no | kept, but unadvertised |

`Frozen` is for a superseded realization whose versioned path must keep
resolving for existing consumers. `Disabled` is for an upstream that is broken:
the product stays in the registry so it can be restored by flipping one field.

**CelesTrak is currently `Disabled`** — as of 2026-07-15, celestrak.org
connection-times-out for both the GP groups and the space-weather file. All 11
products remain in `src/products.rs`; set the `CELESTRAK` constant back to
`Availability::Active` to resume the whole provider in one edit. To test it
before committing to that, force a fetch by name — this works regardless of
availability:

```bash
cargo run -- sync --product starlink
```

## Local development

```bash
cargo test                 # all unit tests
cargo run -- sync --all    # one-shot full sync (needs R2 env vars set)
cargo run -- sync --product starlink   # force a single product
cargo run -- daemon        # run the scheduler loop
```

Configuration is via environment variables (see `.env.example`). Copy it to
`.env` and fill in the R2 credentials; the binary loads `.env` automatically.
For a local `cargo run`, also set `DATA_DIR` to a writable local path (e.g.
`DATA_DIR=./data`) — the default `/data` is the Docker volume mount.

## R2 setup (one-time)

1. **Create the bucket:** `npx wrangler r2 bucket create ssdm-data`
2. **Public custom domain:** R2 → `ssdm-data` → Settings → Public access →
   Connect a custom domain → `yourgreatdomain.com`.
3. **Serve the landing page at `/`:** Rules → Transform Rules → Rewrite URL →
   if URI path equals `/`, rewrite path to `/index.html`.
4. **Create an R2 API token** (Object Read & Write) and put its values in `.env`
   as `BUCKET_ACCESS_KEY_ID` / `BUCKET_SECRET_ACCESS_KEY`. Set `BUCKET_ENDPOINT`
   to `https://<account-id>.r2.cloudflarestorage.com` and `SITE_DOMAIN` to your
   public domain. (Storage is S3-generic — point `BUCKET_ENDPOINT`/`BUCKET_REGION`
   at AWS S3, MinIO, etc. to use a different provider.)

## Deploy

```bash
cp .env.example .env        # fill in R2 credentials
docker compose up -d        # build + run the daemon
docker compose logs -f      # watch sync activity
docker compose run --rm ssdm sync --all   # force a full sync on demand
```

## Teardown

### Stop the sync daemon

```bash
docker compose stop         # pause the daemon (keeps the container + volume)
docker compose down         # stop and remove the container (keeps the /data volume)
docker compose down -v      # also delete the local /data volume (file mirror + status.json)
```

Stopping the daemon only halts syncing — whatever is already in R2 keeps serving
at https://yourgreatdomain.com.

### Delete the bucket (full decommission)

⚠️ This permanently removes the public mirror — every data file and `status.json`
served at https://yourgreatdomain.com. Only do this to retire the service.

1. **Stop the daemon** (above) so nothing re-uploads mid-teardown.
2. **Disconnect public access:** R2 → `ssdm-data` → Settings → remove the
   `yourgreatdomain.com` custom domain, and delete the `/`→`/index.html` rule
   under Rules → Transform Rules.
3. **Empty the bucket** — R2 will not delete a non-empty bucket:
   - Dashboard: R2 → `ssdm-data` → ⋯ → **Empty bucket**, *or*
   - AWS CLI against the R2 S3 endpoint (configured with the same R2 key/secret):
     ```bash
     aws s3 rm s3://ssdm-data --recursive \
       --endpoint-url "https://<account-id>.r2.cloudflarestorage.com"
     ```
4. **Delete the bucket:**
   ```bash
   npx wrangler r2 bucket delete ssdm-data
   ```
5. **Revoke the R2 API token** (Cloudflare → R2 → Manage API Tokens) and delete
   your local `.env`.

## Adding or changing products

Edit `src/products.rs`:

- **Add a product:** add a `Product { … }` entry to `products()`. A dataset that
  spans several files (e.g. a catalog plus its ReadMe) shares one `name`; the
  object key includes the filename, so they do not collide.
- **Add a CelesTrak group:** add its slug to `CELESTRAK_GROUPS`.
- **New C04 realization (e.g. `21u25`):** add `c04_21u25` with
  `availability: Availability::Active, alias_name: Some("c04")`, and set the old
  `c04_20u24` to `availability: Availability::Frozen, alias_name: None`. The old
  versioned path freezes (stays served and listed); the `c04` alias follows the
  new realization.
- **Pause a broken upstream:** set its products to `Availability::Disabled`.

Run `cargo test` (the registry validation test enforces one fetched product per
alias) and redeploy.

Note that upstream fetches are bounded by an inactivity timeout rather than a
total deadline (`src/fetch.rs`), so a large-but-slow product is fine while a
dead host still fails fast. The largest product today (Hipparcos `hip_main.dat`,
~53 MB) takes ~80s to download.
