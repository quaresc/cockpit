// Cockpit settings: every option of config.json, saved by the person's click or key (`act options set`), the checks of
// `cockpit doctor --json` with their fixes, the agents, the two layouts to switch between, and the installed and published
// versions with a check on demand.
// It also makes the "Getting started" checklist for the empty states; a confirmed fix runs the doctor's own command or
// action through the hub (`act doctor fix`, or `act doctor fix_all` for Fix all), never one this sends.

import { div, image } from "gpui-kit";
import * as base from "gpui-base";
import { h_flex, v_flex, InputState } from "gpui-base";
import { props, shared, setShared, openOverlay, closeOverlay, notify, run } from "herdr";
import * as hub from "./hub.js";
import { MOVE_FIRST } from "./person.js";
import { openInEditor, editorLabel } from "./editor.js";
import { versions, checkNow, versionLine, openUpdate } from "./update.js";
import { layoutName } from "./screens.js";
import { tokens, TEXT, RADIUS, Icon, Button, Badge, Tabs, Skeleton, Spinner, Toaster, Mono, Card, AlertDialog, Toggle, AgentLogo, Pills, TextField, viewport } from "./ui.js";

// A Herdr without ScrollHandle still draws the sheet; j/k then move without scrolling.
const ScrollHandle = base.ScrollHandle ?? null;
/** How many checks fail in the last report read, shared so every sidebar and drawer redraws its link. */
const FAILING = "cockpit.doctor.failing";

/** The categories of checks, in the order the sheet lists them. */
const CATEGORIES = {
  tools: "Tools and config",
  install: "Install",
  app: "Herdr app",
  agents: "Agents",
  claude: "Claude Code plugin",
  sessions: "Sessions",
  sources: "Sources",
  connectors: "Connectors",
  rules: "Permission rules",
  spaces: "Spaces",
  notifications: "Notifications",
  browser: "Browser for E2E tests",
};
/** What an agent can do, as the Agents tab says it: the report's key, its word, and a tooltip where the word needs one. */
const CAN = [["start", "start"], ["resume", "resume"], ["fork", "fork"], ["headless", "headless"], ["driven", "in chat", "Cockpit Chat runs it itself, without a terminal"]];

/** The doctor's report, `--cached` for one under ten minutes old; throws when it does not run. */
export async function loadDoctor({ cached = false } = {}) {
  const r = await hub.cockpit(["doctor", "--json", ...(cached ? ["--cached"] : [])]);
  let report;
  try {
    report = JSON.parse(r.stdout);
  } catch {
    throw new Error((r.stderr || r.stdout || "cockpit doctor did not answer").trim().split("\n")[0]);
  }
  const failing = report?.summary?.fail ?? 0;
  if (shared(FAILING) !== failing) setShared(FAILING, failing);
  return report;
}

/** How long after a fix's question opens its confirm takes a click. */
const CONFIRM_AFTER_MS = 400;

/** How long a file read may take, and a whole run of the checks, before the sheet stops waiting for it. */
const READ_MS = 10_000;
const CHECKS_MS = 90_000;

/** The tabs in order, by their number key. */
const TABS = ["options", "health", "agents", "layout"];

/** The two cockpit layouts the Layout tab offers, each with what it is for in a few plain lines. */
export const LAYOUT_CHOICES = [
  {
    name: "cockpit",
    title: "Cockpit",
    points: [
      "Your agents' terminals, with the inbox, review and runbooks beside them",
      "A Home board with what needs you, what comes next and your apps",
      "A sidebar with your quotas, Needs you and every agent and space",
      "Each app on a card, its keys shown on its buttons",
    ],
  },
  {
    name: "cockpit-chat",
    title: "Cockpit Chat",
    points: [
      "Reads one agent at a time as a conversation, its work folded into one line",
      "Can run a session itself, in chat, with no terminal behind it",
      "Questions and permissions are answered right above the box",
      "A quiet, nearly black-and-white look; the sidebar and side panel slide in when you need them",
    ],
  },
];

/** The size the layout pictures are saved at, for their height at any width. */
const PICTURE = { w: 800, h: 512 };

/** Whether a theme colour is dark: its luminance under one half; a colour it cannot read counts as dark. */
export function isDark(color) {
  const hex = /^#([0-9a-f]{6})/i.exec(String(color ?? ""))?.[1];
  if (!hex) return true;
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255);
  return 0.2126 * r + 0.7152 * g + 0.0722 * b < 0.5;
}

/** A layout's picture for the window's theme, as `image()` takes it from the Herdr app's home. */
export const layoutPicture = (name, dark) => `mods/cockpit/layouts/${name}-${dark ? "dark" : "light"}.png`;

/** A tab by the name a caller gives it; the checks' two older names both mean Health. */
const tabOf = (name) => (TABS.includes(name) ? name : name === "problems" || name === "all" ? "health" : null);

/** The sheet's width: 860 px, or the window's width less a margin on each side when it is narrower. */
export const sheetWidth = (window = viewport()) => (window?.width ? Math.max(320, Math.min(860, Math.floor(window.width) - 32)) : 860);

/**
 * Opens Cockpit settings: a window over the cockpit, on `tab` when given (options, health, agents, layout). `person` when the
 * person's click, key or palette pick opened it: what it shows counts as seen at once.
 */
export function openSettings({ tab = null, person = false } = {}) {
  openOverlay({ mod: "cockpit", kind: "window", key: "cockpit-settings", width: sheetWidth(), height: 0.86, props: { app: "settings", tab, person: Boolean(person) } });
}

/**
 * A quiet row that opens Cockpit settings, with how many checks fail when a report was read: the sidebar's and the
 * drawer's. It opens on Health while a check fails, the tab its count is about. `rail` keeps the icon alone.
 */
export function SettingsLink(id, { rail = false } = {}) {
  // A remote desk reads another machine: its settings and checks are that machine's.
  if (hub.remoteMachine()) return div();
  const k = tokens();
  const failing = Number(shared(FAILING) ?? 0);
  const tip = `Cockpit settings${failing ? `: ${failing} failing` : ""}`;
  const row = h_flex()
    .id(id)
    .h(28)
    .gap(8)
    .flex_none()
    .items_center()
    .rounded(RADIUS.md)
    .cursor_pointer()
    .text_color(k.mutedFg)
    .hover((s) => s.bg(k.accent).text_color(k.foreground))
    .on_click(() => openSettings({ tab: failing ? "health" : null, person: true }))
    .tooltip(chat() ? tip : `${tip}  ⌘⇧P, then Cockpit settings`)
    .child(Icon("sliders-horizontal", { size: 14, color: failing && !chat() ? k.destructive : undefined }));
  if (rail) return row.justify_center();
  return row
    .px(8)
    .child(div().flex_1().truncate().text_size(TEXT.sm).child("Cockpit settings"))
    .when(failing > 0, (e) => e.child(div().text_size(TEXT.xs).font_medium().text_color(chat() ? k.foreground : k.destructive).child(`${failing} failing`)));
}

/** The steps not done yet, in order. */
export const toDo = (report) => (report?.getting_started ?? []).filter((s) => !s.done);

/** What a fix runs, for its button: "Fix" when the hub can, null when it only says what to do. */
export const fixVerb = (fix) => (fix?.act ? "Fix" : fix?.command ? "Run" : null);

/** The check a fix goes through: a step names its own (null when no check says it), a check is itself. */
const checkOf = (item) => ("check" in item ? item.check : item.id);

/** Whether the hub can run this item's fix: a check of the report with a command or an action. */
export const runnable = (item) => Boolean(fixVerb(item?.fix) && checkOf(item));

