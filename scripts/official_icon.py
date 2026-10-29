#!/usr/bin/env python3
"""Render the plugin icon from assets/opcua-official.svg (the official wordmark).

The SVG is a 512x512 canvas whose only content is a centred <image> holding the
OPC UA wordmark as a base64 PNG (448x100, transparent). This script decodes it,
places it back at the geometry the SVG declares, and composites it onto a plate
so the mark stays visible on both light and dark host chrome - the official art
is dark ink on transparency, which disappears on a dark sidebar.

Everything is standard library: PNG decode, bilinear resample, box-blur shadow.

Usage:
    python scripts/official_icon.py                      # write assets/plugin.png + ui/icon.png
    python scripts/official_icon.py --preview p.png --size 256
    python scripts/official_icon.py --plate none         # transparent, faithful to the SVG
    python scripts/official_icon.py --plate blue         # blue gradient plate, white mark
"""

import argparse
import base64
import re
import struct
import zlib
from pathlib import Path

SVG = Path("assets/opcua-official.svg")

# Plate gradients (top -> bottom).
PLATES = {
    "light": ((250, 252, 255), (206, 224, 246)),  # soft blue wash, keeps the official ink/colours
    "blue": ((58, 143, 224), (9, 51, 122)),  # mark is recoloured white for this one
    "none": None,
}
SHADOW = (32, 51, 74, 0.22)  # #20334A @ 22% - the SVG's feDropShadow
PLATE_N = 4.2  # squircle exponent -> ~22% corner radius


