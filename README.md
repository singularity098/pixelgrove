# PixelGrove

An ultra-light, fast, self-hosted photo and video library written in Rust.

> **Status:** early development (milestone M1 — indexer). See
> [docs/requirements.md](docs/requirements.md) and [docs/plan.md](docs/plan.md).

## Running with Docker

See [docker-compose.example.yml](docker-compose.example.yml). Mount your libraries under
`/media` and a small directory at `/config`, then open `http://<host>:8080`.

Images are published to `ghcr.io/singularity098/pixelgrove`: `edge` tracks `main`,
and version tags (`1.2.3`, `1.2`, `latest`) are created from `v*` git tags.

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

Apart from `PIXELGROVE_LOG`, the same keys (lower-case, without the prefix) can be used in `pixelgrove.toml`.

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
