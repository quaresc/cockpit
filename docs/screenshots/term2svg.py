"""Terminal capture to SVG: runs a TUI in a pty (or reads ANSI text), emulates the screen with pyte,
and draws it as an SVG window with a subset of FiraCode Nerd Font embedded."""
import base64, fcntl, html, io, os, pty, select, signal, struct, sys, termios, time, unicodedata
import pyte
from fontTools import subset
from fontTools.ttLib import TTFont

FONTS = os.path.expanduser("~/Library/Fonts")
BG, FG = "#101010", "#D2D2D2"
NAMED = {"black": "#101010", "red": "#FF8080", "green": "#99FFE4", "brown": "#FFC799", "yellow": "#FFC799", "blue": "#A0A0FF",
         "magenta": "#FF99CC", "cyan": "#99FFE4", "white": "#D2D2D2", "brightblack": "#505050", "brightwhite": "#FFFFFF"}

class Screen(pyte.Screen):
    # SGR 2 (dim) is kept in the unused blink attribute.
    def select_graphic_rendition(self, *attrs, private=False):
        out, i = [], 0
        while i < len(attrs):
            a = attrs[i]
            if a in (38, 48) and i + 1 < len(attrs):
                n = 5 if attrs[i + 1] == 2 else 3
                out += attrs[i:i + n]; i += n; continue
            out.append(5 if a == 2 else 25 if a == 22 else a); i += 1
        if 22 in attrs: out.append(22)
        super().select_graphic_rendition(*out)

def run(argv, cols, rows, env, keys=(), wait=2.5, cwd=None):
    pid, fd = pty.fork()
    if pid == 0:
        if cwd: os.chdir(cwd)
        os.execvpe(argv[0], argv, {**os.environ, **env})
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out = b""; plan = list(keys); start = time.time()
    while time.time() - start < wait + 0.6 * len(keys):
        r, _, _ = select.select([fd], [], [], 0.1)
        if r:
            try: out += os.read(fd, 1 << 16)
            except OSError: break
        if plan and time.time() - start > 1.5 + 0.6 * (len(keys) - len(plan)):
            os.write(fd, plan.pop(0).encode())
    os.kill(pid, signal.SIGKILL)
    return out.decode("utf-8", "replace")

def screen_of(text, cols, rows):
    screen = Screen(cols, rows); pyte.Stream(screen).feed(text); return screen

def color(c, default):
    if c == "default": return default
    if len(c) == 6 and all(x in "0123456789abcdefABCDEF" for x in c): return "#" + c
    return NAMED.get(c, default)

def font_face(chars, name, file):
    opts = subset.Options(); opts.flavor = None; opts.layout_features = []
    font = TTFont(os.path.join(FONTS, file))
    sub = subset.Subsetter(opts); sub.populate(text="".join(chars) + " abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")
    sub.subset(font); buf = io.BytesIO(); font.save(buf)
    return f"@font-face{{font-family:'{name}';src:url(data:font/ttf;base64,{base64.b64encode(buf.getvalue()).decode()}) format('truetype');font-weight:{'bold' if 'Bold' in file else 'normal'}}}"

def is_emoji(ch):
    return unicodedata.east_asian_width(ch) == "W" and ord(ch) > 0x2000 and not (0xE000 <= ord(ch) <= 0xF8FF or ord(ch) >= 0xF0000)

SIZE = 14
CW, LH, PAD, BAR = SIZE * 0.6, SIZE * 1.45, 18, 34


