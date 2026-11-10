// The agents, as the sidebar and the recap popup draw them: the hub's `agents` view (see docs/app.md) followed live,
// its actions with their feedback, and the pieces both draw: stage marks, the question card answered in place, the
// facts as chips, the quota bars with their pace tick, and the right-click menu of an agent.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { run, notify, openOverlay, agents, spaces, panes, exec, readTextFile, writeTextFile } from "herdr";
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

/** Rate limits (the hub's quota, from the usage sync's limits.json) as thin bars, a tick where the window's time stands: use past it runs ahead of pace. */
export function QuotaBars(quota, { id = "quota", compact = false, labelW = 18, resets = false } = {}) {
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
        // Nothing used and no reset yet: the window starts with the next request.
        const idle = !q.resets_at && !q.percent;
        const tip = q.percent == null
          ? `${q.name}: no data yet`
          : `${q.name}: ${p}% used${left ? `, resets in ${left}` : idle ? ", not started" : ""}${q.elapsed != null ? ` · ${Math.round(q.elapsed * 100)}% of the window gone${q.ahead ? ", ahead of pace" : ""}` : ""}`;
        return h_flex()
          .id(`${id}-${q.name}`)
          .gap(8)
          .items_center()
          .tooltip(tip)
          .child(div().w(labelW).flex_none().truncate().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(q.label ?? q.name))
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
              .child(q.percent == null ? "–" : `${p}%`),
          )
          .when(resets, (e) => e.child(div().w(28).flex_none().text_size(TEXT.xs).text_color(k.dim).child(left ?? (idle ? "idle" : ""))));
      }),
    );
}

// ---------------------------------------------------------------------------------------------------------------------
// Recap with any agent and model. "Auto" leaves it to the hub's own recap; a choice runs herdr/helpers/cockpit-recap.py, which reads
// the agent's transcript (Claude Code, Codex, Antigravity, OpenCode) and asks the chosen agent headless. The choice lives
// in ~/.config/cockpit/config.json (agents.recap, recap.model) and, for a quick read, in localStorage.

/** The recap agents and models the picker offers; Codex's models come from its own cache when it has one. */
export async function recapChoices() {
  const out = [{ label: "Auto (the hub's recap)", agent: null, model: null }];
  out.push(
    { group: "Antigravity", label: "Gemini 3.8 Flash (Low)", agent: "antigravity-cli", model: "gemini-3.8-flash-low" },
    { group: "Antigravity", label: "Gemini 3.8 Flash (Medium)", agent: "antigravity-cli", model: "gemini-3.8-flash-medium" },
    { group: "Antigravity", label: "Gemini 4 Argon (Low)", agent: "antigravity-cli", model: "gemini-4-argon-low" },
    { group: "Antigravity", label: "Claude Sonnet 5.5 (Low)", agent: "antigravity-cli", model: "claude-sonnet-5-5-low" },
  );
  let codex = [];
  try {
    const { home } = await hub.install();
    // Models behind an OpenAI-compatible gateway (litellm.base_url + litellm.models in the cockpit config), e.g. local ones.
    const cfg = JSON.parse((await readTextFile(`${home}/.config/cockpit/config.json`)) ?? "{}");
    if (cfg.litellm?.base_url) for (const m of cfg.litellm.models ?? []) out.push({ group: "Gateway", label: m, agent: "litellm", model: m });
    const d = JSON.parse((await readTextFile(`${home}/.codex/models_cache.json`)) ?? "{}");
    codex = (d.models ?? []).filter((m) => m.slug && !/review/.test(m.slug)).slice(0, 4).map((m) => ({ group: "Codex", label: m.display_name ?? m.slug, agent: "codex", model: m.slug }));
  } catch {}
  out.push({ group: "Codex", label: "Codex (its default model)", agent: "codex", model: null }, ...codex);
  out.push(
    { group: "Claude Code", label: "Claude Haiku", agent: "claude", model: "haiku" },
    { group: "Claude Code", label: "Claude Sonnet", agent: "claude", model: "sonnet" },
    { group: "OpenCode", label: "OpenCode (its default model)", agent: "opencode", model: null },
  );
  return out;
}

