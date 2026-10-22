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

def svg(screen, title, crop_rows=None, size=14):
    cw, lh, pad, bar = size * 0.6, size * 1.45, 18, 34
    rows = crop_rows or screen.lines
    W, H = screen.columns * cw + 2 * pad, rows * lh + 2 * pad + bar
    chars, bgs, texts = set(), [], []
    for y in range(rows):
        line = screen.buffer[y]; x = 0; base = bar + pad + y * lh
        while x < screen.columns:
            ch = line[x]
            fg, bg = color(ch.fg, FG), color(ch.bg, None)
            if ch.reverse: fg, bg = (bg or BG), fg
            wide = unicodedata.east_asian_width(ch.data[:1] or " ") == "W"
            if bg: bgs.append(f'<rect x="{pad + x * cw:.1f}" y="{base:.1f}" width="{cw * (2 if wide else 1) + 0.6:.1f}" height="{lh + 0.6:.1f}" fill="{bg}"/>')
            if ch.data.strip():
                chars.add(ch.data)
                attrs = f'fill="{fg}"' + (' font-weight="bold"' if ch.bold else "") + (' opacity="0.55"' if ch.blink else "") \
                    + (' text-decoration="line-through"' if ch.strikethrough else "") + (' text-decoration="underline"' if ch.underscore else "")
                fam = ' font-family="Apple Color Emoji,Segoe UI Emoji,Noto Color Emoji,sans-serif" font-size="%.1f"' % (size * 0.95) if is_emoji(ch.data) else ""
                texts.append(f'<text x="{pad + x * cw:.1f}" y="{base + lh * 0.74:.1f}" {attrs}{fam}>{html.escape(ch.data)}</text>')
            x += 2 if wide else 1
    faces = font_face(chars, "Cockpit Mono", "FiraCodeNerdFontMono-Regular.ttf") + font_face(chars, "Cockpit Mono", "FiraCodeNerdFontMono-Bold.ttf")
    dots = "".join(f'<circle cx="{pad + 6 + i * 20}" cy="{bar / 2 + 4}" r="6" fill="{c}"/>' for i, c in enumerate(["#FF5F57", "#FEBC2E", "#28C840"]))
    return f'''<svg xmlns="http://www.w3.org/2000/svg" width="{W:.0f}" height="{H:.0f}" viewBox="0 0 {W:.0f} {H:.0f}">
<style>{faces} text{{font-family:'Cockpit Mono',monospace;font-size:{size}px;white-space:pre}}</style>
<rect width="100%" height="100%" rx="12" fill="{BG}" stroke="#2C2C30"/>{dots}
<text x="{W / 2:.0f}" y="{bar / 2 + 8:.0f}" text-anchor="middle" fill="#707070" font-size="12">{html.escape(title)}</text>
{"".join(bgs)}{"".join(texts)}</svg>'''

def last_used_row(screen):
    for y in range(screen.lines - 1, -1, -1):
        if any(screen.buffer[y][x].data.strip() or color(screen.buffer[y][x].bg, None) for x in range(screen.columns)): return y + 1
    return 1
