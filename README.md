# SSDM — Simple Space Data Mirror

A self-hosted service that mirrors Earth Orientation Parameter (EOP) and
space-weather data files (plus selected CelesTrak GP groups) into a public
Cloudflare R2 bucket served at https://simplespacedata.org, for use with
[Brahe](https://github.com/duncaneddy/brahe).

## How it works

A long-running Docker daemon (`src/`) fetches each product in `src/products.rs`
on its own interval, compares the download against a locally persisted
`status.json` (the change-detection source of truth), and uploads changed bytes
to a public R2 bucket under `/<category>/<source>/<name>/latest/<filename>`. A
public R2 custom domain serves the bucket directly via Cloudflare's CDN; the
daemon is only ever in the write path.

Each product has its own cadence (e.g. CelesTrak groups every 2h, daily EOP/space
weather, slower realizations weekly). The daemon sleeps until the soonest product
is due, syncs the due set sequentially, and persists `status.json` after each
product (locally and to R2). Per-host rate limiting and a small stagger keep us
polite to upstreams; failed downloads retry briefly in-run and otherwise wait for
the product's next interval.

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
   as `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`, with `R2_ACCOUNT_ID`.

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
at https://simplespacedata.org.

### Delete the bucket (full decommission)

⚠️ This permanently removes the public mirror — every data file and `status.json`
served at https://simplespacedata.org. Only do this to retire the service.

1. **Stop the daemon** (above) so nothing re-uploads mid-teardown.
2. **Disconnect public access:** R2 → `ssdm-data` → Settings → remove the
   `simplespacedata.org` custom domain, and delete the `/`→`/index.html` rule
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

- **Add a CelesTrak group:** add its slug to `CELESTRAK_GROUPS`.
- **Add a product:** add a `Product { … }` entry to `products()`.
- **New C04 realization (e.g. `21u25`):** add `c04_21u25` with
  `active: true, alias_name: Some("c04")`, and set the old `c04_20u24` to
  `active: false, alias_name: None`. The old versioned path freezes (stays
  served); the `c04` alias follows the new realization.

Run `cargo test` (the registry validation test enforces one active product per
alias) and redeploy.
