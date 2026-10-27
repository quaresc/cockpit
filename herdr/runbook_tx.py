#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["textual>=8,<9"]
# ///
"""runbook.py on Textual, for the config's `apps: "full"`: the same runbook files, acting through runbook.py's own functions.

A tab per runbook with its progress, its steps as rows whose cells wrap instead of being cut, and the selected step's actions
in a panel under it; ctrl+p searches every step's actions. Steps that turn done spin, flash and fade one after the other.
"""

import os
import time
from functools import partial

from rich.text import Text
from textual import on, work
from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.color import Color
from textual.command import DiscoveryHit, Hit, Provider
from textual.containers import Horizontal, Vertical, VerticalScroll
from textual.widgets import Button, Footer, LoadingIndicator, Markdown, ProgressBar, Static, TabbedContent, TabPane

import runbook as rb
import view
from inbox_tx import VESPER, Act, Sure

# Another folder of runbooks to try the app on, leaving the real ones alone.
rb.DIR = os.environ.get("COCKPIT_RUNBOOKS_DIR") or rb.DIR
# Run by runbook-loop.sh in the runbook space: the sidebar count, the mod's refresh requests and notification clicks are this app's.
PINNED = bool(os.environ.get("COCKPIT_SPACE"))

ORANGE, GREEN, RED, DIM = "#FFC799", "#99FFE4", "#FF8080", "dim"
STYLE = {view.ORANGE: ORANGE, view.GREEN: GREEN, view.RED: RED, view.DIM: DIM, view.BOLD + view.ORANGE: f"bold {ORANGE}",
         view.DIM + view.GREEN: f"dim {GREEN}", "": ""}
FLASH_FROM, FLASH_TO = Color(40, 100, 80), Color(16, 16, 16)


def step_acts(app, book, step):
    """The step's actions, as the runbook rows have them, with a key each."""
    state, acts = app.state, []
    is_done = not step["status"].startswith(rb.OPEN)
    handoff = state["handoffs"].get(rb.handoff_id(book, step))
    if not is_done:
        if handoff and rb.in_flight(handoff):
            acts.append(Act("session", "s", partial(rb.focus_session, state, handoff), True))
        else:
            acts.append(Act("run", "r", partial(rb.run_step, state, book, step), True, f"run-{book['slug']}-{step['n']}"))
        acts.append(Act("done", "d", partial(rb.mark_done, state, book, step)))
    ref = rb.PR_REF.match(step["where"])
    if ref:
        acts.append(Act(f"open {step['where']}", "o", partial(rb.open_pr, state, ref[1], int(ref[2])), not acts))
    if step["command"]:
        acts.append(Act("copy command", "c", partial(rb.copy, state, step["command"]), not acts))
    return acts


def book_acts(app, book):
    state = app.state
    return [Act("refresh", "R", partial(rb.update, state, book), True), Act("talk", "t", partial(rb.talk, state, book)),
            Act("delete", "D", partial(rb.delete, state, book), False, f"delete-{book['path']}")]


# ---------------------------------------------------------------------------
# Steps: one row widget each, so a row can animate on its own


