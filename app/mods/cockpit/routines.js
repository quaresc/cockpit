// The routines as a dashboard: the `routines` view the hub pushes. Tiles for the week (what runs, a bar per day of runs
// ok, failed and missed, what failed, what it cost, what runs next), the run inbox (runs you have not read, marked read
// one at a time or all at once), then every routine with its state, schedule, next runs and a strip of its last runs,
// unfolded onto its runs. Beside the list, the selected routine's settings changed in place (model, pause) or the run's
// output; a form adds or changes a routine with schedule preset pills and its next runs read back as you type. Every
// TUI key and action: run, stop, edit, open, talk, pause, model, delete (after a confirm), add, refresh, and the
// worktree a run kept.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState, TextareaState, Textarea } from "gpui-base";
import { readTextFile } from "herdr";
import * as hub from "./hub.js";
import { Markdown, openReader } from "./reader.js";
import {
  tokens, TEXT, RADIUS, Icon, Badge, Button, Kbd, PageHeader, SearchField, TextField, KeyHints, Empty, ErrorState, Skeleton,
  Command, Toaster, Flashes, Scroller, SidePanel, AlertDialog, Chip, Mono, Metric, StageHeading, Progress, Pills, Toggle,
  Spinner, Callout, openMenu, alpha, Overline, Tiles,
} from "./ui.js";

const ICONS = { run: "play", stop: "circle-stop", edit: "pencil", open: "file-text", talk: "message-circle", pause: "circle-pause", model: "sparkles", delete: "trash", worktree: "git-branch" };
const WORD = {
  ok: ["Ok", "circle-check", (k) => k.success, "success"],
  failed: ["Failed", "circle-x", (k) => k.destructive, "destructive"],
  missed: ["Missed", "skip-forward", (k) => k.warning, "warning"],
  stopped: ["Stopped", "circle-stop", (k) => k.mutedFg, "secondary"],
};
const STATE = {
  running: ["Running", "loader", (k) => k.working, "warning"],
  waiting: ["Waiting", "hourglass", (k) => k.warning, "warning"],
  broken: ["Broken", "triangle-alert", (k) => k.destructive, "destructive"],
  over: ["Over budget", "dollar", (k) => k.warning, "warning"],
  paused: ["Paused", "circle-pause", (k) => k.mutedFg, "secondary"],
  failed: WORD.failed,
  missed: WORD.missed,
  ok: WORD.ok,
  never: ["Never ran", "circle-dashed", (k) => k.mutedFg, "outline"],
};
const PRESETS = [
  ["every 30m", "Every 30 min"],
  ["hourly", "Hourly"],
  ["weekdays 09:00", "Weekdays 9:00"],
  ["daily 09:00", "Daily 9:00"],
  ["mon 09:00", "Mondays 9:00"],
  ["fri 16:00", "Fridays 16:00"],
  ["on ci-failed", "On CI failing"],
  ["on review", "On a review asked"],
  ["manual", "By hand"],
];
const MODELS = [["", "Default"], ["lightest", "Lightest"], ["haiku", "Haiku"], ["sonnet", "Sonnet"], ["opus", "Opus"]];
const SIDE_AT = 960;
const cap = (s) => (s ? s[0].toUpperCase() + s.slice(1) : s);
const stroke = (key) => (key && key !== key.toLowerCase() ? `shift-${key.toLowerCase()}` : key);

/** The prompt (or script) of a routine file: what follows its front matter. */
function bodyOf(text) {
  const lines = (text ?? "").split("\n");
  if (lines[0]?.trim() !== "---") return (text ?? "").trim();
  const end = lines.findIndex((l, i) => i > 0 && l.trim() === "---");
  return end < 0 ? (text ?? "").trim() : lines.slice(end + 1).join("\n").trim();
}

export default class Routines {
  init(_props, cx) {
    this.model = null;
    this.status = "connecting";
    this.sel = null;
    this.opened = new Set();
    this.filter = "";
    this.filtering = false;
    this.palette = null;
    this.confirm = null;
    this.form = null;
    this.outputs = new Map();
    this.fresh = new Map();
    this.cleared = new Map();
    this.visible = false;
    this.width = 1200;
    this.height = 800;
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();
    this.search = InputState.new({ placeholder: "Filter routines and runs…" });
    this.search.on("change", (_e, cx) => ((this.filter = this.search.value()), cx.notify()));
    this.search.on("submit", (_e, cx) => ((this.filtering = false), this.handle.focus(), cx.notify()));
    this.query = InputState.new({ placeholder: "Type a command or search…" });
    this.query.on("change", (_e, cx) => (this.palette && (this.palette.at = 0), cx.notify()));
    this.query.on("submit", (_e, cx) => this.runPalette(cx));
    this.fields = {
      name: InputState.new({ placeholder: "From the prompt" }),
      when: InputState.new({ placeholder: "weekdays 09:00, every 30m, on ci-failed api, cron 0 9 * * 1-5" }),
      cwd: InputState.new({ placeholder: "A folder of its own" }),
      body: TextareaState.new({ placeholder: "What Claude should do, unattended. A script for the shell.", rows: 5 }),
    };
    this.fields.body.set_auto_grow(4, 10);
    this.fields.when.on("change", (_e, cx) => this.preview(cx));
    this.sub = hub.subscribe(
      cx,
      { views: ["routines"] },
      (m, cx) => {
        if (m.view === "routines") this.take(m.value, cx);
        if (m.focus === "routines") this.goTo(m.target?.key, cx);
        cx.notify();
      },
      (s, cx) => ((this.status = s), cx.notify()),
    );
  }

  close() {
    this.sub?.stop();
  }

  command(c, cx) {
    if (c === "focus" || c === "show:routines") {
      this.visible = true;
      if (!this.form) this.handle.focus();
    } else if (c.startsWith("show:") && this.visible) this.leave(cx);
    const [scope, what, id, ...rest] = c.split(":");
    if (scope === "routines") {
      const key = rest.join(":");
      if (what === "act" && id === "file") cx.spawn(async (cx) => this.send(cx, "edit", [key]));
      else if (what === "act") {
        const a = this.entry(key)?.acts?.find((x) => x.id === id);
        if (a) ((this.sel = key), this.run(a, cx));
      } else if (what === "read") this.markRead(cx, key);
      else if (what === "change") this.openForm(cx, key);
    }
    cx.notify();
  }

