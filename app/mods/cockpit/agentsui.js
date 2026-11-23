// The agents, as the sidebar and the recap popup draw them: the hub's `agents` view (see docs/app.md) followed live,
// its actions with their feedback, and the pieces both draw: stage marks, the question card answered in place, the
// facts as chips, the quota bars with their pace tick, and the right-click menu of an agent.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { run, notify, openOverlay, agents, shared, setShared } from "herdr";
import * as hub from "./hub.js";
import { sharing } from "./feed.js";
import { rowsOf, answerPayload, changed, CHANGED, agentStatus } from "./fleet.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Spinner, ChoiceKey, Button, Checkbox, Chip, Mono, Markdown, TextField, dur, openMenu, iconOf } from "./ui.js";
import { showWork, layoutName, readInChat, pinChat, PENDING_VIEW } from "./screens.js";
import { QuotaGroups } from "./usage.js";
import { oneTimeKey, take } from "./person.js";

// The agents, Home and queue views, followed once per window whichever copies ask (feed.js).
const following = sharing(hub.subscribe);

// Pinned agents, one list for the sidebar and the Home board: kept in the app's shared state so a pin made in one
// redraws the other at once; shared() is persisted, so pins survive a restart. Each is an agent's `app:<address>`.
const PINS_KEY = "cockpit.pins";
export function pinsNow() {
  const live = shared(PINS_KEY);
  return Array.isArray(live) ? live : [];
}
export function togglePinOf(terminal) {
  const now = pinsNow();
  const next = now.includes(terminal) ? now.filter((p) => p !== terminal) : [...now, terminal];
  setShared(PINS_KEY, next);
  return next;
}

/** Each stage: its label, color token and mark. Whose move it is, yours first. */
export const STAGES = {
  needs_you: { label: "Needs you", tone: "blocked" },
  failed: { label: "Stopped with an error", tone: "destructive" },
  done: { label: "Done", tone: "done" },
  fixing: { label: "Needs fixing", tone: "destructive" },
  idle: { label: "Idle", tone: "idle" },
  working: { label: "Working", tone: "working" },
  review: { label: "In review", tone: "success" },
  merged: { label: "Merged", tone: "merged" },
  snoozed: { label: "Snoozed", tone: "info" },
  dismissed: { label: "Dismissed", tone: "dim" },
};

/** Whether the person can dismiss this row's card: it asks or stopped with an error, here and not yet set aside. */
export const dismissable = (row) => Boolean(row) && !row.herdr && !row.remote && !row.dismissed && (row.stage === "needs_you" || row.stage === "failed");

/** The colour of an agent's status (`agentStatus`'s words): failed is the destructive one. */
export function statusColor(k, status) {
  return status === "failed" ? k.destructive : (k[status] ?? k.idle);
}

export function stageColor(stage) {
  const k = tokens();
  const tone = STAGES[stage]?.tone ?? "dim";
  if (tone === "merged") return k.t.ansi5 ?? k.mutedFg;
  return k[tone] ?? k.dim;
}

/** The word for how long a row has been in its stage: "waiting 4m", "done 9m", "working 2m". */
export function stageAge(row, now = Date.now()) {
  const age = dur(now - (row.since ?? now));
  const word = { needs_you: "waiting", failed: "stopped", done: row.interrupted ? "stopped by you" : "done", working: "working", snoozed: row.snoozed?.label ?? "snoozed", dismissed: "dismissed" }[row.stage];
  return row.stage === "snoozed" || row.stage === "dismissed" ? word : word ? `${word} ${age}` : age;
}

/**
 * The stage's mark: a breathing dot while it needs you, a spinner while it works, a ring at rest. A dismissed agent
 * keeps the colour of its real `status`, as a still ring.
 */
export function StageMark(id, stage, { size = 8, status } = {}) {
  const c = stageColor(stage);
  if (stage === "dismissed") return div().size(size).flex_none().rounded(size / 2).border_1().border_color(statusColor(tokens(), agentStatus(status ?? "blocked")));
  if (stage === "working") return Spinner({ size: size + 4, color: c });
  if (stage === "snoozed") return Icon("alarm-clock", { size: size + 4, color: c });
  if (stage === "merged") return Icon("git-merge", { size: size + 4, color: c });
  if (stage === "failed") return Icon("circle-x", { size: size + 4, color: c });
  if (stage === "idle") return div().size(size).flex_none().rounded(size / 2).border_1().border_color(c);
  return div()
    .id(`${id}-mark`)
    .size(size)
    .flex_none()
    .rounded(size / 2)
    .bg(c)
    .when(stage === "needs_you", (e) => e.loop("opacity", { from: 0.35, to: 1, period: 1200 }));
}

/** Seconds as "1h 20m" / "35m" / "<1m". */
export function hours(secs) {
  const m = Math.round((secs ?? 0) / 60);
  if (m < 1) return "<1m";
  if (m < 60) return `${m}m`;
  return `${Math.floor(m / 60)}h${m % 60 ? ` ${m % 60}m` : ""}`;
}

