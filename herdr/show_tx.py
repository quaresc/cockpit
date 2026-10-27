#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["textual>=8,<9", "textual-image>=0.14,<0.15", "resvg-py>=0.5,<0.6"]
# ///
"""show.py on Textual, for the config's `apps: "full"`: the same ~/.cache/cockpit/show/<name>.json, pid file and answers.

Charts, flow charts and pictures are real images where the terminal draws them (Kitty graphics, Sixel), else half blocks;
markdown, tables, diffs, logs and questions are Textual widgets; sequence charts and CI runs still go through show.py's renderers.
"""

import atexit
import csv
import io
import os
import subprocess
import time
from datetime import datetime
from functools import partial

from PIL import Image as PILImage
from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.command import DiscoveryHit, Hit, Provider
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.theme import Theme
from textual.widgets import Button, Collapsible, DataTable, Footer, Input, Log, Markdown, OptionList, Static
from textual.widgets.option_list import Option
from textual_image.widget import Image, get_cell_size

import cockpit_config
import graphics
import picture
import show
import view

NAME = show.NAME

VESPER = Theme(name="vesper", primary="#FFC799", secondary="#99FFE4", accent="#FFC799", foreground="#D2D2D2",
               background="#101010", surface="#161616", panel="#1E1E22", success="#99FFE4", warning="#FFC799",
               error="#FF8080", dark=True)


def ansi(lines):
    """show.py's Lines as one Rich Text, styles and all."""
    return Text("\n").join(Text.from_ansi("".join(style + text + view.RESET for text, style in line.parts)) for line in lines)


def collect():
    lines = []

    def line():
        lines.append(view.Line())
        return lines[-1]

    return lines, line


# ---------------------------------------------------------------------------
# Blocks


class Legacy(Static):
    """A block drawn by show.py's own renderer, at the widget's width."""

    def __init__(self, block, draw):
        super().__init__()
        self.block, self.renderer = block, draw

    def on_resize(self):
        lines, line = collect()
        self.renderer(self.block, max(20, self.size.width), line)
        self.update(ansi(lines))


class Table(DataTable):
    """Every column at its full width, scrolled sideways when the pane is narrow; a header click sorts by it."""

    def __init__(self, block):
        super().__init__(zebra_stripes=True, cursor_type="row")
        self.block, self.sorted_by = block, None

    def on_mount(self):
        self.fill(self.block["rows"])

    def fill(self, rows):
        columns = self.block["columns"]
        numeric = [any(show.is_number(r[i]) for r in rows) for i in range(len(columns))]
        self.clear(columns=True)
        self.add_columns(*[Text(str(c), justify="right" if numeric[i] else "left") for i, c in enumerate(columns)])
        for row in rows:
            self.add_row(*[Text(show.cell(v), justify="right" if numeric[i] else "left", style="dim" if v is None else "")
                           for i, v in enumerate(row)])

    @on(DataTable.HeaderSelected)
    def sort_by(self, event):
        i = event.column_index
        reverse = self.sorted_by == (i, False)
        self.sorted_by = (i, reverse)
        key = lambda r: (r[i] is None, r[i] if show.is_number(r[i]) else str(r[i] or "").lower())
        self.fill(sorted(self.block["rows"], key=key, reverse=reverse))

    def as_csv(self):
        out = io.StringIO()
        csv.writer(out).writerows([self.block["columns"], *self.block["rows"]])
        return out.getvalue()


class Picture(Vertical):
    """An image the terminal draws, made again at the pane's width whenever it changes."""

    def __init__(self, block, draw):
        super().__init__()
        self.block, self.draw, self.drawn_at = block, draw, None

    def compose(self) -> ComposeResult:
        yield Image()

    def on_resize(self):
        if self.size.width and self.size.width != self.drawn_at:
            self.drawn_at = self.size.width
            self.render_image(self.size.width)

    @work(thread=True, exclusive=True)
    def render_image(self, cols):
        cell = get_cell_size()
        try:
            data, cells = self.draw(self.block, cols * cell.width, cell)
        except Exception as err:  # a missing file, a dead URL or an unreadable format all say so in the pane
            self.app.call_from_thread(self.mount, Static(Text(f"cannot draw it: {err}", style="#FF8080")))
            return
        self.app.call_from_thread(self.show_image, data, cells)

    def show_image(self, data, cells):
        image = self.query_one(Image)
        image.styles.width = cells
        image.image = io.BytesIO(data)


def font_of(cell):
    return cell.height * 0.48


def draw_line(block, width, cell):
    height = (block.get("height") or 12) * cell.height
    return graphics.png(graphics.line_chart(block, width, height, font_of(cell), show.is_number, show.axes, show.cell)), width // cell.width