/** A fix as the person saw it, as the hub compares it: its command, or `app action [args]`. */
export const shownFix = (fix) => fix?.command ?? `${fix?.act?.app ?? ""} ${fix?.act?.action ?? ""} ${JSON.stringify(fix?.act?.args ?? [])}`;

/** The hub actions a fix may name, in plain words. */
const ACTION_WORDS = {
  "sources refresh": () => "Reads every source again now.",
  "sources sign_in": (args) => `Opens the sign-in to ${args?.[0] ?? "that source"} in a tab of the actions space.`,
};

/** The question before a fix runs: `{title, description, confirm}`. */
export function fixQuestion(item) {
  const fix = item.fix ?? {};
  if (fix.act) {
    const words = ACTION_WORDS[`${fix.act.app} ${fix.act.action}`];
    return { title: "Fix it?", description: words ? words(fix.act.args) : `Runs ${fix.act.app} ${fix.act.action}.`, confirm: "Fix" };
  }
  return { title: "Run this command?", description: `${fix.command} runs in a tab of the actions space, where you can watch it.`, confirm: "Run" };
}

/**
 * Runs a confirmed fix through the hub, naming the report it was shown in (`report.at`) and the fix as shown, so the hub
 * runs that one or nothing; copies what it says when nothing runs it. The outcome is `toasts`' toast, and `done(cx)`
 * follows once the hub took it.
 */
export function runFix(item, report, cx, toasts, done) {
  const fix = item.fix;
  if (!runnable(item)) {
    const words = fix?.command ?? fix?.label;
    if (words) {
      cx.write_to_clipboard(words);
      toasts?.push(cx, "Copied what to do", { kind: "info", description: words });
    }
    return;
  }
  cx.spawn(async (cx) => {
    try {
      const r = await hub.act("doctor", "fix", [checkOf(item), report?.at ?? 0, shownFix(fix)]);
      toasts?.push(cx, r.error || r.note || "Fixing", { kind: r.error ? "error" : "info" });
      if (!r.error) done?.(cx);
    } catch (e) {
      toasts?.push(cx, String(e?.message ?? e), { kind: "error" });
    }
  });
}

/**
 * What Fix all would run, from the report shown: `items`, the checks that need a look and whose fix the hub can run,
 * and `left`, how many others need the person.
 */
export function fixAll(report) {
  const needs = (report?.checks ?? []).filter((c) => c.status !== "pass");
  const items = needs.filter(runnable);
  return { items, left: needs.length - items.length };
}

/** Fix all's confirm, one line per fix: `{command}` for a command, `{text}` for an action in plain words. */
export const fixAllLines = (items) => items.map((item) => (item.fix?.act ? { text: fixQuestion(item).description } : { command: item.fix.command, text: "runs in a tab of the actions space" }));

/**
 * Runs the confirmed fixes in one go through the hub, naming the report they were shown in, so the hub runs them all as
 * shown or none. Its toast says how many were fixed and failed, each fix's line under it; `done(cx)` follows once any ran.
 */
export function runFixAll(items, report, cx, toasts, done) {
  cx.spawn(async (cx) => {
    try {
      const r = await hub.act("doctor", "fix_all", [report?.at ?? 0, items.map((item) => [checkOf(item), shownFix(item.fix)])]);
      const [title, ...lines] = String(r.error || r.note || "Fixing").split("\n");
      toasts?.push(cx, title, { kind: r.error ? "error" : "info", description: lines.join("\n") || undefined, ms: 10_000 });
      if (!r.error) done?.(cx);
    } catch (e) {
      toasts?.push(cx, String(e?.message ?? e), { kind: "error" });
    }
  });
}

/** Cockpit Chat's quiet confirm: a plain card, Cancel holding the keyboard, no keys drawn. */
function quietConfirm(id, { title, description, confirmLabel, onConfirm, onCancel }) {
  const k = tokens();
  return div()
    .id(id)
    .absolute()
    .inset_0()
    .bg("#00000066")
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .child(
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          Card({
            id: `${id}-card`,
            title,
            description,
            padding: 24,
            gap: 16,
            footer: h_flex()
              .justify_end()
              .gap(8)
              .child(Button(`${id}-cancel`, { label: "Cancel", variant: "ghost", tooltip: "Leave it  esc", onClick: onCancel }))
              .child(Button(`${id}-ok`, { label: confirmLabel, variant: "outline", onClick: onConfirm })),
          })
            .w(420)
            .bg(k.popover)
            .shadow_lg(),
        ),
    );
}

/** Fix all's question: a line per fix, commands in mono, then how many others need the person. */
function fixAllQuestion(items, left) {
  const k = tokens();
  const line = (l, i) =>
    h_flex()
      .id(`fix-all-line-${i}`)
      .gap(6)
      .min_w(0)
      .items_center()
      .when(Boolean(l.command), (e) => e.child(div().id(`fix-all-command-${i}`).min_w(0).flex_shrink(1).truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.foreground).tooltip(l.command).child(l.command)))
      .child(l.command ? div().flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(l.text) : div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.sm).text_color(k.foreground).child(l.text));
  return {
    title: `Run these ${items.length} fixes?`,
    description: v_flex()
      .gap(6)
      .pt(4)
      .children(fixAllLines(items).map(line))
      .when(left > 0, (e) => e.child(div().pt(4).text_size(TEXT.sm).text_color(k.mutedFg).child(`${left} more need${left === 1 ? "s" : ""} you: see below`))),
    confirm: "Fix all",
  };
}

/** The confirm before a fix or Fix all, as each layout asks it: Cockpit's AlertDialog, Cockpit Chat's quiet card. */
function fixConfirm(id, asking, { onConfirm, onCancel }) {
  const q = asking.items ? fixAllQuestion(asking.items, asking.left) : fixQuestion(asking.item);
  const ask = { title: q.title, description: q.description, confirmLabel: q.confirm, onConfirm, onCancel };
  // A click answers it in both layouts: ⏎ does not, so its key is not drawn.
  return chat() ? quietConfirm(id, ask) : AlertDialog(id, { ...ask, confirmKbd: null });
}

const chat = () => shared("layout") === "cockpit-chat";
const statusIcon = (status) => (status === "pass" ? "circle-check" : status === "fail" ? "circle-x" : "triangle-alert");
// Cockpit Chat keeps colour for agents' states: there the icons' shapes say it, in grey.
const statusColor = (k, status) =>
  chat() ? (status === "pass" ? k.dim : status === "fail" ? k.foreground : k.mutedFg) : status === "pass" ? k.success : status === "fail" ? k.destructive : k.warning;

/** One step of getting started: done or not, and its fix beside it, or the question before it runs. */
function stepRow(k, step, { onFix, compact, asking, onConfirm, onCancel }) {
  const fix = step.fix;
  if (asking === step.id) {
    const q = fixQuestion(step);
    return h_flex()
      .id(`start-${step.id}`)
      .gap(8)
      .min_h(compact ? 28 : 32)
      .items_center()
      .child(Icon("circle-dashed", { size: 15, color: k.mutedFg }))
      .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.foreground).child(q.description))
      .child(Button(`start-cancel-${step.id}`, { label: "Cancel", variant: "ghost", size: "sm", onClick: (_e, cx) => onCancel(cx) }))
      .child(Button(`start-ok-${step.id}`, { label: q.confirm, size: "sm", variant: "outline", onClick: (_e, cx) => onConfirm(step, cx) }));
  }
  return h_flex()
    .id(`start-${step.id}`)
    .gap(10)
    .min_h(compact ? 28 : 32)
    .items_center()
    .child(Icon(step.done ? "circle-check" : "circle-dashed", { size: 15, color: step.done ? statusColor(k, "pass") : k.mutedFg }))
    .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(step.done ? k.mutedFg : k.foreground).when(step.done, (e) => e.line_through()).child(step.label))
    .when(!step.done && Boolean(fix), (e) =>
      e.child(
        Button(`start-fix-${step.id}`, {
          label: runnable(step) ? fixVerb(fix) : "Copy",
          icon: runnable(step) ? "play" : "copy",
          size: "sm",
          variant: "outline",
          tooltip: fix.command ?? fix.label,
          onClick: (_e, cx) => onFix(step, cx),
        }),
      ),
    );
}