/** The row's facts as chips: branch, changes, ticket, PR, CI, time on its ticket. `few` keeps the ones that matter most. */
export function Chips(row, { few = false, id = row.terminal } = {}) {
  const k = tokens();
  const out = [];
  const c = row.changes;
  // What tells a stopped agent from one still waiting: the error it stopped on, or that the person stopped it.
  if (row.stage === "failed" && row.error) out.push(Chip(String(row.error).split("\n")[0].slice(0, 60), { id: `${id}-error`, icon: "circle-x", color: k.destructive, tooltip: String(row.error) }));
  if (row.interrupted) out.push(Chip("Stopped by you", { id: `${id}-stopped`, icon: "circle-stop", tooltip: "You stopped its last turn: it waits for what to do instead" }));
  if (row.ci) out.push(Chip(row.ci.label, { id: `${id}-ci`, color: { fail: k.destructive, pass: k.success, running: k.working }[row.ci.state], icon: { fail: "circle-x", pass: "circle-check", running: "loader" }[row.ci.state] }));
  if (row.pr) {
    const review = { APPROVED: "approved", CHANGES_REQUESTED: "changes asked" }[row.pr.review];
    out.push(
      Chip(`#${row.pr.number}${review ? ` · ${review}` : ""}`, {
        id: `${id}-pr`,
        icon: row.pr.state === "merged" ? "git-merge" : row.pr.draft ? "git-pull-request-draft" : "git-pull-request",
        color: row.pr.review === "CHANGES_REQUESTED" ? k.destructive : row.pr.review === "APPROVED" ? k.success : undefined,
        tooltip: row.pr.title,
        onClick: () => run("cockpit.target:" + row.pr.url),
      }),
    );
  }
  if (!few && row.ticket) out.push(Chip(row.ticket.status ? `${row.ticket.key} ${row.ticket.status}` : row.ticket.key, { id: `${id}-ticket`, icon: "ticket", color: k.info, tooltip: row.ticket.title ?? row.ticket.key, onClick: row.ticket.url ? () => run("cockpit.target:" + row.ticket.url) : undefined }));
  if (!few && row.branch) out.push(Chip(row.branch, { id: `${id}-branch`, mono: true, icon: "git-branch" }));
  if (c && (c.files || c.ahead)) {
    out.push(
      h_flex()
        .h(22)
        .px(8)
        .gap(5)
        .flex_none()
        .items_center()
        .rounded(RADIUS.md)
        .bg(k.muted)
        .text_size(11.5)
        .text_color(k.mutedFg)
        .when(c.files > 0, (e) => e.child(div().text_color(k.foreground).child(`${c.files} ${c.files === 1 ? "file" : "files"}`)).child(div().text_color(k.success).child(`+${c.add}`)).child(div().text_color(k.destructive).child(`−${c.del}`)))
        .when(c.ahead > 0, (e) => e.child(div().child(`${c.files ? "· " : ""}${c.ahead} ahead`))),
    );
  }
  if (!few && row.time?.ticket_today) out.push(Chip(`${hours(row.time.ticket_today)} today`, { id: `${id}-time`, icon: "timer", tooltip: `On ${row.ticket?.key ?? "this work"}: ${hours(row.time.ticket_today)} today, ${hours(row.time.ticket_week)} this week` }));
  return out;
}

/** Provider limits grouped under their logos, with missing readings and unsupported sources named separately. */
export function QuotaBars(quota, { id = "quota", compact = false, rows = [], tightest = false } = {}) {
  return QuotaGroups(quota, { id, compact, rows, tightest });
}

// ---------------------------------------------------------------------------------------------------------------------
// The model and its actions

/** Herdr's own agents as rows (status, title, place), for when the hub does not answer. */
export function herdrRows() {
  const stage = { blocked: "needs_you", failed: "failed", working: "working", done: "done", idle: "idle" };
  return agents().map((a) => ({
    terminal: `app:${a.address}`,
    address: a.address,
    stage: stage[agentStatus(a.status)] ?? "idle",
    title: a.tab_name && !/^\d+$/.test(a.tab_name) ? a.tab_name : a.title || a.address,
    space: { id: a.space, name: a.space_name },
    tab: { id: a.tab, name: a.tab_name },
    icon: iconOf(a),
    program: a.program,
    agent_name: a.program?.name ?? null,
    herdr: true,
  }));
}

/** Follows the agents view and runs its actions; one per view that draws agents, named after it in its answers. */
export class AgentsModel {
  constructor(view = "sidebar") {
    this.view = view;
    this.value = null;
    this.state = "connecting";
    // What is being done to a row: terminal -> a word ("answering", "recapping"), until the next model.
    this.busy = new Map();
    // Choices ticked in a multiple choice before Submit: terminal -> Set(n).
    this.picks = new Map();
    // The free-text reply being typed: {terminal, choice}.
    this.replying = null;
    this.reply = InputState.new({ placeholder: "Your answer… ⏎ sends" });
    // The question each card last drew, by terminal: a key answers what was drawn, not a newer one not drawn yet.
    this.drawn = new Map();
    // Cards whose "what it would do" is opened, by terminal.
    this.opened = new Set();
    // The card dismissed last, while its Undo shows: {terminal, title, until}.
    this.undo = null;
    // The view's own toasts, when it has room for them: the Undo goes there; else the view draws `UndoRow`.
    this.toasts = null;
  }