/** The recap agent and model chosen, or null for Auto. */
export function recapChoice() {
  try {
    const c = JSON.parse(localStorage.getItem("cockpit.recap.choice") ?? "null");
    return c?.agent ? c : null;
  } catch {
    return null;
  }
}

export function recapLabel(c = recapChoice()) {
  return c ? (c.group && !c.label.includes(c.group) ? `${c.label} · ${c.group}` : c.label) : "Auto";
}

/** Keeps a choice: localStorage now, and agents.recap / recap.model in the cockpit config (other keys left as they are). */
export async function setRecapChoice(c) {
  try {
    if (c?.agent) localStorage.setItem("cockpit.recap.choice", JSON.stringify(c));
    else localStorage.removeItem("cockpit.recap.choice");
  } catch {}
  try {
    const { home } = await hub.install();
    const path = `${home}/.config/cockpit/config.json`;
    const before = await readTextFile(path);
    const cfg = before ? JSON.parse(before) : {};
    cfg.agents = cfg.agents ?? {};
    if (c?.agent) {
      cfg.agents.recap = c.agent;
      cfg.recap = { ...(cfg.recap ?? {}), model: c.model ?? null, label: c.label };
    } else {
      delete cfg.agents.recap;
      if (cfg.recap) delete cfg.recap.model, delete cfg.recap.label;
    }
    await writeTextFile(path, JSON.stringify(cfg, null, 2) + "\n", before);
  } catch {}
}

/** The choice the config holds: it wins over localStorage, so an edit there (or another view's pick) takes over. */
export async function loadRecapChoice() {
  try {
    const { home } = await hub.install();
    const cfg = JSON.parse((await readTextFile(`${home}/.config/cockpit/config.json`)) ?? "{}");
    if (cfg.agents?.recap) {
      const c = { agent: cfg.agents.recap, model: cfg.recap?.model ?? null, label: cfg.recap?.label ?? cfg.agents.recap };
      localStorage.setItem("cockpit.recap.choice", JSON.stringify(c));
      return c;
    }
  } catch {}
  return recapChoice();
}

/** The picker: every choice, the current one ticked; picking sends `<prefix><index>` to the view's command(). */
export async function openRecapPicker(prefix) {
  const all = await recapChoices();
  const cur = recapChoice();
  openMenu(
    all.map((c, i) => ({
      label: c.group && !c.label.includes(c.group) ? `${c.group} · ${c.label}` : c.label,
      icon: (cur ? cur.agent === c.agent && (cur.model ?? null) === (c.model ?? null) : !c.agent) ? "check" : undefined,
      command: `${prefix}${i}`,
    })),
    { width: 300 },
  );
}

/** A picker's `<prefix><index>` command: keeps that choice; true when it was one. */
export async function pickRecap(c, prefix) {
  if (!c.startsWith(prefix)) return false;
  const all = await recapChoices();
  await setRecapChoice(all[Number(c.slice(prefix.length))] ?? null);
  return true;
}

const recapFile = (cache, terminal) => `${cache}/recaps/${String(terminal).replace(/^app:/, "").replace(/[^A-Za-z0-9_.-]/g, "_")}.json`;

/** Recaps one agent: the hub's own on Auto, else the chosen agent's through cockpit-recap.py. {text} or {error}. */
export async function runRecap(terminal, choice = recapChoice()) {
  if (!choice?.agent) return hub.act("agents", "recap", [{ terminal, force: true }]);
  const { home } = await hub.install();
  const args = [`${home}/.local/share/cockpit/herdr/helpers/cockpit-recap.py`, String(terminal).replace(/^app:/, ""), "--agent", choice.agent];
  if (choice.model) args.push("--model", choice.model);
  const r = await exec("python3", args, { timeout: 300000 }).catch((e) => ({ stdout: "", stderr: String(e?.message ?? e) }));
  try {
    return JSON.parse(String(r.stdout ?? "").trim().split("\n").pop());
  } catch {
    return { error: String(r.stderr ?? "the recap failed").trim().slice(0, 200) };
  }
}

