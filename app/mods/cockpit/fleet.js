// The agents as the shell's views see them: Herdr's live agents (status, place, times) joined with the hub's `agents`
// view (question, recap, branch, PR, CI, changes, cost), each placed in a stage: whose move it is.

import { agents, spaces, panes, run, notify } from "herdr";
import * as herdr from "herdr";
import * as hub from "./hub.js";
import { showWork, pinChat, readInChat } from "./screens.js";

/** The stages, in the order the Home board shows them. */
export const STAGES = ["needs", "working", "ready", "done"];

/** The statuses cockpit draws. */
const KNOWN = ["working", "blocked", "failed", "done", "idle"];

/**
 * An agent's status as every view reads it, from Herdr's or the hub's word: `error` is failed (stopped with an error),
 * the five cockpit knows stay, and one it does not know is drawn as blocked, so it is looked at rather than missed.
 */
export function agentStatus(raw) {
  if (raw === "error") return "failed";
  if (raw == null || raw === "") return "idle";
  return KNOWN.includes(raw) ? raw : "blocked";
}

/** Each terminal's status as this desktop's Herdr shows it, a runner's under its machine-scoped address (`runner/t7`). */
function deskStatus() {
  const status = new Map();
  for (const s of spaces() ?? []) for (const t of s.tabs ?? []) for (const x of t.terminals ?? []) if (x.terminal) status.set(x.terminal, x.status);
  for (const p of panes() ?? []) if (p.terminal && !status.has(p.terminal)) status.set(p.terminal, p.status);
  return status;
}

/** The agents view's rows, whatever the list is called in its model. A runner's rows take their address on this desktop,
 * and whether they are done and unread from this desktop's Herdr: the runner never learns which agents you opened here,
 * so its own word stays done for good; once Herdr here shows the terminal otherwise, the row rests (idle). */
export function rowsOf(value) {
  const rows = Array.isArray(value) ? value : value?.agents ?? value?.rows ?? [];
  if (!value?.remote) return rows;
  const status = deskStatus();
  return rows.map((r) => {
    const address = `${value.remote}/${addressOf(r)}`;
    const desk = status.get(address);
    const rests = r.stage === "done" && desk !== undefined && desk !== "done";
    return { ...r, remote: value.remote, address, terminal: `app:${address}`, stage: rests ? "idle" : r.stage, unread: Boolean(r.unread) && desk === "done" };
  });
}

/** The agents view's counts, its unread ones as `rowsOf` reads them on a remote desk. */
export function countsOf(value) {
  const counts = value?.counts ?? {};
  return value?.remote ? { ...counts, unread: rowsOf(value).filter((r) => r.unread).length } : counts;
}

const addressOf = (r) => String(r?.address ?? r?.terminal ?? "").replace(/^app:/, "");

/**
 * The question an agent asks, as {text, kind, choices: [{n, label, description, checked}], cursor (an index), ask,
 * asked}, or null. `ask` (a driven session's) and `asked` (a terminal's fingerprint) name it in its answer; a
 * permission has no choice under the cursor.
 */
export function questionOf(row) {
  const q = row?.question;
  if (!q) return null;
  if (typeof q === "string") return { text: q, kind: "text", choices: [], cursor: -1 };
  const choices = (q.choices ?? q.options ?? []).map((c, i) =>
    typeof c === "string" ? { n: i + 1, label: c } : { n: c.n ?? i + 1, label: c.label ?? c.text ?? "", description: c.detail || c.description || null, checked: c.checked ?? null, free: Boolean(c.free) },
  );
  const at = q.permission ? -1 : choices.findIndex((c) => c.n === q.cursor);
  const text = String(q.text ?? q.question ?? "").replace(/\n?>\s*$/, "").trim();
  return { text, kind: q.kind ?? (choices.length ? "single" : "text"), choices, cursor: at, permission: Boolean(q.permission), ask: q.ask ?? null, asked: q.asked ?? null, preview: q.preview ?? "", cut: q.cut === true, unasked: Array.isArray(q.unasked) ? q.unasked : [], terminal_only: q.terminal_only || null, markdown: q.markdown === true, view: q.view || null };
}

/**
 * The `agents answer` act's argument: what was picked (`{choice}`, `{choices}` or `{text}`), the question it answers
 * (its `ask` or `asked`, as shown) and where it came from (`<view>:<click|key|command>`). The hub refuses an answer
 * whose question is not the one asked any more.
 */
