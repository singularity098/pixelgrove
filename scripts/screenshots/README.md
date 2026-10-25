# README screenshots

The images in `docs/screenshots/` show a demo library made from a set of AI-generated
pictures ("Demo_Images", `Krea-2_000NN_.jpg`, not in the repository): `demo_photos.py`
sorts them into trip folders, gives them EXIF dates, cameras and GPS positions, and makes
two short videos from two of them with `ffmpeg`. `shoot.mjs` drives the app with
Playwright at 1440×900 (2× pixel density; the shared folder at 1024×640) and 390×844
for phones; see its header for the steps. The PNGs were then scaled to 75% and saved as
WebP (quality 82), and the three phone shots placed side by side (transparent gaps,
1548 px wide) for `mobile.webp`.

When the logo changed, only the header was redone: the new header was rendered with a
transparent background at the same sizes and laid over the old brand and search box.
Shoot the whole set again when the picture set is at hand.

`demo_library.py` and `landscape.py` make a larger library of generated landscapes
instead, for when the pictures aren't at hand (e.g. for benchmarks).