/** The recap cockpit-recap.py last wrote for a terminal, or null. */
export async function localRecap(terminal) {
  try {
    const { cache } = await hub.install();
    const t = await readTextFile(recapFile(cache, terminal));
    return t ? JSON.parse(t) : null;
  } catch {
    return null;
  }
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
    // Recaps cockpit-recap.py wrote (another agent or model than the hub's): terminal -> {text, at, agent, model}.
    this.local = new Map();
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
        this.loadLocal(cx);
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
    if (!this.down || this.value?.remote) return rowsOf(this.value).map((r) => this.withLocal(r.session ? r : { ...r, session: sessionFor(r) }));
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

  /** A row with the newer of its two recaps: the hub's, or the one written by the chosen recap agent. */
  withLocal(r) {
    const l = this.local?.get(r.terminal);
    if (!l?.text) return r;
    const hubAt = r.recap?.at ?? 0;
    if (r.recap?.text && hubAt > l.at) return r;
    return { ...r, recap: { ...(r.recap ?? {}), text: l.text, at: l.at, fresh: !r.since || l.at >= r.since, running: false, error: null, by: [l.agent, l.model].filter(Boolean).join(" · ") } };
  }

  /** Reads the local recaps of rows not read yet. */
  loadLocal(cx) {
    for (const r of this.value?.agents ?? []) {
      if (this.local.has(r.terminal)) continue;
      this.local.set(r.terminal, null);
      cx.spawn(async (cx) => {
        const l = await localRecap(r.terminal);
        if (l?.text) {
          this.local.set(r.terminal, l);
          cx.notify();
        }
      });
    }
  }

  /** Recaps a row with the chosen agent and model (the hub's own on Auto). */
  recap(cx, terminal) {
    if (!recapChoice()) return this.act(cx, "recap", { terminal, force: true }, "recapping");
    this.busy.set(terminal, "recapping");
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await runRecap(terminal);
      if (r?.text) this.local.set(terminal, r);
      else notify({ title: "Could not recap it", body: r?.error ?? "no recap came", level: "warning", timeout: 5000 });
      this.busy.delete(terminal);
      cx.notify();
    });
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
        row.unread && { label: "Mark as seen", icon: "check", command: c("seen") },
        "separator",
        { label: "Fork to a new tab", icon: "git-fork", command: c("fork", "tab") },
        { label: "Fork to the right", icon: "square-split", command: c("fork", "right") },
        { label: pinned ? "Unpin" : "Pin to the top", icon: pinned ? "pin-off" : "pin", command: c("pin") },
        this.titles && { label: "Retitle its tab", icon: "pencil", command: c("retitle") },
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
    else if (what === "recap") this.recap(cx, terminal);
    else if (what === "snooze") this.act(cx, "snooze", { terminal, until: arg });
    else if (what === "wake") this.act(cx, "snooze", { terminal, until: null });
    else if (what === "seen") this.act(cx, "seen", { terminal });
    else if (what === "fork") this.act(cx, "fork", { terminal, place: arg || "tab" });
    else if (what === "pin") this.togglePin(terminal);
    else if (what === "retitle") this.titles?.retitle(cx, row, { force: true });
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

/** The hub's quota cut to one bar per agent, its fullest window: "Claude Code 5h" → "Claude 5h". */
export function worstQuota(quota) {
  const short = { "Claude Code": "Claude", Codex: "Codex", Antigravity: "Agy" };
  const best = new Map();
  for (const q of quota ?? []) {
    const agent = Object.keys(short).find((a) => q.name.startsWith(a)) ?? q.name;
    if (!best.has(agent) || (q.percent ?? 0) > (best.get(agent).percent ?? 0)) best.set(agent, { ...q, label: `${short[agent] ?? agent} ${q.name.split(" ").pop()}` });
  }
  return [...best.values()];
}