  leave(cx) {
    this.visible = false;
    this.fresh.clear();
    this.cleared.clear();
    cx.spawn(async () => hub.act("routines", "looked", []).catch(() => {}));
  }

  take(model, cx) {
    const before = this.model;
    this.model = model;
    this.runs = new Map();
    for (const r of model?.routines ?? []) for (const run of r.runs) this.runs.set(run.key, run);
    const k = tokens();
    if (before?.ready) {
      const was = new Map((before.routines ?? []).map((r) => [r.slug, r.state]));
      for (const r of model.routines ?? []) if (was.has(r.slug) && was.get(r.slug) !== r.state) this.flashes.start(cx, r.key, (STATE[r.state] ?? STATE.never)[2](k));
      const known = new Set((before.inbox ?? []).map((x) => x.key));
      for (const x of model.inbox ?? []) if (!known.has(x.key)) this.flashes.start(cx, `inbox:${x.key}`, (WORD[x.word] ?? WORD.failed)[2](k));
    }
    if (!this.sel || !this.entry(this.sel)) this.sel = this.selectable()[0] ?? null;
    if (model?.ready && model.since) {
      cx.spawn(async (cx) => {
        const events = await hub.events("routines", model.since).catch(() => []);
        this.fresh = new Map();
        for (const e of events) if ((e.at ?? 0) > (this.fresh.get(e.key) ?? 0)) this.fresh.set(e.key, e.at ?? 0);
        cx.notify();
      });
    }
  }

  routine(slug) {
    return this.model?.routines?.find((r) => r.slug === slug) ?? null;
  }

  /** A routine (by slug) or a run (`slug/run/id`, or `inbox:` before it). */
  entry(key) {
    if (!key) return null;
    const plain = key.replace(/^inbox:/, "");
    return plain.includes("/run/") ? this.runs?.get(plain) ?? null : this.routine(plain);
  }

  marked(slug) {
    return this.fresh.has(slug) && this.fresh.get(slug) > (this.cleared.get(slug) ?? 0);
  }

  select(key) {
    this.sel = key;
    const slug = key?.replace(/^inbox:/, "").split("/")[0];
    if (slug && this.fresh.has(slug)) this.cleared.set(slug, this.fresh.get(slug));
  }

  readOutput(x, text) {
    openReader({ text, title: x.name, meta: [x.when, x.took, x.cost].filter(Boolean).join(" · ") });
  }

  loadOutput(path, cx) {
    this.outputs.set(path, null);
    cx.spawn(async (cx) => {
      const text = await readTextFile(path).catch(() => "");
      this.outputs.set(path, text ?? "");
      cx.notify();
    });
  }

  hits(text) {
    const words = this.filter.toLowerCase().split(/\s+/).filter(Boolean);
    return words.every((w) => text.toLowerCase().includes(w));
  }

  /** The rows as they show: the inbox's runs, then the routines, each unfolded onto its runs. */
  rows() {
    const m = this.model;
    if (!m?.ready) return [];
    const out = [];
    const inbox = (m.inbox ?? []).filter((x) => this.hits(`${x.name} ${x.summary} ${x.why}`));
    if (inbox.length) {
      out.push({ key: "h:inbox", kind: "heading", label: "Inbox", count: inbox.length });
      inbox.forEach((x) => out.push({ key: `inbox:${x.key}`, kind: "inbox", run: x }));
    }
    const routines = m.routines.filter((r) => this.hits(`${r.name} ${r.when} ${r.prompt} ${r.runs.map((x) => x.summary).join(" ")}`));
    out.push({ key: "h:routines", kind: "heading", label: "Routines", count: routines.length });
    for (const r of routines) {
      const open = this.opened.has(r.slug) || (this.filter.trim() !== "" && r.runs.some((x) => this.hits(x.summary)));
      out.push({ key: r.key, kind: "routine", r, fold: r.runs.length ? open : null });
      if (open) r.runs.forEach((x, i) => out.push({ key: x.key, kind: "run", run: x, r, last: i === r.runs.length - 1 }));
    }
    return out;
  }

  selectable() {
    return this.rows().filter((r) => r.kind !== "heading").map((r) => r.key);
  }

  goTo(key, cx) {
    if (!key) return;
    const slug = key.split("/")[0];
    if (!this.routine(slug)) return this.toasts.push(cx, `No routine ${slug} any more`);
    if (key.includes("/run/")) this.opened.add(slug);
    this.filter = "";
    this.search.set_value("");
    this.select(key);
    this.handle.focus();
  }

  move(by) {
    const all = this.selectable();
    if (!all.length) return;
    const at = Math.max(0, all.indexOf(this.sel));
    this.select(all[Math.max(0, Math.min(all.length - 1, at + by))]);
  }

  moveGroup(by) {
    const heads = this.rows().filter((r) => r.kind === "routine" || r.kind === "inbox").map((r) => r.key);
    const all = this.selectable();
    const at = all.indexOf(this.sel);
    const next = by > 0 ? heads.find((h) => all.indexOf(h) > at) : [...heads].reverse().find((h) => all.indexOf(h) < at);
    if (next) this.select(next);
  }

  fold() {
    const key = this.sel?.replace(/^inbox:/, "");
    if (!key) return;
    const slug = key.split("/")[0];
    if (key.includes("/run/") && !this.sel.startsWith("inbox:")) this.select(slug);
    if (!this.opened.delete(slug)) this.opened.add(slug);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async send(cx, action, args, armed) {
    try {
      const r = await hub.request("act", { app: "routines", action, args, armed });
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note) this.toasts.push(cx, r.note, { kind: "success" });
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    }
    cx.notify();
  }

  run(a, cx) {
    if (a.id === "edit") return this.openForm(cx, a.args[0]);
    if (a.confirm) {
      this.confirm = a;
      cx.notify();
      return;
    }
    cx.spawn(async (cx) => this.send(cx, a.action, a.args));
  }

  markRead(cx, key) {
    const run = this.entry(key);
    if (!run?.slug) return;
    cx.spawn(async (cx) => this.send(cx, "read", [run.slug, run.id]));
  }