  /** The pinned agents, as the shared list holds them now. */
  get pins() {
    return pinsNow();
  }

  /** The question a card drew for this row, else the row's own. */
  shown(row) {
    return this.drawn?.get(row.terminal) ?? row.question;
  }

  /** Subscribes; `onChange(value, before, cx)` after each new model. */
  follow(cx, onChange = () => {}) {
    this.reply.on("submit", (_e, cx) => this.sendReply(cx, "key"));
    this.sub = following(
      cx,
      { views: [{ name: "agents", args: { frontend: "app" } }] },
      (m, cx) => {
        if (m.view !== "agents") return;
        const before = this.value;
        this.value = m.value;
        this.busy.clear();
        onChange(m.value, before, cx);
        cx.notify();
      },
      (state, cx) => {
        this.state = state;
        // Down from the first failure until the hub answers again: the retries between say "connecting".
        if (state === "live") this.down = false;
        else if (state.startsWith("error")) [this.down, this.lastError] = [true, state];
        cx.notify();
      },
    );
  }

  stop() {
    this.sub?.stop();
  }

  /** The hub's rows; while it doesn't answer, Herdr's own agents (status, title, place), so none vanishes. */
  get rows() {
    if (!this.down || this.value?.remote) return rowsOf(this.value);
    return herdrRows();
  }

  row(terminal) {
    return this.rows.find((r) => r.terminal === terminal);
  }

  /** An action on one agent, its note as a toast when it says something. */
  /**
   * One of the agents view's actions, bounded on the context's timer and its outcome said; the promise gives
   * `{ok, unconfirmed}`, unconfirmed when it went and no answer came back.
   */
  act(cx, action, arg, busy) {
    if (busy) this.busy.set(arg.terminal, busy);
    cx.notify();
    let taken;
    const done = new Promise((resolve) => (taken = resolve));
    cx.spawn(async (cx) => {
      let ok = false;
      let unconfirmed = false;
      try {
        const r = await hub.act("agents", action, [arg], cx);
        ok = !r.error;
        // The card showed a question that is gone: say so plainly; the hub sends the newer one, and the card redraws.
        if (r.error && changed(r.error)) notify({ title: CHANGED, level: "info", timeout: 4000 });
        else if (r.error) notify({ title: "Could not do it", body: r.error, level: "warning", timeout: 5000 });
        else if (r.note && action === "answer") notify({ title: r.note, level: "success", timeout: 2500 });
        else if (r.note && action !== "focus" && action !== "seen") notify({ title: r.note, level: "info", timeout: 2500 });
      } catch (e) {
        unconfirmed = hub.isUnconfirmed(e);
        if (!unconfirmed) notify({ title: "The cockpit hub did not answer", body: String(e?.message ?? e), level: "warning" });
      }
      this.busy.delete(arg.terminal);
      taken({ ok, unconfirmed });
      cx.notify();
    });
    return done;
  }

