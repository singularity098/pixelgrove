"""Builds a demo library of generated landscapes with EXIF dates, cameras and GPS,
for the README screenshots. Needs numpy and Pillow. Usage: demo_library.py <dir>"""
import os
import random
import sys
from datetime import datetime, timedelta
from multiprocessing import Pool

from PIL import Image

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from landscape import render  # noqa: E402

ROOT = sys.argv[1]

CAMERAS = [
    ("Apple", "iPhone 15 Pro", "iPhone 15 Pro back triple camera 6.86mm f/1.78"),
    ("SONY", "ILCE-7M4", "FE 24-70mm F2.8 GM II"),
    ("FUJIFILM", "X-T5", "XF16-55mmF2.8 R LM WR"),
    ("Google", "Pixel 8", None),
]

# folder, start date, days, count, palettes, (lat, lon) spots, cameras, water bias
ALBUMS = [
    ("2025/Iceland Ring Road", "2025-09-14", 9, 26, ["night", "mist", "noon", "alpine"],
     [(64.1466, -21.9426), (65.6835, -18.0878), (64.2539, -15.2082)], [1, 0], 0.5),
    ("2025/Kyoto in Spring", "2025-04-02", 6, 22, ["sakura", "dawn", "mist"],
     [(35.0116, 135.7681), (34.9671, 135.7727), (35.6762, 139.6503)], [0, 2], 0.3),
    ("2025/Lake Tahoe Weekend", "2025-07-19", 2, 14, ["forest", "noon", "dusk"],
     [(38.9399, -119.9772), (39.0968, -120.0324)], [0, 3], 0.7),
    ("2024/Swiss Alps", "2024-09-03", 7, 24, ["alpine", "noon", "mist", "dawn"],
     [(46.9480, 7.4474), (46.6863, 7.8632), (47.0502, 8.3093)], [1, 2], 0.35),
    ("2024/Big Sur Road Trip", "2024-06-08", 5, 18, ["coast", "dusk", "mist"],
     [(36.2704, -121.8081), (36.6002, -121.8947), (37.7749, -122.4194)], [0, 1], 0.6),
    ("2024/Christmas at Home", "2024-12-23", 4, 10, ["mist", "forest", "night"], [], [0, 3], 0.2),
    ("2024/Morocco", "2024-03-11", 8, 20, ["desert", "dusk", "dawn"],
     [(31.6295, -7.9811), (30.9189, -6.8934), (34.0181, -5.0078)], [2, 0], 0.0),
    ("2023/Scottish Highlands", "2023-10-05", 6, 18, ["mist", "forest", "dusk"],
     [(57.4778, -4.2247), (56.8198, -5.1052), (55.9533, -3.1883)], [1, 3], 0.45),
    ("2023/Norway Fjords", "2023-06-17", 8, 20, ["coast", "alpine", "noon", "night"],
     [(60.3913, 5.3221), (61.1153, 10.4662), (62.4722, 6.1495)], [1, 0], 0.65),
    ("2022/Patagonia", "2022-11-20", 12, 22, ["alpine", "dusk", "mist", "noon"],
     [(-50.3379, -72.2648), (-53.1638, -70.9171), (-49.3315, -72.8863)], [2, 1], 0.4),
    ("2022/Banff", "2022-08-01", 5, 16, ["alpine", "forest", "dawn", "noon"],
     [(51.1784, -115.5708), (51.0447, -114.0719)], [1, 3], 0.55),
    ("Phone/Weekend Walks", "2025-10-11", 2, 12, ["noon", "dawn", "forest"], [], [3], 0.3),
]


def dms(x):
    x = abs(x)
    d = int(x)
    m = int((x - d) * 60)
    s = round(((x - d) * 60 - m) * 60, 2)
    return (d, m, s)


def make(job):
    path, palette, seed, size, water, taken, gps, camera = job
    render(path, palette, seed, size=size, water=water)
    img = Image.open(path)
    exif = img.getexif()
    make_, model, lens = CAMERAS[camera]
    exif[0x010F] = make_
    exif[0x0110] = model
    ex = exif.get_ifd(0x8769)
    ex[0x9003] = taken.strftime("%Y:%m:%d %H:%M:%S")
    if lens:
        ex[0xA434] = lens
    if gps:
        lat, lon = gps
        g = exif.get_ifd(0x8825)
        g[1] = "N" if lat >= 0 else "S"
        g[2] = dms(lat)
        g[3] = "E" if lon >= 0 else "W"
        g[4] = dms(lon)
    img.save(path, quality=88, exif=exif)
    t = taken.timestamp()
    os.utime(path, (t, t))


def main():
    rng = random.Random(42)
    jobs = []
    n = 0
    for folder, start, days, count, palettes, spots, cams, water_bias in ALBUMS:
        os.makedirs(os.path.join(ROOT, folder), exist_ok=True)
        t0 = datetime.fromisoformat(start) + timedelta(hours=8)
        for i in range(count):
            n += 1
            taken = t0 + timedelta(days=days * i / count, minutes=rng.randint(0, 600))
            portrait = rng.random() < 0.18
            size = (900, 1350) if portrait else (1500, 1000)
            gps = None
            if spots:
                lat, lon = rng.choice(spots)
                gps = (lat + rng.uniform(-0.05, 0.05), lon + rng.uniform(-0.05, 0.05))
            prefix = {0: "IMG_", 1: "DSC0", 2: "DSCF", 3: "PXL_"}[cams[i % len(cams)]]
            name = f"{prefix}{1000 + n:04d}.jpg"
            jobs.append((os.path.join(ROOT, folder, name), rng.choice(palettes), n * 7919,
                         size, rng.random() < water_bias, taken, gps, cams[i % len(cams)]))
    with Pool() as pool:
        pool.map(make, jobs, chunksize=4)
    print(len(jobs), "photos")


if __name__ == "__main__":
    main()