  readAll(cx) {
    cx.spawn(async (cx) => this.send(cx, "read_all", []));
  }

  cleanup(cx, run) {
    cx.spawn(async (cx) => this.send(cx, "cleanup", run ? [run.slug, run.id] : []));
  }

  refresh(cx) {
    cx.spawn(async (cx) => this.send(cx, "request_refresh", []));
  }

  menu(key) {
    const e = this.entry(key);
    if (!e) return;
    const run = key.includes("/run/");
    const items = (e.acts ?? []).map((a) => ({ label: a.id === "edit" ? "Change…" : cap(a.label), icon: ICONS[a.id], kbd: a.key, destructive: a.id === "delete", command: `routines:act:${a.id}:${key.replace(/^inbox:/, "")}` }));
    if (run && e.unread) items.push("separator", { label: "Mark read", icon: "mail-check", kbd: "x", command: `routines:read:-:${e.key}` });
    if (!run) items.push("separator", { label: "Edit the file", icon: "file-text", command: `routines:act:file:${e.slug}` });
    openMenu(items);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The form: a new routine, or one changed in place

  openForm(cx, slug) {
    const r = slug ? this.routine(slug) : null;
    this.form = {
      slug: r?.slug ?? null,
      shell: r?.run === "shell",
      model: r?.model ?? "",
      worktree: Boolean(r?.worktree),
      notify: r?.notify ?? "always",
      preview: null,
    };
    this.fields.name.set_value(r?.name ?? "");
    this.fields.when.set_value(r?.whenText ?? "weekdays 09:00");
    this.fields.cwd.set_value(r?.cwd ?? "");
    this.fields.body.set_value(r?.prompt ?? "");
    (r ? this.fields.body : this.fields.name).focus();
    // The shown prompt is cut short: the whole one comes from the file.
    if (r)
      cx.spawn(async (cx) => {
        const text = await readTextFile(String(r.path)).catch(() => "");
        if (this.form?.slug === r.slug && text) this.fields.body.set_value(bodyOf(text));
        cx.notify();
      });
    this.preview(cx);
    cx.notify();
  }

  preview(cx) {
    const when = this.fields.when.value();
    const asked = (this.previewAsked = (this.previewAsked ?? 0) + 1);
    cx.spawn(async (cx) => {
      const p = await hub.view("routines-when", { when }).catch(() => null);
      if (this.form && asked === this.previewAsked) {
        this.form.preview = p;
        cx.notify();
      }
    });
  }

  submit(cx) {
    const f = this.form;
    if (!f) return;
    const body = this.fields.body.value().trim();
    if (!body) return this.toasts.push(cx, f.shell ? "The script is empty" : "The prompt is empty", { kind: "error" });
    if (f.preview && !f.preview.ok) return this.toasts.push(cx, f.preview.error, { kind: "error" });
    const fields = [this.fields.name.value().trim(), this.fields.when.value().trim(), body, this.fields.cwd.value().trim(), f.shell ? "shell" : "claude", f.shell ? "" : f.model, f.worktree, f.notify];
    this.form = null;
    this.handle.focus();
    cx.spawn(async (cx) => (f.slug ? this.send(cx, "update", [f.slug, ...fields]) : this.send(cx, "add", fields)));
    cx.notify();
  }

  cancelForm(cx) {
    this.form = null;
    this.handle.focus();
    cx.notify();
  }

  formView(k, cx) {
    const f = this.form;
    const p = f.preview;
    const label = (text, hint) => h_flex().gap(8).items_center().child(div().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(text)).when(Boolean(hint), (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(hint)));
    const row = (...children) => v_flex().gap(6).children(children);
    const previewLine = !p
      ? div().h(18)
      : p.ok
        ? h_flex()
            .gap(6)
            .items_center()
            .flex_wrap()
            .child(Icon("calendar-clock", { size: 13, color: k.success }))
            .child(div().text_size(TEXT.sm).text_color(k.foreground).child(p.describe))
            .children((p.next ?? []).map((n, i) => Chip(n.text, { mono: true, color: i === 0 ? k.info : undefined })))
            .when(p.kind === "every", (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child("counted from its last run")))
        : h_flex().gap(6).items_center().child(Icon("triangle-alert", { size: 13, color: k.destructive })).child(div().text_size(TEXT.sm).text_color(k.destructive).child(p.error));
    const body = v_flex()
      .id("routine-form-body")
      .flex_1()
      .min_h(0)
      .p(24)
      .gap(16)
      .overflow_y_scrollbar()
      .child(v_flex().gap(4).child(div().text_size(TEXT.lg).font_semibold().child(f.slug ? "Change the routine" : "New routine")).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("The hub runs it on its own, on a schedule or when something happens, and tells you how it went.")))
      .child(row(label("Name"), TextField(this.fields.name, { active: false })))
      .child(
        row(
          label("When"),
          Pills("when-presets", PRESETS, this.fields.when.value().trim(), (v, cx) => {
            this.fields.when.set_value(v);
            this.preview(cx);
            cx.notify();
          }),
          TextField(this.fields.when, { active: false, leading: Icon("clock", { size: 14, color: k.mutedFg }) }),
          previewLine,
        ),
      )
      .child(row(label(f.shell ? "Script" : "Prompt"), div().rounded(RADIUS.md).border_1().border_color(k.input).bg(k.background).px(10).py(6).child(Textarea.new(this.fields.body).w_full().text_size(TEXT.base))))
      .child(row(label("Folder", f.shell ? "the repos folder when empty" : "a folder of its own when empty"), TextField(this.fields.cwd, { active: false, leading: Icon("folder", { size: 14, color: k.mutedFg }) })))
      .child(row(label("Runs with"), Pills("form-kind", [["claude", "Claude"], ["shell", "Shell"]], f.shell ? "shell" : "claude", (v, cx) => ((f.shell = v === "shell"), cx.notify()))))
      .when(!f.shell, (e) => e.child(row(label("Model", f.model === "lightest" ? "haiku with nothing loaded but the tools it lists" : ""), Pills("form-model", MODELS, f.model, (v, cx) => ((f.model = v), cx.notify())))))
      .child(
        h_flex()
          .gap(12)
          .items_center()
          .child(Toggle("form-worktree", f.worktree, (cx) => ((f.worktree = !f.worktree), cx.notify())))
          .child(v_flex().flex_1().min_w(0).child(div().text_size(TEXT.sm).font_medium().child("A worktree per run")).child(div().whitespace_normal().text_size(TEXT.xs).text_color(k.mutedFg).child("Each run on its own branch auto/<name>-<time> in a fresh worktree of the folder's repo; kept only when it left work."))),
      )
      .child(row(label("Tell me"), Pills("form-notify", [["always", "Every run"], ["failed", "When it fails"], ["never", "Never"]], f.notify, (v, cx) => ((f.notify = v), cx.notify()))));
    const card = v_flex()
      .id("routine-form")
      .w(Math.min(600, this.width - 32))
      .h(Math.min(820, this.height - 48))
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .bg(k.popover)
      .shadow_lg()
      .overflow_hidden()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(body)
      .child(
        h_flex()
          .px(24)
          .py(14)
          .flex_none()
          .justify_end()
          .gap(8)
          .border_t_1()
          .border_color(k.border)
          .child(Button("form-cancel", { label: "Cancel", variant: "outline", kbd: "esc", onClick: (_e, cx) => this.cancelForm(cx) }))
          .child(Button("form-ok", { label: f.slug ? "Save" : "Add routine", icon: f.slug ? "check" : "plus", kbd: "⌘⏎", disabled: Boolean(p && !p.ok), onClick: (_e, cx) => this.submit(cx) })),
      );
    return div()
      .id("routine-form-backdrop")
      .absolute()
      .inset_0()
      .bg("#00000066")
      .on_mouse_down("left", (_e, cx) => this.cancelForm(cx))
      .child(v_flex().size_full().items_center().justify_center().child(card));
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p)