  /**
   * Dismisses the row's card until the agent asks something new, Undo beside it (`from` a menu: on Herdr's toast); a
   * dismissed row comes back instead. The person's own act: a click, a real key or a menu's keyed command.
   */
  dismiss(cx, row, back = Boolean(row?.dismissed), from = "card") {
    if (!row || row.herdr || row.remote) return;
    const title = row.title || row.tab?.name || row.address;
    this.busy.set(row.terminal, back ? "bringing it back" : "dismissing");
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        // The wait the card showed goes with it: the hub refuses it once the agent waits on something else.
        const r = await hub.act("agents", "dismiss", [{ terminal: row.terminal, undo: back, ...(back || !row.wait ? {} : { wait: row.wait }) }], cx);
        if (r.error && changed(r.error)) notify({ title: CHANGED, level: "info", timeout: 4000 });
        else if (r.error) notify({ title: "Could not do it", body: r.error, level: "warning", timeout: 5000 });
        else if (!back && from === "menu") undoToast(row.terminal, title);
        else if (!back) this.offerUndo(cx, row.terminal, title);
        else if (this.undo?.terminal === row.terminal) this.undo = null;
      } catch (e) {
        if (!hub.isUnconfirmed(e)) notify({ title: "The cockpit hub did not answer", body: String(e?.message ?? e), level: "warning" });
      }
      this.busy.delete(row.terminal);
      cx.notify();
    });
  }

  /** The Undo of a dismissal, for a few seconds: on the view's toast when it has one, else as its `UndoRow`. */
  offerUndo(cx, terminal, title) {
    const undo = (_e, cx) => {
      this.undo = null;
      this.act(cx, "dismiss", { terminal, undo: true });
    };
    if (this.toasts) return this.toasts.push(cx, `Dismissed ${title}`, { kind: "success", description: "Back when it asks something new.", ms: UNDO_MS, action: { label: "Undo", onClick: undo } });
    const until = Date.now() + UNDO_MS;
    this.undo = { terminal, title, until, onClick: undo };
    cx.timer.after(UNDO_MS, (cx) => {
      if (this.undo?.until === until) this.undo = null;
      cx.notify();
    });
  }

  /** A choice picked by a click or a key (`how`): it answers, ticks a box, or opens the free field. */
  answer(cx, row, choice, how = "click") {
    const q = this.shown(row);
    const picked = q?.choices.find((c) => c.n === choice);
    if (!picked) return;
    // A permission cockpit cannot show in full is answered in the agent's terminal: the hub says so in its own words.
    if (q.terminal_only) return notify({ title: q.terminal_only, level: "info", timeout: 4000 });
    if (picked.free) return this.startReply(cx, row, choice);
    // A choice that runs every tool unasked asks in a step of its own, Cancel first: a second click is not a yes.
    if (unaskedChoice(q, picked)) return confirmUnasked(row.terminal, q, picked, "agents|unasked|");
    if (q?.kind === "multi") {
      const set = this.picksOf(row);
      set.has(choice) ? set.delete(choice) : set.add(choice);
      return cx.notify();
    }
    this.act(cx, "answer", answerPayload(row.terminal, q, { choice }, `${this.view}:${how}`), "answering");
  }

  /** A multiple choice's ticks: what the screen shows until the person changes them here. */
  picksOf(row) {
    if (!this.picks.has(row.terminal)) this.picks.set(row.terminal, new Set((row.question?.choices ?? []).filter((c) => c.checked).map((c) => c.n)));
    return this.picks.get(row.terminal);
  }

  submit(cx, row, how = "click") {
    const choices = [...this.picksOf(row)];
    this.picks.delete(row.terminal);
    this.act(cx, "answer", answerPayload(row.terminal, this.shown(row), { choices }, `${this.view}:${how}`), "answering");
  }

  startReply(cx, row, choice = null) {
    this.replying = { terminal: row.terminal, choice };
    this.reply.set_value("");
    this.reply.focus();
    cx.notify();
  }

  cancelReply(cx) {
    this.replying = null;
    cx.notify();
  }

  sendReply(cx, how = "click") {
    const r = this.replying;
    const text = this.reply.value().trim();
    this.replying = null;
    if (!r || !text) return (cx.notify(), Promise.resolve({ ok: false }));
    const row = this.row(r.terminal);
    const q = row ? this.shown(row) : null;
    const multi = q?.kind === "multi";
    const picked = { text, ...(r.choice ? { choice: r.choice } : {}), ...(multi ? { choices: [...this.picksOf(row)] } : {}) };
    return this.act(cx, "answer", answerPayload(r.terminal, q, picked, `${this.view}:${how}`), "answering");
  }

  /** Brings the agent into view; `person` false (a window command) marks nothing seen. */
  go(cx, row, person = true) {
    // A session cockpit drives has no terminal: it is read in Cockpit Chat.
    if (row.driven) return readInChat(row.terminal, null, person);
    // An agent picked on purpose, the one in front too: the chat follows it again.
    pinChat(null);
    run("agent.focus", { terminal: row.address });
    showWork();
    // A runner's hub takes no writes from a remote desk.
    if (person && !row.herdr && !row.remote) this.act(cx, "seen", { terminal: row.terminal });
  }

  /** The right-click menu of an agent; its items come back as `cockpit.agents|<what>|<terminal>|<arg>`. */
  menu(row) {
    // Every item is a hub action.
    if (row.herdr) return;
    const t = row.terminal;
    const c = (what, arg = "") => `agents|${what}|${t}|${arg}`;
    const pinned = this.pins.includes(t);
    // The Cockpit layout's menus say their keys; Cockpit Chat keeps keys to tooltips and the keys sheet.
    const kbd = (k) => (layoutName() === "cockpit" ? k : undefined);
    if (row.driven) {
      return openMenu(
        [
          { label: "Read in Cockpit Chat", icon: "message-square", kbd: kbd("⏎"), command: c("read") },
          !row.remote && { label: "Open in a terminal", icon: "terminal", command: c("handover") },
          { label: row.recap?.text ? "Recap again" : "Recap", icon: "sparkles", kbd: kbd("r"), command: c("recap") },
          "separator",
          dismissable(row) && { label: "Dismiss until it asks again", icon: "x", command: c("dismiss", row.wait ?? "") },
          row.dismissed && { label: "Bring back to Needs you", icon: "undo", command: c("restore") },
          { label: pinned ? "Unpin" : "Pin to the top", icon: pinned ? "pin-off" : "pin", command: c("pin") },
        ].filter(Boolean),
        { width: 230 },
      );
    }
    openMenu(
      [
        { label: "Go to agent", icon: "corner-down-left", kbd: kbd("⏎"), command: c("go") },
        { label: "Peek", icon: "eye", command: c("peek") },
        { label: row.recap?.text ? "Recap again" : "Recap", icon: "sparkles", kbd: kbd("r"), command: c("recap") },
        "separator",
        row.snoozed ? { label: "Wake", icon: "alarm-clock", command: c("wake") } : { label: "Snooze 1 hour", icon: "alarm-clock", command: c("snooze", "1h") },
        !row.snoozed && { label: "Snooze until 18:00", icon: "clock", command: c("snooze", "18:00") },
        !row.snoozed && { label: "Snooze until tomorrow", icon: "calendar", command: c("snooze", "tomorrow") },
        !row.snoozed && { label: "Snooze till it changes", icon: "hourglass", command: c("snooze", "done") },
        row.unread && !row.remote && { label: "Mark as seen", icon: "check", command: c("seen") },
        dismissable(row) && { label: "Dismiss until it asks again", icon: "x", command: c("dismiss", row.wait ?? "") },
        row.dismissed && !row.remote && { label: "Bring back to Needs you", icon: "undo", command: c("restore") },
        "separator",
        { label: "Fork to a new tab", icon: "git-fork", command: c("fork", "tab") },
        { label: "Fork to the right", icon: "square-split", command: c("fork", "right") },
        { label: pinned ? "Unpin" : "Pin to the top", icon: pinned ? "pin-off" : "pin", command: c("pin") },
      ],
      { width: 230 },
    );
  }

  /**
   * `cockpit.agents|…` from a menu, run by the sidebar's copy alone (every shown copy hears it); true when it was one.
   * Without the menu's key (`key`) it only brings the agent into view: a window command acts on no agent.
   */
  command(c, cx, key = null) {
    if (!c.startsWith("agents|")) return false;
    const [, what, terminal, arg] = c.split("|");
    if (what === "unasked") {
      // Confirmed from the step its menu opened, by that step's one-time key, for the question it was asked about.
      const found = takeConfirm(terminal);
      const row = found && this.row(found.handle);
      const q = row && this.shown(row);
      if (!q) return true;
      if ((q.asked ?? q.ask ?? "") !== found.asked) notify({ title: CHANGED, level: "info", timeout: 4000 });
      else this.act(cx, "answer", answerPayload(row.terminal, q, { choice: found.n, confirm: true }, `${this.view}:confirm`), "answering");
      return true;
    }
    const row = this.row(terminal);
    if (!row) return true;
    const person = take(c, key);
    if (what === "go") this.go(cx, row, person);
    else if (what === "read") readInChat(terminal, null, person);
    // The hand-over asks first, in the chat: the session leaves it for a terminal of its own.
    else if (what === "handover") readInChat(terminal, "handover", person);
    else if (what === "peek") run("terminal.peek", { terminal: row.address });
    else if (what === "pin") this.togglePin(terminal);
    else if (!person) return true;
    else if (what === "recap") this.act(cx, "recap", { terminal, force: true }, "recapping");
    else if (what === "snooze") this.act(cx, "snooze", { terminal, until: arg });
    else if (what === "wake") this.act(cx, "snooze", { terminal, until: null });
    else if (what === "seen") this.act(cx, "seen", { terminal });
    // Each names what it does, and a dismissal the wait its menu showed: the hub refuses one the agent moved on from.
    else if (what === "dismiss") this.dismiss(cx, { ...row, wait: arg || row.wait }, false, "menu");
    else if (what === "restore") this.dismiss(cx, row, true, "menu");
    else if (what === "fork") this.act(cx, "fork", { terminal, place: arg || "tab" });
    cx.notify();
    return true;
  }

  togglePin(terminal) {
    togglePinOf(terminal);
  }

  /** Moves pins this copy kept in its own localStorage (per mod copy) into the shared list, once. */
  loadPins() {
    if (Array.isArray(shared(PINS_KEY))) return;
    try {
      const old = JSON.parse(localStorage.getItem("agents.pins") ?? "[]");
      if (Array.isArray(old) && old.length) setShared(PINS_KEY, old);
    } catch {}
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// The question card

/**
 * What a permission would allow (the command, the diff, the plan), in a box: its first lines always show, the rest
 * folds in a narrow card until its row is clicked. `cut` says its first lines are past the top of the agent's screen;
 * `markdown` draws it rendered (a plan), in a taller box.
 */
export function AllowsBlock(id, text, { open = true, onToggle, cut = false, note = null, markdown = false } = {}) {
  const k = tokens();
  // Nothing of it read: it says so, and its answer is the agent's terminal's.
  if (!text) cut = true;
  const lines = String(text).split("\n");
  const folds = Boolean(onToggle) && lines.length > ALLOWS_FIRST;
  const shown = open || !folds ? String(text) : lines.slice(0, ALLOWS_FIRST).join("\n");
  const rest = lines.length - ALLOWS_FIRST;
  const head = h_flex()
    .id(`${id}-allows`)
    .gap(4)
    .items_center()
    .cursor_pointer()
    .text_size(TEXT.xs)
    .text_color(k.mutedFg)
    .hover((s) => s.text_color(k.foreground))
    .tooltip(open ? "Fold what it would do" : "See exactly what it would do")
    .on_click((_e, cx) => {
      cx.stop_propagation();
      onToggle?.(cx);
    })
    .child(Icon(open ? "chevron-down" : "chevron-right", { size: 12 }))
    .child(open ? "Fold the rest" : `${rest} more line${rest === 1 ? "" : "s"}`);
  return v_flex()
    .gap(4)
    .when(cut, (e) =>
      e.child(
        h_flex()
          .gap(4)
          .items_center()
          .text_size(TEXT.xs)
          .text_color(k.warning)
          .child(Icon("triangle-alert", { size: 12 }))
          .child(note || "Part of what it would run is off its screen: answer it in its terminal"),
      ),
    )
    .when(Boolean(text), (e) => e.child(
      div()
        .id(`${id}-allows-body`)
        .max_h(markdown ? 480 : 220)
        .overflow_y_scroll()
        .p(markdown ? 12 : 8)
        .rounded(RADIUS.md)
        .border_1()
        .border_color(k.border)
        .bg(k.background)
        .child(markdown ? Markdown(`${id}-allows-md`, shown) : Mono(shown, { color: k.foreground })),
    ))
    .when(folds, (e) => e.child(head));
}

/** What a plan is about: its first heading, else its first line, without Markdown marks. */
export function planTitle(text) {
  const lines = String(text ?? "").split("\n").map((l) => l.trim()).filter(Boolean);
  const line = lines.find((l) => l.startsWith("#")) ?? lines[0] ?? "";
  return line.replace(/^#+\s*/, "").replace(/[*`]/g, "").trim();
}

/** The number of the plan a view keeps (`plan-<session>-<n>`), or null. */
export const planNumber = (view) => Number(/-(\d+)$/.exec(view ?? "")?.[1]) || null;

/**
 * Opens a plan kept as a view of `row`'s session in the side panel: the agent comes into view first, so the panel
 * follows its session, then the panel goes to that view.
 */
export function openPlan(row, view) {
  pendView(row, view);
  if (row.driven) readInChat(row.handle ?? row.terminal, null, true);
  else {
    pinChat(null);
    run("agent.focus", { terminal: row.address });
    showWork();
  }
  run(`cockpit.doc:${view}`);
}

/** Tells the side panel to open on `view` once it follows `row`'s session. */
export function pendView(row, view) {
  setShared(PENDING_VIEW, { terminal: row.driven ? (row.handle ?? row.terminal) : row.address, name: view, at: Date.now() });
}

/**
 * A plan asked about, as one line: its number and title, read whole in the side panel with one click (`onOpen`), where
 * it stays once carried out.
 */
export function PlanRow(id, q, onOpen) {
  const k = tokens();
  const n = planNumber(q.view);
  const title = planTitle(q.preview) || "The plan";
  return h_flex()
    .id(`${id}-plan`)
    .w_full()
    .min_h(34)
    .pl(10)
    .pr(4)
    .py(4)
    .gap(8)
    .items_center()
    .rounded(RADIUS.md)
    .border_1()
    .border_color(k.border)
    .bg(k.background)
    .cursor_pointer()
    .hover((s) => s.bg(k.accent))
    .tooltip("Read the whole plan in the side panel")
    .on_click((_e, cx) => {
      cx.stop_propagation();
      onOpen(cx);
    })
    .child(Icon("file-text", { size: 14, color: k.mutedFg }))
    .child(div().flex_none().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(n ? `Plan ${n}` : "Plan"))
    .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.foreground).child(title))
    .child(h_flex().flex_none().gap(4).items_center().px(6).text_size(TEXT.xs).text_color(k.mutedFg).child("Read").child(Icon("panel-right", { size: 13 })));
}

