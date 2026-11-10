// The runbooks: the `runbook` view the hub pushes, a pill per runbook with its progress, the steps with their status,
// command, session and PR, the selected step's details beside them, and every key and action of the TUI app (run,
// done, open, copy, edit, add, refresh, talk, delete, pick several, filter one or every runbook, a command list on
// ctrl+p). With `scope: "focused"` (the right panel) it shows the focused agent's ticket's runbook alone.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { props, focused, panes, shared, settings } from "herdr";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Badge, Button, Kbd, ListRow, PageHeader, SearchField, TextField, KeyHints, Empty, ErrorState,
  Skeleton, Command, Toaster, Flashes, Scroller, Hover, Progress, Ring, Section, SidePanel, AlertDialog, Spinner, openMenu, alpha,
} from "./ui.js";

const mono = () => settings().font ?? "JetBrains Mono";
const ADDER = "\u0000adder";
const TONE = {
  done: ["success", "circle-check", (k) => k.success],
  live: ["info", "rocket", (k) => k.info],
  progress: ["warning", "circle-dot", (k) => k.working],
  waiting: ["warning", "hourglass", (k) => k.warning],
  todo: ["outline", "circle", (k) => k.mutedFg],
  blocked: ["destructive", "circle-x", (k) => k.destructive],
  other: ["secondary", "circle", (k) => k.mutedFg],
};
const PRESETS = [
  ["⬜ to do", "todo"],
  ["🟡 in progress", "progress"],
  ["⏳ waiting", "waiting"],
  ["🚀 live", "live"],
  ["❌ blocked", "blocked"],
];
const COLUMN = { command: "Command", status: "Status", step: "Step", where: "Where" };

/** A model action's key ("R") as gpui spells the keystroke ("shift-r"). */
const stroke = (key) => (key && key !== key.toLowerCase() ? `shift-${key.toLowerCase()}` : key);

export default class Runbook {
  init(p, cx) {
    const given = props() ?? {};
    this.panel = given.scope === "focused";
    this.ticket = given.ticket ? String(given.ticket).toLowerCase() : null;
    this.scope = this.panel ? "runbook-panel" : "runbook";
    this.model = null;
    this.agents = [];
    this.status = "connecting";
    this.slug = null;
    this.sel = null;
    this.picked = new Set();
    this.filter = "";
    this.filtering = false;
    this.global = false;
    this.editing = null; // { key, column }
    this.adding = null; // { slug, after, at }
    this.confirm = null; // { title, description, label, destructive, run }
    this.palette = null; // { at }
    this.notesOpen = true;
    this.showDetails = true;
    this.width = 1000;
    this.lastTerminal = null;
    this.polling = false;
    this.hover = new Hover();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();

    this.editor = InputState.new({ placeholder: "" });
    this.editor.on("submit", (_e, cx) => this.commitEdit(cx));
    this.fields = ["What the step does", "Where: a repo, prod, repo#123 for a PR", "The command, if any"].map((placeholder, i) => {
      const state = InputState.new({ placeholder });
      state.on("submit", (_e, cx) => (i < 2 ? (this.adding && (this.adding.at = i + 1), this.fields[i + 1].focus(), cx.notify()) : this.commitAdd(cx)));
      return state;
    });
    this.search = InputState.new({ placeholder: "Filter steps…" });
    this.search.on("change", (_e, cx) => {
      this.filter = this.search.value();
      this.afterFilter();
      cx.notify();
    });
    this.search.on("submit", (_e, cx) => {
      this.filtering = false;
      this.handle.focus();
      cx.notify();
    });
    this.query = InputState.new({ placeholder: "Type a command or search…" });
    this.query.on("change", (_e, cx) => {
      if (this.palette) this.palette.at = 0;
      cx.notify();
    });
    this.query.on("submit", (_e, cx) => this.runPalette(cx));

    this.sub = hub.subscribe(
      cx,
      { views: this.panel ? ["runbook", "agents"] : ["runbook"], focus: this.panel ? null : "runbook" },
      (m, cx) => {
        if (m.view === "runbook") this.take(m.value, cx);
        if (m.view === "agents") this.agents = m.value?.agents ?? m.value?.rows ?? (Array.isArray(m.value) ? m.value : []);
        if (m.focus === "runbook" && !this.panel) this.goTo(m.target, cx);
        cx.notify();
      },
      (state, cx) => {
        this.status = state;
        cx.notify();
      },
    );
  }

  close() {
    this.sub?.stop();
  }

