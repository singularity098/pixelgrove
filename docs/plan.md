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

As built (M3): JPEGs are decoded at 1/2–1/8 scale straight from DCT data, which makes a
thumbnail (shorter edge 360 px) of a 12 MP photo take ~20–30 ms uncached; the viewer
image (longer edge 2048 px) ~230 ms. EXIF orientation is applied to previews. PNG/WebP/GIF
are decoded fully; AVIF and small files are served as-is. JPEG XL (jxl-oxide, pure
Rust) is always converted, since most browsers can't show it: files transcoded from
JPEG are rebuilt as that JPEG and DCT-scaled (~0.7 s for 12 MP), others fully decoded
(~0.7–1.1 s, peaking at ~200–450 MB, so one at a time). Its EXIF comes from the `Exif`
box, Brotli-compressed or not. Zooming in the viewer loads the original (for JPEG XL
only where the browser shows it). Browse API calls take 1–4 ms at 100k files.

### Media
- **v1 formats:** JPEG, PNG, WebP, GIF, AVIF, JPEG XL; video MP4, WebM, MOV.
- **Later:** HEIC, RAW (CR2/NEF/ARW/DNG), MKV/AVI.
- **Video tiles:** one frame about a second in, extracted by a bundled minimal FFmpeg
  (static, ~11 MB: MP4/MOV/MKV/WebM in; H.264, HEVC, VP8/9, MPEG-4, MJPEG, ProRes; JPEG
  out), kept in the in-memory preview cache like photo previews; the newest 1,000 are made
  after each scan. Without FFmpeg, browsers grab a frame themselves. Measured: 24 tiles of
  4K HEVC and 1080p H.264 phone videos in under 1 s (0.4 MB), against 10 of 24 after 30 s
  (10.5 MB) when the browser had to do it.
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
- Multiple words must all match (in any of those places). `%` and `_` are literal.
- Place names come from a **bundled offline reverse-geocoding dataset** (cities, ~10–30 MB) — no external calls.
- As built (M4): each item's searchable text is stored lower-cased at scan time, so a search
  is one pass over one column: 20–90 ms at 100k files, with no separate full-text index
  (which would have roughly doubled the database).

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
| Viewer | Browse and download — no changeable options (no favorites, sharing or trash) |

- Admins can optionally restrict a user to specific folders.
- **First run:** a setup screen creates the admin; can also be set with environment variables.
- Passwords hashed with Argon2id; cookie-based sessions (HttpOnly, SameSite=Lax, Secure behind
  HTTPS) that expire after 30 days unused. Only a hash of each session token is stored.
- Failed sign-ins are throttled per username (10 per 15 minutes).
- State-changing API calls require a custom header, which blocks cross-site request forgery.
- Viewers have no changeable options, so they can't change their own password (an admin can).
- **Share links** (file or folder): optional expiry, optional password, download on/off;
  folder links show items added later; revocable from the Sharing page.
- Hardening for public exposure (after M8): failed passwords are throttled per client
  address (10 per 15 min; `X-Forwarded-For` trusted only from private networks) and per
  account (100); security headers (CSP, frame-ancestors, nosniff, no-referrer, HSTS
  over HTTPS); location data is removed from files served through share links by
  same-length byte patches applied while streaming, so range requests still work.
- A **public base URL** setting is used to build share links (expected behind a reverse proxy with HTTPS;
  copy-to-clipboard requires HTTPS).

### Scanning
- Watch directories default to `/media/*`; any other path can be added in Settings.
  As built (M8): folders added under **Settings → Media folders** (with a directory
  browser) are stored in the database and start indexing immediately; those from the
  configuration can't be removed there. Folders that overlap an existing one are refused.
- Live filesystem watching (`notify`) **plus** a periodic full rescan (default every 6 h; important for NFS/SMB where watching is unreliable).
- Incremental: files are only re-read when path, size or mtime change.
- Folders containing a `.nomedia` or `.ignore` file are skipped with everything below them.
- Hidden folders skipped; configurable exclude patterns (defaults include `@eaDir`, `.thumbnails`,
  `.pixelgrove-trash`); symlinks not followed.