/** Each agent's 5-hour and weekly window from the usage sync (agent-usage.json), two bars per agent for the sidebar:
 *  Antigravity's fullest model group for each, an idle 5-hour window (nothing used, no reset yet) still drawn. */
export function quotaByAgent(usage) {
  const short = { claude: "Claude", codex: "Codex", antigravity: "Agy" };
  const listed = (usage?.agents ?? []).filter((a) => short[a.id] && a.windows?.length);
  return listed.flatMap((a) =>
    ["5h", "7d"].map((name) => {
      const ws = a.windows.filter((w) => w.name === name && (w.source !== "estimate" || w.budget_tokens));
      const w = ws.reduce((m, x) => (!m || (x.percent ?? 0) > (m.percent ?? 0) ? x : m), null);
      return { ...(w ?? {}), name: `${a.name} ${name}${w?.group ? ` (${w.group})` : ""}`, label: `${short[a.id]} ${name}`, percent: w?.percent ?? null };
    }),
  );
}

// ---------------------------------------------------------------------------------------------------------------------
// Sub-agents herdr cannot see: OpenCode child sessions (oh-my-opencode-slim's explorer, fixer, oracle…) and Claude Code's
// Task agents, inside the agent's own process. bin/cockpit-subagents.py reads them from each agent's session.

const SUB_DONE_MS = 10 * 60_000;

export class SubagentsModel {
  constructor() {
    this.by = {};
    this.key = "";
    this.at = 0;
    this.pending = false;
  }

  /** Asks for the sessions `sessionsOf()` names: every 4 s while one of them works, else every 20 s. */
  follow(cx, sessionsOf) {
    const pull = (cx) => {
      const sessions = [...new Set((sessionsOf() ?? []).filter(Boolean))].sort();
      const key = sessions.join(" ");
      const busy = Object.values(this.by).some((l) => l.some((s) => s.status === "working"));
      if (this.pending || (key === this.key && !busy && Date.now() - this.at < 20000)) return;
      if (!sessions.length) {
        this.by = {};
        this.key = key;
        return;
      }
      this.pending = true;
      cx.spawn(async (cx) => {
        try {
          const { home } = await hub.install();
          const r = await exec("python3", [`${home}/.local/share/cockpit/herdr/helpers/cockpit-subagents.py`, ...sessions], { timeout: 15000 });
          this.by = JSON.parse(r.stdout || "{}").sessions ?? {};
          this.key = key;
          this.at = Date.now();
        } catch {}
        this.pending = false;
        cx.notify();
      });
    };
    pull(cx);
    cx.timer.every(4000, (cx) => pull(cx));
  }

  /** A session's sub-agents worth showing: the working ones, and those that ended in the last ten minutes. */
  of(session, now = Date.now()) {
    return (this.by[session] ?? []).filter((s) => s.status === "working" || now - (s.until ?? 0) < SUB_DONE_MS);
  }

  working(session) {
    return (this.by[session] ?? []).filter((s) => s.status === "working").length;
  }
}

/** The sessions worth asking about: this machine's agents (a remote one's sub-agents live on its machine). */
export function localSessions(rows) {
  return (rows ?? []).filter((r) => !String(r.address ?? r.terminal ?? "").includes("/")).map(sessionFor).filter(Boolean);
}

/** Sessions cockpit-perf.py found for agents the hub gave none (OpenCode's terminal title is its session's title): address -> id. */
const RESOLVED = new Map();

/** An agent's session: the hub's, else the one found from its terminal. */
export const sessionFor = (r) => r?.session ?? RESOLVED.get(String(r?.address ?? r?.terminal ?? "").replace(/^app:/, "")) ?? null;