  command(c, cx) {
    if ((c === "focus" || c === "show:runbook") && !this.panel) this.handle.focus();
    const [scope, action, key, extra] = c.split(":");
    if (scope === this.scope) this.menuAction(action, key, extra, cx);
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The model

  books() {
    return this.model?.books ?? [];
  }

  bookOf(slug) {
    return this.books().find((b) => b.slug === slug);
  }

  /** The runbook shown: the focused agent's in the panel, else the tab picked. */
  book() {
    if (this.panel) return this.bookOf(this.focusSlug());
    return this.bookOf(this.slug) ?? this.books()[0];
  }

  stepOf(key) {
    const slug = key?.split("#")[0];
    return this.bookOf(slug)?.steps.find((s) => s.key === key);
  }

  words() {
    return this.filter.toLowerCase().split(/\s+/).filter(Boolean);
  }

  /** Each word typed is found in one of the step's fields. */
  matches(step) {
    const fields = [step.n, step.step, step.where, step.status, step.command].map((f) => (f ?? "").toLowerCase());
    return this.words().every((w) => fields.some((f) => f.includes(w)));
  }

  shown(book = this.book()) {
    return (book?.steps ?? []).filter((s) => this.matches(s));
  }

  matchCount(book) {
    return this.words().length ? this.shown(book).length : null;
  }

  selectable() {
    const keys = this.shown().map((s) => s.key);
    return this.panel || this.filter ? keys : [...keys, ADDER];
  }

  selected() {
    return this.sel && this.sel !== ADDER ? this.stepOf(this.sel) : null;
  }

  selIndex() {
    return Math.max(0, this.selectable().indexOf(this.sel));
  }

  selectAt(i) {
    const all = this.selectable();
    this.sel = all[Math.max(0, Math.min(i, all.length - 1))] ?? null;
  }

  /** The first open step of the book, else its first. */
  firstOf(book) {
    const steps = this.shown(book);
    return (steps.find((s) => s.open) ?? steps[0])?.key ?? null;
  }

  take(model, cx) {
    const before = this.model;
    this.model = model;
    if (!before) {
      const first = this.books().find((b) => b.todo > 0) ?? this.books()[0];
      this.slug = first?.slug ?? null;
      this.sel = this.firstOf(first);
      this.watchRefreshes(cx);
      return;
    }
    const k = tokens();
    const was = new Map(before.books.map((b) => [b.slug, b]));
    for (const book of this.books()) {
      const old = was.get(book.slug);
      if (!old) {
        if (before.books.length) this.toasts.push(cx, `New runbook: ${book.name}`, { description: book.title });
        continue;
      }
      const steps = new Map(old.steps.map((s) => [s.key, s]));
      const newly = [];
      for (const step of book.steps) {
        const prior = steps.get(step.key);
        if (!prior) this.flashes.start(cx, step.key, k.ring);
        else if (prior.open && !step.open) newly.push(step.n);
        if (step.session?.failed && (!prior?.session?.failed || prior.session.state !== step.session.state)) {
          this.flashes.start(cx, step.key, k.destructive);
          this.toasts.push(cx, `${book.name} step ${step.n} ${step.session.state}`, { kind: "error", description: step.session.message ?? "" });
        }
      }
      if (newly.length) this.played(book, newly, cx);
      if (old.refresh?.running && !book.refresh?.running) this.refreshEnded(book, old, cx);
    }
    if (!this.bookOf(this.slug)) this.slug = this.books()[0]?.slug ?? null;
    if (this.sel !== ADDER && !this.stepOf(this.sel)) this.sel = this.firstOf(this.book());
    for (const key of [...this.picked]) if (!this.stepOf(key)?.open) this.picked.delete(key);
    this.watchRefreshes(cx);
  }

  /** Steps that turned done: they flash one after the other where they show, else a toast offers to show them. */
  played(book, ns, cx) {
    const s = ns.length > 1 ? "s" : "";
    const title = `${book.name}: step${s} ${ns.join(", ")} done`;
    if (this.book()?.slug === book.slug) {
      this.cascade(book, ns, cx);
      this.toasts.push(cx, title, { kind: "success" });
    } else if (!this.panel) {
      this.toasts.push(cx, title, { kind: "success", action: { label: "Show", onClick: (_e, cx) => this.play(book.slug, ns, cx) } });
    }
  }

  cascade(book, ns, cx) {
    const k = tokens();
    ns.forEach((n, i) => cx.timer.after(i * 220, (cx) => this.flashes.start(cx, `${book.slug}#${n}`, k.success)));
  }

  /** A runbook brought into view with these steps playing as done (a notification's click). */
  play(slug, ns, cx) {
    const book = this.bookOf(slug);
    if (!book) return;
    this.slug = slug;
    const steps = ns.length ? ns : book.steps.filter((s) => !s.open).map((s) => s.n);
    if (steps.length) this.sel = `${slug}#${steps[0]}`;
    this.cascade(book, steps, cx);
    cx.notify();
  }

  refreshEnded(book, old, cx) {
    const ok = (book.refresh.code ?? 0) === 0;
    if (ok) this.toasts.push(cx, `${book.name} refreshed`, { kind: "success", description: book.done > old.done ? `${book.done - old.done} more done · ${book.todo} to do` : `${book.todo} to do` });
    else this.toasts.push(cx, `${book.name}: refresh failed`, { kind: "error", description: book.refresh.log, ms: 8000 });
    // Away from the runbooks, the hub's notification says so (the TUI's runbook space does the same).
    if (!this.panel && ok && shared("cockpit.app") !== "runbook") {
      const newly = book.steps.filter((s) => !s.open && old.steps.find((o) => o.key === s.key)?.open).map((s) => s.n);
      hub.act("runbook", "announce", [book.slug, newly, ""]).catch(() => {});
    }
  }

  /** While a refresh runs, the view is asked again every two seconds (its end lands in the cache, not a watched folder). */
  watchRefreshes(cx) {
    if (this.polling || !this.books().some((b) => b.refresh?.running)) return;
    this.polling = true;
    cx.timer.after(2000, (cx) =>
      cx.spawn(async (cx) => {
        this.polling = false;
        try {
          this.take(await hub.view("runbook"), cx);
        } catch {}
        cx.notify();
      }),
    );
  }

  goTo(target, cx) {
    const key = target?.key ?? "";
    const [slug, n] = key.split("#");
    if (!this.bookOf(slug)) {
      if (slug) this.toasts.push(cx, `No runbook ${slug} any more`);
      return;
    }
    if (target.steps?.length) return this.play(slug, target.steps, cx);
    this.slug = slug;
    const step = n ? this.stepOf(key) : null;
    if (step && !this.matches(step)) this.clearFilter();
    this.sel = step ? step.key : this.firstOf(this.bookOf(slug));
    if (step) this.flashes.start(cx, step.key, tokens().ring);
    this.handle.focus();
  }

  /** The focused agent's ticket, as a runbook's slug: from the agents view, else the branch or folder it works in. */
  focusSlug() {
    if (this.ticket) return this.ticket;
    const f = focused();
    if (f?.terminal) this.lastTerminal = f.terminal;
    const terminal = this.lastTerminal;
    if (!terminal) return null;
    const row = this.agents.find((a) => a.terminal === terminal || a.address === terminal || String(a.address ?? "").endsWith(`:${terminal}`));
    const ticket = typeof row?.ticket === "string" ? row.ticket : row?.ticket?.id ?? row?.ticket?.key;
    if (ticket && this.bookOf(ticket.toLowerCase())) return ticket.toLowerCase();
    const pane = panes().find((p) => p.terminal === terminal);
    const where = `${pane?.branch ?? row?.branch ?? ""} ${pane?.cwd ?? ""}`.toLowerCase();
    const found = this.books().filter((b) => where.includes(b.slug)).sort((a, b) => b.slug.length - a.slug.length)[0];
    return found?.slug ?? null;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async hubAct(cx, action, args, armed) {
    try {
      const r = await hub.request("act", { app: "runbook", action, args, armed });
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note) this.toasts.push(cx, r.note, { kind: "success" });
      if (r.refreshing && Object.keys(r.refreshing).length) cx.timer.after(400, (cx) => cx.spawn(async (cx) => (this.take(await hub.view("runbook"), cx), cx.notify())));
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    }
    cx.notify();
  }

  ask(cx, confirm) {
    this.confirm = confirm;
    this.handle.focus();
    cx.notify();
  }

  /** A step's action from the model, as its button or key runs it. */
  stepAct(cx, act, step) {
    const book = this.bookOf(step.key.split("#")[0]);
    if (!book) return;
    const stepArg = { n: step.n, step: step.step, where: step.where, status: step.status, command: step.command };
    switch (act.id) {
      case "run":
        return this.ask(cx, {
          title: `Run ${book.name} step ${step.n}?`,
          description: `${step.step}. Its session checks the preconditions first, and waits for your go before anything that touches production.`,
          label: "Run",
          run: (cx) => this.hubAct(cx, "run_step", [{ $book: book.slug }, stepArg], act.confirm),
        });
      case "session":
        return cx.spawn(async (cx) => this.hubAct(cx, "focus_session", [{ $handoff: `${book.slug}-${step.n}` }]));
      case "done":
        return this.markDone(cx, [[book.slug, step.n]]);
      case "open":
        return step.pr && cx.spawn(async (cx) => this.hubAct(cx, "open_pr", [step.pr.repo, step.pr.number]));
      case "copy":
        return this.copy(cx, [step.command]);
      case "edit":
        return this.startEdit(cx, step.key, "command");
    }
  }

  bookAct(cx, act, book) {
    switch (act.id) {
      case "refresh":
        if (book.refresh?.running) return this.toasts.push(cx, `${book.name} is already refreshing`);
        return cx.spawn(async (cx) => this.hubAct(cx, "update", [{ $book: book.slug }]));
      case "talk":
        return cx.spawn(async (cx) => this.hubAct(cx, "talk", [{ $book: book.slug }]));
      case "delete":
        return this.ask(cx, {
          title: `Delete the ${book.name} runbook?`,
          description: `${book.title}: its file goes, with its ${book.total} steps.`,
          label: "Delete",
          destructive: true,
          run: (cx) => this.hubAct(cx, "delete", [{ $book: book.slug }], act.confirm),
        });
    }
  }

  async viewAct(cx, action, args, undoable = false) {
    try {
      const r = await hub.act("runbook", action, args);
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note)
        this.toasts.push(cx, r.note, {
          kind: "success",
          ms: undoable ? 6000 : 3500,
          action: undoable ? { label: "Undo", onClick: (_e, cx) => cx.spawn(async (cx) => this.viewAct(cx, "undo", [])) } : undefined,
        });
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    }
    cx.notify();
  }