def draw_bar(block, width, cell):
    svg, _ = graphics.bar_chart(block, width, font_of(cell), show.is_number, show.axes, show.cell)
    return graphics.png(svg), width // cell.width


def draw_flow(block, width, cell):
    if block["type"] == "mermaid":
        block = show.mermaid.parse(block["text"])
    svg, natural, _ = graphics.flow_chart(block, font_of(cell))
    shown = min(width, natural)
    return graphics.png(svg, round(shown)), max(1, round(shown / cell.width))


def draw_image(block, width, cell):
    path = picture.fetch(block.get("path") or block["url"])
    if path.endswith(".svg") or open(path, "rb").read(256).lstrip().startswith((b"<svg", b"<?xml")):
        text = open(path, encoding="utf-8").read()
        natural = graphics.svg_size(text)[0] * 2
        shown = min(width, block.get("width", 0) * cell.width or natural)
        return graphics.png(text, round(shown)), max(1, round(shown / cell.width))
    with open(path, "rb") as handle:
        data = handle.read()
    # A tall screenshot shrinks until it fits 24 rows, so it never takes over the pane.
    w, h = PILImage.open(io.BytesIO(data)).size
    shown = min(width, (block.get("width") or 10 ** 6) * cell.width, 24 * cell.height * w / h)
    return data, max(1, round(shown / cell.width))


class Diff(Vertical):
    """One fold per file; a repo diff is read again every few seconds."""

    def __init__(self, block):
        super().__init__()
        self.block, self.patch = block, None

    def on_mount(self):
        if self.block.get("patch") is not None:
            self.show_patch(self.block["patch"], None)
        else:
            self.mount(Static(Text(f"reading {self.block['repo']}…", style="dim")))
            self.read()
            self.set_interval(show.DIFF_EVERY, self.read)

    @work(thread=True, exclusive=True)
    def read(self):
        patch, error = show.git_diff(self.block["repo"], self.block.get("base"))
        self.app.call_from_thread(self.show_patch, patch, error)

    def show_patch(self, patch, error):
        if (patch, error) == self.patch:
            return
        self.patch = (patch, error)
        self.remove_children()
        if error:
            self.mount(Static(Text(error, style="red")))
            return
        files = show.parse_patch(patch or "")
        added, removed = sum(f["added"] for f in files), sum(f["removed"] for f in files)
        self.mount(Static(Text.assemble((f"{len(files)} files  ", "dim"), (f"+{added}", "#99FFE4"), " ", (f"−{removed}", "#FF8080"))))
        for f in files:
            body = Text()
            for kind, old, new, text in f["lines"]:
                number = new if new is not None else old
                body.append(f"{number:>5} " if number is not None else "      ", "dim")
                sign, style = {"add": ("+", "#99FFE4"), "del": ("-", "#FF8080"), "same": (" ", ""), "note": (" ", "dim"),
                               "hunk": ("", "#A0A0FF")}[kind]
                body.append(f"{sign} {text}\n", style)
            self.mount(Collapsible(Static(body.rstrip() or body), title=f"{f['path']}   +{f['added']} −{f['removed']}", collapsed=False))


class LiveLog(Vertical):
    def __init__(self, block, cwd):
        super().__init__()
        self.block, self.cwd, self.every = block, cwd, max(1, block.get("every") or 5)

    def compose(self) -> ComposeResult:
        yield Log(max_lines=self.block.get("lines") or 40, auto_scroll=True)
        yield Static(Text(f"running {self.block['command']}…", style="dim"), classes="status")

    def on_mount(self):
        self.run()
        self.set_interval(self.every, self.run)

    @work(thread=True, exclusive=True)
    def run(self):
        try:
            done = subprocess.run(["sh", "-c", self.block["command"]], cwd=self.cwd, capture_output=True, text=True, timeout=120)
            output, code = done.stdout + done.stderr, done.returncode
        except (OSError, subprocess.SubprocessError) as err:
            output, code = str(err), 1
        self.app.call_from_thread(self.show_output, output, code)

    def show_output(self, output, code):
        log = self.query_one(Log)
        log.clear()
        log.write_lines(show.ANSI.sub("", output).rstrip("\n").splitlines())
        mark = "✓" if code == 0 else f"✗ exit {code}"
        self.query_one(".status", Static).update(Text(f"{mark} · every {self.every}s · {datetime.now():%H:%M:%S}", style="dim" if code == 0 else "red"))


