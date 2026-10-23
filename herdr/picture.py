"""Pictures in a terminal: an image shrunk to the pane and drawn with half blocks, two pixels a cell.

macOS's sips converts and resizes, so nothing needs installing; elsewhere ImageMagick does. A URL is downloaded once
into the cache, and a decoded picture is kept per path, modification time and size.
"""

import hashlib
import os
import shutil
import struct
import subprocess
import tempfile
import urllib.request

CACHE = os.path.join(os.path.expanduser("~"), ".cache", "cockpit", "show", "images")
# What a transparent pixel shows through: the panes' background.
BACKGROUND = (16, 16, 16)
_decoded = {}


def fetch(source):
    """A local path for a path or an http(s) URL, downloading the URL once."""
    if not source.startswith(("http://", "https://")):
        return os.path.expanduser(source)
    os.makedirs(CACHE, exist_ok=True)
    path = os.path.join(CACHE, hashlib.sha1(source.encode()).hexdigest())
    if not os.path.exists(path):
        request = urllib.request.Request(source, headers={"User-Agent": "cockpit"})
        with urllib.request.urlopen(request, timeout=20) as answer, open(path + ".part", "wb") as out:
            out.write(answer.read())
        os.replace(path + ".part", path)
    return path


def size_of(path):
    """The image's width and height in pixels."""
    if shutil.which("sips"):
        out = subprocess.run(["sips", "-g", "pixelWidth", "-g", "pixelHeight", path], capture_output=True, text=True, timeout=20).stdout
        values = dict((k.strip(), v.strip()) for k, v in (line.split(": ", 1) for line in out.splitlines() if ": " in line))
        return int(values["pixelWidth"]), int(values["pixelHeight"])
    magick = shutil.which("magick") or shutil.which("identify")
    if magick:
        argv = [magick, "identify", "-format", "%w %h", path + "[0]"] if magick.endswith("magick") else [magick, "-format", "%w %h", path + "[0]"]
        w, h = subprocess.run(argv, capture_output=True, text=True, timeout=20).stdout.split()
        return int(w), int(h)
    raise RuntimeError("no image converter: needs sips (macOS) or ImageMagick")


def to_bmp(path, w, h, out):
    if shutil.which("sips"):
        subprocess.run(["sips", "-s", "format", "bmp", "-z", str(h), str(w), path, "--out", out], capture_output=True, timeout=30, check=True)
        return
    magick = shutil.which("magick") or shutil.which("convert")
    if not magick:
        raise RuntimeError("no image converter: needs sips (macOS) or ImageMagick")
    subprocess.run([magick, path + "[0]", "-resize", f"{w}x{h}!", "-type", "TrueColor", f"BMP3:{out}"], capture_output=True, timeout=30, check=True)


def read_bmp(data):
    """Rows of (r, g, b), top first, from an uncompressed 24- or 32-bit BMP."""
    offset, = struct.unpack_from("<I", data, 10)
    w, h, _, bpp, compression = struct.unpack_from("<iiHHI", data, 18)
    if bpp not in (24, 32) or compression not in (0, 3):
        raise RuntimeError(f"unsupported BMP: {bpp} bits, compression {compression}")
    masks = struct.unpack_from("<IIII", data, 54) if compression == 3 else (0xFF0000, 0xFF00, 0xFF, 0xFF000000 if bpp == 32 else 0)

    def channel(value, mask):
        if not mask:
            return 255
        shift = (mask & -mask).bit_length() - 1
        return ((value & mask) >> shift) * 255 // (mask >> shift)

    stride = (w * bpp // 8 + 3) & ~3
    rows = []
    for y in range(abs(h)):
        start = offset + (y if h < 0 else abs(h) - 1 - y) * stride
        row = []
        for x in range(w):
            at = start + x * bpp // 8
            if bpp == 24:
                b, g, r = data[at], data[at + 1], data[at + 2]
                row.append((r, g, b))
                continue
            value, = struct.unpack_from("<I", data, at)
            r, g, b, a = (channel(value, m) for m in masks)
            row.append(tuple(round(c * a / 255 + bg * (255 - a) / 255) for c, bg in zip((r, g, b), BACKGROUND)))
        rows.append(row)
    return rows


def pixels(source, cols, max_rows):
    """The picture as rows of pixels, at most `cols` wide and `max_rows` cells (twice as many pixels) high."""
    path = fetch(source)
    key = (path, os.path.getmtime(path), cols, max_rows)
    if key not in _decoded:
        ow, oh = size_of(path)
        # A half-block pixel is about square, so the picture keeps its proportions at one pixel a column.
        w = max(1, min(cols, ow))
        h = max(2, round(oh * w / ow))
        if h > 2 * max_rows:
            h = 2 * max_rows
            w = max(1, round(ow * h / oh))
        h += h % 2
        with tempfile.TemporaryDirectory() as tmp:
            out = os.path.join(tmp, "picture.bmp")
            to_bmp(path, w, h, out)
            with open(out, "rb") as handle:
                _decoded[key] = read_bmp(handle.read())
    return _decoded[key]


def rows(grid):
    """Terminal rows of (text, style) runs: each cell's upper pixel as the foreground of ▀, its lower as the background."""
    out = []
    for y in range(0, len(grid) - 1, 2):
        runs = []
        for top, bottom in zip(grid[y], grid[y + 1]):
            style = "\033[38;2;%d;%d;%dm\033[48;2;%d;%d;%dm" % (*top, *bottom)
            if runs and runs[-1][1] == style:
                runs[-1][0] += "▀"
            else:
                runs.append(["▀", style])
        out.append([(text, style) for text, style in runs])
    return out