  /** Marks steps done at once; the toast's Undo (or u) puts them back. */
  markDone(cx, pairs) {
    const open = pairs.filter(([slug, n]) => this.stepOf(`${slug}#${n}`)?.open);
    if (!open.length) return this.toasts.push(cx, pairs.length > 1 ? "The picked steps are done already" : "That step is done already");
    this.picked.clear();
    cx.spawn(async (cx) => this.viewAct(cx, "done", [open], true));
  }

  copy(cx, commands) {
    const list = commands.filter(Boolean);
    if (!list.length) return this.toasts.push(cx, "No command to copy");
    cx.write_to_clipboard(list.join("\n"));
    this.toasts.push(cx, list.length === 1 ? "Copied" : `Copied ${list.length} commands`, { kind: "success", description: list.length === 1 ? list[0] : undefined });
  }

  /** The picked steps in table order, else the selected one, as [slug, n]. */
  targets() {
    if (this.picked.size) return this.books().flatMap((b) => b.steps.filter((s) => this.picked.has(s.key)).map((s) => [b.slug, s.n]));
    const step = this.selected();
    return step ? [[step.key.split("#")[0], step.n]] : [];
  }

  pick(cx, key = this.sel) {
    if (!this.stepOf(key)) return;
    if (!this.picked.delete(key)) this.picked.add(key);
    cx.notify();
  }

  pickOpen(cx) {
    const open = this.shown().filter((s) => s.open).map((s) => s.key);
    if (open.every((k) => this.picked.has(k))) open.forEach((k) => this.picked.delete(k));
    else open.forEach((k) => this.picked.add(k));
    cx.notify();
  }

  startEdit(cx, key, column) {
    const step = this.stepOf(key);
    if (!step) return;
    this.sel = key;
    this.editing = { key, column };
    this.editor.set_value(column === "status" ? step.status : column === "where" ? step.where : column === "step" ? step.step : step.command);
    this.editor.focus();
    cx.notify();
  }

  commitEdit(cx) {
    const e = this.editing;
    if (!e) return;
    this.editing = null;
    this.handle.focus();
    const [slug, n] = [e.key.split("#")[0], this.stepOf(e.key)?.n];
    if (n !== undefined) cx.spawn(async (cx) => this.viewAct(cx, "set", [slug, n, e.column, this.editor.value()], true));
    cx.notify();
  }

  setStatus(cx, key, preset) {
    const step = this.stepOf(key);
    if (!step) return;
    cx.spawn(async (cx) => this.viewAct(cx, "set", [key.split("#")[0], step.n, "status", preset], true));
  }

  startAdd(cx) {
    const book = this.book();
    if (!book) return;
    const after = this.selected()?.n ?? book.steps[book.steps.length - 1]?.n ?? null;
    this.adding = { slug: book.slug, after, at: 0 };
    this.fields.forEach((f) => f.set_value(""));
    this.fields[0].focus();
    cx.notify();
  }

  commitAdd(cx) {
    const a = this.adding;
    if (!a) return;
    const [step, where, command] = this.fields.map((f) => f.value());
    if (!step.trim()) {
      this.toasts.push(cx, "A step needs its text");
      this.fields[0].focus();
      return;
    }
    this.adding = null;
    this.handle.focus();
    const before = new Set(this.bookOf(a.slug)?.steps.map((s) => s.step));
    cx.spawn(async (cx) => {
      await this.viewAct(cx, "add", [a.slug, a.after, step, where, command], true);
      // The new step is the one whose text was not there.
      const added = this.bookOf(a.slug)?.steps.find((s) => s.step === step.trim() && !before.has(s.step));
      if (added) this.sel = added.key;
      cx.notify();
    });
    cx.notify();
  }

  cancel(cx) {
    this.editing = null;
    this.adding = null;
    this.handle.focus();
    cx.notify();
  }

  clearFilter() {
    this.filter = "";
    this.search.set_value("");
    this.global = false;
  }

  /** Across every runbook, a tab without a match gives way to the first that has one. */
  afterFilter() {
    if (this.global && this.words().length && !this.shown().length) {
      const other = this.books().find((b) => this.shown(b).length);
      if (other) this.slug = other.slug;
    }
    if (!this.selectable().includes(this.sel)) this.sel = this.firstOf(this.book());
  }

  openFilter(cx, global) {
    this.filtering = true;
    if (global) this.global = true;
    this.afterFilter();
    this.search.focus();
    cx.notify();
  }

  nextBook(cx, by) {
    const books = this.books();
    if (!books.length || this.panel) return;
    const at = Math.max(0, books.findIndex((b) => b.slug === this.book()?.slug));
    this.slug = books[(at + by + books.length) % books.length].slug;
    this.sel = this.firstOf(this.book());
    cx.notify();
  }

  selectBook(cx, slug) {
    this.slug = slug;
    this.sel = this.firstOf(this.book());
    this.handle.focus();
    cx.notify();
  }

  menuAction(action, key, extra, cx) {
    const step = this.stepOf(key);
    const book = this.bookOf(key?.split("#")[0]);
    if (action === "book" && book) {
      const act = book.acts.find((a) => a.id === extra);
      if (act) this.bookAct(cx, act, book);
      return;
    }
    if (!step) return;
    this.sel = key;
    if (action === "act") {
      const act = step.acts.find((a) => a.id === extra);
      if (act) this.stepAct(cx, act, step);
    } else if (action === "edit") this.startEdit(cx, key, extra);
    else if (action === "status") this.setStatus(cx, key, PRESETS[Number(extra)]?.[0]);
    else if (action === "done") this.markDone(cx, [[key.split("#")[0], step.n]]);
    else if (action === "pick") this.pick(cx, key);
    else if (action === "add") this.startAdd(cx);
  }

