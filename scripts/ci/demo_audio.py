#!/usr/bin/env python3
"""Demo music for the screenshots: a few short pieces made up here (chords,
an arpeggio, a beat - nobody's music), written as FLAC with their own tags
and, for the first, a cover - so the player shows a title, an artist and a
picture, and the faces have something to move with.

    python3 -I scripts/ci/demo_audio.py OUT_DIR

The FLAC is written by hand (the standard library only: no encoder in CI):
verbatim subframes - uncompressed, but valid FLAC - with the metadata blocks
STREAMINFO, VORBIS_COMMENT and PICTURE. The cover is a PNG drawn here too.
"""
import math
import os
import struct
import sys
import zlib
from array import array

RATE = 22050
BLOCK = 4096


def png(width, height, pixel):
    """A PNG of `pixel(x, y) -> (r, g, b)`."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows.extend(pixel(x, y))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(bytes(rows), 9))
            + chunk(b"IEND", b""))


def cover(size=320):
    """A sun over a grid at dusk (no one's artwork)."""
    def pixel(x, y):
        u, v = x / size, y / size
        if v < 0.62:
            # the sky: violet to orange, the sun in it with its bands
            r, g, b = int(60 + 190 * v), int(20 + 90 * v), int(110 - 60 * v)
            dx, dy = u - 0.5, v - 0.42
            if dx * dx + dy * dy < 0.07 and not (v > 0.42 and int(v * 60) % 3 == 0):
                r, g, b = 255, int(120 + 200 * (0.62 - v)), 80
            return (min(r, 255), min(g, 255), max(b, 0))
        # the ground: dark, a magenta grid
        line = (int((u - 0.5) * 16 / (v - 0.5 + 0.05)) % 2 == 0 and abs(((u - 0.5) * 16 / (v - 0.5 + 0.05)) % 1) < 0.08) \
            or int((v - 0.62) * 120) % 6 == 0
        return (200, 40, 180) if line else (25, 10, 40)

    return png(size, size, pixel)


def crc8(data):
    c = 0
    for b in data:
        c ^= b
        for _ in range(8):
            c = ((c << 1) ^ 0x07) & 0xFF if c & 0x80 else (c << 1) & 0xFF
    return c


def crc16(data):
    c = 0
    for b in data:
        c ^= b << 8
        for _ in range(8):
            c = ((c << 1) ^ 0x8005) & 0xFFFF if c & 0x8000 else (c << 1) & 0xFFFF
    return c


def utf8_number(n):
    if n < 0x80:
        return bytes([n])
    if n < 0x800:
        return bytes([0xC0 | (n >> 6), 0x80 | (n & 0x3F)])
    return bytes([0xE0 | (n >> 12), 0x80 | ((n >> 6) & 0x3F), 0x80 | (n & 0x3F)])


def flac(path, left, right, tags, picture=None):
    total = len(left)
    assert total % BLOCK == 0
    info = struct.pack(">HH", BLOCK, BLOCK) + b"\x00\x00\x00" + b"\x00\x00\x00"
    # sample rate (20 bits), channels - 1 (3), bits - 1 (5), samples (36)
    packed = (RATE << 44) | (1 << 41) | (15 << 36) | total
    info += packed.to_bytes(8, "big") + bytes(16)
    vendor = b"axross-amp-android demo"
    comments = [f"{k}={v}".encode() for k, v in tags.items()]
    vorbis = struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", len(comments))
    for c in comments:
        vorbis += struct.pack("<I", len(c)) + c
    blocks = [(0, info), (4, vorbis)]
    if picture:
        mime = b"image/png"
        pic = struct.pack(">II", 3, len(mime)) + mime + struct.pack(">I", 0)
        pic += struct.pack(">IIIII", 320, 320, 24, 0, len(picture)) + picture
        blocks.append((6, pic))
    out = bytearray(b"fLaC")
    for i, (kind, body) in enumerate(blocks):
        last = 0x80 if i == len(blocks) - 1 else 0
        out += bytes([last | kind]) + len(body).to_bytes(3, "big") + body
    for f in range(total // BLOCK):
        # sync, fixed blocks; 4096 a block, the rate from STREAMINFO;
        # two channels apart, 16 bits; the frame's number
        header = bytes([0xFF, 0xF8, 0xC0, 0x18]) + utf8_number(f)
        header += bytes([crc8(header)])
        frame = bytearray(header)
        for channel in (left, right):
            frame.append(0x02)  # a verbatim subframe, no wasted bits
            chunk = channel[f * BLOCK:(f + 1) * BLOCK]
            chunk.byteswap()
            frame += chunk.tobytes()
            chunk.byteswap()
        frame += struct.pack(">H", crc16(frame))
        out += frame
    with open(path, "wb") as file:
        file.write(out)


def piece(seconds, root, beat, seed):
    """Chords on `root` (Hz), an arpeggio over them, a kick every `beat` s."""
    n = int(seconds * RATE) // BLOCK * BLOCK
    left, right = array("h", bytes(2 * n)), array("h", bytes(2 * n))
    chords = [(1, 5 / 4, 3 / 2), (4 / 3, 5 / 3, 2), (6 / 5, 3 / 2, 9 / 5), (9 / 8, 3 / 2, 15 / 8)]
    tau = 2 * math.pi
    for i in range(n):
        t = i / RATE
        bar = int(t / (beat * 4)) % len(chords)
        chord = chords[(bar + seed) % len(chords)]
        pad = sum(math.sin(tau * root * r * t) for r in chord) / 3
        step = int(t / (beat / 2)) % 6
        arp_f = root * 2 * chord[step % 3] * (2 if step >= 3 else 1)
        local = (t % (beat / 2)) / (beat / 2)
        arp = math.sin(tau * arp_f * t) * math.exp(-5 * local)
        k = t % beat
        kick = math.sin(tau * (50 + 90 * math.exp(-30 * k)) * k) * math.exp(-9 * k)
        s = 0.28 * pad + 0.22 * arp + 0.5 * kick
        left[i] = int(max(-1, min(1, s + 0.05 * arp)) * 26000)
        right[i] = int(max(-1, min(1, s - 0.05 * arp)) * 26000)
    return left, right


def main():
    out = sys.argv[1]
    os.makedirs(out, exist_ok=True)
    pieces = [
        ("01 Glass Harmonics.flac", "Glass Harmonics", 110.0, 0.5, 0, True),
        ("02 Night Drive.flac", "Night Drive", 130.81, 0.43, 1, False),
        ("03 Quiet Tide.flac", "Quiet Tide", 98.0, 0.6, 2, False),
    ]
    picture = cover()
    for name, title, root, beat, seed, with_cover in pieces:
        left, right = piece(75, root, beat, seed)
        flac(os.path.join(out, name), left, right,
             {"TITLE": title, "ARTIST": "axross demo", "ALBUM": "Demo Tones", "TRACKNUMBER": name[:2]},
             picture if with_cover else None)
        print(f"{name}: {os.path.getsize(os.path.join(out, name))} bytes")


if __name__ == "__main__":
    main()