/** Whether picking `choice` turns on a mode that runs every tool unasked: the hub names those choices per question. */
export function unaskedChoice(q, choice) {
  return Array.isArray(q?.unasked) && q.unasked.includes(choice?.n);
}

export { oneTimeKey };

/** The confirm steps a menu opened and nothing answered yet, by their one-time key: one at a time. */
const confirms = new Map();
const CONFIRM_FOR_MS = 60_000;

/**
 * The step a choice that runs every tool unasked takes: a menu with Cancel first and the choice spelled out, Cancel
 * sending the prefix alone. Its item carries a key the menu made, which the view's `command` reads back from
 * `<prefix><key>` with `takeConfirm`, so a window command from outside names no confirm.
 */
export function confirmUnasked(handle, q, choice, prefix) {
  confirms.clear();
  const key = oneTimeKey();
  confirms.set(key, { handle, n: choice.n, asked: q?.asked ?? q?.ask ?? "", at: Date.now() });
  openMenu(
    [
      { label: "Cancel", icon: "x", command: prefix },
      { label: `${choice.label}: every tool runs without asking`, icon: "triangle-alert", command: `${prefix}${key}` },
    ],
    { width: 380 },
  );
}

/** The confirm a menu item names, used once: {handle, n, asked}, or null for a key unknown, used, or past a minute. */
export function takeConfirm(key) {
  const found = confirms.get(key);
  confirms.clear();
  return found && Date.now() - found.at < CONFIRM_FOR_MS ? found : null;
}

