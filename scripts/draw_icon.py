#!/usr/bin/env python3
"""Draw the plugin icon: gradient squircle plate + white OPC / UA wordmark.

Style target: the sibling dbx-plugin-modbus icon - a squircle plate filled with
a vertical brand gradient, carrying the protocol's own wordmark large and
centred. Filling the plate edge to edge is what makes the icon read as "solid"
at 16px; the earlier near-white plate with a thin wordmark looked undersized.

Everything is signed-distance rasterised at 4x and boxed down, so the output is
anti-aliased without Pillow or a font engine.

Usage:
    python scripts/draw_icon.py                       # write assets/plugin.png + ui/icon.png
    python scripts/draw_icon.py --preview out.png --size 256
    python scripts/draw_icon.py --style ua --preview out.png
"""

import argparse
import struct
import zlib
from pathlib import Path

# Plate: OPC UA brand blue, light at the top like the modbus gold gradient.
PLATE_TOP = (58, 143, 224)  # #3a8fe0
PLATE_BOT = (9, 51, 122)  # #09337a
FG = (255, 255, 255)

PLATE_N = 4.2  # squircle exponent -> ~22% corner radius

# --- style "opcua": two rows, OPC over UA (the OPC Foundation lockup) -------
OPC_TOP, OPC_BOT = 0.175, 0.475
OPC_L, OPC_W, OPC_GAP, OPC_T = 0.175, 0.19, 0.035, 0.078
UA_TOP, UA_BOT = 0.545, 0.825
UA_L, UA_W, UA_GAP, UA_T = 0.205, 0.235, 0.06, 0.078

# --- style "ua": single bold UA pair ---------------------------------------
PAIR_TOP, PAIR_BOT = 0.24, 0.76
PAIR_L, PAIR_W, PAIR_GAP, PAIR_T = 0.15, 0.30, 0.10, 0.125
PAIR_BG = (231, 238, 251)
PAIR_FG = (11, 61, 145)


def squircle_sd(x, y, half=0.5, n=PLATE_N):
    """Approximate signed distance to |x/half|^n + |y/half|^n = 1."""
    ax, ay = abs(x) / half, abs(y) / half
    f = ax**n + ay**n - 1.0
    if ax < 1e-9 and ay < 1e-9:
        return -1.0
    gx = n * (ax ** (n - 1)) / half * (1.0 if x >= 0 else -1.0)
    gy = n * (ay ** (n - 1)) / half * (1.0 if y >= 0 else -1.0)
    return f / max((gx * gx + gy * gy) ** 0.5, 1e-9)


