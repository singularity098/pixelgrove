# PixelGrove

An ultra-light, fast, self-hosted photo and video library written in Rust.

> **Status:** early development (milestone M6 — sharing). See
> [docs/requirements.md](docs/requirements.md) and [docs/plan.md](docs/plan.md).

## Running with Docker

See [docker-compose.example.yml](docker-compose.example.yml). Mount your libraries under
`/media` and a small directory at `/config`, then open `http://<host>:8080`.

Images are published to `ghcr.io/singularity098/pixelgrove`: `edge` tracks `main`,
and version tags (`1.2.3`, `1.2`, `latest`) are created from `v*` git tags.

## Accounts

On first visit PixelGrove asks you to create the administrator account (or set
`PIXELGROVE_ADMIN_USER` / `PIXELGROVE_ADMIN_PASSWORD` to create it on start-up). Admins
can add users under **Users**:

| Role | Can |
|---|---|
| Viewer | Browse and download. No changeable options, including their own password. |
| Editor | Also favorites, sharing and trash, and change their own password. |
| Admin | Everything, including settings and user management. |

Any user can be limited to specific folders. Until setup is complete, anyone who can
reach the server can create the admin account, so finish setup before exposing it.

## Trash

Deleting an item moves it into a hidden `.pixelgrove-trash` folder at the top of its
media folder (same volume, so nothing is copied), mirroring where it came from. It can
be restored from **Trash** for 30 days, then it is deleted for good. Trash needs the
media mounted read-write; on a read-only mount (`:ro`) PixelGrove says so and leaves the
file alone. Favorites are per user and stored only in PixelGrove's database.

## Sharing

Editors and admins can share a photo or video (from the viewer) or a folder with
everything below it (from the folder page). Anyone with the link can view it without an
account. Links can expire, need a password, and allow or block downloads (with
downloads off, photos are shown as previews only and videos still play). Folder links
show items added later. Manage links under **Sharing**; admins see everyone's. Set
`PIXELGROVE_PUBLIC_URL` so links use your public address.

## Configuration

Settings come from `/config/pixelgrove.toml` (optional), overridden by environment variables:

| Variable | Default | Description |
|---|---|---|
| `PIXELGROVE_CONFIG_DIR` | `/config` | Database and config file location |
| `PIXELGROVE_BIND` | `0.0.0.0:8080` | Listen address |
| `PIXELGROVE_MEDIA_ROOTS` | `/media` | `:`-separated directories to scan |
| `PIXELGROVE_PUBLIC_URL` | — | External URL used in share links |
| `PIXELGROVE_PREVIEW_CACHE_MB` | `256` | In-memory preview cache size |
| `PIXELGROVE_RESCAN_INTERVAL_HOURS` | `6` | Hours between full rescans |
| `PIXELGROVE_EXCLUDE` | `@eaDir,#recycle,#snapshot,lost+found` | Comma-separated name globs to skip (hidden entries are always skipped) |
| `PIXELGROVE_LOG` | `info` | Log filter (e.g. `debug`) |
| `PIXELGROVE_ADMIN_USER` / `PIXELGROVE_ADMIN_PASSWORD` | — | Creates this admin on first start, instead of the setup screen |

Apart from `PIXELGROVE_LOG` and the `PIXELGROVE_ADMIN_*` variables, the same keys (lower-case, without the prefix) can be used in `pixelgrove.toml`.

## Development

Requirements: Rust (stable, ≥ 1.88) and Node.js 22.

```sh
# Backend on :8080 (serves the UI from web/dist if it has been built)
PIXELGROVE_CONFIG_DIR=./dev-config PIXELGROVE_MEDIA_ROOTS=~/Pictures cargo run

# Frontend with hot reload on :5173, proxying /api to :8080
cd web && npm install && npm run dev
```

Checks run in CI: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`,
`cargo test`, and `npm run check && npm run build` in `web/`.

To benchmark the indexer, generate a synthetic library and point a release build at it:

```sh
python3 scripts/gen-bench-library.py /tmp/bench-lib 100000
PIXELGROVE_CONFIG_DIR=/tmp/bench-cfg PIXELGROVE_MEDIA_ROOTS=/tmp/bench-lib cargo run --release
curl localhost:8080/api/library/status
```

A release build (`cd web && npm run build && cd .. && cargo build --release`) embeds the
UI in the single `pixelgrove` binary.

## License

[MIT](LICENSE)
