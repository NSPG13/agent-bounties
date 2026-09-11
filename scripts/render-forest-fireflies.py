#!/usr/bin/env python3
"""Bake a deterministic firefly pass into a six-second RGB24 video stream.

Usage: ffmpeg ... -pix_fmt rgb24 -f rawvideo - |
       python3 scripts/render-forest-fireflies.py |
       ffmpeg -f rawvideo -pixel_format rgb24 -video_size 3840x2160 -framerate 30 ...

This is offline video postproduction. It adds no browser animation or dependency.
Requires NumPy. The source AI generation is preserved separately.
"""
import math
import random
import sys

import numpy as np

WIDTH, HEIGHT, FPS, SECONDS = 3840, 2160, 30, 6
TAU = 2 * math.pi
rng = random.Random(20260910)
flies = []
for index in range(64):
    depth = rng.uniform(.16, .55) if index < 44 else rng.uniform(.65, 1)
    # Keep the central conversation and headline free of large foreground lights.
    x = rng.uniform(.035, .965)
    y = rng.uniform(.14, .94)
    if .29 < x < .76 and y > .55:
        x = rng.choice((rng.uniform(.035, .26), rng.uniform(.79, .965)))
    if index >= 44:
        y = rng.uniform(.56, .95)
        x = rng.choice((rng.uniform(.03, .25), rng.uniform(.8, .97)))
    core = 1.1 + depth * 2.0
    halo = 3.5 + depth * 8.5
    radius = math.ceil(halo * 3)
    yy, xx = np.mgrid[-radius:radius + 1, -radius:radius + 1]
    distance = xx * xx + yy * yy
    # Small luminous abdomen, soft green glow, and restrained distant bloom.
    inner = np.exp(-distance / (2 * core * core))
    outer = np.exp(-distance / (2 * halo * halo))
    alpha = np.clip(inner * .84 + outer * .11, 0, .96)[..., None]
    tint = np.array([rng.uniform(100, 145), 255, rng.uniform(68, 120)], dtype=np.float32)
    white = np.array([205, 255, 172], dtype=np.float32)
    color = tint + inner[..., None] * (white - tint)
    pulses = [(rng.uniform(0, SECONDS), rng.uniform(.65, 1.35), rng.uniform(.72, 1))]
    if rng.random() < .4:
        pulses.append(((pulses[0][0] + rng.uniform(2, 4)) % SECONDS, rng.uniform(.4, .8), rng.uniform(.45, .75)))
    flies.append({
        "x": x * WIDTH, "y": y * HEIGHT, "depth": depth,
        "travel": rng.uniform(48, 104) * (.65 + depth),
        "phases": [rng.uniform(0, TAU) for _ in range(6)],
        "alpha": alpha.astype(np.float32), "color": color.astype(np.float32),
        "radius": radius, "pulses": pulses,
    })


def brightness(t, pulses):
    level = 0.0
    for center, duration, peak in pulses:
        distance = (t - center + SECONDS / 2) % SECONDS - SECONDS / 2
        if abs(distance) < duration / 2:
            level += peak * (.5 + .5 * math.cos(TAU * distance / duration)) ** 1.5
    return min(1, level)


frame_bytes = WIDTH * HEIGHT * 3
for frame_number in range(FPS * SECONDS):
    raw = sys.stdin.buffer.read(frame_bytes)
    if len(raw) != frame_bytes:
        raise SystemExit(f"Truncated frame {frame_number}: {len(raw)} of {frame_bytes} bytes")
    frame = np.frombuffer(raw, dtype=np.uint8).reshape(HEIGHT, WIDTH, 3).copy()
    t = frame_number / FPS
    angle = TAU * t / SECONDS
    for fly in flies:
        glow = brightness(t, fly["pulses"])
        if glow < .0005:
            continue
        a, b, c, d, e, f = fly["phases"]
        # Flight depends only on time: dark insects continue along their paths.
        # Closed smooth curves keep position, velocity and glow continuous at wrap.
        x = fly["x"] + fly["travel"] * (.55 * math.sin(angle + a) + .27 * math.sin(2 * angle + b) + .1 * math.sin(3 * angle + c))
        y = fly["y"] + fly["travel"] * (.39 * math.cos(angle + d) + .19 * math.sin(2 * angle + e) + .08 * math.sin(3 * angle + f))
        r = fly["radius"]
        left, top = round(x) - r, round(y) - r
        right, bottom = left + 2 * r + 1, top + 2 * r + 1
        x0, y0, x1, y1 = max(0, left), max(0, top), min(WIDTH, right), min(HEIGHT, bottom)
        if x0 >= x1 or y0 >= y1:
            continue
        sy, sx = slice(y0 - top, y1 - top), slice(x0 - left, x1 - left)
        alpha = fly["alpha"][sy, sx] * glow
        patch = frame[y0:y1, x0:x1]
        patch[:] = np.rint(patch * (1 - alpha) + fly["color"][sy, sx] * alpha).astype(np.uint8)
    sys.stdout.buffer.write(frame.tobytes())
if sys.stdin.buffer.read(1):
    raise SystemExit("Input contains more than the expected 180 frames.")