### Viewer & slideshow
- Zoom button, mouse-wheel zoom, pinch-zoom and swipe on mobile, keyboard navigation.
- Actions: copy to clipboard (converted to PNG), download, share, info/metadata panel, favorite.
- Slideshow from a photo or a folder: configurable interval, shuffle, include subfolders, include videos.
  As built (M8): full screen with a cross-fade, the next photo decoded before it's shown,
  the screen kept awake; videos play to the end. It pages through the folder (or the
  whole subtree, newest first) as it goes, so a slideshow of the entire library starts
  instantly; shuffle draws from everything loaded so far while loading the rest in the
  background, without repeats until all have been shown. Also on shared folder links.

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

A synthetic library can be generated with `scripts/gen-bench-library.py`.

**Measured (M1, 100k small JPEGs in 1,026 folders, 4-core VM with SSD):**

| Metric | Result |
|---|---|
| Initial scan | 3.8–4.6 s (real photos on spinning disks/NAS will be I/O-bound and slower) |
| No-change rescan | 0.4–0.6 s |
| Database size | 14 MB (21 MB from M4, with stored search text) |
| Memory, idle after scans | ~45 MB (17 MB with the Docker image's allocator settings) |
| Memory, peak during first scan | ~145 MB |

**Final pass (M8, same library and VM, release build, best of 5 requests):**

| Metric | Result | Target |
|---|---|---|
| Initial scan | 5.6 s | < 5 min |
| No-change rescan | 0.4 s | < 30 s |
| Albums / folder / folder media / timeline page | 1–2 ms | < 50 ms |
| Search (one word / two words) | 14 / 29 ms | < 100 ms |
| Slideshow of the whole library (first page / page at 90k) | 44 / 118 ms | — |
| Timeline months, Explore, Map points | 50 / 147 / 11 ms | — |
| Thumbnail (12 MP JPEG, uncached) | 20–30 ms (2 ms for the small bench files) | — |
| Memory, idle after restart + rescan | 21–23 MB | < 50 MB + cache |
| Memory, after exercising every API (warm SQLite caches) | ~50 MB | — |
| Memory, peak during first scan | 110 MB | — |
| `/config` (database) | 25 MB | tens of MB |
| Docker image | ~25 MB (the static binary and a minimal FFmpeg, on `scratch`) | < 30 MB |

---

## 3. Milestones

| # | Milestone | Contents |
|---|---|---|
| M0 ✅ | **Scaffold** | Cargo project, Svelte app embedded via `rust-embed`, config loading, Dockerfile (multi-arch), GitHub Actions CI + GHCR publish |
| M1 ✅ | **Indexer** | SQLite schema + migrations, parallel scanner, EXIF/video metadata, folder aggregates, watcher, periodic rescan, exclusions |
| M2 ✅ | **Auth & users** | First-run setup, login, sessions, roles, folder restrictions, user management |
| M3 ✅ | **Core browsing** | App shell + sidebar, dark/light theme, Photos (folders) view, folder view, preview pipeline, photo/video viewer (zoom, wheel, swipe, download, clipboard, info) |
| M4 ✅ | **Timeline, File View, Search** | Timeline with scrubber, File View tree + filter, global search |
| M5 ✅ | **Favorites, Trash, Storage** | Per-user favorites, trash with restore/purge, storage display |
| M6 ✅ | **Sharing** | Share links for files/folders, public share pages, Sharing management view |
| M7 ✅ | **Map & Explore** | Offline reverse geocoding, map with clustering, Explore facets |
| M7+ ✅ | **Installable app (PWA)** | Manifest, icons (incl. maskable / Apple), service worker caching the app shell (never photos or API data) |
| M8 ✅ | **Slideshow & polish** | Slideshow, mobile/desktop polish, 100k-file benchmark pass, docs (README, docker-compose example) |
