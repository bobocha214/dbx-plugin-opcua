#!/usr/bin/env python3
"""Trim the transparent margin around the plugin icons and re-center the art.

The DBX host renders `assets/plugin.png` at a fixed box size, so any empty
margin inside the PNG silently shrinks the visible mark. This script crops to
the alpha bounding box and scales the art back up to `fill` of the canvas.

Pure standard library (zlib + struct) - no Pillow required.

Usage:
    python scripts/refit_icons.py [--fill 0.96]
    python scripts/refit_icons.py --check      # report current fill only
"""

import argparse
import struct
import zlib
from pathlib import Path

TARGETS = [Path("assets/plugin.png"), Path("ui/icon.png")]


def read_png(path):
    data = path.read_bytes()
    assert data[:8] == b"\x89PNG\r\n\x1a\n", "not a PNG"
    pos, idat = 8, bytearray()
    w = h = depth = ctype = interlace = None
    while pos < len(data):
        (length,) = struct.unpack(">I", data[pos : pos + 4])
        ctag = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if ctag == b"IHDR":
            w, h, depth, ctype, _, _, interlace = struct.unpack(">IIBBBBB", body)
        elif ctag == b"IDAT":
            idat += body
        pos += 12 + length
    assert depth == 8 and ctype == 6 and interlace == 0, "expect 8-bit RGBA, non-interlaced"
    channels = 4
    stride = w * channels
    raw = zlib.decompress(bytes(idat))
    rows, prev, p = [], bytearray(stride), 0
    for _ in range(h):
        f = raw[p]
        p += 1
        line = bytearray(raw[p : p + stride])
        p += stride
        for x in range(stride):
            a = line[x - channels] if x >= channels else 0
            b = prev[x]
            c = prev[x - channels] if x >= channels else 0
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


def alpha_bbox(w, h, rows):
    minx, miny, maxx, maxy = w, h, -1, -1
    for y in range(h):
        row = rows[y]
        for x in range(w):
            if row[x * 4 + 3] > 8:
                if x < minx:
                    minx = x
                if x > maxx:
                    maxx = x
                if y < miny:
                    miny = y
                if y > maxy:
                    maxy = y
    return minx, miny, maxx, maxy


def resize(rows, w, h, box, dst):
    """Bilinear-resample the cropped `box` region into a dst x dst canvas."""
    x0, y0, x1, y1 = box
    sw, sh = x1 - x0 + 1, y1 - y0 + 1
    out = []
    for j in range(dst):
        sy = (j + 0.5) * sh / dst - 0.5 + y0
        j0 = int(sy)
        fy = sy - j0
        j0 = max(y0, min(y1 - 1, j0))
        j1 = min(y1, j0 + 1)
        line = bytearray(dst * 4)
        for i in range(dst):
            sx = (i + 0.5) * sw / dst - 0.5 + x0
            i0 = int(sx)
            fx = sx - i0
            i0 = max(x0, min(x1 - 1, i0))
            i1 = min(x1, i0 + 1)
            r0, r1 = rows[j0], rows[j1]
            for c in range(4):
                p00 = r0[i0 * 4 + c]
                p10 = r0[i1 * 4 + c]
                p01 = r1[i0 * 4 + c]
                p11 = r1[i1 * 4 + c]
                top = p00 + (p10 - p00) * fx
                bot = p01 + (p11 - p01) * fx
                line[i * 4 + c] = int(top + (bot - top) * fy) & 0xFF
        out.append(bytes(line))
    return out


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
    path.write_bytes(png)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--fill", type=float, default=0.96, help="art size / canvas size")
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    for path in TARGETS:
        w, h, rows = read_png(path)
        box = alpha_bbox(w, h, rows)
        fw = (box[2] - box[0] + 1) / w
        fh = (box[3] - box[1] + 1) / h
        if args.check:
            print(f"{path} {w}x{h} fill {fw:.0%}x{fh:.0%}")
            continue
        art = int(round(w * args.fill))
        small = resize(rows, w, h, box, art)
        off = (w - art) // 2
        canvas = []
        for j in range(w):
            line = bytearray(w * 4)
            if off <= j < off + art:
                line[off * 4 : (off + art) * 4] = small[j - off]
            canvas.append(bytes(line))
        write_png(path, w, canvas)
        w2, h2, rows2 = read_png(path)
        b2 = alpha_bbox(w2, h2, rows2)
        print(
            f"{path} {w}x{h} fill {fw:.0%}x{fh:.0%} -> "
            f"{(b2[2]-b2[0]+1)/w2:.0%}x{(b2[3]-b2[1]+1)/h2:.0%} "
            f"margins L{b2[0]} R{w2-1-b2[2]} T{b2[1]} B{h2-1-b2[3]}"
        )


if __name__ == "__main__":
    main()
