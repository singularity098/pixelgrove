"""Procedural landscape 'photos' for README screenshots: layered ridges with
atmospheric perspective, sky gradients, sun glow, clouds, water reflections,
pine silhouettes, auroras and film grain."""
import math
import random

import numpy as np
from PIL import Image, ImageFilter

PALETTES = {
    # sky top, sky horizon, sun colour, far ridge, near ridge
    "dawn": ((70, 90, 150), (250, 190, 170), (255, 236, 200), (150, 130, 170), (40, 35, 60)),
    "sakura": ((120, 150, 205), (250, 215, 220), (255, 245, 235), (190, 160, 190), (70, 50, 80)),
    "noon": ((40, 110, 200), (175, 215, 240), (255, 255, 240), (120, 150, 185), (35, 70, 60)),
    "alpine": ((30, 90, 180), (200, 225, 245), (255, 255, 245), (225, 235, 245), (60, 80, 100)),
    "dusk": ((40, 30, 90), (250, 120, 70), (255, 210, 120), (150, 70, 90), (30, 15, 35)),
    "desert": ((90, 140, 200), (250, 205, 150), (255, 240, 200), (215, 140, 90), (120, 55, 35)),
    "coast": ((60, 120, 190), (250, 170, 120), (255, 225, 170), (90, 110, 150), (20, 40, 60)),
    "night": ((5, 10, 30), (20, 50, 70), (230, 240, 255), (30, 50, 70), (5, 10, 20)),
    "forest": ((100, 150, 190), (215, 225, 210), (255, 250, 230), (110, 140, 130), (20, 45, 35)),
    "mist": ((140, 160, 180), (220, 225, 230), (250, 250, 245), (165, 180, 190), (60, 75, 85)),
}


def lerp(a, b, t):
    a, b = np.asarray(a, float), np.asarray(b, float)
    return a + (b - a) * np.asarray(t, float)[..., None]


def ridge(width, rng, roughness=0.55, octaves=8):
    """1-D fractal noise, normalised to 0..1."""
    xs = np.linspace(0, 1, width)
    y = np.zeros(width)
    amp, freq = 1.0, 2.0
    for _ in range(octaves):
        phase = rng.uniform(0, 100)
        knots = rng.uniform(-1, 1, int(freq) + 3)
        pos = xs * freq + phase % 1
        i = np.floor(pos).astype(int)
        f = pos - i
        f = f * f * (3 - 2 * f)
        y += amp * (knots[i] * (1 - f) + knots[i + 1] * f)
        amp *= roughness
        freq *= 2.0
    return (y - y.min()) / (np.ptp(y) + 1e-9)


