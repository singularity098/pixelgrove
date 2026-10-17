# PixelGrove — Decisions, Architecture & Milestones

Source requirements: [requirements.md](requirements.md). This document records the
decisions made while reviewing them and the plan for building v1.

**Target scale:** ~14k files today; designed and tested for up to **100k files**.

---

## 1. Decisions

### Stack
| Area | Decision |
|---|---|
| Backend | Rust, `axum` + `tokio`, a single static binary |
| Frontend | Svelte 5 + Vite SPA, compiled and **embedded in the binary** |
| Database | SQLite (WAL mode, FTS5) — one file in `/config` |
| Packaging | Docker image (`linux/amd64` + `linux/arm64`) published to GHCR via GitHub Actions |
| CI | GitHub Actions: `cargo fmt`, `clippy`, tests, frontend lint/build on every PR |
| License | MIT (proposed — easy to change) |

### Previews (no thumbnails on disk)
1. Use the **embedded EXIF preview** when the file has one (most camera/phone JPEGs) — near-free.
2. Otherwise **resize on the fly**, held in a size-capped **in-memory LRU cache** (default 256 MB, configurable, lost on restart).
3. Strong HTTP caching headers (`ETag` from path+size+mtime) so browsers keep previews.
4. Nothing preview-related is ever written to `/config` or the media volumes.

### Media
- **v1 formats:** JPEG, PNG, WebP, GIF, AVIF; video MP4, WebM, MOV.
- **Later:** HEIC, RAW (CR2/NEF/ARW/DNG), MKV/AVI.
- **Video tiles:** the browser renders the first frame (`preload="metadata"`); no ffmpeg in v1.
- Video is streamed directly with HTTP range requests (seeking works).
- **Timestamp:** EXIF `DateTimeOriginal` (or video creation time), falling back to file mtime.

### Views
| Sidebar item | Behaviour |
|---|---|
| **Photos** (default) | Flat, Android-style list of every folder containing media, sorted newest-first by its most recent item; that item is the cover. Opening a folder shows its subfolders (as tiles) above its media, newest first. |
| **Timeline** | Every item across the library in one stream, grouped by day/month, with a date scrubber. |
| **Explore** | No ML. Browse by place, camera/lens, media type and year. |
| **Map** | Leaflet + OpenStreetMap tiles, clustered markers from GPS EXIF. |
| **Sharing** | List of active share links; copy, edit, revoke. |
| **Favorites** | The current user's favorites. |
| **File View** | True folder tree (including folders with no media) in a list layout with name/size/date, plus a filter box that shows only matching folders. |
| **Trash** | Deleted items, restore or delete permanently. |
| **Storage** | Used/free space for each mounted media volume (shown at the bottom of the sidebar). |

### Search
- Case-insensitive substring match over folder names, file names and metadata
  (camera, lens, date, place name, embedded description/keywords).
- Place names come from a **bundled offline reverse-geocoding dataset** (cities, ~10–30 MB) — no external calls.

### Changing files
- **Trash:** deleting moves the file to `<watch root>/.pixelgrove-trash/<relative path>`;
  it is purged automatically after 30 days. Trash is disabled automatically for read-only mounts.
- No other edits in v1 (no rotate/rename/move/upload).
- **Favorites** are per-user and stored in the database only (never written into files).

### Users & sharing
| Role | Can |
|---|---|
| Admin | Everything, incl. settings, watch directories, users |
| Editor | Browse, download, favorite, share, delete (trash) |
| Viewer | Browse, download, favorite — no settings or other changes |

- Admins can optionally restrict a user to specific folders.
- **First run:** a setup screen creates the admin; can also be set with environment variables.
- Passwords hashed with Argon2; cookie-based sessions.
- **Share links** (file or folder): optional expiry, optional password, download on/off;
  folder links show items added later; revocable from the Sharing page.
- A **public base URL** setting is used to build share links (expected behind a reverse proxy with HTTPS;
  copy-to-clipboard requires HTTPS).

