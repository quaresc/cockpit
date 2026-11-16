// The agents, as the sidebar and the recap popup draw them: the hub's `agents` view (see docs/app.md) followed live,
// its actions with their feedback, and the pieces both draw: stage marks, the question card answered in place, the
// facts as chips, the quota bars with their pace tick, and the right-click menu of an agent.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { run, notify, openOverlay, agents } from "herdr";
import * as hub from "./hub.js";
import { rowsOf } from "./fleet.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Spinner, Kbd, Button, Checkbox, Chip, Mono, TextField, dur, openMenu } from "./ui.js";

/** Each stage: its label, color token and mark. Whose move it is, yours first. */
export const STAGES = {
  needs_you: { label: "Needs you", tone: "blocked" },
  done: { label: "Done", tone: "done" },
  fixing: { label: "Needs fixing", tone: "destructive" },
  idle: { label: "Idle", tone: "idle" },
  working: { label: "Working", tone: "working" },
  review: { label: "In review", tone: "success" },
  merged: { label: "Merged", tone: "merged" },
  snoozed: { label: "Snoozed", tone: "info" },
};

export function stageColor(stage) {
  const k = tokens();
  const tone = STAGES[stage]?.tone ?? "dim";
  if (tone === "merged") return k.t.ansi5 ?? k.mutedFg;
  return k[tone] ?? k.dim;
}

/** The word for how long a row has been in its stage: "waiting 4m", "done 9m", "working 2m". */
export function stageAge(row, now = Date.now()) {
  const age = dur(now - (row.since ?? now));
  const word = { needs_you: "waiting", done: "done", working: "working", snoozed: row.snoozed?.label ?? "snoozed" }[row.stage];
  return row.stage === "snoozed" ? word : word ? `${word} ${age}` : age;
}

