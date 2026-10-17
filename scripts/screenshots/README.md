# README screenshots

The images in `docs/screenshots/` show a generated demo library: procedural landscapes
(`landscape.py`) with made-up dates, cameras and GPS positions (`demo_library.py`), so
no real photos are needed. `shoot.mjs` drives the app with Playwright at 1440×900
(2× pixel density) and 390×844 for phones; see its header for the steps. The PNGs were
then scaled to 75% and saved as WebP (quality 82), and the three phone shots placed side
by side for `mobile.webp`. The two videos in the demo are short `ffmpeg` zooms over a
generated landscape.