def capsule_sd(px, py, ax, ay, bx, by, r):
    dx, dy = bx - ax, by - ay
    t = ((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy)
    t = 0.0 if t < 0.0 else (1.0 if t > 1.0 else t)
    return ((px - ax - t * dx) ** 2 + (py - ay - t * dy) ** 2) ** 0.5 - r


def ring_sd(px, py, cx, cy, r, thick):
    return abs(((px - cx) ** 2 + (py - cy) ** 2) ** 0.5 - r) - thick / 2.0


def box_sd(px, py, cx, cy, hw, hh):
    return max(abs(px - cx) - hw, abs(py - cy) - hh)


def u_sd(x, y, left, w, top, bot, t):
    half = t / 2.0
    r = (w - t) / 2.0
    cy = bot - r
    res = min(
        capsule_sd(x, y, left + half, top, left + half, cy, half),
        capsule_sd(x, y, left + w - half, top, left + w - half, cy, half),
    )
    return min(res, max(ring_sd(x, y, left + w / 2.0, cy, r, t), cy - y))


def a_sd(x, y, left, w, top, bot, t):
    half = t / 2.0
    apex_x, apex_y = left + w / 2.0, top
    res = min(
        capsule_sd(x, y, apex_x, apex_y, left, bot, half),
        capsule_sd(x, y, apex_x, apex_y, left + w, bot, half),
    )
    bar_y = top + 0.60 * (bot - top)
    inset = (w / 2.0) * (bar_y - top) / (bot - top)
    return min(res, capsule_sd(x, y, left + inset, bar_y, left + w - inset, bar_y, half * 0.92))


def rbox_sd(px, py, cx, cy, hw, hh, r):
    qx = abs(px - cx) - (hw - r)
    qy = abs(py - cy) - (hh - r)
    return (max(qx, 0.0) ** 2 + max(qy, 0.0) ** 2) ** 0.5 + min(max(qx, qy), 0.0) - r


def rbox_ring_sd(px, py, cx, cy, hw, hh, r, t):
    """Outline stroke of a rounded box - the backbone of O / C / P / U."""
    return abs(rbox_sd(px, py, cx, cy, hw, hh, r) + t / 2.0) - t / 2.0


def o_sd(x, y, left, w, top, bot, t):
    cx, cy = left + w / 2.0, (top + bot) / 2.0
    hw, hh = w / 2.0, (bot - top) / 2.0
    return rbox_ring_sd(x, y, cx, cy, hw, hh, min(hw, hh) * 0.62, t)


def c_sd(x, y, left, w, top, bot, t):
    cx, cy = left + w / 2.0, (top + bot) / 2.0
    hw, hh = w / 2.0, (bot - top) / 2.0
    ring = rbox_ring_sd(x, y, cx, cy, hw, hh, min(hw, hh) * 0.62, t)
    notch = box_sd(x, y, cx + hw * 1.02, cy, hw * 0.95, hh * 0.60)
    return max(ring, -notch)


def p_sd(x, y, left, w, top, bot, t):
    half = t / 2.0
    stem = capsule_sd(x, y, left + half, top, left + half, bot, half)
    bowl_h = (bot - top) * 0.60
    cx, cy = left + w / 2.0, top + bowl_h / 2.0
    hw, hh = w / 2.0, bowl_h / 2.0
    bowl = rbox_ring_sd(x, y, cx, cy, hw, hh, min(hw, hh) * 0.62, t)
    bowl = max(bowl, y - (top + bowl_h))  # keep only the upper half
    return min(stem, bowl)


def glyph_sd(x, y, style="opcua"):
    if style == "ua":
        return min(
            u_sd(x, y, PAIR_L, PAIR_W, PAIR_TOP, PAIR_BOT, PAIR_T),
            a_sd(x, y, PAIR_L + PAIR_W + PAIR_GAP, PAIR_W, PAIR_TOP, PAIR_BOT, PAIR_T),
        )
    opc = min(
        o_sd(x, y, OPC_L, OPC_W, OPC_TOP, OPC_BOT, OPC_T),
        p_sd(x, y, OPC_L + OPC_W + OPC_GAP, OPC_W, OPC_TOP, OPC_BOT, OPC_T),
        c_sd(x, y, OPC_L + 2 * (OPC_W + OPC_GAP), OPC_W, OPC_TOP, OPC_BOT, OPC_T),
    )
    ua = min(
        u_sd(x, y, UA_L, UA_W, UA_TOP, UA_BOT, UA_T),
        a_sd(x, y, UA_L + UA_W + UA_GAP, UA_W, UA_TOP, UA_BOT, UA_T),
    )
    return min(opc, ua)


def plate_color(y, style="opcua"):
    if style == "ua":
        return PAIR_BG
    t = 0.0 if y < -0.5 else (1.0 if y > 0.5 else y + 0.5)
    return (
        PLATE_TOP[0] + (PLATE_BOT[0] - PLATE_TOP[0]) * t,
        PLATE_TOP[1] + (PLATE_BOT[1] - PLATE_TOP[1]) * t,
        PLATE_TOP[2] + (PLATE_BOT[2] - PLATE_TOP[2]) * t,
    )


def render(size, style="opcua", ss=4):
    n = size * ss
    px = 1.0 / n
    fg = PAIR_FG if style == "ua" else FG
    acc = [[0.0, 0.0, 0.0, 0.0] for _ in range(size * size)]
    for j in range(n):
        y = (j + 0.5) / n - 0.5
        row_base = (j // ss) * size
        br, bg_, bb = plate_color(y, style)
        for i in range(n):
            x = (i + 0.5) / n - 0.5
            a_bg = 0.5 - squircle_sd(x, y) / px
            if a_bg <= 0.0:
                continue
            if a_bg > 1.0:
                a_bg = 1.0
            a_g = 0.5 - glyph_sd(x + 0.5, y + 0.5, style) / px
            a_g = 0.0 if a_g < 0.0 else (a_g if a_g < 1.0 else 1.0)
            slot = acc[row_base + (i // ss)]
            r = br + (fg[0] - br) * a_g
            g = bg_ + (fg[1] - bg_) * a_g
            b = bb + (fg[2] - bb) * a_g
            slot[0] += r * a_bg
            slot[1] += g * a_bg
            slot[2] += b * a_bg
            slot[3] += a_bg
    area = float(ss * ss)
    out = []
    for row in range(size):
        line = bytearray(4 * size)
        for col in range(size):
            r, g, b, a = acc[row * size + col]
            if a <= 0.0:
                continue
            inv = 1.0 / a
            o = col * 4
            line[o + 0] = clamp8(r * inv)
            line[o + 1] = clamp8(g * inv)
            line[o + 2] = clamp8(b * inv)
            line[o + 3] = clamp8(a * (255.0 / area))
        out.append(bytes(line))
    return out


def clamp8(v):
    iv = int(v + 0.5)
    return 0 if iv < 0 else (255 if iv > 255 else iv)


def write_png(path, size, rows):
    def chunk(tag, body):
        return (
            struct.pack(">I", len(body))
            + tag
            + body
            + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)
        )

    raw = b"".join(b"\x00" + r for r in rows)
    png = b"\x89PNG\r\n\x1a\n"
    png += chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
    png += chunk(b"IDAT", zlib.compress(raw, 9))
    png += chunk(b"IEND", b"")
    Path(path).write_bytes(png)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--preview", type=str, default=None)
    ap.add_argument("--size", type=int, default=256)
    ap.add_argument("--ss", type=int, default=4)
    ap.add_argument("--style", choices=("opcua", "ua"), default="opcua")
    args = ap.parse_args()

    if args.preview:
        write_png(args.preview, args.size, render(args.size, args.style, args.ss))
        print("preview ->", args.preview)
        return

    for path, size in ((Path("assets/plugin.png"), 256), (Path("ui/icon.png"), 128)):
        write_png(path, size, render(size, args.style, args.ss))
        print(f"wrote {path} {size}x{size} (style={args.style})")


if __name__ == "__main__":
    main()