  menu(step) {
    const s = this.scope;
    const acts = step.acts.filter((a) => a.id !== "edit");
    openMenu(
      [
        ...acts.map((a) => ({ label: cap(a.label), icon: ACT_ICON[a.id], kbd: a.key, command: `${s}:act:${step.key}:${a.id}` })),
        "separator",
        { label: "Edit command", icon: "pencil", kbd: "e", command: `${s}:edit:${step.key}:command` },
        { label: "Edit status", icon: "pencil", kbd: "E", command: `${s}:edit:${step.key}:status` },
        { label: "Edit text", icon: "pencil", command: `${s}:edit:${step.key}:step` },
        { label: "Edit where", icon: "pencil", command: `${s}:edit:${step.key}:where` },
        "separator",
        ...PRESETS.map(([label], i) => ({ label: `Status: ${label}`, command: `${s}:status:${step.key}:${i}` })),
        "separator",
        step.open && { label: this.picked.has(step.key) ? "Unpick" : "Pick", icon: "square-check", kbd: "v", command: `${s}:pick:${step.key}` },
        !this.panel && { label: "Add a step below", icon: "plus", kbd: "a", command: `${s}:add:${step.key}` },
      ],
      { width: 240 },
    );
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p): every runbook's actions, then every step's and a jump to each

  paletteGroups(cx) {
    const q = this.query.value().toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (text) => q.every((w) => text.toLowerCase().includes(w));
    const books = [];
    const steps = [];
    for (const book of this.books()) {
      for (const act of book.acts) books.push({ id: `p-${book.slug}-${act.id}`, label: `${cap(act.label)}  ${book.name} runbook`, hint: book.title, icon: ACT_ICON[act.id], shortcut: act.key, main: act.main, run: (cx) => this.bookAct(cx, act, book) });
      for (const step of book.steps) {
        const what = `${book.name} #${step.n} ${step.step}`;
        for (const act of step.acts) steps.push({ id: `p-${step.key}-${act.id}`, label: `${cap(act.label)}  ${what}`, hint: step.status, icon: ACT_ICON[act.id], run: (cx) => ((this.slug = book.slug), (this.sel = step.key), this.stepAct(cx, act, step)) });
        steps.push({ id: `p-${step.key}-go`, label: `Go to  ${what}`, hint: step.status, icon: "chevron-right", run: (cx) => this.goTo({ key: step.key }, cx) });
      }
    }
    return [
      { heading: "Runbooks", items: books.filter((i) => (q.length ? hit(i.label) : i.main)) },
      { heading: "Steps", items: q.length ? steps.filter((i) => hit(`${i.label} ${i.hint}`)) : [] },
    ];
  }

  openPalette(cx) {
    this.palette = { at: 0 };
    this.query.set_value("");
    this.query.focus();
    cx.notify();
  }

  closePalette(cx) {
    this.palette = null;
    this.handle.focus();
    cx.notify();
  }