/** The stage's mark: a breathing dot while it needs you, a spinner while it works, a ring at rest. */
export function StageMark(id, stage, { size = 8 } = {}) {
  const c = stageColor(stage);
  if (stage === "working") return Spinner({ size: size + 4, color: c });
  if (stage === "snoozed") return Icon("alarm-clock", { size: size + 4, color: c });
  if (stage === "merged") return Icon("git-merge", { size: size + 4, color: c });
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

/** Claude's 5h and 7d limits as thin bars, a tick where the window's time stands: use past it runs ahead of pace. */
export function QuotaBars(quota, { id = "quota", compact = false } = {}) {
  const k = tokens();
  if (!quota?.length) return null;
  return v_flex()
    .id(id)
    .gap(compact ? 4 : 6)
    .children(
      quota.map((q) => {
        const p = Math.max(0, Math.min(100, q.percent ?? 0));
        const tone = p >= 95 ? k.destructive : p >= 80 || q.ahead ? k.warning : k.success;
        const left = q.resets_at ? dur(q.resets_at - Date.now()) : null;
        const tip = `${q.name}: ${p}% used${left ? `, resets in ${left}` : ""}${q.elapsed != null ? ` · ${Math.round(q.elapsed * 100)}% of the window gone${q.ahead ? ", ahead of pace" : ""}` : ""}`;
        return h_flex()
          .id(`${id}-${q.name}`)
          .gap(8)
          .items_center()
          .tooltip(tip)
          .child(div().w(18).flex_none().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(q.name))
          .child(
            div()
              .relative()
              .flex_1()
              .h(6)
              .rounded(3)
              .bg(k.muted)
              .child(div().absolute().left(0).top(0).h_full().rounded(3).bg(tone).w(`${p}%`).transition("width", { duration: 400, easing: "ease-out" }))
              .when(q.elapsed != null, (e) => e.child(div().absolute().top(-2).h(10).w(2).rounded(1).bg(k.foreground).opacity(0.7).left(`${Math.min(99, q.elapsed * 100)}%`))),
          )
          .child(
            h_flex()
              .w(compact ? 34 : 40)
              .flex_none()
              .justify_end()
              .gap(2)
              .text_size(TEXT.xs)
              .font_family("JetBrains Mono")
              .text_color(q.ahead || p >= 80 ? tone : k.mutedFg)
              .when(Boolean(q.ahead), (e) => e.child("▲"))
              .child(`${p}%`),
          );
      }),
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// The model and its actions

/** Follows the agents view and runs its actions; one per view that draws agents. */
export class AgentsModel {
  constructor() {
    this.value = null;
    this.state = "connecting";
    // What is being done to a row: terminal -> a word ("answering", "recapping"), until the next model.
    this.busy = new Map();
    // Choices ticked in a multiple choice before Submit: terminal -> Set(n).
    this.picks = new Map();
    // The free-text reply being typed: {terminal, choice}.
    this.replying = null;
    this.reply = InputState.new({ placeholder: "Your answer… ⏎ sends" });
    this.pins = [];
  }

  /** Subscribes; `onChange(value, before, cx)` after each new model. */
  follow(cx, onChange = () => {}) {
    this.reply.on("submit", (_e, cx) => this.sendReply(cx));
    this.sub = hub.subscribe(
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
        else if (state.startsWith("error")) this.down = true;
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
    const stage = { blocked: "needs_you", working: "working", done: "done", idle: "idle" };
    return agents().map((a) => ({
      terminal: `app:${a.address}`,
      address: a.address,
      stage: stage[a.status] ?? "idle",
      title: a.tab_name && !/^\d+$/.test(a.tab_name) ? a.tab_name : a.title || a.address,
      space: { id: a.space, name: a.space_name },
      tab: { id: a.tab, name: a.tab_name },
      herdr: true,
    }));
  }

  row(terminal) {
    return this.rows.find((r) => r.terminal === terminal);
  }

  /** An action on one agent, its note as a toast when it says something. */
  act(cx, action, arg, busy) {
    if (busy) this.busy.set(arg.terminal, busy);
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("agents", action, [arg]);
        if (r.error) notify({ title: "Could not do it", body: r.error, level: "warning", timeout: 5000 });
        else if (r.note && action === "answer") notify({ title: r.note, level: "success", timeout: 2500 });
        else if (r.note && action !== "focus" && action !== "seen") notify({ title: r.note, level: "info", timeout: 2500 });
      } catch (e) {
        notify({ title: "The cockpit hub did not answer", body: String(e?.message ?? e), level: "warning" });
      }
      this.busy.delete(arg.terminal);
      cx.notify();
    });
  }

  answer(cx, row, choice) {
    const q = row.question;
    const picked = q?.choices.find((c) => c.n === choice);
    if (picked?.free) return this.startReply(cx, row, choice);
    if (q?.kind === "multi") {
      const set = this.picksOf(row);
      set.has(choice) ? set.delete(choice) : set.add(choice);
      return cx.notify();
    }
    this.act(cx, "answer", { terminal: row.terminal, choice }, "answering");
  }

  /** A multiple choice's ticks: what the screen shows until the person changes them here. */
  picksOf(row) {
    if (!this.picks.has(row.terminal)) this.picks.set(row.terminal, new Set((row.question?.choices ?? []).filter((c) => c.checked).map((c) => c.n)));
    return this.picks.get(row.terminal);
  }

  submit(cx, row) {
    const choices = [...this.picksOf(row)];
    this.picks.delete(row.terminal);
    this.act(cx, "answer", { terminal: row.terminal, choices }, "answering");
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

  sendReply(cx) {
    const r = this.replying;
    const text = this.reply.value().trim();
    this.replying = null;
    if (!r || !text) return cx.notify();
    const row = this.row(r.terminal);
    const multi = row?.question?.kind === "multi";
    this.act(cx, "answer", { terminal: r.terminal, text, ...(r.choice ? { choice: r.choice } : {}), ...(multi ? { choices: [...this.picksOf(row)] } : {}) }, "answering");
  }

  go(cx, row) {
    run("agent.focus", { terminal: row.address });
    run("layout.select", { stack: "center", child: "work" });
    // A runner's hub takes no writes from a remote desk.
    if (!row.herdr && !row.remote) this.act(cx, "seen", { terminal: row.terminal });
  }

  /** The right-click menu of an agent; its items come back as `cockpit.agents|<what>|<terminal>|<arg>`. */
  menu(row) {
    // Every item is a hub action.
    if (row.herdr) return;
    const t = row.terminal;
    const c = (what, arg = "") => `agents|${what}|${t}|${arg}`;
    const pinned = this.pins.includes(t);
    openMenu(
      [
        { label: "Go to agent", icon: "corner-down-left", kbd: "⏎", command: c("go") },
        { label: "Peek", icon: "eye", command: c("peek") },
        { label: row.recap?.text ? "Recap again" : "Recap", icon: "sparkles", kbd: "r", command: c("recap") },
        "separator",
        row.snoozed ? { label: "Wake", icon: "alarm-clock", command: c("wake") } : { label: "Snooze 1 hour", icon: "alarm-clock", command: c("snooze", "1h") },
        !row.snoozed && { label: "Snooze until 18:00", icon: "clock", command: c("snooze", "18:00") },
        !row.snoozed && { label: "Snooze until tomorrow", icon: "calendar", command: c("snooze", "tomorrow") },
        !row.snoozed && { label: "Snooze till it changes", icon: "hourglass", command: c("snooze", "done") },
        row.unread && !row.remote && { label: "Mark as seen", icon: "check", command: c("seen") },
        "separator",
        { label: "Fork to a new tab", icon: "git-fork", command: c("fork", "tab") },
        { label: "Fork to the right", icon: "square-split", command: c("fork", "right") },
        { label: pinned ? "Unpin" : "Pin to the top", icon: pinned ? "pin-off" : "pin", command: c("pin") },
      ],
      { width: 230 },
    );
  }

  /** `cockpit.agents|…` from a menu, run by the sidebar's copy alone (every shown copy hears it); true when it was one. */
  command(c, cx) {
    if (!c.startsWith("agents|")) return false;
    const [, what, terminal, arg] = c.split("|");
    const row = this.row(terminal);
    if (!row) return true;
    if (what === "go") this.go(cx, row);
    else if (what === "peek") run("terminal.peek", { terminal: row.address });
    else if (what === "recap") this.act(cx, "recap", { terminal, force: true }, "recapping");
    else if (what === "snooze") this.act(cx, "snooze", { terminal, until: arg });
    else if (what === "wake") this.act(cx, "snooze", { terminal, until: null });
    else if (what === "seen") this.act(cx, "seen", { terminal });
    else if (what === "fork") this.act(cx, "fork", { terminal, place: arg || "tab" });
    else if (what === "pin") this.togglePin(terminal);
    cx.notify();
    return true;
  }

  togglePin(terminal) {
    this.pins = this.pins.includes(terminal) ? this.pins.filter((p) => p !== terminal) : [...this.pins, terminal];
    try {
      localStorage.setItem("agents.pins", JSON.stringify(this.pins));
    } catch {}
  }

  loadPins() {
    try {
      this.pins = JSON.parse(localStorage.getItem("agents.pins") ?? "[]");
    } catch {
      this.pins = [];
    }
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// The question card

/**
 * A waiting agent's question, answered where it shows: a click on a choice answers (or ticks, in a multiple
 * choice, then Submit), the free-text choice opens a field, a question without choices takes words. `wide` draws the
 * popup's roomier card; `header` is drawn above.
 */
export function QuestionCard(id, row, model, { wide = false, header, footer } = {}) {
  const k = tokens();
  const q = row.question ?? { text: row.said || "It is waiting for you.", kind: "text", choices: [] };
  const tone = k.blocked;
  const busy = model.busy.get(row.terminal);
  const replying = model.replying?.terminal === row.terminal;
  const multi = q.kind === "multi";
  const picks = multi ? model.picksOf(row) : null;
  const choice = (c, i) => {
    const on = multi ? picks.has(c.n) : q.cursor === c.n;
    return h_flex()
      .id(`${id}-c${c.n}`)
      .min_h(wide ? 28 : 26)
      .px(6)
      .gap(8)
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .hover((s) => s.bg(alpha(tone, 0.14)))
      .when(on && !multi, (e) => e.bg(alpha(tone, 0.1)))
      .on_click((_e, cx) => {
        cx.stop_propagation();
        model.answer(cx, row, c.n);
      })
      .tooltip(c.free ? "Type an answer" : multi ? "Tick, then Submit" : `Answer “${c.label}”`)
      .child(multi && !c.free ? Checkbox(`${id}-box${c.n}`, { checked: on, onClick: (_e, cx) => model.answer(cx, row, c.n) }) : Kbd(String(c.n)))
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
  return v_flex()
    .id(id)
    .w_full()
    .flex_none()
    .p(wide ? 12 : 10)
    .gap(wide ? 8 : 6)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(alpha(tone, 0.45))
    .bg(alpha(tone, 0.07))
    .on_mouse_down("right", () => model.menu(row))
    .when(Boolean(header), (e) => e.child(header))
    .child(
      h_flex()
        .gap(6)
        .items_start()
        .child(div().pt(1).child(Icon(q.permission ? "shield-check" : "circle-help", { size: 14, color: tone })))
        .child(div().flex_1().min_w(0).text_size(wide ? TEXT.base : TEXT.sm).font_medium().text_color(k.foreground).child(q.text || "It is waiting for you.")),
    )
    .when(Boolean(q.preview) && wide, (e) => e.child(div().p(8).rounded(RADIUS.md).border_1().border_color(k.border).bg(k.background).child(Mono(q.preview, { color: k.foreground }))))
    .when(q.choices.length > 0, (e) => e.child(v_flex().gap(1).children(q.choices.map(choice))))
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
    .when(!busy, (e) => e.child(h_flex().gap(6).items_center().children(actions).child(div().flex_1()).when(Boolean(footer), (f) => f.child(footer))));
}