/** What a blocked agent with nothing to answer here waits on, as the hub words it (`waits`), or null. */
export const waitsText = (row) => (row?.question ? null : row?.waits?.text ?? null);

/**
 * A waiting agent that asks nothing cockpit can read: what it waits on (its terminal, or the limit it reached) and the
 * way to it. It takes no reply: its screen shows no question to answer.
 */
export function WaitsCard(id, row, model, { wide = false, header, footer, focus, dismiss = true } = {}) {
  const k = tokens();
  const tone = k.blocked;
  const busy = model.busy.get(row.terminal);
  model.drawn?.delete(row.terminal);
  const held = Boolean(focus?.is_focused?.());
  const actions = [
    Button(`${id}-go`, { label: "Go to agent", icon: "corner-down-left", variant: "outline", size: "sm", tooltip: "Answer it in its terminal", onClick: (_e, cx) => model.go(cx, row) }),
    Button(`${id}-snooze`, { icon: "alarm-clock", variant: "ghost", size: "icon-sm", tooltip: "Snooze…", onClick: () => model.menu(row) }),
  ];
  const aside = !dismiss ? null : dismissable(row) ? DismissButton(id, row, model) : row.dismissed && !row.remote ? RestoreButton(id, row, model) : null;
  return v_flex()
    .id(id)
    .w_full()
    .flex_none()
    .p(wide ? 12 : 10)
    .gap(wide ? 8 : 6)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(held ? tone : alpha(tone, 0.45))
    .bg(alpha(tone, 0.07))
    .when(Boolean(focus), (e) => e.track_focus(focus))
    .on_mouse_down("right", () => model.menu(row))
    .when(Boolean(header), (e) => e.child(header))
    .child(
      h_flex()
        .gap(6)
        .items_start()
        .child(div().pt(1).child(Icon(row.waits.kind === "limit" ? "hourglass" : "terminal", { size: 14, color: tone })))
        .child(div().flex_1().min_w(0).whitespace_normal().text_size(wide ? TEXT.base : TEXT.sm).font_medium().text_color(k.foreground).child(row.waits.text)),
    )
    .when(Boolean(busy), (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(Spinner({ size: 12, color: tone })).child(`${busy}…`)))
    .when(!busy, (e) => e.child(h_flex().gap(6).items_center().children(actions).child(div().flex_1()).when(Boolean(footer), (f) => f.child(footer)).when(Boolean(aside), (f) => f.child(aside))));
}

