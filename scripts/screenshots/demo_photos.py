"""Builds the README demo library from a set of photos: they are sorted into trip
folders and given EXIF dates, cameras and GPS positions, and two short videos are made
from two of them with ffmpeg.

The photos are the "Demo_Images" set (Krea-2_000NN_.jpg, generated images with no
EXIF), which isn't in the repository. Needs Pillow and ffmpeg (libx264).
Usage: demo_photos.py <photos dir> <library dir>
"""
import os
import random
import subprocess
import sys
from datetime import datetime, timedelta

from PIL import Image

PHOTOS, ROOT = sys.argv[1], sys.argv[2]

CAMERAS = [
    ("Apple", "iPhone 16 Pro", "iPhone 16 Pro back triple camera 6.765mm f/1.78", "IMG_"),
    ("SONY", "ILCE-7M4", "FE 24-70mm F2.8 GM II", "DSC0"),
    ("FUJIFILM", "X-T5", "XF16-55mmF2.8 R LM WR", "DSCF"),
    ("Google", "Pixel 9", None, "PXL_"),
]

# folder, first day, days, photo numbers, (lat, lon) spots, cameras, video (photo, name)
TRIPS = [
    ("2025/Lapland & Svalbard", "2025-02-08", 2, [2, 23, 4, 24, 1, 21, 3, 22],
     [(68.4385, 17.4273), (69.6492, 18.9553), (78.2232, 15.6267)], [0, 1], (23, "aurora")),
    ("2025/Monteverde", "2025-06-14", 1, [5, 7, 26, 28], [(10.3010, -84.8255)], [2, 0], None),
    ("2025/Corcovado", "2025-06-19", 1, [6, 8, 25, 27], [(8.5381, -83.5891)], [0, 2], None),
    ("2024/Merzouga", "2024-10-19", 1, [9, 11, 29, 30], [(31.0802, -4.0133)], [2, 0], None),
    ("2024/Wadi Rum", "2024-10-24", 1, [10, 12, 31, 32], [(29.5759, 35.4204)], [0, 2], None),
    ("2024/Maldives", "2024-07-27", 1, [13, 16, 33, 36], [(3.2028, 73.2207)], [3, 0], None),
    ("2024/Algarve", "2024-08-05", 1, [14, 15, 34, 35], [(37.0179, -7.9304)], [3, 1],
     (35, "waves")),
    ("2023/Patagonia", "2023-09-02", 1, [17, 20, 37], [(-49.2713, -72.9411), (-50.3379, -72.2648)],
     [1, 2], None),
    ("2023/Chamonix", "2023-10-07", 1, [18, 19, 38, 40], [(45.9237, 6.8694)], [2, 1], None),
]


def dms(x):
    x = abs(x)
    d = int(x)
    m = int((x - d) * 60)
    s = round(((x - d) * 60 - m) * 60, 2)
    return (d, m, s)


def with_exif(src, dest, taken, gps, camera):
    make, model, lens, _ = CAMERAS[camera]
    img = Image.open(src)
    exif = img.getexif()
    exif[0x010F] = make
    exif[0x0110] = model
    ex = exif.get_ifd(0x8769)
    ex[0x9003] = taken.strftime("%Y:%m:%d %H:%M:%S")
    if lens:
        ex[0xA434] = lens
    lat, lon = gps
    g = exif.get_ifd(0x8825)
    g[1] = "N" if lat >= 0 else "S"
    g[2] = dms(lat)
    g[3] = "E" if lon >= 0 else "W"
    g[4] = dms(lon)
    img.save(dest, quality=90, exif=exif)
    t = taken.timestamp()
    os.utime(dest, (t, t))


def video(src, dest, taken):
    """A slow 6-second zoom into the photo."""
    w, h = Image.open(src).size
    size = "720x1280" if h > w else "1280x720"
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-loop", "1", "-i", src, "-vf",
         f"scale=-2:2160,zoompan=z='1+0.0008*on':x='iw/2-iw/zoom/2':y='ih/2-ih/zoom/2'"
         f":d=150:s={size}:fps=25,format=yuv420p",
         "-t", "6", "-c:v", "libx264", "-crf", "26", "-preset", "slow",
         "-metadata", f"creation_time={taken.strftime('%Y-%m-%dT%H:%M:%SZ')}", dest],
        check=True,
    )
    t = taken.timestamp()
    os.utime(dest, (t, t))


def main():
    rng = random.Random(7)
    n = 0
    for folder, start, days, photos, spots, cams, clip in TRIPS:
        os.makedirs(os.path.join(ROOT, folder), exist_ok=True)
        t0 = datetime.fromisoformat(start) + timedelta(hours=7)
        for i, number in enumerate(photos):
            n += 1
            camera = cams[i % len(cams)]
            day = days * i // len(photos)
            taken = t0 + timedelta(days=day, minutes=rng.randint(0, 720))
            lat, lon = spots[i % len(spots)]
            gps = (lat + rng.uniform(-0.04, 0.04), lon + rng.uniform(-0.04, 0.04))
            name = f"{CAMERAS[camera][3]}{4100 + n * 7:04d}.jpg"
            src = os.path.join(PHOTOS, f"Krea-2_{number:05d}_.jpg")
            dest = os.path.join(ROOT, folder, name)
            with_exif(src, dest, taken, gps, camera)
            if clip and clip[0] == number:
                video(src, os.path.join(ROOT, folder, f"{clip[1]}.mp4"),
                      taken + timedelta(minutes=3))
    print(n, "photos")


if __name__ == "__main__":
    main()