# ---------------------------------------------------------------- PNG decode
def read_png(data: bytes):
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, idat = 8, bytearray()
    w = h = depth = ctype = None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        tag = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if tag == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        elif tag == b"IDAT":
            idat += body
        pos += 12 + length
    assert depth == 8 and ctype == 6, f"expect 8-bit RGBA, got depth={depth} type={ctype}"
    stride, ch = w * 4, 4
    raw = zlib.decompress(bytes(idat))
    rows, prev, p = [], bytearray(stride), 0
    for _ in range(h):
        f = raw[p]
        p += 1
        line = bytearray(raw[p : p + stride])
        p += stride
        for x in range(stride):
            a = line[x - ch] if x >= ch else 0
            b = prev[x]
            c = prev[x - ch] if x >= ch else 0
            if f == 1:
                line[x] = (line[x] + a) & 0xFF
            elif f == 2:
                line[x] = (line[x] + b) & 0xFF
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 0xFF
            elif f == 4:
                pa, pb, pc = abs(b - c), abs(a - c), abs(a + b - 2 * c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[x] = (line[x] + pred) & 0xFF
        rows.append(bytes(line))
        prev = line
    return w, h, rows


# ------------------------------------------------------------- SVG geometry
def svg_layout(text: str):
    vb = re.search(r'viewBox="([\d.\s-]+)"', text)
    nums = [float(x) for x in vb.group(1).replace(",", " ").split()]
    vw, vh = (nums[2], nums[3]) if len(nums) >= 4 else (512.0, 512.0)
    img = re.search(r"<image[^>]*>", text).group(0)

    def num(attr):
        m = re.search(attr + r'="([\d.]+)"', img)
        return float(m.group(1))

    x, y, w, h = num("x"), num("y"), num("width"), num("height")
    mark = svg_embedded_png(text)
    mw, mh = mark[0], mark[1]
    # preserveAspectRatio="xMidYMid meet"
    s = min(w / mw, h / mh)
    dw, dh = mw * s, mh * s
    return vw, vh, (x + (w - dw) / 2.0, y + (h - dh) / 2.0, dw, dh), mark


def svg_embedded_png(text: str):
    m = re.search(r'href="data:image/png;base64,([A-Za-z0-9+/=\s]+)"', text)
    if not m:
        raise SystemExit("no embedded PNG found in the SVG")
    return read_png(base64.b64decode(re.sub(r"\s", "", m.group(1))))


# ------------------------------------------------------------------ helpers
def squircle_sd(x, y, half=0.5, n=PLATE_N):
    ax, ay = abs(x) / half, abs(y) / half
    f = ax**n + ay**n - 1.0
    if ax < 1e-9 and ay < 1e-9:
        return -1.0
    gx = n * (ax ** (n - 1)) / half * (1.0 if x >= 0 else -1.0)
    gy = n * (ay ** (n - 1)) / half * (1.0 if y >= 0 else -1.0)
    return f / max((gx * gx + gy * gy) ** 0.5, 1e-9)


def blur(a, size, radius, passes=3):
    """Separable box blur, 3 passes ~= gaussian."""
    cur = a
    for _ in range(passes):
        tmp = [0.0] * (size * size)
        for j in range(size):
            row = j * size
            for i in range(size):
                s = 0.0
                for k in range(-radius, radius + 1):
                    s += cur[row + min(max(i + k, 0), size - 1)]
                tmp[row + i] = s / (2 * radius + 1)
        nxt = [0.0] * (size * size)
        for i in range(size):
            for j in range(size):
                s = 0.0
                for k in range(-radius, radius + 1):
                    s += tmp[min(max(j + k, 0), size - 1) * size + i]
                nxt[j * size + i] = s / (2 * radius + 1)
        cur = nxt
    return cur


def sample_mark(mark, u, v):
    """Bilinear sample of the mark in normalised [0,1] coords, premultiplied."""
    w, h, rows = mark
    if u < 0.0 or u > 1.0 or v < 0.0 or v > 1.0:
        return 0.0, 0.0, 0.0, 0.0
    x = u * w - 0.5
    y = v * h - 0.5
    i0 = int(x)
    j0 = int(y)
    fx, fy = x - i0, y - j0
    i0 = min(max(i0, 0), w - 1)
    j0 = min(max(j0, 0), h - 1)
    i1, j1 = min(i0 + 1, w - 1), min(j0 + 1, h - 1)
    r0, r1 = rows[j0], rows[j1]
    out = []
    for c in range(4):
        p00 = r0[i0 * 4 + c]
        p10 = r0[i1 * 4 + c]
        p01 = r1[i0 * 4 + c]
        p11 = r1[i1 * 4 + c]
        top = p00 + (p10 - p00) * fx
        bot = p01 + (p11 - p01) * fx
        out.append(top + (bot - top) * fy)
    a = out[3] / 255.0
    return out[0] * a, out[1] * a, out[2] * a, a


def render(size, plate="light"):
    vw, vh, (mx, my, mw, mh), mark = svg_layout(SVG.read_text(encoding="utf-8"))
    # mark rect in normalised canvas coords
    rx, ry, rw, rh = mx / vw, my / vh, mw / vw, mh / vh

    plate_top, plate_bot = (None, None) if plate == "none" else PLATES[plate]
    white_mark = plate == "blue"

    # shadow: rasterise the mark alpha once, blur it, offset down
    aa = [0.0] * (size * size)
    for j in range(size):
        v = (j + 0.5) / size
        if ry <= v <= ry + rh:
            for i in range(size):
                u = (i + 0.5) / size
                if rx <= u <= rx + rw:
                    aa[j * size + i] = sample_mark(mark, (u - rx) / rw, (v - ry) / rh)[3]
    shadow = blur(aa, size, max(1, round(size * 0.012)), passes=3)
    dy = max(1, round(size * 0.011))
    sr, sg, sb, sa = SHADOW
    sr, sg, sb = sr * sa, sg * sa, sb * sa

    rows_out = []
    ss = 3  # sub-samples per axis; the mark is downscaled ~2x, single taps alias
    for j in range(size):
        line = bytearray(4 * size)
        for i in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for sj in range(ss):
                fy = (sj + 0.5) / ss
                y = (j + fy) / size - 0.5
                v = (j + fy) / size
                for si in range(ss):
                    fx = (si + 0.5) / ss
                    x = (i + fx) / size - 0.5
                    u = (i + fx) / size
                    pr = pg = pb = 0.0
                    # plate
                    if plate != "none":
                        t = 0.0 if y < -0.5 else (1.0 if y > 0.5 else y + 0.5)
                        r = plate_top[0] + (plate_bot[0] - plate_top[0]) * t
                        g = plate_top[1] + (plate_bot[1] - plate_top[1]) * t
                        b = plate_top[2] + (plate_bot[2] - plate_top[2]) * t
                        cov = 0.5 - squircle_sd(x, y) * size
                        cov = 0.0 if cov < 0.0 else (1.0 if cov > 1.0 else cov)
                        pr, pg, pb, a_dst = r * cov, g * cov, b * cov, cov
                    else:
                        a_dst = 0.0
                    # shadow sits under the mark
                    sj2 = j - dy
                    if 0 <= sj2 < size:
                        sv = shadow[sj2 * size + i]
                        if sv > 0.0:
                            k = 1.0 - sv * sa
                            pr = sr * sv + pr * k
                            pg = sg * sv + pg * k
                            pb = sb * sv + pb * k
                            a_dst = sv * sa + a_dst * k
                    # mark
                    if rx <= u <= rx + rw and ry <= v <= ry + rh:
                        mr, mg, mb, ma = sample_mark(mark, (u - rx) / rw, (v - ry) / rh)
                        if ma > 0.0:
                            if white_mark:
                                mr = mg = mb = ma
                            k = 1.0 - ma
                            pr = mr * 255.0 + pr * k
                            pg = mg * 255.0 + pg * k
                            pb = mb * 255.0 + pb * k
                            a_dst = ma + a_dst * k
                    acc[0] += pr
                    acc[1] += pg
                    acc[2] += pb
                    acc[3] += a_dst
            inv_area = 1.0 / (ss * ss)
            pr, pg, pb, a_dst = (c * inv_area for c in acc)
            o = i * 4
            if a_dst <= 0.0:
                continue
            inv = 1.0 / a_dst
            line[o + 0] = clamp8(pr * inv)
            line[o + 1] = clamp8(pg * inv)
            line[o + 2] = clamp8(pb * inv)
            line[o + 3] = clamp8(a_dst * 255.0)
        rows_out.append(bytes(line))
    return rows_out


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
    ap.add_argument("--plate", choices=tuple(PLATES), default="light")
    args = ap.parse_args()

    if args.preview:
        write_png(args.preview, args.size, render(args.size, args.plate))
        print(f"preview -> {args.preview} (plate={args.plate})")
        return

    for path, size in ((Path("assets/plugin.png"), 256), (Path("ui/icon.png"), 128)):
        write_png(path, size, render(size, args.plate))
        print(f"wrote {path} {size}x{size} (plate={args.plate})")


if __name__ == "__main__":
    main()