  paletteGroups(cx) {
    const q = this.query.value().toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (t) => q.every((w) => t.toLowerCase().includes(w));
    const t = this.model?.tiles ?? {};
    const own = [
      { id: "p-add", label: "Add a routine", shortcut: "a", icon: "plus", run: () => this.openForm(cx) },
      { id: "p-refresh", label: "Read the routines again", shortcut: "R", icon: "refresh-cw", run: () => this.refresh(cx) },
      ...(t.unread ? [{ id: "p-read", label: `Mark the ${t.unread} unread runs read`, shortcut: "X", icon: "mail-check", run: () => this.readAll(cx) }] : []),
      ...(t.worktrees ? [{ id: "p-clean", label: "Clean up the run worktrees whose work landed", shortcut: "C", icon: "brush-cleaning", run: () => this.cleanup(cx) }] : []),
    ];
    const acts = [];
    for (const r of this.model?.routines ?? []) {
      for (const a of r.acts) acts.push({ id: `p-${a.id}-${r.slug}`, label: `${a.id === "edit" ? "Change" : cap(a.label)}  ${r.name}`, hint: r.when, icon: ICONS[a.id], main: a.main, run: () => ((this.sel = r.key), this.run(a, cx)) });
      acts.push({ id: `p-go-${r.slug}`, label: `Go to  ${r.name}`, hint: r.when, icon: "chevron-right", run: () => this.goTo(r.slug, cx) });
      for (const x of r.runs) for (const a of x.acts) acts.push({ id: `p-${a.id}-${x.key}`, label: `${cap(a.label)}  ${r.name} ${x.when}`, hint: x.summary, icon: ICONS[a.id], run: () => ((this.sel = x.key), this.run(a, cx)) });
    }
    return [
      { heading: "Routines", items: own.filter((i) => hit(i.label)) },
      { heading: q.length ? "Actions" : "Run", items: (q.length ? acts : acts.filter((a) => a.main)).filter((i) => hit(`${i.label} ${i.hint ?? ""}`)) },
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
    const all = this.paletteGroups(cx).flatMap((g) => g.items);
    const item = picked ?? all[this.palette?.at ?? 0];
    this.closePalette(cx);
    if (item) item.run();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

  key(e, cx) {
    const s = e.keystroke;
    if (this.form) {
      if (s === "escape") this.cancelForm(cx);
      else if (s === "cmd-enter" || s === "ctrl-enter") this.submit(cx);
      else return false;
      return true;
    }
    if (this.confirm) {
      if (s === "escape") this.confirm = null;
      else if (s === "enter") {
        const a = this.confirm;
        this.confirm = null;
        cx.spawn(async (cx) => this.send(cx, a.action, a.args, a.confirm));
      } else return false;
      cx.notify();
      return true;
    }
    if (this.palette) {
      const n = this.paletteGroups(cx).flatMap((g) => g.items).length;
      if (s === "escape") this.closePalette(cx);
      else if (s === "down" || s === "ctrl-n") this.palette.at = Math.min(n - 1, this.palette.at + 1);
      else if (s === "up" || s === "ctrl-p") this.palette.at = Math.max(0, this.palette.at - 1);
      else return false;
      cx.notify();
      return true;
    }
    if (this.filtering) {
      if (s !== "escape") return false;
      this.filtering = false;
      this.filter = "";
      this.search.set_value("");
      this.handle.focus();
      cx.notify();
      return true;
    }
    const entry = this.entry(this.sel);
    const acts = entry?.acts ?? [];
    const own = acts.find((a) => a.key && stroke(a.key) === s);
    if (own) {
      this.run(own, cx);
      cx.notify();
      return true;
    }
    switch (s) {
      case "ctrl-p":
      case "cmd-shift-p":
        this.openPalette(cx);
        break;
      case "j":
      case "down":
        this.move(1);
        break;
      case "k":
      case "up":
        this.move(-1);
        break;
      case "pagedown":
      case "ctrl-d":
        this.move(10);
        break;
      case "pageup":
      case "ctrl-u":
        this.move(-10);
        break;
      case "g":
      case "home":
        this.move(-1e6);
        break;
      case "shift-g":
      case "end":
        this.move(1e6);
        break;
      case "]":
        this.moveGroup(1);
        break;
      case "[":
        this.moveGroup(-1);
        break;
      case "a":
        this.openForm(cx);
        break;
      case "shift-r":
        this.refresh(cx);
        break;
      case "x":
        if (entry?.unread) this.markRead(cx, entry.key);
        break;
      case "v":
        if (entry?.output) this.readOutput(entry, this.outputs.get(entry.output) || entry.summary || "");
        break;
      case "shift-x":
        this.readAll(cx);
        break;
      case "shift-c":
        this.cleanup(cx);
        break;
      case "space":
        this.fold();
        break;
      case "enter": {
        const main = acts.find((a) => a.main);
        if (main) this.run(main, cx);
        else this.fold();
        break;
      }
      case "/":
        this.filtering = true;
        this.search.focus();
        break;
      case "escape":
        this.filter = "";
        this.search.set_value("");
        break;
      case "m":
      case "shift-f10":
        if (this.sel) this.menu(this.sel);
        break;
      default:
        return false;
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  weekChart(k) {
    const days = this.model.days ?? [];
    const max = Math.max(1, ...days.map((d) => d.ok + d.failed + d.missed));
    const H = 34;
    return h_flex()
      .h(H + 16)
      .gap(5)
      .items_end()
      .children(
        days.map((d, i) => {
          const n = d.ok + d.failed + d.missed;
          const part = (v, c) => (v > 0 ? div().w_full().flex_none().h(Math.max(2, Math.round((v / max) * H))).bg(c) : null);
          return v_flex()
            .id(`week-${i}`)
            .flex_1()
            .min_w(10)
            .gap(3)
            .items_center()
            .tooltip(`${d.label} ${d.date} · ${d.ok} ok · ${d.failed} failed · ${d.missed} missed${d.cost ? ` · ~$${d.cost.toFixed(2)}` : ""}`)
            .child(
              v_flex()
                .w_full()
                .h(H)
                .justify_end()
                .rounded(2)
                .overflow_hidden()
                .when(n === 0, (e) => e.child(div().w_full().h(2).bg(alpha(k.border, 0.8))))
                .children([part(d.missed, k.warning), part(d.failed, k.destructive), part(d.ok, d.today ? k.success : alpha(k.success, 0.55))].filter(Boolean)),
            )
            .child(div().text_size(10).text_color(d.today ? k.foreground : k.dim).child(d.label.slice(0, 2)));
        }),
      );
  }

  tiles(k, cx) {
    const t = this.model.tiles;
    const next = t.next;
    const room = this.width - (this.width >= SIDE_AT ? 400 : 0);
    return Tiles(
      [
        Metric("tile-running", { label: "Running", value: t.running, tone: t.running ? k.working : k.foreground, spin: t.running > 0, icon: "play", hint: t.waiting ? `${t.waiting} waiting their turn` : `${t.active} of ${t.routines} active` }),
        v_flex()
          .id("tile-week")
          .flex_1()
          .min_w(200)
          .px(16)
          .py(12)
          .gap(4)
          .rounded(RADIUS.lg)
          .border_1()
          .border_color(k.border)
          .bg(k.card)
          .child(h_flex().gap(6).items_center().child(Icon("chart-column", { size: 13, color: k.mutedFg })).child(div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child("THIS WEEK")).child(div().flex_1()).child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${t.weekRuns} runs`)))
          .child(this.weekChart(k)),
        Metric("tile-failing", {
          label: "Needs a look",
          value: t.failing,
          tone: t.failing ? k.destructive : k.foreground,
          tint: t.failing > 0,
          icon: "triangle-alert",
          hint: t.weekMissed ? `${t.weekFailed} failed · ${t.weekMissed} missed this week` : `${t.weekFailed} failed this week`,
          onClick: (_e, cx) => {
            const bad = this.model.routines.find((r) => ["failed", "broken", "missed", "over"].includes(r.state));
            if (bad) this.select(bad.key);
            cx.notify();
          },
        }),
        Metric("tile-spent", {
          label: "Spent",
          value: t.spent,
          icon: "dollar",
          tone: t.overBudget ? k.warning : k.foreground,
          tint: Boolean(t.overBudget),
          hint: t.budget ? `${t.overBudget ? "over the" : "of"} $${t.budget.toFixed(2)} a week budgeted` : "estimated by Claude, over 7 days",
        }),
        room >= 860
          ? Metric("tile-next", {
              label: "Next run",
              value: next ? next.text : "—",
              icon: "alarm-clock",
              hint: next ? next.name : "nothing scheduled",
              onClick: next ? (_e, cx) => (this.select(next.slug), cx.notify()) : undefined,
            })
          : null,
      ],
      { narrow: room < 640 },
    );
  }

  historyStrip(id, history, k) {
    return h_flex()
      .flex_none()
      .gap(2)
      .items_center()
      .children(
        history.map((h, i) =>
          div()
            .id(`${id}-${i}`)
            .w(6)
            .h(14)
            .rounded(1.5)
            .bg((WORD[h.word] ?? WORD.failed)[2](k))
            .when(h.word === "ok", (e) => e.opacity(0.75))
            .tooltip(`${h.when} · ${h.word} · ${h.summary}`),
        ),
      );
  }

  stateIcon(s, k, size = 16) {
    const [, icon, c] = STATE[s] ?? STATE.never;
    return s === "running" ? Spinner({ size, color: c(k) }) : Icon(icon, { size, color: c(k) });
  }

  routineRow(row, k, sel) {
    const r = row.r;
    const flash = this.flashes.of(r.key);
    const wide = this.width >= 700;
    const chips = [r.run === "shell" ? "shell" : r.model || "claude", r.worktree ? "worktree" : null, r.over ? "over budget" : null].filter(Boolean);
    const sub = r.running ? `running for ${r.running.for} · ${r.running.why}` : r.queued ? "waiting its turn" : r.paused ? "paused" : r.error ? r.error : r.problems[0] ?? (r.nextText ? `next ${r.nextText}` : r.trigger === "event" ? "on its event" : "by hand");
    return h_flex()
      .id(`row-${r.key}`)
      .h(58)
      .px(12)
      .gap(12)
      .items_center()
      .rounded(RADIUS.md)
      .relative()
      .cursor_pointer()
      .border_1()
      .border_color(this.marked(r.slug) ? k.ring : alpha(k.border, 0))
      .bg(sel ? k.selected : k.background)
      .when(!sel, (e) => e.hover((s) => s.bg(k.accent)))
      .child(div().w(14).flex_none().child(row.fold === null ? "" : Icon(row.fold ? "chevron-down" : "chevron-right", { size: 14, color: k.dim })))
      .child(this.stateIcon(r.state, k))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .child(
            h_flex()
              .gap(8)
              .items_center()
              .child(div().flex_none().text_size(TEXT.base).font_semibold().text_color(r.paused ? k.mutedFg : k.foreground).child(r.name))
              .child(div().flex_none().text_size(TEXT.sm).text_color(r.error ? k.destructive : k.mutedFg).child(r.error ? "when?" : r.when))
              .when(r.unread > 0, (e) => e.child(Badge(`${r.unread} new`, "info"))),
          )
          .child(div().truncate().text_size(TEXT.xs).text_color(r.running ? k.working : r.problems.length || r.error ? k.warning : k.mutedFg).child(sub)),
      )
      .when(wide, (e) => e.child(h_flex().gap(4).flex_none().children(chips.map((c) => Chip(c, { mono: c !== "over budget", color: c === "over budget" ? k.warning : undefined })))))
      .when(wide && r.history.length > 0, (e) => e.child(this.historyStrip(`hist-${r.slug}`, r.history, k)))
      .child(div().w(64).flex_none().text_right().text_size(TEXT.xs).text_color(k.mutedFg).child(r.lastCost ?? ""))
      .when(Boolean(flash), (e) => e.child(div().id(`fl-${r.key}`).absolute().inset_0().rounded(RADIUS.md).bg(alpha(flash?.color ?? k.ring, 0.25)).opacity(flash?.on ? 1 : 0).transition("opacity", { duration: 1100, easing: "ease-out" })));
  }

  runRow(row, k, sel, inbox) {
    const x = row.run;
    const [, icon, c] = WORD[x.word] ?? WORD.failed;
    const flash = this.flashes.of(row.key);
    return h_flex()
      .id(`row-${row.key}`)
      .h(inbox ? 48 : 38)
      .pl(inbox ? 12 : 40)
      .pr(12)
      .gap(10)
      .items_center()
      .rounded(RADIUS.md)
      .relative()
      .cursor_pointer()
      .bg(sel ? k.selected : k.background)
      .when(!sel, (e) => e.hover((s) => s.bg(k.accent)))
      .when(!inbox, (e) => e.child(div().absolute().left(20).top(0).w(1).h(row.last ? 19 : 38).bg(k.border)).child(div().absolute().left(20).top(19).w(12).h(1).bg(k.border)))
      .child(div().size(7).flex_none().rounded(4).bg(x.unread ? k.ring : alpha(k.border, 0)))
      .child(Icon(icon, { size: 14, color: c(k) }))
      .child(
        inbox
          ? v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(h_flex().gap(8).items_center().child(div().text_size(TEXT.base).font_medium().child(x.name)).child(div().text_size(TEXT.xs).font_medium().text_color(c(k)).child(x.word === "ok" ? "done" : x.word)).when(x.late, (e) => e.child(Badge("late", "warning"))))
              .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(x.summary))
          : h_flex()
              .flex_1()
              .min_w(0)
              .gap(10)
              .items_center()
              .child(Mono(x.when, { size: TEXT.xs }))
              .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(x.ok ? k.foreground : c(k)).child(x.summary))
              .when(x.late, (e) => e.child(Badge("late", "warning")))
              .when(Boolean(x.worktree?.kept), (e) => e.child(Chip(x.worktree.branch, { icon: "git-branch", mono: true }))),
      )
      .child(div().flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(inbox ? x.ago : [x.took, x.cost].filter(Boolean).join(" · ")))
      .when(inbox && x.unread, (e) =>
        e.child(Button(`read-${row.key}`, { icon: "mail-check", variant: "ghost", size: "icon-sm", tooltip: "Mark read  x", onClick: (_e, cx) => (cx.stop_propagation(), this.markRead(cx, x.key)) })),
      )
      .when(Boolean(flash), (e) => e.child(div().id(`fl-${row.key}`).absolute().inset_0().rounded(RADIUS.md).bg(alpha(flash?.color ?? k.ring, 0.25)).opacity(flash?.on ? 1 : 0).transition("opacity", { duration: 1100, easing: "ease-out" })));
  }

  list(k, cx) {
    const rows = this.rows();
    if (!this.model.routines.length)
      return Empty({
        icon: "timer",
        title: "No routines yet",
        description: "A routine is a prompt or a script the hub runs on its own: every morning, every 30 minutes, or when CI fails.",
        action: Button("empty-add", { label: "New routine", icon: "plus", kbd: "a", onClick: (_e, cx) => this.openForm(cx) }),
      });
    const at = rows.findIndex((r) => r.key === this.sel);
    const items = rows.map((row) => {
      if (row.kind === "heading")
        return {
          height: 34,
          el: div()
            .pt(8)
            .child(
              StageHeading(`head-${row.key}`, row.label, {
                color: row.label === "Inbox" ? k.ring : k.mutedFg,
                count: row.count,
                pulse: row.label === "Inbox",
                action:
                  row.label === "Inbox"
                    ? Button("inbox-read-all", { label: "Mark all read", variant: "ghost", size: "sm", kbd: "X", onClick: (_e, cx) => this.readAll(cx) })
                    : this.model.tiles.worktrees
                      ? Button("clean-worktrees", { label: `Clean up ${this.model.tiles.worktrees} ${this.model.tiles.worktrees === 1 ? "worktree" : "worktrees"}`, icon: "brush-cleaning", variant: "ghost", size: "sm", kbd: "C", onClick: (_e, cx) => this.cleanup(cx) })
                      : null,
              }),
            ),
        };
      const sel = row.key === this.sel;
      const base = row.kind === "routine" ? this.routineRow(row, k, sel) : this.runRow(row, k, sel, row.kind === "inbox");
      const el = base
        .on_click((e, cx) => {
          const already = this.sel === row.key;
          this.select(row.key);
          this.handle.focus();
          if (row.kind === "routine" && (already || e.click_count > 1)) this.fold();
          else if (row.kind !== "routine" && e.click_count > 1) {
            const open = row.run.acts.find((a) => a.id === "open");
            if (open) this.run(open, cx);
          }
          cx.notify();
        })
        .on_mouse_down("right", (_e, cx) => {
          this.select(row.key);
          this.menu(row.key);
          cx.notify();
        });
      return { height: row.kind === "routine" ? 58 : row.kind === "inbox" ? 48 : 38, el };
    });
    return this.scroller.view("routines-list", items, at, { gap: 2, padding: 4 });
  }

  actionButtons(prefix, acts) {
    return h_flex()
      .gap(6)
      .flex_wrap()
      .children(
        acts
          .filter((a) => a.id !== "model")
          .map((a) =>
            Button(`${prefix}-${a.id}`, {
              label: a.id === "edit" ? "Change" : cap(a.label),
              icon: ICONS[a.id],
              kbd: a.key,
              size: "sm",
              variant: a.main ? "default" : a.id === "delete" ? "ghost" : "outline",
              onClick: (_e, cx) => this.run(a, cx),
            }),
          ),
      );
  }

  section(title, k) {
    return Overline(title).pt(4);
  }

  routineDetails(r, k, cx) {
    const out = [];
    const [label, , c, variant] = STATE[r.state] ?? STATE.never;
    out.push(h_flex().gap(8).items_center().flex_wrap().child(Badge(label, variant, { dot: c(k) })).child(div().text_size(TEXT.sm).text_color(k.foreground).child(r.when)));
    if (r.error) out.push(Callout(`err-${r.slug}`, { tone: k.destructive, label: "When?", title: r.whenText || "(empty)", meta: r.error }));
    for (const [i, p] of r.problems.entries()) out.push(Callout(`prob-${r.slug}-${i}`, { tone: k.warning, label: "Can't call", icon: "triangle-alert", title: p }));
    if (r.running) out.push(Callout(`run-${r.slug}`, { tone: k.working, label: "Running", title: `for ${r.running.for}`, meta: r.running.why, actions: [Button(`stop-${r.slug}`, { label: "Stop", icon: "circle-stop", variant: "outline", size: "sm", kbd: "s", onClick: (_e, cx) => cx.spawn(async (cx) => this.send(cx, "stop", [r.slug])) })] }));
    if (r.nextRuns.length && !r.paused) out.push(h_flex().gap(6).items_center().flex_wrap().child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Next")).children(r.nextRuns.map((n, i) => Chip(n.text, { mono: true, color: i === 0 ? k.info : undefined }))));
    if (r.history.length) {
      out.push(this.section(`Last ${r.history.length} runs`, k));
      out.push(this.historyStrip(`dhist-${r.slug}`, r.history, k));
    }
    out.push(this.section("Settings", k));
    out.push(
      h_flex()
        .gap(10)
        .items_center()
        .child(Toggle(`pause-${r.slug}`, !r.paused, (cx) => cx.spawn(async (cx) => this.send(cx, "pause", [r.slug, !r.paused])), { tooltip: r.paused ? "Resume  p" : "Pause  p" }))
        .child(div().text_size(TEXT.sm).child(r.paused ? "Paused: what it misses does not run" : "Active")),
    );
    if (r.run !== "shell") out.push(Pills(`model-${r.slug}`, MODELS, r.model, (v, cx) => cx.spawn(async (cx) => this.send(cx, "model", [r.slug, v]))));
    out.push(div().whitespace_normal().text_size(TEXT.xs).text_color(k.mutedFg).child(r.settings));
    if (r.budget) out.push(v_flex().gap(4).child(h_flex().justify_between().text_size(TEXT.xs).text_color(k.mutedFg).child("This week").child(`${r.spent} of $${r.budget.toFixed(2)}`)).child(Progress(r.spentWeek / r.budget, { color: r.over ? k.warning : k.success })));
    if (r.tools.length) out.push(h_flex().gap(4).flex_wrap().children(r.tools.map((t) => Chip(t, { mono: true }))));
    out.push(this.section(r.run === "shell" ? "Script" : "Prompt", k));
    out.push(div().p(10).rounded(RADIUS.md).bg(k.muted).whitespace_normal().text_size(TEXT.sm).text_color(k.foreground).when(r.run === "shell", (e) => e.font_family("JetBrains Mono")).child(r.prompt));
    out.push(this.section("Actions", k));
    out.push(this.actionButtons(`d-${r.slug}`, r.acts));
    out.push(h_flex().child(Button(`file-${r.slug}`, { label: "Edit the file", icon: "file-text", variant: "ghost", size: "sm", tooltip: String(r.path), onClick: (_e, cx) => cx.spawn(async (cx) => this.send(cx, "edit", [r.slug])) })));
    return out;
  }

  runDetails(x, k, cx) {
    const out = [];
    const [label, , c, variant] = WORD[x.word] ?? WORD.failed;
    out.push(
      h_flex()
        .gap(8)
        .items_center()
        .flex_wrap()
        .child(Badge(label, variant, { dot: c(k) }))
        .when(x.late, (e) => e.child(Badge("ran late", "warning")))
        .when(x.timedOut, (e) => e.child(Badge("timed out", "destructive")))
        .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child([x.when, x.took, x.cost, x.code ? `exit ${x.code}` : null].filter(Boolean).join(" · "))),
    );
    out.push(div().text_size(TEXT.sm).text_color(k.mutedFg).child(`Why: ${x.why}`));
    if (x.worktree?.kept)
      out.push(
        Callout(`wt-${x.key}`, {
          tone: k.info,
          label: "Worktree",
          icon: "git-branch",
          title: x.worktree.describe,
          meta: x.worktree.path,
          actions: [
            Button(`wt-open-${x.key}`, { label: "Open", variant: "outline", size: "sm", kbd: "b", onClick: (_e, cx) => cx.spawn(async (cx) => this.send(cx, "open_worktree", [x.slug, x.id])) }),
            Button(`wt-rm-${x.key}`, { label: "Remove", icon: "trash", variant: "ghost", size: "sm", tooltip: "Removes it as git allows: never by force", onClick: (_e, cx) => this.cleanup(cx, x) }),
          ],
        }),
      );
    const text = this.outputs.get(x.output);
    const shown = text || x.summary || "";
    out.push(
      h_flex()
        .items_center()
        .gap(6)
        .child(div().flex_1().child(this.section("Output", k)))
        .when(Boolean(shown), (e) =>
          e
            .child(Button(`r-copy-${x.key}`, { icon: "copy", variant: "ghost", size: "icon-sm", tooltip: "Copy the output", onClick: (_e, cx) => (cx.write_to_clipboard(shown), cx.notify()) }))
            .child(Button(`r-read-out-${x.key}`, { label: "Read", icon: "file-text", variant: "outline", size: "sm", kbd: "v", tooltip: "The whole output in a window", onClick: () => this.readOutput(x, shown) })),
        ),
    );
    // Rendered as the Markdown it is; a long one shows its start, the window shows all of it.
    out.push(
      text === undefined || text === null
        ? Skeleton({ h: 60 })
        : div().p(12).rounded(RADIUS.md).bg(k.muted).text_color(k.foreground).child(Markdown(`r-md-${x.key}`, shown.length > 2400 ? `${shown.slice(0, 2400)}\n\n…` : shown)),
    );
    out.push(this.section("Actions", k));
    out.push(
      h_flex()
        .gap(6)
        .flex_wrap()
        .child(this.actionButtons(`r-${x.key}`, x.acts))
        .when(x.unread, (e) => e.child(Button(`r-read-${x.key}`, { label: "Mark read", icon: "mail-check", variant: "ghost", size: "sm", kbd: "x", onClick: (_e, cx) => this.markRead(cx, x.key) }))),
    );
    return out;
  }

  side(k, cx) {
    const e = this.entry(this.sel);
    if (!e) return null;
    const run = Boolean(e.id && e.slug && e.started !== undefined);
    if (run && !this.outputs.has(e.output)) this.loadOutput(e.output, cx);
    return SidePanel("routines-side", {
      title: run ? e.name : e.name,
      description: run ? `run ${e.ago}` : e.when,
      width: 400,
      children: run ? this.runDetails(e, k, cx) : this.routineDetails(e, k, cx),
    });
  }

  body(k, cx) {
    const m = this.model;
    if (!m) {
      if (this.status.startsWith("error")) return ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) });
      return v_flex().flex_1().p(24).gap(16).child(h_flex().gap(12).children([0, 1, 2, 3].map(() => Skeleton({ h: 96 })))).children([0, 1, 2, 3].map(() => Skeleton({ h: 52 })));
    }
    if (!m.ready)
      return Empty({ icon: "timer", title: "No routines yet", description: "Add one: the hub runs it on its own.", action: Button("empty-add", { label: "New routine", icon: "plus", kbd: "a", onClick: (_e, cx) => this.openForm(cx) }) });
    return h_flex()
      .flex_1()
      .min_h(0)
      .child(
        v_flex()
          .flex_1()
          .h_full()
          .min_w(0)
          .min_h(0)
          .child(v_flex().px(24).pt(16).pb(8).flex_none().child(this.tiles(k, cx)))
          .child(h_flex().px(24).pb(4).flex_none().child(h_flex().id("routines-filter").flex_1().on_mouse_down("left", (_e, cx) => ((this.filtering = true), cx.notify())).child(SearchField(this.search, { active: this.filtering, kbd: "/" }))))
          .child(v_flex().flex_1().min_h(0).px(20).child(this.list(k, cx))),
      )
      .when(this.width >= SIDE_AT, (e) => e.child(this.side(k, cx) ?? div()));
  }

  render(cx) {
    const k = tokens();
    const m = this.model;
    const t = m?.tiles;
    const e = this.entry(this.sel);
    const description = m?.ready ? `${t.routines} routines · ${t.running ? `${t.running} running · ` : ""}${t.next ? `next ${t.next.name} ${t.next.text}` : "nothing scheduled"}` : "Prompts and scripts the hub runs on its own";
    let out = v_flex()
      .id("routines")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        if (Math.abs(size.width - this.width) > 4 || Math.abs(size.height - this.height) > 4) {
          this.width = size.width;
          this.height = size.height;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => {
        this.visible = true;
        if (!this.filtering && !this.palette && !this.form) this.handle.focus();
      })
      .on_key_down((ev, cx) => {
        if (this.key(ev, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "timer",
          title: "Routines",
          description,
          actions: [
            Button("routines-commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
            Button("routines-refresh", { label: "Refresh", icon: "refresh-cw", variant: "outline", size: "sm", kbd: "R", tooltip: "Read them again", onClick: (_e, cx) => this.refresh(cx) }),
            Button("routines-add", { label: "New routine", icon: "plus", size: "sm", kbd: "a", onClick: (_e, cx) => this.openForm(cx) }),
          ],
        }),
      )
      .child(this.body(k, cx))
      .when(this.width < SIDE_AT && Boolean(e?.acts?.length), (x) =>
        x.child(h_flex().px(24).py(10).gap(8).flex_none().items_center().border_t_1().border_color(k.border).bg(alpha(k.muted, 0.5)).child(this.actionButtons("bar", e.acts))),
      )
      .child(KeyHints([["j k", "move"], ["⏎", "run"], ["space", "runs"], ["a", "new"], ["e", "change"], ["x", "read"], ["m", "menu"], ["/", "filter"], ["ctrl p", "commands"]]));
    const toasts = this.toasts.render();
    if (toasts) out = out.child(toasts);
    if (this.form) out = out.child(this.formView(k, cx));
    if (this.confirm) {
      const a = this.confirm;
      out = out.child(
        AlertDialog("routines-confirm", {
          title: `${cap(a.label)}?`,
          description: a.question,
          confirmLabel: cap(a.label),
          destructive: a.id === "delete",
          onCancel: (_e, cx) => ((this.confirm = null), cx.notify()),
          onConfirm: (_e, cx) => {
            this.confirm = null;
            cx.spawn(async (cx) => this.send(cx, a.action, a.args, a.confirm));
            cx.notify();
          },
        }),
      );
    }
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id("routines-palette-backdrop")
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
                  .id("routines-palette")
                  .w(580)
                  .max_h(440)
                  .rounded(RADIUS.lg)
                  .border_1()
                  .border_color(k.border)
                  .bg(k.popover)
                  .shadow_lg()
                  .overflow_hidden()
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .child(Command("routines-commands-list", { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id })),
              ),
          ),
      );
    }
    return out;
  }
}
