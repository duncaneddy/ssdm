# SSDM — Simple Space Data Mirror

A Cloudflare Worker that mirrors Earth Orientation Parameter (EOP) and
space-weather data files (plus selected CelesTrak GP groups) into a public R2
bucket served at https://simplespacedata.org, for use with
[Brahe](https://github.com/duncaneddy/brahe).

## How it works

A cron-only Worker fetches each product in `src/products.rs` once daily and
writes the latest bytes to R2 under `/<category>/<source>/<name>/latest/<filename>`.
A public R2 custom domain serves the bucket directly via Cloudflare's CDN; the
Worker is never in the read path.

## Available data

See https://simplespacedata.org for the live, auto-generated product list. Example URLs:

- `https://simplespacedata.org/eop/iers/finals_all/latest/finals.all.iau2000.txt`
- `https://simplespacedata.org/eop/iers/c04/latest/EOP_C04_one_file_1962-now.txt` (stable alias)
- `https://simplespacedata.org/space_weather/celestrak/sw_all/latest/sw19571001.txt`
- `https://simplespacedata.org/catalog/celestrak/active/latest/active.json`

## Local development

```bash
cargo test                                   # host unit tests (registry, keys, page)
cargo check --target wasm32-unknown-unknown  # type-check worker I/O
npx wrangler dev --test-scheduled            # run locally
curl http://localhost:8787/__scheduled       # trigger the cron handler
```

## One-time Cloudflare setup

1. **Create the bucket:** `npx wrangler r2 bucket create ssdm-data`
2. **Public custom domain:** R2 → `ssdm-data` → Settings → Public access →
   Connect a custom domain → `simplespacedata.org`.
3. **Serve the landing page at `/`:** Rules → Transform Rules → Rewrite URL →
   if URI path equals `/`, rewrite path to `/index.html`. (R2 public buckets do
   not auto-resolve `/` to `index.html`.)
4. **Deploy secrets (GitHub repo → Settings → Secrets):**
   `CLOUDFLARE_API_TOKEN` (Workers Scripts: Edit + R2: Edit) and
   `CLOUDFLARE_ACCOUNT_ID`.
5. **Monitoring (native, no code):** Cloudflare → Notifications → enable Worker
   error and usage/billing alerts. The Worker fails loudly (a thrown summary on
   any product failure) so these fire; use `npx wrangler tail` to see which
   product/URL/status failed.

## Deploy

Push to `main` (CI deploys), or run `npx wrangler deploy` manually.

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