class Choice(Vertical):
    """Options with their preview under the cursor, and a free answer the current pane cannot take."""

    def __init__(self, block, n):
        super().__init__()
        self.block, self.n = block, n

    def compose(self) -> ComposeResult:
        if self.block.get("prompt"):
            yield Static(Text(self.block["prompt"], style="bold"))
        if self.block.get("answered"):
            yield Static(Text(f"✓ answered: {self.block['answered']}", style="#99FFE4"))
            return
        yield OptionList(*[Option(o["label"], id=str(i)) for i, o in enumerate(self.block["options"])])
        yield Input(placeholder="or type another answer, enter sends")
        yield Markdown("", classes="preview")

    @on(OptionList.OptionHighlighted)
    def preview(self, event):
        self.query_one(".preview", Markdown).update(self.block["options"][int(event.option.id)].get("preview") or "")

    @on(OptionList.OptionSelected)
    def pick(self, event):
        option = self.block["options"][int(event.option.id)]
        self.send(option.get("value", option["label"]), option["label"])

    @on(Input.Submitted)
    def typed(self, event):
        if event.value.strip():
            self.send(event.value.strip(), event.value.strip())

    def send(self, value, label):
        view.write_json(show.ANSWER.format(self.n), {"block": self.n, "id": self.block.get("id"), "type": "choice", "value": value,
                                                     "label": label, "at": time.time()})
        self.remove_children()
        self.mount(Static(Text(f"✓ sent to the session: {label}", style="#99FFE4")))


class Confirm(Vertical):
    def __init__(self, block, n):
        super().__init__()
        self.block, self.n = block, n

    def compose(self) -> ComposeResult:
        yield Markdown(self.block["summary"])
        if self.block.get("consequence"):
            yield Static(Text(self.block["consequence"], style="#FF8080"))
        with Horizontal(classes="buttons"):
            yield Button("Cancel", id="no")
            yield Button(self.block.get("confirm_label") or "Confirm", id="yes", variant="error")

    @on(Button.Pressed)
    def pressed(self, event):
        yes = event.button.id == "yes"
        label = (self.block.get("confirm_label") or "Confirm") if yes else "cancelled"
        view.write_json(show.ANSWER.format(self.n), {"block": self.n, "id": self.block.get("id"), "type": "confirm", "value": yes,
                                                     "label": label, "at": time.time()})
        self.remove_children()
        self.mount(Static(Text(f"✓ answered: {label}", style="#99FFE4" if yes else "dim")))


def widget(block, n, cwd):
    kind = block["type"]
    if kind == "markdown":
        return Markdown(block["text"], open_links=False)
    if kind == "table":
        return Table(block)
    if kind == "chart":
        return Picture(block, draw_bar if block.get("kind") == "bar" else draw_line)
    if kind == "flow" or (kind == "mermaid" and block["text"].lstrip().startswith(("flowchart", "graph"))):
        return Picture(block, draw_flow)
    if kind == "image":
        return Picture(block, draw_image)
    if kind == "diff":
        return Diff(block)
    if kind == "log":
        return LiveLog(block, cwd)
    if kind == "choice":
        return Choice(block, n)
    if kind == "confirm":
        return Confirm(block, n)
    draw = {"sequence": show.sequence_chart, "mermaid": show.mermaid_chart, "ci": show.ci}.get(kind)
    return Legacy(block, draw) if draw else Static(Text(f"{kind}: not ported", style="dim"))


# ---------------------------------------------------------------------------
# The command palette: every block to jump to, every table to copy, the view's page to open


class ViewCommands(Provider):
    def commands(self):
        app = self.app
        for section in app.query(".block"):
            title = section.block_title
            yield f"Go to: {title}", partial(app.query_one("#body").scroll_to_widget, section, top=True), "scroll the view to this block"
            table = next(iter(section.query(Table)), None)
            if table:
                yield f"Copy as CSV: {title}", partial(app.copy, table.as_csv(), title), "the table's rows to the clipboard"
        if (app.spec or {}).get("url"):
            yield "Open the view's page in the browser", app.action_browser, app.spec["url"]

    async def discover(self):
        for text, callback, help_ in self.commands():
            yield DiscoveryHit(text, callback, help=help_)

    async def search(self, query):
        matcher = self.matcher(query)
        for text, callback, help_ in self.commands():
            if (score := matcher.match(text)) > 0:
                yield Hit(score, matcher.highlight(text), callback, help=help_)


# ---------------------------------------------------------------------------
# The app


