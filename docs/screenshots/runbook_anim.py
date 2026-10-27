"""An animated SVG of a runbook refresh landing: three steps turn done and play their animation.
--full draws runbook_tx.py, the Textual version, into runbook-done-full.svg; run it with the venv's python, which has Textual."""
import fcntl, json, os, pty, select, signal, struct, sys, termios, time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from term2svg import screen_of, rows_svg, window

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
BOOK = os.path.join(HOME, ".config", "cockpit", "runbooks", "eng-410.md")
FULL = "--full" in sys.argv
COLS, ROWS, CROP = (140, 30, 30) if FULL else (140, 26, 17)
HOLD_BEFORE, HOLD_AFTER = 1.2, 2.8

pid, fd = pty.fork()
if pid == 0:
    os.chdir(os.path.join(HERE, "..", "..", "herdr"))
    argv = [sys.executable, "runbook_tx.py"] if FULL else ["python3", "runbook.py"]
    os.execvpe(argv[0], argv, {**os.environ, "HOME": HOME, "HERDR_BIN_PATH": "/usr/bin/true", "TERM": "xterm-256color"})
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

out, start, changed, samples = b"", time.time(), None, []
while time.time() - start < (9 if FULL else 7):
    if select.select([fd], [], [], 0.02)[0]:
        out += os.read(fd, 1 << 16)
    now = time.time() - start
    if changed is None and now > (4.0 if FULL else 1.5):
        # The background refresh writes the file: backfill done, enforcement and banner merged.
        text = open(BOOK).read()
        for a, b in [("⬜ to do | `bin/rails", "✅ backfilled | `bin/rails"), ("⏳ approved, CI green", "✅ merged"), ("🟡 CI running", "✅ merged")]:
            text = text.replace(a, b)
        open(BOOK, "w").write(text)
        changed = now
    if changed is not None and (not samples or now - samples[-1][0] > 0.12):
        samples.append((now, out.decode("utf-8", "replace")))
os.kill(pid, signal.SIGKILL)

# Distinct screens in order, from just before the change to the end of the animation.
frames = []
for at, text in samples:
    screen = screen_of(text, COLS, ROWS)
    key = "\n".join("".join(c.data + c.bg for c in screen.buffer[y].values()) for y in range(CROP))
    if not frames or frames[-1][2] != key:
        frames.append((at - changed, screen, key))
base = frames[0][1]
anim = HOLD_BEFORE + (frames[-1][0] - frames[0][0]) + HOLD_AFTER
chars, layers, style = set(), [], ""
for i, (at, screen, key) in enumerate(frames[1:], 1):
    begin = HOLD_BEFORE + at - frames[0][0]
    end = HOLD_BEFORE + frames[i + 1][0] - frames[0][0] if i + 1 < len(frames) else anim
    # Each layer stays up to the end of the loop and only repaints the rows that changed since the previous one.
    differ = [y for y in range(CROP) if key.split("\n")[y] != frames[i - 1][2].split("\n")[y]]
    a = 100 * begin / anim
    style += f"@keyframes f{i}{{0%,{a:.2f}%{{visibility:hidden}}{a:.2f}%,100%{{visibility:visible}}}}" \
             f".f{i}{{visibility:hidden;animation:f{i} {anim:.2f}s step-end infinite}}"
    layers.append(f'<g class="f{i}">{rows_svg(screen, differ, chars, cover=True)}</g>')
body = rows_svg(base, range(CROP), chars) + "".join(layers)
with open(os.path.join(HERE, "..", "runbook-done-full.svg" if FULL else "runbook-done.svg"), "w") as f:
    f.write(window(COLS, CROP, "runbook · a refresh lands" + (" · full" if FULL else ""), chars, body, style))
print(len(frames), "frames,", f"{anim:.1f}s loop")
if os.environ.get("FRAMES"):
    from term2svg import svg
    for i in map(int, os.environ["FRAMES"].split(",")):
        open(os.path.join(os.environ.get("FRAMES_DIR", HERE), f"frame-{i}.svg"), "w").write(svg(frames[i][1], f"frame {i}", CROP))