export function answerPayload(terminal, question, picked, source) {
  const out = { terminal, ...picked };
  if (question?.ask) out.ask = question.ask;
  if (question?.asked) out.asked = question.asked;
  out.source = source;
  return out;
}

/** What the hub says to an answer whose question moved on; the card draws again from the newer view. */
export const CHANGED = "The question changed — look again";

/** Whether an act's error is that refusal. */
export const changed = (error) => String(error ?? "").includes(CHANGED);

/** CI as GitHub's words: SUCCESS, FAILURE, PENDING. */
function ciWord(ci) {
  const state = typeof ci === "object" && ci ? ci.state ?? ci.outcome : ci;
  return { pass: "SUCCESS", passed: "SUCCESS", succeeded: "SUCCESS", fail: "FAILURE", failed: "FAILURE", running: "PENDING", pending: "PENDING" }[state] ?? state ?? null;
}

const prOf = (row) => {
  const pr = row?.pr;
  if (!pr) return null;
  if (typeof pr === "number") return { number: pr };
  return pr;
};

/**
 * Every agent now: Herdr's own list (live) with what the hub knows about it. `rows` is the agents view's model, `home`
 * Home's (its merged PRs mark agents whose branch landed). Each: {address, handle, status, stage, title, space, tab,
 * since, working, question, recap, branch, repo, ticket, pr, ci, changes, cost, program, focused}.
 */
export function fleet(rows, home) {
  const byAddress = new Map(rowsOf(rows).map((r) => [addressOf(r), r]));
  const tabs = new Map();
  if (!rows?.remote) for (const s of spaces()) for (const t of s.tabs) tabs.set(t.id, { tab: t, space: s });
  const merged = new Set((home?.merged ?? []).map((m) => `${m.repo}/${m.branch}`));
  // Herdr's list, plus the agents only the hub sees (a Claude session attached from the background, the demo's).
  const live = rows?.remote ? [] : agents();
  const seen = new Set(live.map((a) => a.address));
  for (const r of rowsOf(rows)) {
    const address = addressOf(r);
    if (!address || seen.has(address) || (r.frontend && r.frontend !== "app")) continue;
    live.push({ address, status: r.status, title: r.title, tab: r.tab?.id, tab_name: r.tab?.name, space: r.space?.id, space_name: r.space?.name, focused: false });
  }
  return live.map((a) => {
    const r = byAddress.get(a.address) ?? {};
    const place = tabs.get(a.tab);
    const pr = prOf(r);
    const branch = r.branch ?? place?.space?.branch ?? null;
    const repo = r.repo ?? pr?.repo ?? null;
    const landed = Boolean(branch && repo && merged.has(`${repo}/${branch}`)) || pr?.state === "MERGED" || pr?.merged === true;
    const agent = {
      remote: r.remote,
      address: a.address,
      // A session cockpit drives is its own handle; a Herdr terminal's is its address.
      handle: r.driven ? r.terminal : `app:${a.address}`,
      driven: Boolean(r.driven),
      // Herdr's word can stay blocked after the prompt went: the hub read the agent's screen and transcript since.
      status: a.status == null ? a.status : agentStatus(a.status) === "blocked" && r.status && r.status !== "blocked" ? agentStatus(r.status) : agentStatus(a.status),
      interrupted: Boolean(r.interrupted),
      error: r.error ?? null,
      // The hub names an agent (`names::pick`, its tab's label included); until it lists one, Herdr's title stands in.
      title: r.title ?? a.title ?? a.address,
      space: a.space,
      space_name: a.space_name,
      tab: a.tab,
      tab_name: a.tab_name,
      program: a.program,
      // Herdr's logo for its agent and the name people read, from the hub; Herdr's own program says them meanwhile.
      icon: r.icon ?? null,
      agent_name: r.agent_name ?? a.program?.name ?? null,
      focused: a.focused,
      since: r.since ?? r.waiting_since ?? r.waitingSince ?? place?.tab?.changed_at ?? null,
      working: place?.tab?.working_since ?? (a.status === "working" ? r.since ?? null : null),
      question: questionOf(r),
      // Blocked with nothing to answer here: what it waits on, as the hub words it.
      waits: r.question ? null : r.waits ?? null,
      recap: typeof r.recap === "object" && r.recap ? r.recap.text ?? null : r.recap ?? null,
      branch,
      repo,
      ticket: typeof r.ticket === "object" && r.ticket ? r.ticket.key ?? r.ticket.id : r.ticket ?? null,
      ticketUrl: typeof r.ticket === "object" && r.ticket ? r.ticket.url ?? null : null,
      pr,
      ci: ciWord(r.ci) ?? pr?.ci ?? null,
      checks: typeof r.ci === "object" && r.ci?.total ? { done: r.ci.done, total: r.ci.total } : null,
      changes: r.changes ? { added: r.changes.added ?? r.changes.add ?? 0, removed: r.changes.removed ?? r.changes.del ?? 0, files: r.changes.files ?? 0, ahead: r.changes.ahead ?? 0, behind: r.changes.behind ?? 0 } : null,
      cost: r.cost ?? null,
      time: r.time ?? r.time_on_ticket ?? null,
      snoozed: r.snoozed ?? null,
      // Set aside by the person until it asks something new: {at}, or null.
      dismissed: r.dismissed ?? null,
      hubStage: r.stage ?? null,
      seen: r.seen ?? null,
      landed,
      row: r,
    };
    agent.stage = stageOf(agent);
    return agent;
  });
}