class ShowApp(App):
    TITLE = NAME
    COMMANDS = App.COMMANDS | {ViewCommands}
    CSS = """
    Screen { background: $background; }
    #head { height: 1; padding: 0 1; }
    #body { padding: 0 1; }
    .block, Diff, LiveLog, Choice, Confirm { height: auto; }
    .block { margin: 1 0 0 0; }
    .block-title { text-style: bold; }
    Markdown { margin: 0; padding: 0; background: $background; }
    DataTable { height: auto; max-height: 18; }
    Picture { height: auto; }
    Picture Image { width: auto; height: auto; }
    Collapsible { padding: 0; border-top: none; background: $background; }
    OptionList { height: auto; max-height: 8; }
    Log { height: auto; max-height: 12; }
    .buttons { height: auto; }
    .buttons Button { margin-right: 2; }
    """
    BINDINGS = [
        Binding("q", "quit", "hide"),
        Binding("j", "scroll(1)", "down", show=False),
        Binding("k", "scroll(-1)", "up", show=False),
        Binding("g", "edge(False)", "top", show=False),
        Binding("G", "edge(True)", "bottom", show=False),
        Binding("o", "browser", "browser"),
        Binding("r", "reviewr", "reviewr"),
    ]

    def __init__(self):
        super().__init__()
        self.spec, self.mtime, self.shown_at = None, None, None

    def compose(self) -> ComposeResult:
        yield Static(id="head")
        yield VerticalScroll(id="body")
        yield Footer()

    def on_mount(self):
        self.register_theme(VESPER)
        self.theme = "vesper"
        self.poll()
        self.set_interval(1, self.poll)

    def poll(self):
        mtime = os.path.getmtime(show.SPEC) if os.path.exists(show.SPEC) else 0
        if mtime != self.mtime:
            self.mtime, self.spec = mtime, view.load_json(show.SPEC, None)
            self.build()

    def build(self):
        body, spec = self.query_one("#body"), self.spec
        # A new call fades its blocks in from the top; a refresh of the same call redraws in place, where it was scrolled to.
        is_new, scrolled = (spec or {}).get("at") != self.shown_at, body.scroll_y
        self.shown_at = (spec or {}).get("at")
        body.remove_children()
        if not spec:
            self.query_one("#head", Static).update(Text(f"waiting for the view {NAME}…", style="dim"))
            return
        at = datetime.fromtimestamp(spec.get("at", 0) / 1000).strftime("%H:%M:%S")
        self.query_one("#head", Static).update(Text.assemble(("◆  ", "#FFC799"), (spec.get("title") or NAME, "bold #FFC799"), (f"   {NAME} · {at}", "dim")))
        cwd = spec.get("cwd") or os.path.expanduser("~")
        for n, block in enumerate(spec.get("blocks", [])):
            section = Vertical(classes="block")
            section.block_title = block.get("title") or block["type"]
            parts = [Static(Text(block["title"]), classes="block-title")] if block.get("title") else []
            if block.get("error"):
                parts.append(Static(Text(block["error"], style="#FF8080")))
            parts.append(widget(block, n, cwd))
            if is_new:
                section.styles.opacity = 0
            body.mount(section)
            section.mount_all(parts)
            if is_new:
                section.styles.animate("opacity", 1.0, duration=0.4, delay=n * 0.06, easing="out_cubic")
        if not is_new:
            self.call_after_refresh(body.scroll_to, y=scrolled, animate=False)

    def action_scroll(self, lines):
        self.query_one("#body").scroll_relative(y=lines, animate=False)

    def action_edge(self, bottom):
        body = self.query_one("#body")
        body.scroll_end(animate=False) if bottom else body.scroll_home(animate=False)

    def action_reviewr(self):
        repo = next((b["repo"] for b in (self.spec or {}).get("blocks", []) if b["type"] == "diff" and b.get("repo") and b.get("patch") is None), None)
        if not repo:
            self.notify("no repo diff in this view")
        elif not show.has_reviewr():
            self.notify("herdr-reviewr is not installed")
        else:
            state = view.new_state()
            show.open_reviewr(state, repo)
            self.notify(state["note"])

    def action_browser(self):
        if (self.spec or {}).get("url"):
            cockpit_config.open_url(self.spec["url"])
            self.notify(f"opened {self.spec['url']}")

    def copy(self, text, title):
        self.copy_to_clipboard(text)
        self.notify(f"copied {title} as CSV")

    @on(Markdown.LinkClicked)
    def link(self, event):
        cockpit_config.open_url(event.href)


if __name__ == "__main__":
    os.makedirs(show.DIR, exist_ok=True)
    # The mod reads this to update the open pane instead of splitting a new one.
    with open(show.ALIVE, "w", encoding="utf-8") as handle:
        handle.write(str(os.getpid()))
    atexit.register(show.forget)
    ShowApp().run()