def rows_svg(screen, ys, chars, cover=False, at=None):
    """The rows' cells as SVG; `cover` first paints each row's background, to draw over an earlier frame.
    `at` draws them from that display row on instead of at their own place."""
    out = []
    for n, y in enumerate(ys):
        line = screen.buffer[y]; x = 0; base = BAR + PAD + ((at + n) if at is not None else y) * LH
        if cover:
            # A little taller than the row, so no anti-aliased edge of the frame below shows through.
            out.append(f'<rect x="{PAD - 2}" y="{base - 0.6:.1f}" width="{screen.columns * CW + 4:.1f}" height="{LH + 1.8:.1f}" fill="{BG}"/>')
        texts, run, fills = [], None, []
        def flush():
            if run:
                # textLength pins the run to its cells, whatever the viewer's font metrics.
                text = run[2].rstrip()
                fit = f' textLength="{len(text) * CW:.1f}" lengthAdjust="spacingAndGlyphs"' if len(text) > 1 else ""
                texts.append(f'<text x="{PAD + run[0] * CW:.1f}" y="{base + LH * 0.74:.1f}" {run[1]}{fit}>{html.escape(text)}</text>')
        while x < screen.columns:
            ch = line[x]
            fg, bg = color(ch.fg, FG), color(ch.bg, None)
            if ch.reverse: fg, bg = (bg or BG), fg
            wide = unicodedata.east_asian_width(ch.data[:1] or " ") == "W"
            span = 2 if wide else 1
            if bg and fills and fills[-1][2] == bg and fills[-1][0] + fills[-1][1] == x:
                fills[-1][1] += span
            elif bg:
                fills.append([x, span, bg])
            if ch.data.strip():
                chars.add(ch.data)
                attrs = f'fill="{fg}"' + (' font-weight="bold"' if ch.bold else "") + (' opacity="0.55"' if ch.blink else "") \
                    + (' text-decoration="line-through"' if ch.strikethrough else "") + (' text-decoration="underline"' if ch.underscore else "")
                if is_emoji(ch.data) or wide:
                    flush(); run = None
                    fam = ' font-family="Apple Color Emoji,Segoe UI Emoji,Noto Color Emoji,sans-serif" font-size="%.1f"' % (SIZE * 0.95) if is_emoji(ch.data) else ""
                    texts.append(f'<text x="{PAD + x * CW:.1f}" y="{base + LH * 0.74:.1f}" {attrs}{fam}>{html.escape(ch.data)}</text>')
                elif run and run[1] == attrs and run[0] + len(run[2]) == x:
                    run[2] += ch.data
                else:
                    flush(); run = [x, attrs, ch.data]
            elif run and run[0] + len(run[2]) == x and ch.data == " ":
                run[2] += " "
            x += 2 if wide else 1
        flush()
        out += [f'<rect x="{PAD + fx * CW:.1f}" y="{base:.1f}" width="{CW * n + 0.6:.1f}" height="{LH + 0.6:.1f}" fill="{c}"/>' for fx, n, c in fills]
        out += texts
    return "".join(out)