/**
 * The "Getting started" checklist of a doctor report: each step done or not, with its fix; null when every step is
 * done. `onFix(step, cx)` asks before a step's fix runs; `asking` is the step being asked about, which `onConfirm(step,
 * cx)` runs and `onCancel(cx)` leaves.
 */
export function StartedList(report, { onFix, compact = false, title = "Getting started", asking = null, onConfirm, onCancel } = {}) {
  const steps = report?.getting_started ?? [];
  const left = steps.filter((s) => !s.done).length;
  if (!steps.length || !left) return null;
  const k = tokens();
  return v_flex()
    .id("getting-started")
    .w_full()
    .max_w(compact ? 520 : 640)
    .gap(4)
    .p(compact ? 14 : 16)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(k.border)
    .bg(k.card)
    .child(
      h_flex()
        .gap(8)
        .items_center()
        .pb(4)
        .child(div().flex_1().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(title))
        .child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${steps.length - left} of ${steps.length} done`)),
    )
    .children(steps.map((s) => stepRow(k, s, { onFix, compact, asking, onConfirm, onCancel })));
}

/**
 * A self-contained "Getting started" for an empty state: once the empty state shows it reads the kept doctor report (or
 * runs one), then draws `StartedList`, or nothing when every step is done. Make it in `init`, call `want(cx)` from
 * `render` while the empty state shows, and draw `el()` there: `this.started = new Started()` …
 * `this.started.want(cx)` … `this.started.el({ compact: true })`.
 */
export class Started {
  constructor() {
    this.report = null;
    this.loading = false;
    this.loaded = false;
    /** The step asked about, with the report it was shown from. */
    this.asking = null;
    this.toasts = new Toaster();
  }

  /** Reads the report the first time the empty state shows, and again after a fix (`cached: false`). */
  want(cx, { cached = true } = {}) {
    if (this.loading || (this.loaded && cached)) return;
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        this.report = await loadDoctor({ cached });
      } catch {
        this.report = null;
      }
      // A question asked from another report is not this one's.
      if (this.asking && this.asking.at !== this.report?.at) this.asking = null;
      this.loading = false;
      this.loaded = true;
      cx.notify();
    });
  }

  /** The checklist, with this component's own toasts, or null. */
  el({ compact = true, title } = {}) {
    const list = StartedList(this.report, {
      compact,
      title,
      asking: this.asking?.step.id ?? null,
      onFix: (step, cx) => {
        if (!runnable(step)) return runFix(step, this.report, cx, this.toasts);
        this.asking = { step, at: this.report?.at };
        cx.notify();
      },
      onCancel: (cx) => ((this.asking = null), cx.notify()),
      onConfirm: (_step, cx) => {
        const { step, at } = this.asking ?? {};
        this.asking = null;
        // An action is done when the hub answers: read again; a command runs on in its tab, the list waits.
        if (step) runFix(step, { at }, cx, this.toasts, (cx) => (step.fix?.act ? this.want(cx, { cached: false }) : null));
        cx.notify();
      },
    });
    if (!list) return null;
    const toasts = this.toasts.render();
    return toasts ? div().relative().w_full().child(list).child(toasts) : list;
  }
}

/** A heading over a group of rows: a category of checks, a section of options. */
function heading(k, id, text, spaced) {
  return div().id(id).pt(spaced ? 14 : 0).pb(2).text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(String(text).toUpperCase());
}

/** An option's value in words, as its toast and its default line say it: on, a choice's name, a number with its unit. */
export function optionWords(o, value = o.value) {
  if (o.kind === "toggle" || o.kind === undefined) return value ? "on" : "off";
  if (o.kind === "choice") return (o.choices ?? []).find((c) => c.value === value)?.label ?? String(value ?? "");
  if (o.kind === "number") return `${value}${o.unit ? ` ${o.unit}` : ""}`;
  return value === "" || value === null || value === undefined ? (o.blank ?? "empty") : String(value);
}

/** What a field's text is sent as: a whole number for a number field, else the text; the hub checks either. */
export const typedValue = (o, text) => (o.kind === "number" && /^-?\d+$/.test(String(text).trim()) ? Number(String(text).trim()) : String(text));

/** The next value of a choice (the one before with -1), going round. */
export function turned(o, by = 1) {
  const values = (o.choices ?? []).map((c) => c.value);
  if (!values.length) return o.value;
  const at = Math.max(0, values.indexOf(o.value));
  return values[(at + by + values.length) % values.length];
}

/** The sections of an options view in its order, each with its options and their place in the whole list. */
export function sectioned(view) {
  const options = view?.options ?? [];
  const names = [...(view?.sections ?? []), ...options.map((o) => o.section ?? "Options")].filter((name, i, all) => all.indexOf(name) === i);
  return names.map((title) => ({ title, rows: options.map((o, i) => [o, i]).filter(([o]) => (o.section ?? "Options") === title) })).filter((s) => s.rows.length > 0);
}

export default class Settings {
  init(p, cx) {
    this.p = p ?? props() ?? {};
    this.report = null;
    this.error = null;
    this.running = false;
    this.tab = tabOf(this.p.tab) ?? "options";
    /** Health lists every check, not only the ones that need a look. */
    this.all = this.p.tab === "all";
    this.at = 0;
    /** The hub's `options` view, its options, and the one selected among them. */
    this.view = null;
    this.options = null;
    this.optionAt = 0;
    /** What the window's link to the hub says: connecting, live, or "error: …" with the reason. */
    this.hubState = "connecting";
    this.starting = false;
    /** The options being saved, why one was not, and each field's input with what was typed in it. */
    this.saving = new Set();
    this.errors = {};
    this.fields = new Map();
    this.drafts = {};
    this.field = null;
    /** What runs and what is published, and whether a check asked for is under way. */
    this.v = null;
    this.checking = false;
    this.editor = "your editor";
    /** The fix asked about, with the report it was shown from. */
    this.asking = null;
    /** A command a fix typed into a tab runs on: the sheet keeps its report until checked again. */
    this.fixing = null;
    this.rowAt = {};
    // Any local process can open the sheet (`cockpit.settings`): ⏎ and space set or run nothing until the person
    // moves in it (j/k) or clicks in it.
    this.pointed = true;
    /** The person opened the sheet or acted in it: from then on what it shows counts as seen. */
    this.person = Boolean(this.p.person);
    this.toasts = new Toaster();
    this.handle = cx.focus_handle();
    this.handle.focus();
    this.scroll = ScrollHandle ? ScrollHandle.new() : null;
    this.check(cx, true);
    this.follow(cx);
    this.readVersions(cx);
  }

  /** Follows the hub's `options` view for as long as the sheet is open, and what the link to the hub says. */
  follow(cx) {
    this.sub = hub.subscribe(
      cx,
      { views: ["options"] },
      (m, cx) => {
        if (m.view !== "options") return;
        this.adopt(m.value);
        cx.notify();
      },
      (state, cx) => {
        this.hubState = state;
        cx.notify();
      },
    );
  }

  /**
   * The versions as the files say them, with no look at what is published, and the editor's name for its button. A read
   * the window never answers (one made while a layout switches) is given up and made again.
   */
  readVersions(cx) {
    cx.spawn(async (cx) => {
      for (let tries = 0; tries < 3 && !this.v; tries++) this.v = await hub.within(cx, versions(), READ_MS, "the versions").catch(() => null);
      cx.notify();
      this.editor = await hub.within(cx, editorLabel(), READ_MS, "the editor's name").catch(() => this.editor);
      cx.notify();
    });
  }

  /** One click or `u`: asks the hub to look at what is published now, and shows what it found or why it failed. */
  checkUpdates(cx) {
    if (this.checking) return;
    this.checking = true;
    cx.notify();
    cx.spawn(async (cx) => {
      this.v = await checkNow(cx);
      this.checking = false;
      cx.notify();
    });
  }

  /** Whether what is typed in an option's field differs from its value. */
  dirty(o, value = o.value) {
    const draft = this.drafts[o.key];
    return draft !== undefined && draft !== String(value ?? "");
  }

  /** A text or number option's input, on its value: typing notes a draft, ⏎ saves it. */
  fieldFor(o) {
    const state = InputState.new({ placeholder: o.kind === "number" ? String(o.default ?? "") : (o.blank ?? ""), value: String(o.value ?? "") });
    state.on("change", (_e, cx) => {
      this.drafts[o.key] = state.value();
      delete this.errors[o.key];
      cx?.notify?.();
    });
    state.on("submit", (_e, cx) => this.commit(o.key, cx));
    // The keyboard gets into a field by the person's click, or their ⏎ on its row.
    state.on("focus", (_e, cx) => {
      this.field = o.key;
      this.pointed = false;
      const at = (this.options ?? []).findIndex((x) => x.key === o.key);
      if (at >= 0) this.optionAt = at;
      cx?.notify?.();
    });
    state.on("blur", (_e, cx) => {
      if (this.field === o.key) this.field = null;
      cx?.notify?.();
    });
    return state;
  }

  /**
   * Takes the hub's view. A field shows its option's new value unless the person is typing in it or left a draft
   * there; `settled` names an option just saved, whose field takes the value as the hub wrote it.
   */
  adopt(view, settled = null) {
    if (!view || view.error) return;
    const before = new Map((this.options ?? []).map((o) => [o.key, o.value]));
    this.view = view;
    this.options = view.options ?? [];
    for (const o of this.options) {
      if (o.kind !== "text" && o.kind !== "number") continue;
      const shown = String(o.value ?? "");
      const state = this.fields.get(o.key);
      if (!state) this.fields.set(o.key, this.fieldFor(o));
      else if (settled === o.key || (this.field !== o.key && !this.dirty(o, before.get(o.key)))) {
        if (state.value() !== shown) state.set_value(shown);
        delete this.drafts[o.key];
      }
    }
    this.optionAt = Math.min(this.optionAt, Math.max(0, this.options.length - 1));
    this.seenShown();
  }

  /**
   * The options tab in front, opened or acted in by the person: what it lists is noted as seen, and keeps its "new"
   * marks while the sheet is open. A sheet a bare command opened notes nothing.
   */
  seenShown() {
    const unseen = (this.options ?? []).filter((o) => !o.seen && !this.noted?.has(o.key)).map((o) => o.key);
    if (!this.person || this.tab !== "options" || !unseen.length) return;
    this.noted = new Set([...(this.noted ?? []), ...unseen]);
    hub.act("options", "shown", [{ keys: unseen }]).catch(() => {});
  }

  /** An option of the list shown with some of it changed. */
  patch(key, change) {
    this.options = (this.options ?? []).map((x) => (x.key === key ? { ...x, ...change } : x));
  }

  /**
   * One option set through the hub, from a click or the person's own key. It shows at once; a save the hub refuses puts
   * it back and says why under it; a saved one says so in a toast that takes it back.
   */
  setOption(o, value, cx, { undo = true } = {}) {
    if (!o || o.by_hand || this.saving.has(o.key)) return;
    const before = o.value;
    this.saving.add(o.key);
    delete this.errors[o.key];
    this.patch(o.key, { value });
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("options", "set", [{ [o.key]: value }], cx);
        if (r?.unknown) throw new Error("this cockpit's hub cannot save options: update it first");
        if (r?.error) throw new Error(r.error);
        // The value as the hub wrote it (a time comes back as 08:05), and the field with it.
        const view = await hub.view("options").catch(() => null);
        if (view && !view.error) this.adopt(view, o.key);
        else this.fields.get(o.key)?.set_value(String(value));
        delete this.drafts[o.key];
        const now = (this.options ?? []).find((x) => x.key === o.key) ?? { ...o, value };
        this.toasts.push(cx, `${o.title}: ${optionWords(now)}`, {
          kind: "success",
          ms: 8000,
          action: undo ? { label: "Undo", onClick: (_e, cx) => this.setOption(now, before, cx, { undo: false }) } : undefined,
        });
      } catch (e) {
        this.patch(o.key, { value: before });
        this.errors[o.key] = String(e?.message ?? e).replace(/^set: /, "");
        // Why it was not saved shows under the option: it comes into view.
        const at = (this.options ?? []).findIndex((x) => x.key === o.key);
        if (at >= 0) this.pick(at);
      }
      this.saving.delete(o.key);
      cx.notify();
    });
  }

  /** What is typed in an option's field, saved when it differs from its value. */
  commit(key, cx) {
    const o = (this.options ?? []).find((x) => x.key === key);
    const state = this.fields.get(key);
    if (!o || !state) return;
    const text = state.value();
    if (text === String(o.value ?? "")) return;
    this.setOption(o, typedValue(o, text), cx);
  }

  /** A field's text back to its option's value, and the keyboard back to the sheet. */
  leaveField(cx) {
    const key = this.field;
    const o = (this.options ?? []).find((x) => x.key === key);
    if (o) this.fields.get(key)?.set_value(String(o.value ?? ""));
    delete this.drafts[key];
    delete this.errors[key];
    this.field = null;
    this.handle.focus();
    cx.notify();
  }

  /** The selected option's key: a switch flips, a choice takes its next value, ⏎ puts the keyboard in a field. */
  press(o, s, cx) {
    if (!o || o.by_hand) return;
    if (o.kind === "toggle") this.setOption(o, !o.value, cx);
    else if (o.kind === "choice") this.setOption(o, turned(o), cx);
    else if (s === "enter" && this.fields.get(o.key)) {
      this.field = o.key;
      this.fields.get(o.key).focus();
    }
  }

  /** Selects the `at`-th option and scrolls it into view; the last one brings what only config.json holds with it. */
  pick(at) {
    this.optionAt = at;
    const last = at === (this.options?.length ?? 0) - 1 && this.rowAt.end !== undefined;
    const row = last ? this.rowAt.end : this.rowAt[`option:${this.options?.[at]?.key}`];
    if (row !== undefined) this.scroll?.scroll_to_item(row);
  }

  /** config.json in the person's editor, as Open in editor opens a folder. */
  openFile(cx) {
    cx.spawn(async (cx) => {
      try {
        const path = this.view?.path ?? `${(await hub.install()).home}/.config/cockpit/config.json`;
        const failed = await openInEditor(path);
        if (failed) this.toasts.push(cx, "config.json did not open", { kind: "error", description: failed });
        else this.toasts.push(cx, `Opened config.json in ${this.editor}`, { kind: "info", description: path });
      } catch (e) {
        this.toasts.push(cx, "config.json did not open", { kind: "error", description: String(e?.message ?? e) });
      }
    });
  }

  /** Start it: the hub started by the person's click, a stopped one too; the sheet's link finds it by itself. */
  startHub(cx) {
    if (this.starting) return;
    this.starting = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await hub.startHub({ explicit: true, cx });
      } catch (e) {
        this.toasts.push(cx, "The hub did not start", { kind: "error", description: String(e?.message ?? e), ms: 8000 });
      }
      this.starting = false;
      cx.notify();
    });
  }

  /** The person's own key or click in the sheet: from then on what it shows counts as seen. */
  acted() {
    if (this.person) return;
    this.person = true;
    this.seenShown();
  }

  /** Shows a tab from its top, j/k starting there; ⏎ and space wait again for a move or a click on a row of it. */
  show(tab) {
    if (tab !== this.tab) {
      this.scroll?.scroll_to_item(0);
      this.pointed = true;
    }
    this.tab = tab;
    this.at = 0;
    this.seenShown();
  }

  /** Reads the kept report first when it is recent, then runs every check again; a run already going is enough. */
  check(cx, firstCached = false) {
    if (this.running) return;
    this.running = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        if (firstCached && !this.report) {
          this.report = await hub.within(cx, loadDoctor({ cached: true }), READ_MS, "cockpit doctor").catch(() => null);
          cx.notify();
        }
        this.report = await hub.within(cx, loadDoctor(), CHECKS_MS, "cockpit doctor");
        this.error = null;
        this.fixing = null;
        // A question asked from another report is not this one's: it closes, and the new checks say what is left.
        if (this.asking && this.asking.at !== this.report?.at) {
          this.asking = null;
          this.toasts.push(cx, "The checks ran again: look again", { kind: "info" });
        }
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.running = false;
      cx.notify();
    });
  }

  /** The sheet closed by the person: esc or its close button. */
  dismiss() {
    closeOverlay(props()?.overlay);
  }

  /** The sheet went: its link to the hub stops. */
  close() {
    this.sub?.stop();
    this.scroll?.release();
  }

  /** Asks once before every fix the hub can run, from the report shown; nothing when fewer than two. */
  askAll(cx) {
    const { items, left } = fixAll(this.report);
    if (items.length < 2) return;
    this.asking = { items, left, at: this.report?.at, opened: Date.now() };
    cx.notify();
  }

  /** Asks before `item`'s fix runs, from the report shown; a fix in words is copied at once. */
  ask(item, cx) {
    if (!runnable(item)) return runFix(item, this.report, cx, this.toasts);
    this.asking = { item, at: this.report?.at, opened: Date.now() };
    cx.notify();
  }

  /**
   * The asked fix, run as it was shown. An action is done when the hub answers, so the checks run again; a command
   * runs on in its tab, so the sheet waits for Check again.
   */
  confirmed(cx) {
    // A click that lands as the question opens was meant for what was there before.
    if (this.asking && Date.now() - (this.asking.opened ?? 0) < CONFIRM_AFTER_MS) return;
    const { item, items, at } = this.asking ?? {};
    this.asking = null;
    // Once a command runs in its tab the sheet waits for Check again; actions alone are done, so it checks at once.
    if (items) {
      const commands = items.filter((i) => !i.fix?.act).length;
      runFixAll(items, { at }, cx, this.toasts, (cx) => (commands ? ((this.fixing = `${commands === 1 ? "A command runs in its tab" : `${commands} commands run in their tabs`}: Check again once done`), cx.notify()) : this.check(cx)));
    }
    if (item) runFix(item, { at }, cx, this.toasts, (cx) => (item.fix?.act ? this.check(cx) : ((this.fixing = `${item.fix.command} runs in its tab: Check again once it is done`), cx.notify())));
    cx.notify();
  }

  /** The checks Health lists, in the order it draws them: the ones that need a look, or every one. */
  shown() {
    const checks = this.report?.checks ?? [];
    if (this.tab !== "health") return [];
    const list = this.all ? checks : checks.filter((c) => c.status !== "pass");
    const order = Object.keys(CATEGORIES);
    const rank = (c) => (order.includes(c.category) ? order.indexOf(c.category) : order.length);
    return [...list].sort((a, b) => rank(a) - rank(b));
  }

  /**
   * The sheet's keys: what sets an option, asks about a fix or opens the file takes the person's own key, not held,
   * after a move or a click on a row of that tab; an injected key only looks around. A fix's question is answered by a
   * click only, and while a field has the keyboard every key but esc is the field's.
   */
  key(e, cx) {
    const s = e.keystroke;
    const shown = this.shown();
    const own = !e.injected && !e.is_held;
    if (own) this.acted();
    if (this.asking) {
      if (s === "escape") this.asking = null;
      cx.notify();
      return true;
    }
    if (this.field) {
      if (s !== "escape") return false;
      this.leaveField(cx);
      return true;
    }
    const options = this.tab === "options" ? (this.options ?? []) : null;
    const o = options?.[this.optionAt];
    const move = ["j", "k", "down", "up"].includes(s);
    if (move && own) this.pointed = false;
    const turns = Boolean(options) && ["h", "l", "left", "right"].includes(s) && o?.kind === "choice" && !o.by_hand;
    const fixesAll = this.tab === "health" && s === "f" && fixAll(this.report).items.length > 1;
    const acts = turns || (options && (s === "space" || s === "enter" || s === "o")) || (!options && s === "enter" && shown[this.at]?.fix) || fixesAll;
    if (acts && !own) return true;
    if (acts && this.pointed) {
      this.toasts.push(cx, MOVE_FIRST, { kind: "info" });
      return true;
    }
    if (s === "escape") this.dismiss();
    else if (s === "u") this.checkUpdates(cx);
    else if (s === "shift-r" || s === "r") this.check(cx);
    else if (["1", "2", "3", "4"].includes(s)) this.show(TABS[Number(s) - 1]);
    else if (s === "a" && this.tab === "health") (this.all = !this.all), (this.at = 0), (this.pointed = true);
    else if (s === "o" && options) this.openFile(cx);
    else if (options && (s === "j" || s === "down")) this.pick(Math.min(this.optionAt + 1, Math.max(0, options.length - 1)));
    else if (options && (s === "k" || s === "up")) this.pick(Math.max(this.optionAt - 1, 0));
    else if (turns) this.setOption(o, turned(o, s === "h" || s === "left" ? -1 : 1), cx);
    else if (options && (s === "space" || s === "enter")) this.press(o, s, cx);
    else if (s === "j" || s === "down") this.reveal(Math.min(this.at + 1, Math.max(0, shown.length - 1)), shown);
    else if (s === "k" || s === "up") this.reveal(Math.max(this.at - 1, 0), shown);
    else if (s === "enter" && shown[this.at]?.fix) this.ask(shown[this.at], cx);
    else if (fixesAll) this.askAll(cx);
    else return false;
    cx.notify();
    return true;
  }

  /** A click on a check: it is the one selected, and ⏎ asks about its fix. */
  pick_check(id, cx) {
    const at = this.shown().findIndex((c) => c.id === id);
    if (at < 0) return;
    this.at = at;
    this.pointed = false;
    cx.notify();
  }

  /** Selects the `at`-th shown check and scrolls it into view (its place among the body's rows, as drawn last). */
  reveal(at, shown) {
    this.at = at;
    const row = this.rowAt[shown[at]?.id];
    if (row !== undefined) this.scroll?.scroll_to_item(row);
  }

  checkRow(k, c, cx, selected) {
    const verb = fixVerb(c.fix);
    return h_flex()
      .id(`check-${c.id}`)
      .on_click((_e, cx) => this.pick_check(c.id, cx))
      .gap(10)
      .py(6)
      .px(8)
      .mx(-8)
      .rounded(RADIUS.md)
      .when(selected, (e) => e.bg(k.selected))
      .items_start()
      .child(div().pt(1).child(Icon(statusIcon(c.status), { size: 15, color: statusColor(k, c.status) })))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(2)
          .child(div().text_size(TEXT.sm).text_color(k.foreground).child(c.message))
          .when(Boolean(c.fix?.label) && c.status !== "pass", (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(c.fix.label))),
      )
      .when(c.status !== "pass" && Boolean(c.fix), (e) =>
        e.child(
          Button(`fix-${c.id}`, {
            label: verb ?? "Copy",
            icon: verb ? "play" : "copy",
            size: "sm",
            variant: c.status === "fail" && verb ? "default" : "outline",
            // ⏎ runs the selected check's fix: its key shows there, a Kbd in Cockpit and in the tooltip in Cockpit Chat.
            kbd: selected ? "⏎" : undefined,
            tooltip: c.fix.act ? c.fix.label : verb ? `Runs ${c.fix.command} in a tab of the actions space` : "Copy what to do",
            onClick: (_e, cx) => this.ask(c, cx),
          }),
        ),
      );
  }

  /** The checks as rows, each category's heading before its own; `this.rowAt` notes where each check lands after `first` rows. */
  checksEl(k, list, cx, first = 0) {
    if (!list.length) {
      return [
        v_flex()
          .items_center()
          .gap(8)
          .p(32)
          .child(Icon("circle-check", { size: 22, color: chat() ? k.mutedFg : k.success }))
          .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Nothing needs a look: every check passed.")),
      ];
    }
    const groups = Object.keys(CATEGORIES).map((cat) => [cat, list.filter((c) => c.category === cat)]);
    const others = list.filter((c) => !CATEGORIES[c.category]);
    if (others.length) groups.push(["other", others]);
    const picked = this.shown()[this.at]?.id;
    const rows = [];
    for (const [cat, items] of groups.filter(([, items]) => items.length)) {
      rows.push(heading(k, `cat-${cat}`, CATEGORIES[cat] ?? "Other", rows.length > 0));
      for (const c of items) {
        this.rowAt[c.id] = first + rows.length;
        rows.push(this.checkRow(k, c, cx, c.id === picked));
      }
    }
    return rows;
  }

  /**
   * The window's link to the hub when it is not live: what it is doing or why it last failed, in the error's own
   * words, with Start it where this window may start one. Null while the hub answers.
   */
  hubRow(k, id) {
    const state = this.hubState;
    if (state === "live") return null;
    const connecting = state === "connecting";
    const missing = hub.isNotInstalled(state);
    const stopped = hub.isStoppedOnPurpose(state);
    const last = connecting ? (hub.lastFailure()?.message ?? "") : hub.downReason(state);
    const words = this.starting ? "Starting the cockpit hub…" : connecting ? "Reaching the cockpit hub…" : missing ? "Cockpit isn't set up in this Herdr" : `${hub.downWords(state)}: this window cannot reach it`;
    const why = stopped ? "It was stopped with cockpit hub stop, and no window starts it again until you do." : missing ? "Run cockpit app install, then open this again." : last ? `${connecting ? "The last try failed" : "Why"}: ${last}` : "";
    const startable = !connecting && !missing && !hub.remoteMachine() && !this.starting;
    return h_flex()
      .id(id)
      .gap(10)
      .px(12)
      .py(10)
      .items_start()
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .bg(k.muted)
      .child(div().pt(1).child(connecting || this.starting ? Spinner({ size: 14, color: k.mutedFg }) : Icon(missing ? "wrench" : "triangle-alert", { size: 15, color: chat() ? k.foreground : k.destructive })))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(2)
          .child(div().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(words))
          .when(Boolean(why), (e) => e.child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child(why))),
      )
      .when(startable, (e) => e.child(Button(`${id}-start`, { label: "Start it", size: "sm", variant: "outline", tooltip: "Start the cockpit hub", onClick: (_e, cx) => this.startHub(cx) })));
  }

  /** What sets an option: its switch, its choices as pills, or its field with Save once something else is typed. */
  control(k, o, i) {
    if (o.by_hand) return Badge("set in config.json", "outline", { tooltip: `${o.key} holds more than one value there: change it in the file` });
    const select = (cx) => {
      this.optionAt = i;
      this.pointed = false;
      cx.notify();
    };
    if ((o.kind ?? "toggle") === "toggle") {
      return Toggle(`option-toggle-${o.key}`, Boolean(o.value), (cx) => (select(cx), this.setOption(o, !o.value, cx)), { tooltip: `${o.value ? "Turn it off" : "Turn it on"}  space` });
    }
    if (o.kind === "choice") {
      const picks = (o.choices ?? []).map((c) => [c.value, c.label]);
      return h_flex().max_w(360).justify_end().child(Pills(`option-pick-${o.key}`, picks, o.value, (value, cx) => (select(cx), value !== o.value && this.setOption(o, value, cx))));
    }
    const state = this.fields.get(o.key);
    if (!state) return div();
    const busy = this.saving.has(o.key);
    // The field keeps the press it took: the sheet behind it would take the keyboard back to itself.
    const field = h_flex()
      .id(`option-field-${o.key}`)
      .w(o.kind === "number" ? 88 : 220)
      .on_mouse_down("left", (_e, cx) => {
        this.pointed = false;
        this.optionAt = i;
        cx.stop_propagation();
        cx.notify();
      })
      .child(TextField(state, { active: this.field === o.key }));
    return h_flex()
      .gap(6)
      .items_center()
      .child(field)
      .when(o.kind === "number" && Boolean(o.unit), (e) => e.child(div().w(24).text_size(TEXT.sm).text_color(k.mutedFg).child(o.unit)))
      .when(busy, (e) => e.child(Spinner({ size: 13, color: k.mutedFg })))
      .when(this.dirty(o) && !busy, (e) => e.child(Button(`option-save-${o.key}`, { label: "Save", size: "sm", kbd: "⏎", tooltip: "Save it", onClick: (_e, cx) => this.commit(o.key, cx) })));
  }

  /** One option: its name, its line, its default and key, why it was not saved when it was not, and what sets it. */
  optionRow(k, o, i) {
    const error = this.errors[o.key];
    const tone = chat() ? k.foreground : k.destructive;
    const meta = [`Default: ${optionWords(o, o.default)}`, o.since ? `since ${o.since}` : null, o.key].filter(Boolean).join(" · ");
    return h_flex()
      .id(`option-${o.key}`)
      .gap(16)
      .py(10)
      .px(8)
      .mx(-8)
      .items_start()
      .rounded(RADIUS.md)
      .border_1()
      // A ring, not a fill: an off switch's track is the selected row's colour.
      .border_color(i === this.optionAt ? k.ring : k.background)
      .on_click((_e, cx) => ((this.optionAt = i), (this.pointed = false), cx.notify()))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(3)
          .child(
            h_flex()
              .gap(8)
              .items_center()
              .child(div().text_size(TEXT.base).font_medium().text_color(k.foreground).child(o.title))
              .when(o.seen === false, (e) => e.child(Badge("new", "outline"))),
          )
          .child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child(o.description))
          .child(div().truncate().text_size(TEXT.xs).text_color(k.dim).child(meta))
          .when(Boolean(error), (e) =>
            e.child(
              h_flex()
                .id(`option-error-${o.key}`)
                .gap(6)
                .pt(2)
                .items_start()
                .child(div().pt(1).child(Icon("triangle-alert", { size: 13, color: tone })))
                .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.sm).text_color(tone).child(`Not saved: ${error}`)),
            ),
          ),
      )
      .child(div().pt(2).flex_none().child(this.control(k, o, i)));
  }

  /** What config.json holds that no switch or field sets, each key with its line, and the button that opens the file. */
  fileRows(k) {
    const open = Button("options-open-file", { label: `Open config.json in ${this.editor}`, icon: "external", variant: "outline", size: "sm", kbd: "O", tooltip: "~/.config/cockpit/config.json", onClick: (_e, cx) => this.openFile(cx) });
    const list = this.view?.in_file ?? [];
    return [
      heading(k, "options-in-file", "In config.json", true),
      h_flex()
        .id("options-file-intro")
        .gap(12)
        .pb(6)
        .items_center()
        .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child("What no switch or field sets. Edit the file by hand, then run cockpit doctor: it says when the file does not read."))
        .child(open),
      ...list.map((f, i) =>
        h_flex()
          .id(`options-file-${i}`)
          .gap(12)
          .py(4)
          .items_start()
          .child(div().w(168).flex_none().child(Mono(f.key, { size: TEXT.sm, color: k.foreground })))
          .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child(f.description)),
      ),
    ];
  }

  /**
   * The Options tab's rows: every option section by section, then what only the file holds. While the hub does not
   * answer it says so with Start it, and still opens the file.
   */
  optionRows(k) {
    const down = this.hubRow(k, "options-hub");
    if (!this.options) {
      if (!down || this.hubState === "connecting") return [...(down ? [div().pb(12).child(down)] : []), v_flex().gap(10).p(24).children([0, 1, 2].map((i) => Skeleton({ h: 16, w: 260 + i * 80 })))];
      return [
        down,
        h_flex()
          .id("options-unread")
          .gap(12)
          .pt(14)
          .items_center()
          .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child("The options are read and saved through the hub. config.json itself opens without it, and cockpit options lists them in a terminal."))
          .child(Button("options-open-file", { label: `Open config.json in ${this.editor}`, icon: "external", variant: "outline", size: "sm", onClick: (_e, cx) => this.openFile(cx) })),
      ];
    }
    if (!this.options.length) return [div().p(24).text_size(TEXT.sm).text_color(k.mutedFg).child("This cockpit has no options to set from here: cockpit options lists them in a terminal.")];
    const rows = down ? [div().pb(12).child(down)] : [];
    rows.push(div().pb(4).text_size(TEXT.sm).text_color(k.mutedFg).child("Saved in ~/.config/cockpit/config.json as you change them; cockpit options lists them in a terminal."));
    for (const section of sectioned(this.view)) {
      rows.push(heading(k, `section-${section.title}`, section.title, true));
      for (const [o, i] of section.rows) {
        this.rowAt[`option:${o.key}`] = rows.length;
        rows.push(this.optionRow(k, o, i));
      }
    }
    rows.push(...this.fileRows(k));
    this.rowAt.end = rows.length - 1;
    return rows;
  }

  /**
   * The window shows `name` instead, as the update dialog's Try does: a toast says how to come back. Only a click on its
   * button calls it, and the layout shown already changes nothing.
   */
  useLayout(name) {
    const choice = LAYOUT_CHOICES.find((l) => l.name === name);
    if (!choice || name === layoutName()) return;
    notify({ title: `This is ${choice.title}`, body: "Cockpit settings, under Layout, switches back.", level: "info", timeout: 8000 });
    this.dismiss();
    run("layout.switch", { name });
  }

  /** The Layout tab: a card per cockpit layout with its picture in the window's theme, side by side, stacked when narrow. */
  layoutEl(k) {
    const shown = layoutName();
    const dark = isDark(k.background);
    const room = Math.max(280, (this.width ?? 860) - 40);
    const stacked = room < 600;
    const cardW = stacked ? room : Math.floor((room - 16) / 2);
    const picW = cardW - 26;
    const card = (l) => {
      const using = l.name === shown;
      return v_flex()
        .id(`layout-${l.name}`)
        .w(cardW)
        .flex_none()
        .gap(12)
        .p(12)
        .rounded(RADIUS.lg)
        .border_1()
        .border_color(using && !chat() ? k.ring : k.border)
        .child(div().rounded(RADIUS.md).overflow_hidden().border_1().border_color(k.border).child(image(layoutPicture(l.name, dark)).w(picW).h(Math.round((picW * PICTURE.h) / PICTURE.w))))
        .child(
          h_flex()
            .h(28)
            .gap(8)
            .items_center()
            .child(div().flex_1().min_w(0).truncate().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(l.title))
            .when(using, (e) => e.child(Badge("In use", "secondary", { icon: "check" })))
            .when(!using, (e) =>
              e.child(Button(`layout-use-${l.name}`, { label: "Use this layout", icon: "layout", size: "sm", tooltip: `Switch this window to ${l.title}`, onClick: () => this.useLayout(l.name) })),
            ),
        )
        .child(
          v_flex()
            .gap(6)
            .children(
              l.points.map((point) =>
                h_flex()
                  .gap(8)
                  .items_start()
                  .child(div().mt(7).size(4).flex_none().rounded(2).bg(k.dim))
                  .child(div().flex_1().min_w(0).whitespace_normal().text_size(TEXT.base).text_color(k.mutedFg).child(point)),
              ),
            ),
        );
    };
    return v_flex()
      .gap(12)
      .child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child("Two ways to see the same agents, apps and hub. Switching keeps everything where it was."))
      .child((stacked ? v_flex() : h_flex().items_start()).gap(16).children(LAYOUT_CHOICES.map(card)));
  }

  agentsEl(k) {
    const agents = this.report?.agents ?? [];
    const row = (a) =>
      v_flex()
        .id(`agent-${a.id}`)
        .gap(6)
        .py(10)
        .border_b_1()
        .border_color(k.border)
        .child(
          h_flex()
            .gap(8)
            .items_center()
            .child(AgentLogo(a.icon ?? "agent", { size: 16, color: a.installed ? k.foreground : k.mutedFg }))
            .child(div().text_size(TEXT.base).font_medium().text_color(a.installed ? k.foreground : k.mutedFg).child(a.name))
            .when(a.default, (e) => e.child(Badge("default", "secondary")))
            .when(!a.installed, (e) => e.child(Badge("not installed", "outline")))
            .child(div().flex_1())
            .when(Boolean(a.path), (e) => e.child(Mono(a.path, { color: k.dim }))),
        )
        .child(
          h_flex()
            .gap(6)
            .flex_wrap()
            .children(CAN.map(([key, word, tip]) => Badge(word, a.can?.[key] ? "secondary" : "outline", { icon: a.can?.[key] ? "check" : "x", id: tip ? `agent-${a.id}-${key}` : undefined, tooltip: tip })))
            .child(Badge(a.tools ? `cockpit's tools: ${a.tools === "plugin" ? "its plugin" : "MCP"}${a.connected === false ? ", not connected" : ""}` : "no cockpit tools", a.connected === false && !chat() ? "warning" : "outline")),
        )
        .child(
          div()
            .text_size(TEXT.xs)
            .text_color(k.mutedFg)
            .child(`Usage: ${a.usage ?? "not reported"}${a.limits ? ` · limits: ${a.limits}` : ""}`),
        );
    const installed = agents.filter((a) => a.installed);
    const rest = agents.filter((a) => !a.installed);
    return v_flex()
      .gap(0)
      .children(installed.map(row))
      .when(rest.length > 0, (e) => e.child(div().pt(14).pb(2).text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(`NOT INSTALLED · ${rest.length}`)).children(rest.map(row)));
  }

  /**
   * Health's rows: what the checks found and Check again, the link to the hub when it is down (with why), getting
   * started, then the checks that need a look or every one.
   */
  healthRows(k, cx) {
    const r = this.report;
    const checks = r?.checks ?? [];
    const problems = checks.filter((c) => c.status !== "pass");
    const fixable = fixAll(r).items;
    const s = r?.summary ?? {};
    const at = r?.at ? new Date(r.at).toTimeString().slice(0, 5) : null;
    const summary = this.fixing
      ? this.fixing
      : r
        ? [s.fail ? `${s.fail} failing` : null, s.warn ? `${s.warn} to look at` : null, `${s.pass ?? 0} passed`, at ? `checked ${at}` : null].filter(Boolean).join(" · ")
        : (this.error ?? "Checking every part of cockpit…");
    const rows = [
      h_flex()
        .id("health-top")
        .gap(10)
        .pb(12)
        .items_center()
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(this.error && !chat() ? k.destructive : k.mutedFg).child(summary))
        .when(this.running, (e) => e.child(Spinner({ size: 14, color: k.mutedFg })))
        .child(
          Pills("health-filter", [["needs", `Needs a look ${problems.length}`, chat() ? "Only what needs a look  A" : undefined], ["all", `All checks ${checks.length}`, chat() ? "Every check  A" : undefined]], this.all ? "all" : "needs", (value, cx) => ((this.all = value === "all"), (this.at = 0), cx.notify())),
        )
        .when(fixable.length > 1, (e) =>
          e.child(
            Button("diag-fix-all", {
              label: `Fix all (${fixable.length})`,
              icon: "play",
              size: "sm",
              kbd: "F",
              tooltip: `Run the ${fixable.length} fixes cockpit can run, after one confirm`,
              onClick: (_e, cx) => this.askAll(cx),
            }),
          ),
        )
        .child(Button("diag-again", { label: "Check again", icon: "refresh-cw", variant: "outline", size: "sm", kbd: "R", tooltip: "Run every check again", onClick: (_e, cx) => this.check(cx) })),
    ];
    const down = this.hubRow(k, "health-hub");
    if (down) rows.push(div().pb(12).child(down));
    if (!r && !this.error) return [...rows, v_flex().gap(10).p(24).children([0, 1, 2, 3, 4].map((i) => Skeleton({ h: 16, w: 240 + ((i * 97) % 300) })))];
    const started = this.all ? null : StartedList(r, { onFix: (step, cx) => this.ask(step, cx) });
    if (started) rows.push(div().pb(16).child(started));
    if (this.all && !down) {
      rows.push(heading(k, "cat-window", "This window", false));
      rows.push(
        h_flex()
          .id("check-window-hub")
          .gap(10)
          .py(6)
          .items_start()
          .child(div().pt(1).child(Icon("circle-check", { size: 15, color: statusColor(k, "pass") })))
          .child(div().flex_1().min_w(0).text_size(TEXT.sm).text_color(k.foreground).child("This window reaches the cockpit hub")),
      );
    }
    // Nothing needs a look only when the hub answers too.
    if (!(down && !this.all && !problems.length)) rows.push(...this.checksEl(k, this.all ? checks : problems, cx, rows.length));
    return rows;
  }

  render(cx) {
    const k = tokens();
    const r = this.report;
    const problems = (r?.checks ?? []).filter((c) => c.status !== "pass");
    const line = versionLine(this.v, { checking: this.checking });
    const narrow = (this.width ?? 860) < 640;
    // The body's rows are the scroll area's own children, so j/k can bring the selected check or option into view.
    this.rowAt = {};
    const rows = this.tab === "options" ? this.optionRows(k) : this.tab === "layout" ? [this.layoutEl(k)] : this.tab === "agents" ? (!r && !this.error ? [v_flex().gap(10).p(24).children([0, 1, 2].map((i) => Skeleton({ h: 16, w: 240 + i * 90 })))] : [this.agentsEl(k)]) : this.healthRows(k, cx);
    let root = v_flex()
      .id("settings")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .on_size_change((size, cx) => {
        if (Math.abs((this.width ?? 0) - size.width) < 1) return;
        this.width = size.width;
        cx.notify();
      })
      .on_mouse_down("left", (_e, cx) => {
        this.acted();
        // A click outside a field takes the keyboard back to the sheet.
        if (!this.field) this.handle.focus();
        cx.notify();
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(
        h_flex()
          .h(60)
          .px(20)
          .gap(10)
          .flex_none()
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .child(h_flex().size(32).flex_none().rounded(RADIUS.lg).items_center().justify_center().border_1().border_color(k.border).bg(k.muted).child(Icon("sliders-horizontal", { size: 16, color: k.foreground })))
          .child(
            v_flex()
              .flex_1()
              .min_w(0)
              .gap(2)
              .child(div().text_size(TEXT.lg).font_semibold().child("Cockpit settings"))
              .child(div().id("settings-version").truncate().text_size(TEXT.sm).text_color(line.state === "failed" && !chat() ? k.destructive : k.mutedFg).tooltip(line.text).child(line.text)),
          )
          .when(this.checking, (e) => e.child(Spinner({ size: 14, color: k.mutedFg })))
          .when(line.update && !this.checking, (e) =>
            e.child(Button("settings-update", { label: narrow ? "Update" : `Update to ${this.v.latest}`, icon: "arrow-up", size: "sm", tooltip: `What ${this.v.latest} changes, and Install`, onClick: () => (this.dismiss(), openUpdate({ person: true })) })),
          )
          .child(
            Button("settings-check", {
              label: "Check for updates",
              icon: "refresh-cw",
              variant: "outline",
              size: narrow ? "icon-sm" : "sm",
              kbd: "U",
              disabled: this.checking,
              tooltip: narrow ? "Check for updates  U" : "Ask now whether a newer cockpit is published",
              onClick: (_e, cx) => this.checkUpdates(cx),
            }),
          )
          .child(Button("settings-close", { icon: "x", variant: "ghost", size: "icon", tooltip: "Close  esc", onClick: () => this.dismiss() })),
      )
      .child(
        h_flex()
          .px(20)
          .py(10)
          .flex_none()
          .child(
            Tabs("settings-tabs", {
              items: [
                { value: "options", label: "Options", count: (this.options ?? []).filter((o) => o.seen === false).length || null },
                { value: "health", label: "Health", count: r ? problems.length + (this.hubState.startsWith("error") ? 1 : 0) : null },
                { value: "agents", label: "Agents", count: r ? (r.agents ?? []).filter((a) => a.installed).length : null },
                { value: "layout", label: "Layout" },
              ],
              value: this.tab,
              onChange: (v, cx) => (this.show(v), cx.notify()),
            }),
          ),
      )
      .child(
        ((area) => (this.scroll ? area.track_scroll(this.scroll) : area))(v_flex().id("settings-body").flex_1().min_h(0).px(20).pb(20).overflow_y_scrollbar()).children(rows),
      );
    const toasts = this.toasts.render();
    if (toasts) root = root.child(toasts);
    if (this.asking) {
      root = root.child(
        fixConfirm("diag-confirm", this.asking, { onCancel: (_e, cx) => ((this.asking = null), cx.notify()), onConfirm: (_e, cx) => this.confirmed(cx) }),
      );
    }
    return root;
  }
}
