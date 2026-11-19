// Start work's form as data, shared by the dialog (start.js): what the person picked, the request the hub plans and
// runs from it, what a blocker's fix changes, and how a plan or a run reads. No natives, so tests load it as it is.

/** A new form's state; `source` is a ticket's key, a PR's URL, a prompt or nothing, `id` a run it shows. */
export function emptyState(source = "", id = `g${Date.now()}`) {
  return { id, source: String(source ?? ""), repo: null, agent: null, mode: null, here: false, base: "", branch: "", model: "", effort: "", fast: "", extra: {} };
}

/** The request as the hub's `start_work` fetch and `start work` act take it; fast mode is left out unless picked. */
export function requestOf(s) {
  const fast = { on: true, off: false }[s.fast ?? ""];
  return {
    id: s.id, text: s.source.trim(), worktree: s.here ? "here" : "new", base: s.base.trim(), branch: s.branch.trim(), model: s.model.trim(),
    effort: s.effort ?? "", ...(fast === undefined ? {} : { fast }), repo: s.repo ?? "", agent: s.agent ?? "", mode: s.mode ?? "", ...s.extra,
  };
}

/** The state once a blocker's fix is applied: a branch, a repo or the worktree go to their fields, the rest are flags. */
export function applyFix(s, set = {}) {
  const next = { ...s, extra: { ...s.extra } };
  for (const [k, v] of Object.entries(set)) {
    if (k === "branch") next.branch = String(v ?? "");
    else if (k === "repo") next.repo = v;
    else if (k === "worktree") next.here = v === "here";
    else next.extra[k] = v;
  }
  return next;
}

/** The state with a field changed; a new source or repo drops the fixes picked for the last plan, another agent its picks. */
export function edited(s, field, value) {
  const next = { ...s, [field]: value };
  if (field === "source" || field === "repo") next.extra = {};
  if (field === "agent" && value !== s.agent) Object.assign(next, { model: "", effort: "", fast: "" });
  if (field === "here") delete (next.extra = { ...next.extra }).dirty;
  return next;
}

/** What an inbox row starts work on: a ticket's key, a PR's URL. */
export function sourceOf(row) {
  if (!row) return "";
  if (row.kind === "ticket") return row.key ?? "";
  const url = row.item?.url ?? "";
  return url.includes("/pull/") ? url : "";
}

/** Where the form stands: checking | ready | focus | blocked | running | failed | done | cancelled. */
export function phase(plan, run, { checking = false, started = false } = {}) {
  if (run?.state && run.state !== "running") return run.state;
  if (run || started) return "running";
  if (checking || !plan) return "checking";
  if (plan.error) return "blocked";
  if (!plan.ok) return "blocked";
  return plan.place === "focus" ? "focus" : "ready";
}

/** The source as the hub read it, in a few words: "Ticket · ENG-412 Store and enforce daily quotas"; null for a prompt. */
export function resolved(plan) {
  const word = { ticket: "Ticket", pr: "PR" }[plan?.kind];
  return word && plan.title ? `${word} · ${plan.title}` : null;
}

/** A step's mark: the icon and tone a list shows for its state. */
export function stepMark(state) {
  return (
    { done: { icon: "circle-check", tone: "success" }, running: { icon: null, tone: "info" }, failed: { icon: "circle-x", tone: "destructive" }, skipped: { icon: "minus", tone: "dim" } }[state] ??
    { icon: "circle-dot", tone: "dim" }
  );
}

/** The plan's options (read as a prompt, run a fork's setup), then every blocker's fixes: as drawn, as 1–9 pick. */
export function choicesOf(plan) {
  // A source already open is brought into view: the dialog draws no options for it.
  const options = plan?.place === "focus" ? [] : (plan?.options ?? []);
  return [...options, ...(plan?.blockers ?? []).flatMap((b) => b.fixes ?? [])];
}

/** The agent the form starts, from the plan's choices: the one picked, else the plan's. */
export function agentOf(plan, s) {
  const id = s?.agent ?? plan?.agent ?? plan?.choices?.agent;
  return (plan?.choices?.agents ?? []).find((a) => a.id === id) ?? null;
}

/** Whether the form asks for a model: the agent's table row takes one (or the hub did not say). */
export function takesModel(agent) {
  return agent?.models !== false;
}

/** A path under the home folder written from `~`. */
export function shortPath(text, home) {
  if (!home || home === "/") return String(text ?? "");
  return String(text ?? "").split(home).join("~");
}