  runPalette(cx, picked) {
    const item = picked ?? this.paletteGroups(cx).flatMap((g) => g.items)[this.palette?.at ?? 0];
    this.closePalette(cx);
    if (item) item.run(cx);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

  key(e, cx) {
    if (this.panel && !this.handle.is_focused()) return false;
    const k = e.keystroke;
    if (this.confirm) {
      if (k === "escape") this.confirm = null;
      else if (k === "enter") {
        const run = this.confirm.run;
        this.confirm = null;
        cx.spawn(async (cx) => run(cx));
      } else return true;
      cx.notify();
      return true;
    }
    if (this.palette) {
      const n = this.paletteGroups(cx).flatMap((g) => g.items).length;
      if (k === "escape") this.closePalette(cx);
      else if (k === "down" || k === "ctrl-n") this.palette.at = Math.min(n - 1, this.palette.at + 1);
      else if (k === "up" || k === "ctrl-p") this.palette.at = Math.max(0, this.palette.at - 1);
      else return false;
      cx.notify();
      return true;
    }
    if (this.editing || this.adding) {
      if (k === "escape") this.cancel(cx);
      return k === "escape";
    }
    if (this.filtering) {
      if (k === "escape") {
        this.filtering = false;
        this.clearFilter();
        this.handle.focus();
        cx.notify();
        return true;
      }
      if (k === "down" || k === "up") {
        this.selectAt(this.selIndex() + (k === "down" ? 1 : -1));
        cx.notify();
        return true;
      }
      return false;
    }
    const step = this.selected();
    const book = this.book();
    const n = this.selectable().length;
    switch (k) {
      case "ctrl-p":
        if (!this.panel) this.openPalette(cx);
        break;
      case "j":
      case "down":
        this.selectAt(this.selIndex() + 1);
        break;
      case "k":
      case "up":
        this.selectAt(this.selIndex() - 1);
        break;
      case "pagedown":
      case "ctrl-d":
        this.selectAt(this.selIndex() + 10);
        break;
      case "pageup":
      case "ctrl-u":
        this.selectAt(this.selIndex() - 10);
        break;
      case "g":
      case "home":
        this.selectAt(0);
        break;
      case "shift-g":
      case "end":
        this.selectAt(n - 1);
        break;
      case "]":
        this.nextBook(cx, 1);
        break;
      case "[":
        this.nextBook(cx, -1);
        break;
      case "/":
        this.openFilter(cx, false);
        break;
      case "ctrl-/":
      case "ctrl-7":
      case "ctrl-_":
        this.openFilter(cx, true);
        break;
      case "escape":
        if (this.picked.size) this.picked.clear();
        else if (this.filter) this.clearFilter();
        else return false;
        break;
      case "v":
        this.pick(cx);
        break;
      case "shift-v":
        this.pickOpen(cx);
        break;
      case "u":
        cx.spawn(async (cx) => this.viewAct(cx, "undo", []));
        break;
      case "e":
        if (step) this.startEdit(cx, step.key, "command");
        break;
      case "shift-e":
        if (step) this.startEdit(cx, step.key, "status");
        break;
      case "a":
        if (!this.panel) this.startAdd(cx);
        break;
      case "m":
      case "shift-f10":
        if (step) this.menu(step);
        break;
      case "enter":
        if (this.sel === ADDER) this.startAdd(cx);
        else if (step) {
          const act = step.acts.find((a) => a.main) ?? step.acts[0];
          if (act) this.stepAct(cx, act, step);
        }
        break;
      default: {
        if (this.picked.size && k === "x") return this.markDone(cx, this.targets()), cx.notify(), true;
        if (this.picked.size && k === "c") return this.copy(cx, this.targets().map(([s, n]) => this.stepOf(`${s}#${n}`)?.command)), cx.notify(), true;
        const act = step?.acts.find((a) => stroke(a.key) === k);
        if (act) {
          this.stepAct(cx, act, step);
          break;
        }
        const bact = book?.acts.find((a) => stroke(a.key) === k);
        if (bact) {
          this.bookAct(cx, bact, book);
          break;
        }
        return false;
      }
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  /** The text with the filter's words tinted where they occur. */
  marked(text, k, { color, size } = {}) {
    const words = this.words();
    let el = h_flex().min_w(0).overflow_hidden().whitespace_nowrap();
    if (color) el = el.text_color(color);
    if (size) el = el.text_size(size);
    if (!words.length) return el.child(div().truncate().child(text));
    const lower = text.toLowerCase();
    const spans = [];
    let at = 0;
    while (at < text.length) {
      let next = null;
      for (const w of words) {
        const i = lower.indexOf(w, at);
        if (i >= 0 && (!next || i < next[0])) next = [i, w.length];
      }
      if (!next) {
        spans.push(div().child(text.slice(at)));
        break;
      }
      if (next[0] > at) spans.push(div().child(text.slice(at, next[0])));
      spans.push(div().rounded(2).bg(alpha(k.warning, 0.3)).child(text.slice(next[0], next[0] + next[1])));
      at = next[0] + next[1];
    }
    return el.children(spans);
  }

  statusBadge(step, { id } = {}) {
    const [variant, icon] = TONE[step.tone] ?? TONE.other;
    return Badge(step.label || step.status || "—", variant, {
      icon,
      id,
      tooltip: id ? "Set the status" : undefined,
      onClick: id ? () => this.menu(step) : undefined,
    });
  }

  prChip(step, k, id) {
    const pr = step.pr;
    const tone = pr.badge?.tone;
    return h_flex()
      .id(id)
      .h(22)
      .px(6)
      .gap(5)
      .flex_none()
      .items_center()
      .rounded(RADIUS.md)
      .border_1()
      .border_color(k.border)
      .cursor_pointer()
      .hover((s) => s.bg(k.accent))
      .tooltip(`Open ${step.where} in the browser  o`)
      .on_click((_e, cx) => {
        cx.stop_propagation();
        cx.spawn(async (cx) => this.hubAct(cx, "open_pr", [pr.repo, pr.number]));
      })
      .child(Icon("git-pull-request", { size: 12, color: k.mutedFg }))
      .child(div().font_family(mono()).text_size(TEXT.xs).child(step.where))
      .when(Boolean(pr.badge), (e) => e.child(div().size(6).rounded(3).bg(tone === "destructive" ? k.destructive : tone === "warning" ? k.warning : tone === "success" ? k.success : k.info)).child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(pr.badge.text)));
  }

  whereChip(step, k, id) {
    if (step.pr) return this.prChip(step, k, id);
    if (!step.where) return div();
    return h_flex().h(22).px(6).flex_none().items_center().rounded(RADIUS.md).bg(k.muted).font_family(mono()).text_size(TEXT.xs).text_color(k.mutedFg).child(step.where);
  }

  sessionLine(step, k) {
    const s = step.session;
    const color = s.tone === "success" ? k.success : s.tone === "warning" ? k.warning : s.tone === "working" ? k.working : k.destructive;
    return h_flex()
      .gap(6)
      .items_center()
      .text_size(TEXT.sm)
      .text_color(color)
      .child(s.running ? Spinner({ size: 12, color }) : Icon(s.failed ? "alert" : s.state === "done" ? "check" : "message", { size: 12, color }))
      .child(div().truncate().child(s.text ?? s.state));
  }

  rowHeight(step) {
    if (this.editing?.key === step.key) return 44;
    if (this.panel) return 34 + (step.session?.text ? 18 : 0);
    return 40 + (step.command ? 22 : 0) + (step.session?.text ? 20 : 0);
  }

  stepRow(step, k) {
    const sel = this.sel === step.key;
    const editing = this.editing?.key === step.key;
    if (editing) {
      return h_flex()
        .h(44)
        .px(10)
        .gap(10)
        .items_center()
        .rounded(RADIUS.md)
        .bg(k.selected)
        .child(div().w(70).flex_none().text_size(TEXT.sm).font_medium().text_color(k.mutedFg).child(`#${step.n} ${COLUMN[this.editing.column]}`))
        .child(TextField(this.editor, { leading: this.editing.column === "command" ? div().font_family(mono()).text_color(k.mutedFg).child("$") : undefined }))
        .child(Kbd("⏎"))
        .child(Kbd("esc"));
    }
    const done = !step.open;
    const muted = done ? k.mutedFg : k.foreground;
    const [, icon, color] = TONE[step.tone] ?? TONE.other;
    const id = `${this.scope}-row-${step.key}`;
    if (this.panel) {
      return ListRow(id, {
        height: this.rowHeight(step),
        selected: sel,
        marked: this.picked.has(step.key),
        flash: this.flashes.of(step.key),
        leading: Icon(icon, { size: 14, color: color(k) }),
        title: v_flex()
          .flex_1()
          .min_w(0)
          .gap(2)
          .child(h_flex().gap(6).min_w(0).child(div().flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(step.n)).child(this.marked(step.step, k, { color: muted })))
          .when(Boolean(step.session?.text), (e) => e.child(this.sessionLine(step, k))),
        tooltip: `${step.step}\n${step.status}${step.command ? `\n$ ${step.command}` : ""}`,
        hover: this.hover,
        onClick: (_e, cx) => this.clickRow(cx, step, _e),
        onRightClick: () => this.menu(step),
      });
    }
    const hoverActs = step.acts.filter((a) => ["run", "session", "done", "copy"].includes(a.id)).slice(0, 3);
    return ListRow(id, {
      height: this.rowHeight(step),
      selected: sel,
      marked: this.picked.has(step.key),
      flash: this.flashes.of(step.key),
      leading: h_flex()
        .gap(10)
        .items_center()
        .child(div().w(26).flex_none().text_size(TEXT.sm).font_family(mono()).text_color(k.mutedFg).child(step.n))
        .child(h_flex().w(150).flex_none().overflow_hidden().child(this.statusBadge(step, { id: `${id}-status` }))),
      title: v_flex()
        .flex_1()
        .min_w(0)
        .gap(3)
        .child(this.marked(step.step, k, { color: muted }))
        .when(Boolean(step.command), (e) =>
          e.child(
            h_flex()
              .h(19)
              .px(6)
              .gap(6)
              .max_w_full()
              .items_center()
              .rounded(RADIUS.sm)
              .bg(alpha(k.muted, 0.8))
              .font_family(mono())
              .text_size(TEXT.xs)
              .child(div().flex_none().text_color(k.mutedFg).child("$"))
              .child(this.marked(step.command, k, { color: done ? k.mutedFg : k.warning })),
          ),
        )
        .when(Boolean(step.session?.text), (e) => e.child(this.sessionLine(step, k))),
      meta: this.whereChip(step, k, `${id}-where`),
      hover: this.hover,
      actions: hoverActs.map((a) =>
        Button(`${id}-${a.id}`, { icon: ACT_ICON[a.id], variant: "ghost", size: "icon-sm", tooltip: `${cap(a.label)}  ${a.key}`, onClick: (_e, cx) => (cx.stop_propagation(), (this.sel = step.key), this.stepAct(cx, a, step)) }),
      ).concat([Button(`${id}-more`, { icon: "more", variant: "ghost", size: "icon-sm", tooltip: "More  m", onClick: () => this.menu(step) })]),
      onClick: (e, cx) => this.clickRow(cx, step, e),
      onDoubleClick: (_e, cx) => this.startEdit(cx, step.key, step.command ? "command" : "step"),
      onRightClick: () => this.menu(step),
    });
  }

  /** A click selects; cmd-click picks, shift-click picks the open steps from the selection to here. */
  clickRow(cx, step, e) {
    if (e?.modifiers?.platform) this.pick(cx, step.key);
    else if (e?.modifiers?.shift && this.sel) {
      const keys = this.shown().map((s) => s.key);
      const [a, b] = [keys.indexOf(this.sel), keys.indexOf(step.key)].sort((x, y) => x - y);
      keys.slice(a, b + 1).filter((key) => this.stepOf(key)?.open).forEach((key) => this.picked.add(key));
    }
    this.sel = step.key;
    this.handle.focus();
    cx.notify();
  }

  addForm(k) {
    const a = this.adding;
    const label = (text) => div().w(64).flex_none().text_size(TEXT.sm).font_medium().text_color(k.mutedFg).child(text);
    return v_flex()
      .mx(24)
      .mb(8)
      .p(12)
      .gap(8)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.ring)
      .bg(k.popover)
      .flex_none()
      .child(div().text_size(TEXT.sm).font_medium().child(`A new step in ${this.bookOf(a.slug)?.name ?? ""}, ${a.after ? `under step ${a.after}` : "at the end"}`))
      .children(["Step", "Where", "Command"].map((name, i) => h_flex().gap(8).items_center().child(label(name)).child(TextField(this.fields[i], { active: a.at === i }))))
      .child(
        h_flex()
          .gap(8)
          .justify_end()
          .child(Button("add-cancel", { label: "Cancel", variant: "outline", size: "sm", kbd: "esc", onClick: (_e, cx) => this.cancel(cx) }))
          .child(Button("add-save", { label: "Add step", icon: "plus", size: "sm", kbd: "⏎", onClick: (_e, cx) => this.commitAdd(cx) })),
      );
  }

  rows(k) {
    const rows = this.shown().map((s) => ({ height: this.rowHeight(s), el: this.stepRow(s, k) }));
    if (!this.panel && !this.filter && !this.adding)
      rows.push({
        height: 36,
        el: ListRow(`${this.scope}-adder`, { selected: this.sel === ADDER, muted: true, leading: Icon("plus", { size: 16 }), title: "Add a step", meta: Kbd("a"), onClick: (_e, cx) => this.startAdd(cx) }),
      });
    return rows;
  }

  tabs(k) {
    return h_flex()
      .px(24)
      .py(10)
      .gap(6)
      .flex_none()
      .flex_wrap()
      .children(
        this.books().map((b) => {
          const on = b.slug === this.book()?.slug;
          const found = this.global ? this.matchCount(b) : null;
          return h_flex()
            .id(`${this.scope}-tab-${b.slug}`)
            .h(32)
            .pl(8)
            .pr(10)
            .gap(8)
            .items_center()
            .rounded(RADIUS.md)
            .border_1()
            .border_color(on ? k.ring : k.border)
            .bg(on ? k.selected : k.background)
            .cursor_pointer()
            .hover((s) => s.bg(k.accent))
            .tooltip(`${b.title}\n${b.done} of ${b.total} done · updated ${b.updated}`)
            .when(found === 0, (e) => e.opacity(0.45))
            .on_click((_e, cx) => this.selectBook(cx, b.slug))
            .child(Ring(b.total ? b.done / b.total : 0, { size: 18, stroke: 2.5, color: b.todo ? k.ring : k.success, label: " " }))
            .child(div().text_size(TEXT.sm).font_semibold().text_color(on ? k.foreground : k.mutedFg).child(b.name))
            .when(Boolean(b.refresh?.running), (e) => e.child(Spinner({ size: 12, color: k.working })))
            .child(
              found !== null
                ? Badge(`${found} found`, found ? "warning" : "outline")
                : b.todo
                  ? div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${b.todo} to do`)
                  : Icon("check", { size: 14, color: k.success }),
            );
        }),
      );
  }

  bookHead(book, k) {
    const r = book.refresh ?? {};
    const secs = r.running && r.since ? Math.max(0, Math.round((Date.now() - r.since) / 1000)) : null;
    const act = (id) => book.acts.find((a) => a.id === id);
    const btn = (id, icon, variant) => {
      const a = act(id);
      return a ? Button(`${this.scope}-book-${id}`, { label: cap(a.label), icon, variant, size: "sm", kbd: a.key, onClick: (_e, cx) => this.bookAct(cx, a, book) }) : div();
    };
    return v_flex()
      .px(24)
      .pt(4)
      .pb(12)
      .gap(10)
      .flex_none()
      .child(
        h_flex()
          .gap(12)
          .items_center()
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(div().truncate().text_size(TEXT.lg).font_semibold().child(book.title))
              .child(
                h_flex()
                  .gap(8)
                  .items_center()
                  .text_size(TEXT.sm)
                  .text_color(k.mutedFg)
                  .child(`${book.done} of ${book.total} done · updated ${book.updated || "never"}`)
                  .when(Boolean(r.running), (e) => e.child(Badge(`refreshing${secs !== null ? ` · ${secs}s` : ""}`, "warning", { icon: "loader" })))
                  .when(!r.running && r.code !== null && r.code !== undefined && r.code !== 0, (e) => e.child(Badge("last refresh failed", "destructive", { id: `${this.scope}-refresh-failed`, tooltip: r.log }))),
              ),
          )
          .child(h_flex().gap(6).flex_none().child(btn("refresh", "refresh", "outline")).child(btn("talk", "message", "outline")).child(btn("delete", "trash", "ghost"))),
      )
      .child(Progress(book.total ? book.done / book.total : 0, { color: book.todo ? k.ring : k.success }));
  }

  notes(book, k) {
    if (!book.notes?.length) return div();
    return v_flex()
      .px(16)
      .pb(8)
      .flex_none()
      .child(
        Section(`${this.scope}-notes`, {
          title: "Notes",
          count: book.notes.length,
          open: this.notesOpen,
          onToggle: (_e, cx) => ((this.notesOpen = !this.notesOpen), cx.notify()),
          children: book.notes.map((parts, i) =>
            h_flex()
              .id(`${this.scope}-note-${i}`)
              .px(8)
              .gap(8)
              .items_start()
              .text_size(TEXT.sm)
              .child(div().mt(6).size(4).rounded(2).flex_none().bg(k.mutedFg))
              .child(
                h_flex()
                  .flex_1()
                  .flex_wrap()
                  .gap(0)
                  .children(parts.map((p) => (p.code ? div().px(4).mx(1).rounded(RADIUS.sm).bg(k.muted).font_family(mono()).text_size(TEXT.xs).text_color(k.warning).child(p.text) : div().child(p.text)))),
              ),
          ),
        }),
      );
  }

  /** The selected step beside the list: everything about it, its actions as buttons. */
  details(step, book, k) {
    const id = `${this.scope}-side`;
    const s = step.session;
    return SidePanel(id, {
      title: `Step ${step.n}`,
      description: book.title.startsWith(book.name) ? book.title : `${book.name} · ${book.title}`,
      width: 340,
      onClose: (_e, cx) => ((this.showDetails = false), cx.notify()),
      children: [
        div().whitespace_normal().text_size(TEXT.base).font_medium().child(step.step),
        h_flex()
          .gap(8)
          .items_center()
          .flex_wrap()
          .child(this.statusBadge(step, { id: `${id}-status` }))
          .child(this.whereChip(step, k, `${id}-where`)),
        ...(step.command
          ? [
              v_flex()
                .gap(6)
                .child(div().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child("COMMAND"))
                .child(
                  div()
                    .p(10)
                    .rounded(RADIUS.md)
                    .bg(k.muted)
                    .whitespace_normal()
                    .font_family(mono())
                    .text_size(TEXT.sm)
                    .text_color(step.open ? k.warning : k.mutedFg)
                    .child(`$ ${step.command}`),
                )
                .child(
                  h_flex()
                    .gap(6)
                    .child(Button(`${id}-copy`, { label: "Copy", icon: "copy", variant: "outline", size: "sm", kbd: "c", onClick: (_e, cx) => this.copy(cx, [step.command]) }))
                    .child(Button(`${id}-edit`, { label: "Edit", icon: "pencil", variant: "ghost", size: "sm", kbd: "e", onClick: (_e, cx) => this.startEdit(cx, step.key, "command") })),
                ),
            ]
          : [h_flex().child(Button(`${id}-addcmd`, { label: "Add a command", icon: "plus", variant: "ghost", size: "sm", kbd: "e", onClick: (_e, cx) => this.startEdit(cx, step.key, "command") }))]),
        ...(s?.text
          ? [
              h_flex()
                .p(10)
                .gap(8)
                .items_start()
                .rounded(RADIUS.md)
                .bg(alpha(s.tone === "success" ? k.success : s.tone === "destructive" ? k.destructive : k.warning, 0.12))
                .child(v_flex().flex_1().min_w(0).gap(6).child(this.sessionLine(step, k)).when(s.state !== "done", (e) => e.child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child("Its session owes the outcome: it sets the status when it is done.")))),
            ]
          : []),
      ],
      footer: !step.acts.some((a) => a.id !== "edit" && a.id !== "copy")
        ? null
        : h_flex()
        .flex_1()
        .gap(6)
        .flex_wrap()
        .children(
          step.acts
            .filter((a) => a.id !== "edit" && a.id !== "copy")
            .map((a, i) => Button(`${id}-act-${a.id}`, { label: cap(a.label), icon: ACT_ICON[a.id], kbd: a.key, size: "sm", variant: a.main ? "default" : i === 0 ? "default" : "outline", onClick: (_e, cx) => this.stepAct(cx, a, step) })),
        ),
    });
  }

  actionBar(k, cx) {
    let acts;
    if (this.picked.size) {
      const n = this.targets().length;
      acts = [
        { id: "done", label: `Mark ${n} done`, kbd: "x", icon: "check", run: (cx) => this.markDone(cx, this.targets()) },
        { id: "copy", label: `Copy ${n} commands`, kbd: "c", icon: "copy", variant: "outline", run: (cx) => this.copy(cx, this.targets().map(([s, n]) => this.stepOf(`${s}#${n}`)?.command)) },
        { id: "all", label: "All open", kbd: "V", variant: "ghost", run: (cx) => this.pickOpen(cx) },
        { id: "unpick", label: "Unpick", kbd: "esc", variant: "ghost", run: (cx) => (this.picked.clear(), cx.notify()) },
      ];
    } else {
      const step = this.selected();
      acts = (step?.acts ?? []).map((a, i) => ({ id: a.id, label: cap(a.label), kbd: a.key, icon: ACT_ICON[a.id], variant: a.main || i === 0 ? "default" : "outline", run: (cx) => this.stepAct(cx, a, step) }));
    }
    return h_flex()
      .px(this.panel ? 12 : 24)
      .py(10)
      .gap(6)
      .flex_none()
      .flex_wrap()
      .items_center()
      .border_t_1()
      .border_color(k.border)
      .bg(alpha(k.muted, 0.5))
      .children(acts.map((a) => Button(`${this.scope}-bar-${a.id}`, { label: this.panel ? undefined : a.label, icon: a.icon ?? "chevron-right", kbd: this.panel ? undefined : a.kbd, size: this.panel ? "icon-sm" : "sm", variant: a.variant, tooltip: this.panel ? `${a.label}  ${a.kbd ?? ""}` : undefined, onClick: (_e, cx) => a.run(cx) })))
      .when(this.status !== "live", (e) => e.child(div().flex_1()).child(Badge(this.status.startsWith("error") ? "offline" : "connecting", this.status.startsWith("error") ? "destructive" : "secondary")));
  }

  loading(k) {
    if (this.status.startsWith("error")) return ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) });
    return v_flex().p(24).gap(12).children([0, 1, 2, 3].map((i) => h_flex().gap(10).items_center().child(Skeleton({ w: 24, h: 16 })).child(Skeleton({ w: 110, h: 20 })).child(Skeleton({ w: 160 + ((i * 97) % 240) }))));
  }

