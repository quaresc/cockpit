// The agents as the shell's views see them: Herdr's live agents (status, place, times) joined with the hub's `agents`
// view (question, recap, branch, PR, CI, changes, cost), each placed in a stage: whose move it is.

import { agents, spaces, panes, run, setShared, notify } from "herdr";
import * as hub from "./hub.js";

/** The stages, in the order the Home board shows them. */
export const STAGES = ["needs", "working", "ready", "done"];

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

/** The question an agent asks, as {text, kind, choices: [{n, label, description, checked}], cursor (an index)}, or null. */
export function questionOf(row) {
  const q = row?.question;
  if (!q) return null;
  if (typeof q === "string") return { text: q, kind: "text", choices: [], cursor: -1 };
  const choices = (q.choices ?? q.options ?? []).map((c, i) =>
    typeof c === "string" ? { n: i + 1, label: c } : { n: c.n ?? i + 1, label: c.label ?? c.text ?? "", description: c.detail || c.description || null, checked: c.checked ?? null, free: Boolean(c.free) },
  );
  const at = choices.findIndex((c) => c.n === q.cursor);
  const text = String(q.text ?? q.question ?? "").replace(/\n?>\s*$/, "").trim();
  return { text, kind: q.kind ?? (choices.length ? "single" : "text"), choices, cursor: at, permission: Boolean(q.permission) };
}

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
      handle: `app:${a.address}`,
      status: a.status,
      title: r.title ?? r.task ?? (a.tab_name && !/^\d+$/.test(a.tab_name) ? a.tab_name : a.title) ?? a.address,
      space: a.space,
      space_name: a.space_name,
      tab: a.tab,
      tab_name: a.tab_name,
      program: a.program,
      focused: a.focused,
      since: r.since ?? r.waiting_since ?? r.waitingSince ?? place?.tab?.changed_at ?? null,
      working: place?.tab?.working_since ?? (a.status === "working" ? r.since ?? null : null),
      question: questionOf(r),
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
      hubStage: r.stage ?? null,
      seen: r.seen ?? null,
      landed,
      row: r,
    };
    agent.stage = stageOf(agent);
    return agent;
  });
}

/** The agents view's stages folded into Home's four: needs you, working, ready to land (approved and green), done. */
const STAGE = { needs_you: "needs", fixing: "needs", working: "working", done: "done", idle: "done", merged: "done", snoozed: "done", review: "done" };

/** Whose move: needs you (blocked, or its PR to fix), working, ready to land (its PR approved and green), done. */
export function stageOf(a) {
  const pr = a.pr;
  const green = pr && !a.landed && pr.state !== "merged" && (pr.review === "APPROVED" || pr.approved) && (pr.ci ?? a.ci) === "SUCCESS";
  if (a.hubStage) return a.hubStage === "review" && green ? "ready" : STAGE[a.hubStage] ?? "done";
  if (a.status === "blocked") return "needs";
  if (a.status === "working") return "working";
  return green ? "ready" : "done";
}

/** The agent's terminal in front: the work area, its tab, its pane focused. */
export function goToAgent(agent) {
  if (!agent) return;
  run("agent.focus", { terminal: agent.address });
  run("layout.select", { stack: "center", child: "work" });
  setShared("cockpit.app", "work");
}

/** Explains why a remote row cannot run tab commands: its tab ids belong to the runner, not this desktop. */
export function refuseRemoteAgent(agent) {
  if (!agent?.remote) return false;
  notify({ title: "Read-only remote desk", body: "Archiving a runner's agent needs a local cockpit installation.", level: "info" });
  return true;
}

/** The hub's agents actions, by the contract the agents view keeps: answer, recap, snooze, seen, fork. */
export const agentAct = (action, args) => hub.act("agents", action, [args]);

/** The agents by stage, for the bar, the bell and Home to agree: only agents, never PRs or tickets on their own. */
export function counts(list, _home) {
  const by = (stage) => list.filter((a) => a.stage === stage).length;
  return { needs: by("needs"), working: by("working"), ready: by("ready"), done: by("done") };
}