// ---------------------------------------------------------------------------------------------------------------------
// What the agents cost the machines (herdr/helpers/cockpit-perf.py): this computer's CPU, memory and GPU, the GPU of the machines in perf.remotes, and per
// agent its processes' CPU and memory and its latest answer's tokens per second. Every 5 s; the last two minutes kept for
// the sparklines.

const PERF_KEEP = 24;

export class PerfModel {
  constructor() {
    this.value = null;
    this.history = { cpu: [], gpu: [] };
    this.remoteGpu = new Map();
    this.agentCpu = new Map();
    this.pending = false;
  }

  /** `rowsOf()` gives the hub's agent rows; only this machine's are measured (a remote one's processes are elsewhere). */
  follow(cx, rowsOf) {
    const pull = (cx) => {
      if (this.pending) return;
      const live = new Map(agents().map((a) => [a.address, a]));
      const cwd = new Map(panes().filter((p) => p.terminal).map((p) => [p.terminal, p.cwd]));
      const list = (rowsOf() ?? [])
        .filter((r) => !String(r.address ?? "").includes("/") && live.has(r.address))
        .map((r) => ({ address: r.address, pane: live.get(r.address).pane, session: r.session ?? null, program: r.program ?? live.get(r.address).app ?? live.get(r.address).program?.key, cwd: cwd.get(r.address) ?? null, name: live.get(r.address).tab_name }));
      this.pending = true;
      cx.spawn(async (cx) => {
        try {
          const { home } = await hub.install();
          const r = await exec("python3", [`${home}/.local/share/cockpit/herdr/helpers/cockpit-perf.py`, JSON.stringify(list)], { timeout: 15000 });
          const v = JSON.parse(r.stdout || "null");
          if (v) {
            this.value = v;
            const keep = (arr, x) => (x == null ? arr : [...arr, x].slice(-PERF_KEEP));
            this.history.cpu = keep(this.history.cpu, v.system?.cpu);
            this.history.gpu = keep(this.history.gpu, v.system?.gpu);
            for (const [name, r] of Object.entries(v.remote ?? {})) this.remoteGpu.set(name, keep(this.remoteGpu.get(name) ?? [], r.gpu));
            for (const [address, a] of Object.entries(v.agents ?? {})) if (a.session) RESOLVED.set(address, a.session);
            for (const [address, a] of Object.entries(v.agents ?? {})) if (a.cpu != null) this.agentCpu.set(address, keep(this.agentCpu.get(address) ?? [], a.cpu));
          }
        } catch {}
        this.pending = false;
        cx.notify();
      });
    };
    pull(cx);
    cx.timer.every(5000, (cx) => pull(cx));
  }

  /** An agent's numbers: {cpu, mem, tps, tokens, at, history}, or null. */
  of(address) {
    const a = this.value?.agents?.[String(address ?? "").replace(/^app:/, "")];
    return a ? { ...a, history: this.agentCpu.get(String(address).replace(/^app:/, "")) ?? [] } : null;
  }

  /** Tokens per second summed over the agents whose latest answer ended in the last minute. */
  liveTps(now = Date.now()) {
    return Object.values(this.value?.agents ?? {}).filter((a) => a.tps && now - (a.at ?? 0) < 60_000).reduce((n, a) => n + a.tps, 0);
  }
}

/** "42 tok/s", tenths under ten. */
export const tpsText = (t) => `${t >= 10 ? Math.round(t) : t.toFixed(1)} tok/s`;

// ---------------------------------------------------------------------------------------------------------------------
// AI tab titles: an agent's tab nobody named gets a few words for its task (herdr/helpers/cockpit-recap.py --title: the config's
// titles.agent/titles.model, else the recap agent's), again when its session changes or after an hour of work. A name a
// person gave (the tab's name is not the one set here) is never touched; "Retitle its tab" in an agent's menu forces it.
// Only this window's tabs with that one agent in them; `titles.enabled: false` in the cockpit config turns it off.