/**
 * The Undo of a dismissal made from a menu, on Herdr's toast, which shows wherever the person is. Its click only opens
 * a menu naming the agent: the window's own key goes with the pick made there.
 */
export function undoToast(terminal, title) {
  notify({ title: `Dismissed ${title}`, body: "Back when it asks something new. Click to bring it back.", level: "success", timeout: 8000, action: { name: `cockpit.dismiss:undo:${terminal}` } });
}

/** How long a dismissal's Undo shows. */
const UNDO_MS = 8000;

/** A card's Dismiss, at the end of its actions: it sets the agent aside until it asks something new. */
export function DismissButton(id, row, model) {
  return Button(`${id}-dismiss`, {
    icon: "x",
    variant: "ghost",
    size: "icon-sm",
    tooltip: "Dismiss until it asks something new",
    onClick: (_e, cx) => (cx.stop_propagation(), model.dismiss(cx, row)),
  });
}

/** A dismissed agent's card says so where its Dismiss was, and brings it back from there. */
export function RestoreButton(id, row, model) {
  return Button(`${id}-restore`, {
    label: "Bring back",
    icon: "undo",
    variant: "ghost",
    size: "sm",
    tooltip: "Dismissed: back to Needs you",
    onClick: (_e, cx) => (cx.stop_propagation(), model.dismiss(cx, row)),
  });
}

/**
 * The Undo of the card dismissed last, as a row for a view too narrow for a toast (the sidebar, the drawer): null once
 * its time is up.
 */
export function UndoRow(id, model) {
  const u = model.undo;
  if (!u || u.until < Date.now()) return null;
  const k = tokens();
  return h_flex()
    .id(id)
    .w_full()
    .flex_none()
    .min_h(34)
    .pl(10)
    .pr(6)
    .py(4)
    .gap(8)
    .items_center()
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(k.border)
    .bg(k.popover)
    .child(Icon("check", { size: 13, color: k.success }))
    .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.foreground).tooltip(`Dismissed ${u.title}: back when it asks something new`).child(`Dismissed ${u.title}`))
    .child(Button(`${id}-undo`, { label: "Undo", variant: "outline", size: "sm", onClick: u.onClick }));
}

/** How many of what a permission would do always show, folded or not: never a command that was not drawn. */
const ALLOWS_FIRST = 3;

/**
 * A waiting agent's question, answered where it shows: a click on a choice answers (or ticks, in a multiple
 * choice, then Submit), the free-text choice opens a field, a question without choices takes words. `wide` draws the
 * popup's roomier card; `header` is drawn above; `focus` is a handle the card holds the keyboard with.
 */
