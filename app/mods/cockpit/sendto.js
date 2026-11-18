// Who a review goes to: the agents working in the reviewed repository (sessions cockpit drives among them), the one the
// panel follows by default, the person's choice kept per repository, or a new agent Start work opens there on the review.

/**
 * The shared key holding the choice per repository: `{[repo]: {handle, name} | {space, name} | "new"}`. `space` names
 * the agent a review just started, by its space's label.
 */
export const CHOICES = "review.to";
/** The shared key a view sets to have the review panel follow an agent: `{handle, cwd}`, or null to follow the focus. */
export const SUBJECT = "review.subject";

/** Whether an agent row works in the reviewed repo: `here` is the handles whose folder's git toplevel is that repo,
 * as the review view lists them, so a worktree nested inside it is not counted. */
function worksIn(row, here) {
  return (here ?? []).includes(row?.terminal);
}

/** What a row is called in the menu and the toasts: its task, else its tab, else its agent. */
export function nameOf(row) {
  return row?.title || row?.tab?.name || row?.program || row?.terminal || "the agent";
}

/** How the menu describes a row beside its name. */
function aboutOf(row) {
  return `${row?.driven ? "in chat" : row?.program || "agent"}, ${row?.status ?? "idle"}`;
}

/** Whether a row is the terminal a handle or address names. */
export function isRow(row, handle) {
  return Boolean(handle) && (row?.terminal === handle || row?.address === handle);
}

/** The agents working in the repo, the most recently active first: working ones, then by their latest change. */
export function agentsIn(rows, here) {
  const rank = (r) => (r.status === "working" ? 2 : r.status === "idle" ? 0 : 1);
  return (rows ?? []).filter((r) => worksIn(r, here)).sort((a, b) => rank(b) - rank(a) || (b.since ?? 0) - (a.since ?? 0));
}

/**
 * Who the comments go to: `{kind: "agent", row}`, `{kind: "gone", name}` for a chosen agent that stopped or works
 * elsewhere since, or `{kind: "new"}`. The person's choice for the repo first, then the agent the panel follows, then
 * the repo's most recently active agent; with none, a new agent.
 */
export function recipient(rows, here, { chosen = null, followed = null, now = Date.now() } = {}) {
  const working = agentsIn(rows, here);
  if (chosen === "new") return { kind: "new" };
  if (chosen?.handle || chosen?.space) {
    const row = working.find((r) => (chosen.handle ? isRow(r, chosen.handle) : r.space?.name === chosen.space || r.tab?.name === chosen.space));
    const name = chosen.name ?? chosen.handle ?? chosen.space;
    // The agent a review just started is starting until it shows up, a couple of minutes at most.
    if (!row && chosen.space && now - (chosen.at ?? 0) < STARTING_MS) return { kind: "starting", name };
    return row ? { kind: "agent", row } : { kind: "gone", name };
  }
  const row = working.find((r) => isRow(r, followed)) ?? working[0];
  return row ? { kind: "agent", row } : { kind: "new" };
}

/** How long the agent a review started may take to show up before it is said gone. */
const STARTING_MS = 120_000;

/** What the To button says for a recipient. */
export function recipientLabel(to) {
  if (to.kind === "agent") return nameOf(to.row);
  if (to.kind === "gone") return `${to.name} (gone)`;
  if (to.kind === "starting") return `${to.name} (starting)`;
  return "a new agent";
}

/** The recipient menu: the repo's agents, a long name cut so what it is stays in view, then a new agent; each item runs
 * `cockpit.review:to:<handle|new>|<repo>`, so only the review of that repo takes it. */
export function menuItems(rows, here, to, repo) {
  const current = to.kind === "agent" ? to.row.terminal : to.kind === "new" ? "new" : null;
  const short = (text) => (text.length > 30 ? `${text.slice(0, 29)}…` : text);
  const items = agentsIn(rows, here).map((r) => ({
    label: `${short(nameOf(r))} · ${aboutOf(r)}`,
    icon: r.terminal === current ? "check" : r.driven ? "message-circle" : "terminal",
    command: `review:to:${r.terminal}|${repo}`,
  }));
  if (items.length) items.push("separator");
  items.push({ label: "A new agent, started on the review", icon: current === "new" ? "check" : "plus", command: `review:to:new|${repo}` });
  return items;
}

/**
 * The Start work request that opens a new agent in the repo's checkout, the review as its prompt: in a space of its
 * own (`label`), and leaving Start work's own choices of repo, agent and mode as they were.
 */
export function newAgentRequest(repo, prompt, now) {
  return { id: `review-${now}`, source: "prompt", text: prompt, repo, worktree: "here", dirty: true, label: `review-${now}`, remember: false };
}

/** An action's error as the person reads it, without the action's name the hub puts first. */
export function plainError(error) {
  return String(error ?? "").replace(/^(send|work): /, "");
}