  overlays(root, k, cx) {
    const toasts = this.toasts.render();
    let out = toasts ? root.child(toasts) : root;
    if (this.confirm) {
      const c = this.confirm;
      out = out.child(
        AlertDialog(`${this.scope}-confirm`, {
          title: c.title,
          description: c.description,
          confirmLabel: c.label,
          destructive: c.destructive,
          onCancel: (_e, cx) => ((this.confirm = null), cx.notify()),
          onConfirm: (_e, cx) => {
            this.confirm = null;
            cx.spawn(async (cx) => c.run(cx));
            cx.notify();
          },
        }),
      );
    }
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, shortcut: i.shortcut, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id(`${this.scope}-palette-backdrop`)
          .absolute()
          .inset_0()
          .bg("#00000055")
          .on_mouse_down("left", (_e, cx) => this.closePalette(cx))
          .child(
            v_flex()
              .absolute()
              .top(72)
              .left(0)
              .right(0)
              .items_center()
              .child(
                v_flex()
                  .id(`${this.scope}-palette-card`)
                  .w(Math.min(600, this.width - 32))
                  .max_h(440)
                  .rounded(RADIUS.lg)
                  .border_1()
                  .border_color(k.border)
                  .bg(k.popover)
                  .shadow_lg()
                  .overflow_hidden()
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .child(Command(`${this.scope}-commands`, { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id })),
              ),
          ),
      );
    }
    return out;
  }

  root(k) {
    return v_flex()
      .id(this.scope)
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        const w = size.width / (settings().zoom ?? 1);
        if (Math.abs(w - this.width) > 1) {
          this.width = w;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => {
        if (!this.editing && !this.adding && !this.filtering && !this.palette) this.handle.focus();
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      });
  }

  renderPanel(k, cx) {
    const book = this.book();
    let root = this.root(k);
    if (!this.model) return this.overlays(root.child(this.loading(k)), k, cx);
    if (!book) {
      const has = Boolean(this.lastTerminal);
      return this.overlays(
        root.child(
          Empty({
            icon: "book",
            title: has ? "No runbook for this agent" : "No agent focused",
            description: has ? "Nothing it works on has a runbook yet: in Claude Code, ask the agent for /runbook." : "Focus an agent working on a ticket to see its runbook here.",
            action: Button("panel-open-all", { label: "All runbooks", icon: "book", variant: "outline", size: "sm", onClick: () => hub.go({ app: "runbook", key: this.books()[0]?.slug ?? "" }).catch(() => {}) }),
          }),
        ),
        k,
        cx,
      );
    }
    if (this.sel === null || !this.stepOf(this.sel) || !this.sel.startsWith(`${book.slug}#`)) this.sel = this.firstOf(book);
    const r = book.refresh ?? {};
    const keys = this.selectable();
    root = root
      .child(
        h_flex()
          .h(52)
          .px(12)
          .gap(10)
          .flex_none()
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .child(Ring(book.total ? book.done / book.total : 0, { size: 30, stroke: 3, color: book.todo ? k.ring : k.success, label: `${book.done}/${book.total}` }))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .child(h_flex().gap(6).items_center().child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(book.name)).when(Boolean(r.running), (e) => e.child(Spinner({ size: 11, color: k.working }))))
              .child(div().truncate().text_size(TEXT.base).font_semibold().child(book.title)),
          )
          .child(Button("panel-refresh", { icon: "refresh", variant: "ghost", size: "icon-sm", tooltip: "Refresh  R", onClick: (_e, cx) => this.bookAct(cx, book.acts.find((a) => a.id === "refresh"), book) }))
          .child(Button("panel-open", { icon: "external", variant: "ghost", size: "icon-sm", tooltip: "Open in Runbooks", onClick: () => hub.go({ app: "runbook", key: this.sel ?? book.slug }).catch(() => {}) })),
      )
      .child(v_flex().flex_1().min_h(0).px(4).child(this.scroller.view(`${this.scope}-list`, this.rows(k), keys.indexOf(this.sel), { padding: 6 })))
      .child(this.actionBar(k, cx));
    return this.overlays(root, k, cx);
  }

  render(cx) {
    const k = tokens();
    if (this.panel) return this.renderPanel(k, cx);
    const m = this.model;
    const book = this.book();
    const left = m?.left ?? 0;
    const description = m ? (this.books().length ? `${this.books().length} runbook${this.books().length > 1 ? "s" : ""} · ${left} step${left === 1 ? "" : "s"} left` : "No runbook yet") : "Loading…";
    let root = this.root(k).child(
      PageHeader({
        icon: "book",
        title: "Runbooks",
        description,
        aside: this.picked.size ? Badge(`${this.picked.size} picked`, "info") : null,
        actions: [
          Button("runbook-undo", { icon: "undo", variant: "ghost", size: "icon", tooltip: "Undo  u", disabled: !m?.can_undo, onClick: (_e, cx) => cx.spawn(async (cx) => this.viewAct(cx, "undo", [])) }),
          Button("runbook-commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
          Button("runbook-details", { icon: "panel-right", variant: "ghost", size: "icon", tooltip: "Step details", onClick: (_e, cx) => ((this.showDetails = !this.showDetails), cx.notify()) }),
          Button("runbook-add", { label: "Add step", icon: "plus", size: "sm", kbd: "a", disabled: !book, onClick: (_e, cx) => this.startAdd(cx) }),
        ],
      }),
    );
    if (!m) return this.overlays(root.child(this.loading(k)), k, cx);
    if (!book) {
      return this.overlays(
        root.child(Empty({ icon: "book", title: "No runbook yet", description: "Ask your agent for a runbook on the ticket you are on: its deploy steps, PRs and commands land here." })),
        k,
        cx,
      );
    }
    if (!this.sel || (this.sel !== ADDER && !this.stepOf(this.sel)) || (this.sel !== ADDER && !this.sel.startsWith(`${book.slug}#`))) this.sel = this.firstOf(book);
    const keys = this.selectable();
    const wide = this.width >= 900;
    const step = this.selected();
    const shown = this.shown();
    const list =
      this.filter && !shown.length
        ? Empty({ icon: "search", title: "No step matches", description: this.global ? `No runbook has a step with every word of “${this.filter}”.` : `No step of ${book.name} has every word of “${this.filter}”.` })
        : this.scroller.view(`${this.scope}-list`, this.rows(k), keys.indexOf(this.sel));
    const main = v_flex()
      .flex_1()
      .min_w(0)
      .h_full()
      .child(this.tabs(k))
      .child(this.bookHead(book, k))
      .child(
        h_flex()
          .px(24)
          .pb(8)
          .gap(8)
          .flex_none()
          .items_center()
          .child(h_flex().id(`${this.scope}-filter`).flex_1().on_mouse_down("left", (_e, cx) => ((this.filtering = true), cx.notify())).child(SearchField(this.search, { active: this.filtering, kbd: "/" })))
          .child(
            Button(`${this.scope}-global`, {
              label: "Every runbook",
              icon: "search",
              variant: this.global ? "secondary" : "outline",
              size: "sm",
              kbd: "ctrl /",
              onClick: (_e, cx) => ((this.global = !this.global), this.afterFilter(), cx.notify()),
            }),
          ),
      )
      .when(Boolean(this.adding), (e) => e.child(this.addForm(k)))
      .child(v_flex().flex_1().min_h(0).px(14).child(list))
      .child(this.notes(book, k));
    root = root
      .child(h_flex().flex_1().min_h(0).child(main).when(wide && this.showDetails && Boolean(step) && !this.picked.size, (e) => e.child(this.details(step, book, k))))
      .when(!wide || !this.showDetails || this.picked.size > 0 || !step, (e) => e.child(this.actionBar(k, cx)))
      .child(
        KeyHints(
          this.picked.size
            ? [["x", "mark done"], ["c", "copy commands"], ["v", "pick"], ["V", "all open"], ["esc", "unpick"]]
            : [["j k", "move"], ["[ ]", "runbooks"], ["⏎", "main action"], ["v", "pick"], ["e", "edit"], ["a", "add"], ["/", "filter"], ["m", "menu"], ["ctrl p", "commands"]],
        ),
      );
    return this.overlays(root, k, cx);
  }
}

const ACT_ICON = { run: "play", session: "terminal", done: "check", open: "git-pull-request", copy: "copy", edit: "pencil", refresh: "refresh", talk: "message", delete: "trash" };

function cap(text) {
  return text ? text[0].toUpperCase() + text.slice(1) : "";
}