### Scanning
- Watch directories default to `/media/*`; any other path can be added in Settings.
- Live filesystem watching (`notify`) **plus** a periodic full rescan (default every 6 h; important for NFS/SMB where watching is unreliable).
- Incremental: files are only re-read when path, size or mtime change.
- Hidden folders skipped; configurable exclude patterns (defaults include `@eaDir`, `.thumbnails`,
  `.pixelgrove-trash`); symlinks not followed.

### Viewer & slideshow
- Zoom button, mouse-wheel zoom, pinch-zoom and swipe on mobile, keyboard navigation.
- Actions: copy to clipboard (converted to PNG), download, share, info/metadata panel, favorite.
- Slideshow from a photo or a folder: configurable interval, shuffle, include subfolders, include videos.

### UI
- Dark mode by default, light mode available (and "follow system").
- Responsive layout for both desktop and mobile; collapsible sidebar (drawer on mobile).

### Out of scope for v1
ML (faces/objects), Live Photos, burst grouping, albums independent of folders,
multiple languages, editing, upload.

---

## 2. Architecture

```
┌──────────────── Docker container ────────────────┐
│  pixelgrove (single Rust binary)                  │
│                                                   │
│  axum HTTP ── /api/*   JSON API (auth, browse,   │
│     │                  search, share, settings)   │
│     ├──────── /media/* originals (range requests) │
│     ├──────── /preview/* EXIF preview / resized   │
│     │             └─ in-memory LRU cache          │
│     └──────── /*       embedded Svelte SPA        │
│                                                   │
│  Indexer: scanner (parallel) + notify watcher     │
│           + periodic rescan + trash purge         │
│                                                   │
│  SQLite (WAL) ── /config/pixelgrove.db            │
└───────────────────────────────────────────────────┘
   /config  (small: db + config file)
   /media/… (your library, ro or rw)
```

### Data model (outline)
- `roots` — watch directories, writable flag, last scan.
- `folders` — path, parent, plus aggregates (item count, newest timestamp, cover item) kept
  current by the indexer so the Photos view is a single indexed query.
- `media` — folder, file name, size, mtime, taken_at, type, dimensions, duration, camera, lens,
  GPS, place, description/keywords, trashed_at.
- `media_fts` — FTS5 (trigram tokenizer) over names + metadata, for substring search.
- `users`, `sessions`, `favorites (user, media)`, `user_folder_access`, `shares`.

### Performance targets (100k files, modest NAS/mini-PC)
| Metric | Target |
|---|---|
| Initial scan | < 5 min (bounded by disk I/O reading EXIF headers) |
| No-change rescan | < 30 s (stat only) |
| Browse/list API | < 50 ms |
| Search | < 100 ms |
| Idle memory | < 50 MB + preview cache |
| Docker image | < 30 MB |
| `/config` size | a few tens of MB (database only) |

A synthetic 100k-file library will be generated for benchmarks in CI/local testing.

---

## 3. Milestones

| # | Milestone | Contents |
|---|---|---|
| M0 ✅ | **Scaffold** | Cargo project, Svelte app embedded via `rust-embed`, config loading, Dockerfile (multi-arch), GitHub Actions CI + GHCR publish |
| M1 | **Indexer** | SQLite schema + migrations, parallel scanner, EXIF/video metadata, folder aggregates, watcher, periodic rescan, exclusions |
| M2 | **Auth & users** | First-run setup, login, sessions, roles, folder restrictions, user management |
| M3 | **Core browsing** | App shell + sidebar, dark/light theme, Photos (folders) view, folder view, preview pipeline, photo/video viewer (zoom, wheel, swipe, download, clipboard, info) |
| M4 | **Timeline, File View, Search** | Timeline with scrubber, File View tree + filter, global search |
| M5 | **Favorites, Trash, Storage** | Per-user favorites, trash with restore/purge, storage display |
| M6 | **Sharing** | Share links for files/folders, public share pages, Sharing management view |
| M7 | **Map & Explore** | Offline reverse geocoding, map with clustering, Explore facets |
| M8 | **Slideshow & polish** | Slideshow, mobile/desktop polish, 100k-file benchmark pass, docs (README, docker-compose example) |