class StepRow(Horizontal):
    can_focus = True

    def __init__(self, book, step):
        super().__init__()
        self.book, self.step, self.spinner, self.frames = book, step, None, 0

    def compose(self) -> ComposeResult:
        yield Static(classes="n")
        yield Static(classes="status")
        yield Static(classes="what")
        yield Static(classes="where")

    def on_mount(self):
        self.paint()

    def set(self, book, step):
        self.book, self.step = book, step
        if not self.spinner:
            self.paint()

    def paint(self, frame=None, spark=False):
        step, state = self.step, self.app.state
        is_done = not step["status"].startswith(rb.OPEN)
        self.query_one(".n", Static).update(Text(step["n"], DIM))
        if frame:
            status = Text(f"{frame} {step['status'].split(' ', 1)[-1]}", ORANGE)
        elif spark:
            status = Text(f"{step['status']} ✦", f"bold {GREEN}")
        else:
            status = Text(step["status"], DIM if is_done else "")
        self.query_one(".status", Static).update(status)
        what = Text(step["step"], DIM if is_done else "")
        if step["command"]:
            what.append(f"\n$ {step['command']}", DIM if is_done else ORANGE)
        handoff = state["handoffs"].get(rb.handoff_id(self.book, step))
        said = rb.handoff_line(handoff) if handoff else None
        if said:
            what.append(f"\n{said[0]}", STYLE.get(said[1], ""))
        self.query_one(".what", Static).update(what)
        ref = rb.PR_REF.match(step["where"])
        where = Text(step["where"], f"underline {DIM}" if ref else DIM)
        if ref:
            badge, badge_style = rb.live_badge(ref[1], int(ref[2]), self.app.inbox)
            where.append(f"\n{badge.strip()}", STYLE.get(badge_style, ""))
        self.query_one(".where", Static).update(where)

    def play(self, delay):
        """The step turning done: a spinner, then a teal flash fading out, a spark for the first second."""
        self.set_timer(max(delay, 0.01), self.spin)

    def spin(self):
        self.frames = 0
        self.spinner = self.set_interval(0.07, self.tick)

    def tick(self):
        self.frames += 1
        if self.frames * 0.07 < 0.5:
            self.paint(frame=rb.SPINNER[self.frames % len(rb.SPINNER)])
            return
        self.spinner.stop()
        self.spinner = None
        self.paint(spark=True)
        # The cells too, or their text keeps the background it was drawn on.
        for part in (self, *self.query(Static)):
            part.styles.background = FLASH_FROM
            part.styles.animate("background", FLASH_TO, duration=rb.FLASH - 0.5, easing="out_cubic", on_complete=self.settle if part is self else None)
        self.set_timer(1.0, self.paint)

    def settle(self):
        for part in (self, *self.query(Static)):
            part.styles.clear_rule("background")

    def on_focus(self):
        self.app.call_later(self.app.show_step)

    def on_click(self):
        self.focus()


class StepList(VerticalScroll):
    def __init__(self, book):
        super().__init__()
        self.book = book

    def compose(self) -> ComposeResult:
        with Horizontal(classes="columns"):
            for label, kind in (("#", "n"), ("status", "status"), ("step", "what"), ("where", "where")):
                yield Static(Text(label, DIM), classes=kind)

    async def show(self, book):
        """Rows updated in place while the steps stay the same, so an animation running on one keeps going."""
        self.book = book
        rows = list(self.query(StepRow))
        if [r.step["n"] for r in rows] == [s["n"] for s in book["steps"]]:
            for row, step in zip(rows, book["steps"]):
                row.set(book, step)
        else:
            await self.remove_children(StepRow)
            await self.mount_all([StepRow(book, step) for step in book["steps"]])
        longest = max([len(s["where"]) for s in book["steps"]] + [8])
        for cell in self.query(".where"):
            cell.styles.width = min(longest + 2, 24)

    def play(self, steps):
        rows = {r.step["n"]: r for r in self.query(StepRow)}
        for i, n in enumerate(steps):
            if n in rows:
                rows[n].play(i * rb.CASCADE)
        if steps and steps[0] in rows:
            self.scroll_to_widget(rows[steps[0]], animate=True)


class StepCommands(Provider):
    def hits(self, main_only):
        app = self.app
        for book in app.state["books"]:
            name = book["slug"].upper()
            for act in book_acts(app, book):
                if not main_only or act.main:
                    yield f"{act.label}  {name} runbook", partial(app.act, act, f"{act.label} the {name} runbook"), book["title"]
            for step in book["steps"]:
                if main_only:
                    continue
                for act in step_acts(app, book, step):
                    yield f"{act.label}  {name} #{step['n']} {step['step']}", partial(app.act, act, f"{act.label} {name} step {step['n']}: {step['step']}"), step["status"]
                yield f"go to  {name} #{step['n']} {step['step']}", partial(app.go_to, book["slug"], step["n"]), step["status"]

    async def discover(self):
        for text, callback, help_ in self.hits(main_only=True):
            yield DiscoveryHit(text, callback, help=help_)

    async def search(self, query):
        matcher = self.matcher(query)
        for text, callback, help_ in self.hits(main_only=False):
            if (score := matcher.match(text)) > 0:
                yield Hit(score, matcher.highlight(text), callback, help=help_)