const TITLE_PREFIX = [[/opencode/, "OC"], [/claude/, "CC"], [/codex/, "CX"], [/antigravity|agy|gemini/, "AG"]];
const TITLE_EVERY_MS = 60 * 60_000;
const TITLE_RETRY_MS = 10 * 60_000;
const TITLE_EMPTY_RETRY_MS = 30_000;

export class TitlesModel {
  constructor() {
    // terminal → {name, session, at}: the names set here, so the next one may replace them.
    try {
      this.applied = JSON.parse(localStorage.getItem("cockpit.titles.applied") ?? "{}");
    } catch {
      this.applied = {};
    }
    this.next = new Map();
    this.pending = false;
    this.enabled = true;
  }

  follow(cx, rowsOf) {
    cx.spawn(async () => {
      try {
        const { home } = await hub.install();
        const text = await readTextFile(`${home}/.config/cockpit/config.json`);
        this.enabled = (text ? JSON.parse(text).titles?.enabled : undefined) !== false;
      } catch {}
    });
    cx.timer.every(5000, (cx) => this.tick(cx, rowsOf() ?? []));
  }

  /** The window's tab an agent row runs in, when it is the only agent there (a TUI agent's tab is not ours to rename). */
  placeOf(r) {
    const address = String(r.address ?? r.terminal ?? "").replace(/^app:/, "");
    const all = agents();
    const a = all.find((x) => x.address === address);
    if (!a || all.filter((x) => x.tab === a.tab).length > 1) return null;
    for (const s of spaces()) {
      const tab = s.tabs.find((t) => t.id === a.tab);
      if (tab) return { tab, agent: a };
    }
    return null;
  }

  ours(r, tab) {
    return !tab.named || tab.name === this.applied[r.terminal]?.name;
  }

  due(r, now) {
    if (!r.session || String(r.address ?? r.terminal ?? "").includes("/") || now < (this.next.get(r.terminal) ?? 0)) return null;
    const place = this.placeOf(r);
    if (!place || !this.ours(r, place.tab)) return null;
    const mine = this.applied[r.terminal];
    const stale = r.stage === "working" && now - (mine?.at ?? 0) > TITLE_EVERY_MS;
    if (place.tab.named && mine?.session === r.session && !stale) return null;
    return place;
  }

  tick(cx, rows) {
    if (!this.enabled || this.pending) return;
    const now = Date.now();
    const r = rows.find((r) => this.due(r, now));
    if (r) this.retitle(cx, r);
  }

  retitle(cx, r, { force = false } = {}) {
    const place = this.placeOf(r);
    if (!place || this.pending) return;
    this.pending = true;
    this.next.set(r.terminal, Date.now() + TITLE_RETRY_MS);
    cx.spawn(async (cx) => {
      try {
        const { home } = await hub.install();
        const out = await exec("python3", [`${home}/.local/share/cockpit/herdr/helpers/cockpit-recap.py`, r.terminal, "--title"], { timeout: 300000 });
        const t = JSON.parse(out.stdout || "{}");
        // No prompt yet: ask again soon, the first one names it.
        if (!t.name && /nothing to recap|no session/.test(t.error ?? "")) this.next.set(r.terminal, Date.now() + TITLE_EMPTY_RETRY_MS);
        const now = this.placeOf(r);
        if (t.name && now && (force || this.ours(r, now.tab))) {
          const program = String(r.program ?? r.agent ?? now.agent.app ?? now.agent.program?.name ?? "").toLowerCase();
          const prefix = TITLE_PREFIX.find(([re]) => re.test(program))?.[1];
          const name = prefix ? `${prefix} | ${t.name}` : t.name;
          run("tab.rename", { id: now.tab.id, name });
          this.applied[r.terminal] = { name, session: r.session, at: Date.now() };
          try {
            localStorage.setItem("cockpit.titles.applied", JSON.stringify(this.applied));
          } catch {}
        } else if (force && t.error) notify({ title: "Could not title it", body: t.error, level: "warning", timeout: 5000 });
      } catch {}
      this.pending = false;
      cx.notify();
    });
  }
}