export function QuestionCard(id, row, model, { wide = false, header, footer, focus, dismiss = true } = {}) {
  if (!row.question && row.waits) return WaitsCard(id, row, model, { wide, header, footer, focus, dismiss });
  const k = tokens();
  const q = row.question ?? { text: row.said || "It is waiting for you.", kind: "text", choices: [] };
  model.drawn?.set(row.terminal, q);
  const opened = wide || Boolean(model.opened?.has(row.terminal));
  const toggle = wide ? undefined : (cx) => {
    model.opened ??= new Set();
    model.opened.has(row.terminal) ? model.opened.delete(row.terminal) : model.opened.add(row.terminal);
    cx.notify();
  };
  // Each question draws its own choices: a click begun on one question never lands on the next one drawn there.
  const named = String(q.ask ?? q.asked ?? "").replace(/[^A-Za-z0-9_-]/g, "");
  const tone = k.blocked;
  const busy = model.busy.get(row.terminal);
  const replying = model.replying?.terminal === row.terminal;
  const multi = q.kind === "multi";
  const picks = multi ? model.picksOf(row) : null;
  const choice = (c, i) => {
    // A permission is answered on purpose: nothing is drawn as already picked.
    const on = multi ? picks.has(c.n) : !q.permission && q.cursor === c.n;
    return h_flex()
      .id(`${id}-${named}-c${c.n}`)
      .min_h(wide ? 28 : 26)
      .px(6)
      .gap(8)
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .hover((s) => s.bg(alpha(tone, 0.14)))
      .when(on && !multi, (e) => e.bg(alpha(tone, 0.1)))
      .on_click((e, cx) => {
        cx.stop_propagation();
        // A double click answers once: its second click is not another pick.
        if ((e?.click_count ?? 1) > 1) return;
        model.answer(cx, row, c.n);
      })
      .tooltip(c.free ? `Type an answer  ${c.n}` : multi ? `Tick, then Submit  ${c.n}` : `Answer “${c.label}”  ${c.n}`)
      .child(multi && !c.free ? Checkbox(`${id}-${named}-box${c.n}`, { checked: on, onClick: (_e, cx) => model.answer(cx, row, c.n) }) : ChoiceKey(c.n))
      .child(
        h_flex()
          .flex_1()
          .min_w(0)
          .gap(6)
          .items_center()
          .child(div().flex_none().max_w(wide ? 360 : 170).truncate().text_size(TEXT.sm).text_color(c.free ? k.mutedFg : k.foreground).child(c.free ? "Type something…" : c.label))
          .when(Boolean(c.detail) && wide, (e) => e.child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(c.detail))),
      )
      .when(Boolean(c.detail) && !wide, (e) => e.tooltip(`${c.label}: ${c.detail}`));
  };
  const actions = [
    multi &&
      Button(`${id}-submit`, { label: "Submit", size: "sm", onClick: (_e, cx) => model.submit(cx, row), tooltip: "Send the ticked choices" }),
    q.kind === "text" && !replying && Button(`${id}-reply`, { label: "Reply", icon: "reply", size: "sm", onClick: (_e, cx) => model.startReply(cx, row) }),
    Button(`${id}-go`, { icon: "corner-down-left", variant: "outline", size: "icon-sm", tooltip: "Go to the agent", onClick: (_e, cx) => model.go(cx, row) }),
    Button(`${id}-snooze`, { icon: "alarm-clock", variant: "ghost", size: "icon-sm", tooltip: "Snooze…", onClick: () => model.menu(row) }),
  ].filter(Boolean);
  const aside = !dismiss ? null : dismissable(row) ? DismissButton(id, row, model) : row.dismissed && !row.remote ? RestoreButton(id, row, model) : null;
  const held = Boolean(focus?.is_focused?.());
  return v_flex()
    .id(id)
    .w_full()
    .flex_none()
    .p(wide ? 12 : 10)
    .gap(wide ? 8 : 6)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(held ? tone : alpha(tone, 0.45))
    .bg(alpha(tone, 0.07))
    .when(Boolean(focus), (e) => e.track_focus(focus))
    .on_mouse_down("right", () => model.menu(row))
    .when(Boolean(header), (e) => e.child(header))
    .child(
      h_flex()
        .gap(6)
        .items_start()
        .child(div().pt(1).child(Icon(q.permission ? "shield-check" : "circle-help", { size: 14, color: tone })))
        .child(div().flex_1().min_w(0).text_size(wide ? TEXT.base : TEXT.sm).font_medium().text_color(k.foreground).child(q.text || "It is waiting for you.")),
    )
    // A permission always shows what it would allow; a plan kept as a view is one line, read whole in the side panel.
    .when(Boolean(q.permission) && Boolean(q.view), (e) => e.child(PlanRow(`${id}-${named}`, q, () => openPlan(row, q.view))))
    .when(Boolean(q.permission) && !q.view, (e) => e.child(AllowsBlock(`${id}-${named}`, q.preview, { open: opened, onToggle: toggle, cut: q.cut === true, note: q.terminal_only, markdown: q.markdown === true })))
    .when(Boolean(q.preview) && !q.permission && wide, (e) => e.child(div().p(8).rounded(RADIUS.md).border_1().border_color(k.border).bg(k.background).child(Mono(q.preview, { color: k.foreground }))))
    // One cockpit cannot show in full offers no choice here: it is answered in the agent's terminal (Go to the agent).
    .when(q.choices.length > 0 && !q.terminal_only, (e) => e.child(v_flex().gap(1).children(q.choices.map(choice))))
    .when(replying, (e) =>
      e.child(
        h_flex()
          .gap(6)
          .items_center()
          .child(TextField(model.reply))
          .child(Button(`${id}-send`, { icon: "send", size: "icon-sm", tooltip: "Send  ⏎", onClick: (_e, cx) => model.sendReply(cx) }))
          .child(Button(`${id}-cancel`, { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Cancel", onClick: (_e, cx) => model.cancelReply(cx) })),
      ),
    )
    .when(Boolean(busy), (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(Spinner({ size: 12, color: tone })).child(`${busy}…`)))
    .when(!busy, (e) => e.child(h_flex().gap(6).items_center().children(actions).child(div().flex_1()).when(Boolean(footer), (f) => f.child(footer)).when(Boolean(aside), (f) => f.child(aside))));
}