# ---------------------------------------------------------------------------
# The app


class RunbookApp(App):
    TITLE = "runbook"
    COMMANDS = App.COMMANDS | {StepCommands}
    CSS = """
    Screen { background: $background; }
    #head { height: 1; padding: 0 1; }
    TabbedContent, TabPane { background: $background; }
    TabbedContent, TabbedContent > ContentSwitcher, TabPane { height: 1fr; }
    TabPane { padding: 0 1; }
    .sub { height: 1; }
    .sub Static { width: auto; }
    .sub LoadingIndicator { width: 8; height: 1; color: $primary; }
    ProgressBar { height: 1; margin-bottom: 1; }
    Bar > .bar--bar, Bar > .bar--complete { color: $success; }
    StepList { height: 1fr; }
    StepList Horizontal { height: auto; }
    StepRow { padding: 0 0 1 0; }
    StepRow:focus { background: $panel; }
    StepList Static { height: auto; padding-right: 1; }
    .n { width: 4; }
    .status { width: 32%; }
    .what { width: 1fr; }
    .where { width: 22; }
    .columns { margin-bottom: 1; }
    .notes { height: auto; max-height: 6; margin: 1 0 0 0; padding: 0; background: $background; }
    #panel { height: auto; max-height: 12; border-top: solid $panel; padding: 0 1; }
    #actions { height: auto; margin-top: 1; }
    #actions Button { margin-right: 1; }
    Sure { align: center middle; }
    .dialog { width: 64; height: auto; border: round $primary; background: $surface; padding: 1 2; }
    .dialog-buttons { height: auto; margin-top: 1; }
    .dialog-buttons Button { margin-right: 2; }
    """
    BINDINGS = [Binding("q", "quit", "quit"), Binding("j,down", "row(1)", "next step", show=False), Binding("k,up", "row(-1)", "previous step", show=False),
                Binding("enter", "primary", "act", show=False), Binding("R", "book('refresh')", "refresh"), Binding("t", "book('talk')", "talk"),
                Binding("D", "book('delete')", "delete", show=False), Binding("p", "tab(-1)", "prev runbook"), Binding("n", "tab(1)", "next runbook")]

    def __init__(self):
        super().__init__()
        self.state = view.new_state(books=rb.read_books(), toggled=set(), refreshing={}, refresh_started={}, flash={}, newly={},
                                    announced=set(), focus_at=time.time(), reasons={}, seen=None, was_busy=False, handoffs={})
        self.slugs, self.ended, self.played, self.inbox = [], set(), set(), {}

    def compose(self) -> ComposeResult:
        yield Static(id="head")
        yield TabbedContent()
        with Vertical(id="panel"):
            yield Static(id="info")
            yield Horizontal(id="actions")
        yield Footer()

    async def on_mount(self):
        self.register_theme(VESPER)
        self.theme = "vesper"
        await self.reload()
        self.set_interval(1, self.reload)

    async def reload(self):
        """Runbook files changing redraw the tabs and play the steps that turned done; a refresh ticks its timer, and says when it ends.
        In its pinned space, the app also takes the mod's refresh requests and notification clicks, like runbook.py."""
        state, now = self.state, rb.stamp()
        changed = now != state["seen"]
        if changed:
            state["seen"] = now
            rb.reload(state)
            if PINNED:
                rb.report(state["books"])
        replay = None
        if PINNED:
            if rb.take_focus(state):
                replay = (state["scroll_to"], [n for slug, n in state["flash"] if slug == state["scroll_to"]])
            rb.take_requests(state)
        state["flash"] = {}
        busy = any(proc.poll() is None for proc in state["refreshing"].values())
        if changed or busy:
            await self.draw()
        tabs = self.query_one(TabbedContent)
        played = [(slug, [n for n in steps if (slug, n) not in self.played]) for slug, steps in state["newly"].items()]
        for slug, steps in played + ([replay] if replay else []):
            if steps and slug in self.slugs:
                tabs.active = f"book-{slug}"
                tabs.get_pane(f"book-{slug}").query_one(StepList).play(steps)
                self.notify(f"{slug.upper()}: step{'s' if len(steps) > 1 else ''} {', '.join(steps)} done", timeout=5)
                self.played |= {(slug, n) for n in steps}
        # A refresh's newly done steps stay until it ends, for its notification to count them.
        for slug in list(state["newly"]):
            proc = state["refreshing"].get(slug)
            if not proc or (proc.poll() is not None and (slug, state["refresh_started"].get(slug)) in self.ended):
                del state["newly"][slug]
        for slug, proc in state["refreshing"].items():
            run = (slug, state["refresh_started"].get(slug))
            if proc.poll() is not None and run not in self.ended:
                self.ended.add(run)
                self.notify(f"{slug.upper()} refreshed" if proc.returncode == 0 else f"{slug.upper()}: refresh failed, see its log",
                            severity="information" if proc.returncode == 0 else "error")
                if PINNED and proc.returncode == 0:
                    state["announced"].add(slug)
                    rb.announce(state, slug)
                state["newly"].pop(slug, None)
        # A step open again can play again once it is done.
        self.played = {(slug, n) for slug, n in self.played
                       if any(b["slug"] == slug and s["n"] == n and not s["status"].startswith(rb.OPEN) for b in state["books"] for s in b["steps"])}

    async def draw(self):
        state, books = self.state, self.state["books"]
        self.inbox = view.load_json(rb.INBOX, {})
        left = sum(1 for b in books for s in b["steps"] if s["status"].startswith(rb.OPEN))
        self.query_one("#head", Static).update(Text.assemble((f"{rb.ICON['runbook']}  ", ORANGE), ("Runbooks", f"bold {ORANGE}"),
                                                             (f"   {len(books)} · {left} steps left", DIM)))
        tabs = self.query_one(TabbedContent)
        slugs = [b["slug"] for b in books]
        if slugs != self.slugs:
            active = tabs.active
            await tabs.clear_panes()
            for book in books:
                await tabs.add_pane(TabPane(book["slug"].upper(), Horizontal(Static(), LoadingIndicator(), classes="sub"), ProgressBar(show_eta=False),
                                            StepList(book), Markdown("", classes="notes"), id=f"book-{book['slug']}"))
            self.slugs = slugs
            if active in [f"book-{s}" for s in slugs]:
                tabs.active = active
        for book in books:
            pane = tabs.get_pane(f"book-{book['slug']}")
            todo = sum(1 for s in book["steps"] if s["status"].startswith(rb.OPEN))
            tabs.get_tab(pane.id).label = Text.assemble(book["slug"].upper(), (f" {todo}" if todo else " ✓", ORANGE if todo else GREEN))
            status = rb.refresh_status(state, book)
            pane.query_one(".sub Static", Static).update(Text.assemble(
                (book["title"] or book["slug"], f"bold {ORANGE if todo else GREEN}"), (f"   updated {book['updated']}", DIM),
                (f"   {status[0]}", STYLE.get(status[1], "")) if status else ("   R refreshes", DIM)))
            pane.query_one(LoadingIndicator).display = bool(status and status[1] == view.ORANGE)
            bar = pane.query_one(ProgressBar)
            bar.update(total=len(book["steps"]) or 1)
            bar.animate("progress", float(len(book["steps"]) - todo), duration=0.9, easing="out_cubic")
            await pane.query_one(StepList).show(book)
            pane.query_one(".notes", Markdown).update("\n".join(f"- {n}" for n in book["notes"]))
        if not isinstance(self.focused, StepRow):
            self.focus_first()
        await self.show_step()

    def focus_first(self):
        tabs = self.query_one(TabbedContent)
        if tabs.active:
            rows = tabs.get_pane(tabs.active).query(StepRow)
            if rows:
                rows.first().focus(scroll_visible=False)

    def current(self):
        if isinstance(self.focused, StepRow):
            return self.focused.book, self.focused.step
        tabs = self.query_one(TabbedContent)
        book = next((b for b in self.state["books"] if f"book-{b['slug']}" == tabs.active), None)
        return book, (book["steps"][0] if book and book["steps"] else None)

    async def show_step(self):
        book, step = self.current()
        info = self.query_one("#info", Static)
        bar = self.query_one("#actions", Horizontal)
        await bar.remove_children()
        if not step:
            info.update(Text("No runbook yet: run /runbook in a Claude session working on a ticket.", DIM) if not book else Text(""))
            return
        lines = [Text.assemble((f"#{step['n']}  ", DIM), (step["step"], "bold")), Text.assemble(("status ", DIM), step["status"])]
        if step["command"]:
            lines.append(Text.assemble(("$ ", DIM), (step["command"], ORANGE)))
        info.update(Text("\n").join(lines))
        await bar.mount_all([Button(f"{a.label}  {a.key}", name=str(n), variant="primary" if a.main else "default", compact=True)
                             for n, a in enumerate(step_acts(self, book, step))])

    @on(TabbedContent.TabActivated)
    def activated(self):
        self.call_after_refresh(self.focus_first)

    @on(Button.Pressed, "#actions Button")
    def pressed(self, event):
        book, step = self.current()
        act = step_acts(self, book, step)[int(event.button.name)]
        self.act(act, f"{act.label} {book['slug'].upper()} step {step['n']}: {step['step']}")

    def on_key(self, event):
        book, step = self.current()
        act = next((a for a in (step_acts(self, book, step) if step else []) if a.key == event.character), None)
        if act:
            event.stop()
            self.act(act, f"{act.label} {book['slug'].upper()} step {step['n']}: {step['step']}")

    def act(self, act, question):
        if act.confirm:
            self.push_screen(Sure(f"{question}?"), lambda yes: yes and self.perform(partial(self.armed, act)))
        else:
            self.perform(act.run)

    def armed(self, act):
        # runbook.py's actions run on their second press within a few seconds; the dialog was the first.
        self.state["confirm"] = (act.confirm, time.time())
        act.run()

    @work(thread=True)
    def perform(self, run):
        before = self.state["note_at"]
        run()
        if self.state["note_at"] != before:
            self.call_from_thread(self.notify, self.state["note"])

    def action_row(self, step):
        (self.screen.focus_next if step > 0 else self.screen.focus_previous)(StepRow)

    def action_primary(self):
        book, step = self.current()
        acts = step_acts(self, book, step) if step else []
        if acts:
            self.act(acts[0], f"{acts[0].label} {book['slug'].upper()} step {step['n']}: {step['step']}")

    def action_book(self, label):
        book, _ = self.current()
        act = next((a for a in book_acts(self, book) if a.label == label), None) if book else None
        if act:
            self.act(act, f"{label} the {book['slug'].upper()} runbook")

    def action_tab(self, step):
        tabs = self.query_one(TabbedContent)
        if self.slugs:
            n = [f"book-{s}" for s in self.slugs].index(tabs.active) if tabs.active else 0
            tabs.active = f"book-{self.slugs[(n + step) % len(self.slugs)]}"

    def go_to(self, slug, n):
        tabs = self.query_one(TabbedContent)
        tabs.active = f"book-{slug}"
        row = next((r for r in tabs.get_pane(tabs.active).query(StepRow) if r.step["n"] == n), None)
        if row:
            self.call_after_refresh(row.focus)


if __name__ == "__main__":
    RunbookApp().run()