def window(columns, rows, title, chars, body, style=""):
    W, H = columns * CW + 2 * PAD, rows * LH + 2 * PAD + BAR
    faces = font_face(chars, "Cockpit Mono", "FiraCodeNerdFontMono-Regular.ttf") + font_face(chars, "Cockpit Mono", "FiraCodeNerdFontMono-Bold.ttf")
    dots = "".join(f'<circle cx="{PAD + 6 + i * 20}" cy="{BAR / 2 + 4}" r="6" fill="{c}"/>' for i, c in enumerate(["#FF5F57", "#FEBC2E", "#28C840"]))
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W:.0f}" height="{H:.0f}" viewBox="0 0 {W:.0f} {H:.0f}">
<style>{faces} text{{font-family:'Cockpit Mono',monospace;font-size:{SIZE}px;white-space:pre}}{style}</style>
<rect width="100%" height="100%" rx="12" fill="{BG}" stroke="#2C2C30"/>{dots}
<text x="{W / 2:.0f}" y="{BAR / 2 + 8:.0f}" text-anchor="middle" fill="#707070" font-size="12">{html.escape(title)}</text>
{body}</svg>'''


def svg(screen, title, crop_rows=None):
    rows = crop_rows or screen.lines
    chars = set()
    body = rows_svg(screen, range(rows), chars)
    return window(screen.columns, rows, title, chars, body)


def last_used_row(screen):
    for y in range(screen.lines - 1, -1, -1):
        if any(screen.buffer[y][x].data.strip() or color(screen.buffer[y][x].bg, None) for x in range(screen.columns)): return y + 1
    return 1


def run_steps(argv, cols, rows, env, keys, cwd=None, settle=1.5, gap=0.5):
    """Runs a TUI and returns its screen text after start-up and after each key, one key every `gap` seconds."""
    pid, fd = pty.fork()
    if pid == 0:
        if cwd: os.chdir(cwd)
        os.execvpe(argv[0], argv, {**os.environ, **env})
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    out, shots, start, sent = b"", [], time.time(), 0
    while sent <= len(keys):
        if select.select([fd], [], [], 0.05)[0]:
            try: out += os.read(fd, 1 << 16)
            except OSError: break
        if time.time() - start > settle + gap * sent:
            shots.append(out.decode("utf-8", "replace"))
            if sent < len(keys): os.write(fd, keys[sent].encode())
            sent += 1
    os.kill(pid, signal.SIGKILL)
    return shots


def animate(frames, title, extra=None):
    """An animated SVG window from [(seconds, screen), ...], looping: the first screen is the base, each later one a layer
    of the rows that changed, shown from its start to the loop's end. `extra(i)` may add SVG drawn with frame i's layer."""
    cols, rows = frames[0][1].columns, frames[0][1].lines
    loop = sum(d for d, _ in frames)
    keys = [["".join(s.buffer[y][x].data + str(s.buffer[y][x].fg) + str(s.buffer[y][x].bg) for x in range(cols)) for y in range(rows)]
            for _, s in frames]
    chars, layers, style, begin = set(), [], "", 0
    for i, (duration, screen) in enumerate(frames):
        if i:
            differ = [y for y in range(rows) if keys[i - 1][y] != keys[i][y]]
            a = 100 * begin / loop
            style += f"@keyframes s{i}{{0%,{a:.2f}%{{visibility:hidden}}{a:.2f}%,100%{{visibility:visible}}}}" \
                     f".s{i}{{visibility:hidden;animation:s{i} {loop:.2f}s step-end infinite}}"
            layers.append(f'<g class="s{i}">{rows_svg(screen, differ, chars, cover=True)}{extra(i) if extra else ""}</g>')
        begin += duration
    body = rows_svg(frames[0][1], range(rows), chars) + (extra(0) if extra else "") + "".join(layers)
    return window(cols, rows, title, chars, body, style)


def put_ops():
    """A cursor-addressed drawing list and its put(row, col, text, fg, bg, bold) for composed screens."""
    ops = []

    def put(r, c, text, fg=None, bg=None, bold=False):
        sgr = ["0"] + (["1"] if bold else [])
        if fg: sgr.append("38;2;%d;%d;%d" % tuple(int(fg[i:i + 2], 16) for i in (0, 2, 4)))
        if bg: sgr.append("48;2;%d;%d;%d" % tuple(int(bg[i:i + 2], 16) for i in (0, 2, 4)))
        ops.append(f"\033[{r};{c}H\033[{';'.join(sgr)}m{text}\033[0m")
    return ops, put


def picture(path, width, height):
    """A screenshot as an embedded JPEG fitting width × height pixels, and its size once fitted."""
    from PIL import Image  # only the pictures of real pages need it
    image = Image.open(path).convert("RGB")
    scale = min(width / image.width, height / image.height)
    size = (round(image.width * scale), round(image.height * scale))
    buf = io.BytesIO()
    image.resize((size[0] * 2, size[1] * 2), Image.LANCZOS).save(buf, "JPEG", quality=82, optimize=True)
    return base64.b64encode(buf.getvalue()).decode(), size


def timed_pictures(frames, pages, box):
    """Screenshots shown with an animation's frames: `pages` maps a frame index to a picture's path, shown from that frame
    until the next one replaces it (None hides it). `box` is (x, y, width, height). Returns the CSS and the SVG to add."""
    loop = sum(d for d, _ in frames)
    starts = [sum(d for d, _ in frames[:i]) for i in range(len(frames))]
    x0, y0, w, h = box
    style, images, order = "", [], sorted(pages)
    for n, i in enumerate(order):
        if pages[i] is None:
            continue
        data, (iw, ih) = picture(pages[i], w, h)
        a = 100 * starts[i] / loop
        b = 100 * (starts[order[n + 1]] if n + 1 < len(order) else loop) / loop
        style += f"@keyframes p{i}{{0%,{a:.2f}%{{visibility:hidden}}{a:.2f}%,{b:.2f}%{{visibility:visible}}{b:.2f}%,100%{{visibility:hidden}}}}" \
                 f".p{i}{{visibility:hidden;animation:p{i} {loop:.2f}s step-end infinite}}"
        images.append(f'<image class="p{i}" href="data:image/jpeg;base64,{data}" x="{x0 + (w - iw) / 2:.1f}" y="{y0:.1f}" '
                      f'width="{iw}" height="{ih}"/>')
    return style, "".join(images)


def with_pictures(svg_text, style, images):
    """An SVG with timed_pictures' CSS and pictures added."""
    return svg_text.replace("</style>", style + "</style>", 1).replace("</svg>", images + "</svg>")
