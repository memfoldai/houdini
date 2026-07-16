#!/usr/bin/env python3
"""Generate the app-icon master PNG (packaging/appicon-1024.png).

This is the Finder/Dock/DMG icon — colorful, unlike the monochrome menu-bar
template glyph, but sharing its "aperture" motif (a ring with a center dot) so
the app reads as one thing. Standard library only (no Pillow): the PNG is
encoded by hand with zlib, so this runs on any Python 3.

Run once when the design changes; bundle.sh consumes the committed PNG:
    python3 packaging/make_appicon.py
"""

import math
import struct
import zlib
from pathlib import Path

SIZE = 1024
SS = 2  # supersampling per axis for smooth edges

# Rounded-square (superellipse) container, Apple-ish squircle exponent.
SQUIRCLE_N = 5.0
SQUIRCLE_A = SIZE * 0.46  # half-extent → ~92% of the canvas

# Aperture glyph, as fractions of SIZE.
RING_OUTER = SIZE * 0.27
RING_WIDTH = SIZE * 0.058
DOT_RADIUS = SIZE * 0.082

# Vertical gradient (top → bottom), indigo → violet.
TOP = (79, 70, 229)
BOTTOM = (139, 92, 246)
GLYPH = (255, 255, 255)


def lerp(a, b, t):
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def in_squircle(dx, dy):
    return (abs(dx) / SQUIRCLE_A) ** SQUIRCLE_N + (abs(dy) / SQUIRCLE_A) ** SQUIRCLE_N <= 1.0


def in_glyph(dx, dy):
    d = math.hypot(dx, dy)
    on_ring = RING_OUTER - RING_WIDTH <= d <= RING_OUTER
    return on_ring or d <= DOT_RADIUS


def sample(px, py, c):
    """Coverage of squircle and glyph at a subpixel point."""
    dx, dy = px - c, py - c
    sq = in_squircle(dx, dy)
    gl = sq and in_glyph(dx, dy)
    return sq, gl


def main():
    c = (SIZE - 1) / 2.0
    rows = []
    for y in range(SIZE):
        # Row gradient (same across the row; cheap).
        base = lerp(TOP, BOTTOM, y / (SIZE - 1))
        row = bytearray()
        for x in range(SIZE):
            sq_hits = gl_hits = 0
            for sy in range(SS):
                for sx in range(SS):
                    px = x + (sx + 0.5) / SS - 0.5
                    py = y + (sy + 0.5) / SS - 0.5
                    sq, gl = sample(px, py, c)
                    sq_hits += sq
                    gl_hits += gl
            n = SS * SS
            alpha = round(255 * sq_hits / n)
            if alpha == 0:
                row += b"\x00\x00\x00\x00"
                continue
            g = gl_hits / n
            rgb = lerp(base, GLYPH, g)
            row += bytes((rgb[0], rgb[1], rgb[2], alpha))
        rows.append(b"\x00" + bytes(row))  # filter byte 0 per scanline

    raw = b"".join(rows)
    out = Path(__file__).with_name("appicon-1024.png")
    out.write_bytes(_png(SIZE, SIZE, raw))
    print(f"wrote {out} ({out.stat().st_size} bytes)")


def _chunk(tag, data):
    return struct.pack(">I", len(data)) + tag + data + struct.pack(
        ">I", zlib.crc32(tag + data) & 0xFFFFFFFF
    )


def _png(w, h, raw_rgba_scanlines):
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)  # 8-bit RGBA
    idat = zlib.compress(raw_rgba_scanlines, 9)
    return sig + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", idat) + _chunk(b"IEND", b"")


if __name__ == "__main__":
    main()
