// The control socket's requests: one JSON object per line with an `op`, answered by one JSON object per line. A browser
// is named by its `owner`, the session it belongs to (`app:t3`). `subscribe`
// turns the connection into a stream of events, one object per line: {event, ...}.

/** Every op the shell answers, with the fields it needs. */
export const OPS = {
  ping: [],
  status: [],
  quit: [],
  subscribe: [],
  configure: [],
  open: ["owner"],
  "tabs.list": [],
  "tab.activate": ["owner", "tab"],
  "tab.close": ["owner"],
  "tab.reload": ["owner"],
  "tab.back": ["owner"],
  "tab.forward": ["owner"],
  "tab.navigate": ["owner", "url"],
  "tab.viewport": ["owner"],
  "tab.devtools": ["owner"],
  "tab.sleep": ["owner", "tab"],
  "agent.endpoint": ["owner"],
  "agent.touch": ["owner"],
  "slot.set": [],
  "slot.drop": [],
  "profiles.list": [],
  "profile.sources": [],
  "profile.create": ["name"],
  "profile.rename": ["profile", "name"],
  "profile.delete": ["profile"],
  "profile.use": ["owner"],
  "profile.import": ["profile"],
  "grab.start": ["owner"],
  "grab.stop": [],
  "notes.list": ["owner"],
  "notes.send": ["owner"],
  "notes.clear": ["owner"],
  "notes.sent": ["owner"],
  "owners.list": [],
  "owner.forget": ["owner"],
  "worktree.forget": ["worktree"],
  "history.query": ["q"],
  "downloads.list": [],
};

/** The ops that change what a person owns: asked without the hub's token, the person confirms them in a dialog. */
export const GUARDED = new Set(["profile.create", "profile.delete", "profile.import"]);

/** The ops only the hub may ask, with its token: they hand a person's words to an agent. */
export const HUB_ONLY = new Set(["notes.send"]);

/** A line as a request, or the error to answer with. */
export function parse(line) {
  let request;
  try {
    request = JSON.parse(line);
  } catch {
    return { error: "a request is one JSON object per line" };
  }
  if (!request || typeof request !== "object" || Array.isArray(request)) return { error: "a request is one JSON object per line" };
  const needs = OPS[request.op];
  if (!needs) return { error: `unknown op ${JSON.stringify(request.op ?? null)}` };
  const missing = needs.find((field) => request[field] === undefined || request[field] === null || request[field] === "");
  if (missing) return { error: `${request.op} needs ${missing}` };
  if (request.owner !== undefined && (typeof request.owner !== "string" || !/^[\w:.@/-]{1,200}$/.test(request.owner))) return { error: "owner names a session, such as app:t3" };
  if (request.worktree !== undefined && request.worktree !== "" && (typeof request.worktree !== "string" || !request.worktree.startsWith("/"))) return { error: "worktree is a folder's full path" };
  return { request };
}

/** Whether a request carries the token the hub gave the shell when it started it. */
export const trusted = (request, token) => !!token && typeof request.token === "string" && request.token === token;