/**
 * The agents view's rows that need you, in its order: those asking first, then those stopped with an error, as Home's
 * NEXT MOVE and the bell count them.
 */
export function needing(rows) {
  // One the person dismissed is out until it asks something new, its question still on its row.
  const asks = (r) => !r.dismissed && (r.stage === "needs_you" || Boolean(r.question));
  const failed = (r) => !asks(r) && !r.dismissed && (r.stage === "failed" || agentStatus(r.status) === "failed");
  return [...rows.filter(asks), ...rows.filter(failed)];
}

/** The Dock badge for this many agents that need you: their count, none at zero or when turned off. */
export const badgeText = (n, on = true) => (on && n > 0 ? String(n) : null);

/**
 * Puts the count on the Herdr app's Dock icon when it changes, once for every copy that draws it; a Herdr older than
 * 0.1.16 has no badge.
 */
export function showBadge(n, on = true) {
  const text = badgeText(n, on);
  if (globalThis.cockpitBadge === text) return;
  globalThis.cockpitBadge = text;
  herdr.setBadge?.(text);
}

/**
 * The agents view's stages folded into Home's four: needs you, working, ready to land (approved and green), done. A
 * dismissed agent is in none of them: it is counted nowhere and offered no Archive while it still waits.
 */
const STAGE = { needs_you: "needs", failed: "needs", fixing: "needs", working: "working", done: "done", idle: "done", merged: "done", snoozed: "done", dismissed: "dismissed", review: "done" };

/** Whose move: needs you (blocked, or its PR to fix), working, ready to land (its PR approved and green), done. */
export function stageOf(a) {
  const pr = a.pr;
  const green = pr && !a.landed && pr.state !== "merged" && (pr.review === "APPROVED" || pr.approved) && (pr.ci ?? a.ci) === "SUCCESS";
  if (a.hubStage) return a.hubStage === "review" && green ? "ready" : STAGE[a.hubStage] ?? "done";
  if (a.status === "blocked" || a.status === "failed") return "needs";
  if (a.status === "working") return "working";
  return green ? "ready" : "done";
}

/** The agent's terminal in front: the work area, its tab, its pane focused; a session cockpit drives is read in the chat. */
export function goToAgent(agent, person = true) {
  if (!agent) return;
  if (agent.driven) return readInChat(agent.handle ?? agent.terminal, null, person);
  // Going to an agent on purpose, even the one in front, is the chat following it again.
  pinChat(null);
  run("agent.focus", { terminal: agent.address });
  showWork();
}

/** Explains why a remote row cannot run tab commands: its tab ids belong to the runner, not this desktop. */
export function refuseRemoteAgent(agent) {
  if (!agent?.remote) return false;
  notify({ title: "Read-only remote desk", body: "Archiving a runner's agent needs a local cockpit installation.", level: "info" });
  return true;
}

/** The hub's agents actions, by the contract the agents view keeps: answer, recap, snooze, seen, dismiss, fork. */
export const agentAct = (action, args) => hub.act("agents", action, [args]);

/** The agents by stage, for the bar, the bell and Home to agree: only agents, never PRs or tickets on their own. */
export function counts(list, _home) {
  const by = (stage) => list.filter((a) => a.stage === stage).length;
  return { needs: by("needs"), working: by("working"), ready: by("ready"), done: by("done") };
}