def render(path, palette, seed, size=(1600, 1067), water=None, trees=None, aurora=None, sun=None,
           layers=None, peaks=None):
    rng = np.random.default_rng(seed)
    r = random.Random(seed)
    w, h = size
    sky_top, horizon, sun_c, far_c, near_c = PALETTES[palette]
    horizon_y = r.uniform(0.52, 0.66) if water is None else r.uniform(0.5, 0.58)
    water = r.random() < 0.35 if water is None else water
    trees = r.random() < 0.3 if trees is None else trees
    aurora = palette == "night" if aurora is None else aurora
    layers = layers or r.randint(3, 5)
    peaks = r.uniform(0.18, 0.42) if peaks is None else peaks

    ys = np.linspace(0, 1, h)[:, None]
    t = np.clip(ys / horizon_y, 0, 1) ** 1.3
    img = lerp(sky_top, horizon, np.broadcast_to(t, (h, w)))

    # Sun or moon with glow.
    if sun is None:
        sun = r.random() < 0.8
    if sun:
        sx, sy = r.uniform(0.2, 0.8) * w, r.uniform(0.25, 0.85) * horizon_y * h
        yy, xx = np.mgrid[0:h, 0:w]
        d = np.hypot(xx - sx, yy - sy) / w
        glow = np.exp(-d * (9 if palette != "night" else 25)) * (0.75 if palette in ("dusk", "coast", "dawn", "desert") else 0.4)
        img = img + (np.asarray(sun_c, float) - img) * glow[..., None]
        disc = np.clip((0.022 - d) * 400, 0, 1)
        img = img + (np.asarray(sun_c, float) - img) * disc[..., None]

    if palette == "night":
        stars = rng.random((h, w)) > 0.9985
        stars &= (ys < horizon_y)
        img[stars] = [240, 245, 255]

    if aurora:
        yy, xx = np.mgrid[0:h, 0:w]
        curve = 0.25 + 0.12 * np.sin(xx / w * r.uniform(4, 7) + r.uniform(0, 6)) + 0.05 * np.sin(xx / w * 13)
        band = np.exp(-((yy / h - curve * horizon_y * 1.6) ** 2) / 0.004)
        rays = 0.6 + 0.4 * np.sin(xx / w * 140 + np.sin(xx / w * 9) * 6)
        a = (band * rays)[..., None]
        img = img + (np.array([80, 255, 170]) - img) * a * 0.75
        band2 = np.exp(-((yy / h - (curve - 0.1) * horizon_y * 1.6) ** 2) / 0.002) * rays
        img = img + (np.array([170, 90, 255]) - img) * band2[..., None] * 0.35

    # Soft clouds.
    if palette not in ("night",) and r.random() < 0.75:
        cloud = Image.new("L", (w, h), 0)
        from PIL import ImageDraw
        dr = ImageDraw.Draw(cloud)
        for _ in range(r.randint(6, 16)):
            cx, cy = r.uniform(-0.1, 1.1) * w, r.uniform(0.05, 0.75) * horizon_y * h
            for _ in range(r.randint(3, 7)):
                rx, ry = r.uniform(40, 160), r.uniform(10, 35)
                ox, oy = r.uniform(-120, 120), r.uniform(-15, 15)
                dr.ellipse([cx + ox - rx, cy + oy - ry, cx + ox + rx, cy + oy + ry], fill=r.randint(90, 170))
        cloud = np.asarray(cloud.filter(ImageFilter.GaussianBlur(18)), float) / 255
        tint = np.asarray(lerp(horizon, (255, 255, 255), 0.5))
        img = img + (tint - img) * cloud[..., None] * 0.8

    # Ridges, far to near.
    land_top = horizon_y
    xs = np.arange(w)
    for i in range(layers):
        depth = i / max(layers - 1, 1)
        base = horizon_y * h + depth * (0.16 * h)
        height = peaks * h * (1 - 0.45 * depth) * r.uniform(0.7, 1.15)
        rough = r.uniform(0.45, 0.6) + (0.08 if palette == "alpine" else 0)
        line = base - ridge(w, rng, rough) * height
        if palette == "desert" and i < layers - 1:
            line = base - (ridge(w, rng, 0.35, 4) ** 0.7) * height * 0.5
        color = lerp(far_c, near_c, depth ** 0.9)
        haze = np.asarray(lerp(color, horizon, 0.35 * (1 - depth)))
        mask = ys >= line[None, :] / h
        # Snow caps on alpine far ridges.
        shade = np.broadcast_to(haze, img.shape).copy()
        if palette == "alpine" and depth < 0.7:
            snow = ys < (line[None, :] + height * 0.35) / h
            shade[snow & mask] = lerp(shade[snow & mask], (250, 252, 255), 0.75)
        # Vertical gradient within the layer.
        grad = np.clip((ys - line[None, :] / h) * 3, 0, 0.25)
        shade = shade * (1 - grad[..., None] * 0.6)
        img = np.where(mask[..., None], shade, img)
        if trees and i == layers - 1:
            from PIL import ImageDraw
            tl = Image.new("L", (w, h), 0)
            td = ImageDraw.Draw(tl)
            x = 0
            while x < w:
                th = r.uniform(18, 75) * (0.5 + depth) * (h / 1067)
                tw = th * r.uniform(0.28, 0.4)
                by = line[min(int(x), w - 1)] + r.uniform(5, 25)
                td.polygon([(x, by - th), (x - tw, by), (x + tw, by)], fill=255)
                x += r.uniform(3, 45) * (w / 1600) + (r.random() < 0.15) * r.uniform(60, 200)
            tm = np.asarray(tl, float)[..., None] / 255
            img = img * (1 - tm) + np.asarray(near_c, float) * 0.8 * tm
        land_top = min(land_top, line.min() / h)

    # Water: reflect what's above the waterline.
    if water:
        wl = int(h * r.uniform(0.66, 0.78))
        above = img[max(0, 2 * wl - h):wl][::-1]
        refl = np.zeros_like(img[wl:])
        refl[: above.shape[0]] = above
        refl = np.asarray(Image.fromarray(np.clip(refl, 0, 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(3)), float)
        ripple = (np.sin(np.arange(h - wl)[:, None] ** 1.35 * 0.9 + np.linspace(0, 40, w)[None, :] * 0.2) * 0.5 + 0.5)
        refl = refl * (0.72 + 0.12 * ripple[..., None])
        refl = lerp(refl, np.asarray(sky_top) * 0.5, 0.15)
        img[wl:] = refl

    # Vignette and grain.
    yy, xx = np.mgrid[0:h, 0:w]
    v = 1 - 0.28 * (((xx / w - 0.5) ** 2 + (yy / h - 0.5) ** 2) * 2)
    img = img * v[..., None]
    img = img + rng.normal(0, 3.2, img.shape)
    Image.fromarray(np.clip(img, 0, 255).astype(np.uint8)).save(path, quality=90)


if __name__ == "__main__":
    import sys
    for i, p in enumerate(PALETTES):
        render(f"{sys.argv[1]}/{p}.jpg", p, i + 1, size=(800, 533))
