// The palette (cmd-shift-p in the cockpit layout), a superset of Herdr's on its geometry: Herdr's commands with their
// keys, layouts, then cockpit's apps and commands, the spaces, tabs and agents, and what the hub finds (tickets, PRs,
// steps, meetings, releases, the week's events, past sessions). Over an agent, a found item goes into it: put in
// its prompt (ctrl-r), asked about (ctrl-a) or looked into by a fork (ctrl-f). Enter runs, ↑/↓ move, Esc closes.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState, Input } from "gpui-base";
import { commands, layouts, keys, spaces, agents, focused, terminals, run, props, closeOverlay, openOverlay, shared, setShared, window as win } from "herdr";
import * as hub from "./hub.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Kbd, Spinner, Button } from "./ui.js";
import { openInEditor, editorLabel } from "./editor.js";

// Herdr's palette: 620 wide, 60 from the top, 10 rows of 32 under a 36 search line, group titles 26.
export const WIDTH = 620;
const TOP = 60;
// Herdr keeps a popup 8px inside the window: the full-window popup starts there.
const INSET = 8;
const SHOWN = 10;
const ROW_H = 32;
const HEAD_H = 26;
const LIST_H = SHOWN * ROW_H;
const PAGE_H = LIST_H + HEAD_H;
const FOUND_HEAD = "Found in cockpit";

const GROUPS = ["Recent", "Session", "Here", "Cockpit", "Places", "Tab", "Pane", "Space", "Layout", "Terminal", "Agent", "Window", "App"];

// [child of the center stack, label, icon, the command its key is bound to].
const APPS = [
  ["home", "Home", "layout-dashboard", "cockpit.home"],
  ["work", "Work", "terminal"],
  ["inbox", "Inbox", "inbox"],
  ["runbook", "Runbook", "book"],
  ["todo", "To-do", "list-todo"],
  ["meetings", "Meetings", "calendar"],
  ["monitor", "Monitor", "activity"],
  ["routines", "Routines", "clock"],
  ["mail", "Mail", "mail"],
];

// cockpit's window keys (keys.json's layout:cockpit), by name: [command, title, icon, keywords].
const COCKPIT_COMMANDS = [
  ["cockpit.recap", "Recap the agents", "sparkles", "whose move stages summary"],
  ["cockpit.next-waiting", "Next agent waiting for you", "bell", "needs you question jump"],
  ["cockpit.next:blocked", "Next blocked agent", "circle-dot", "needs you"],
  ["cockpit.next:done", "Next finished agent", "circle-check", "done"],
  ["cockpit.agent-view", "Agent view", "bot", "focus lone terminal tab toggle"],
  ["cockpit.panel", "Side panel", "panel-right", "show review runbook files changes toggle"],
  ["cockpit.panel:show", "Side panel: Show", "layout", "charts tables diagrams"],
  ["cockpit.panel:review", "Side panel: Review", "git-pull-request", "diff comments"],
  ["cockpit.panel:runbook", "Side panel: Runbook", "book", "steps ticket"],
  ["cockpit.panel:files", "Side panel: Files", "folder", "tree"],
  ["cockpit.panel:changes", "Side panel: Changes", "git-fork", "git diff"],
  ["cockpit.queue", "Prompt queue", "list", "follow-ups send later"],
  ["cockpit.resume", "Resume stopped agents", "play", "restart reboot resume session"],
  ["cockpit.pr", "This branch's PR", "git-pull-request", "pull request github"],
  ["cockpit.ticket", "This branch's ticket", "ticket", "linear jira issue"],
];

// Herdr's own names for its commands: [command, args, group, title, keywords].
const NAMED = [
  ["tab.new", {}, "Tab", "New tab", "tab agent new task"],
  ["mod.open", { name: "picker" }, "Tab", "Go to space or tab…", "switch jump find"],
  ["tab.rename", {}, "Tab", "Rename tab", "tab"],
  ["tab.close", {}, "Tab", "Close tab", "tab kill"],
  ["tab.next", {}, "Tab", "Next tab", "tab"],
  ["tab.prev", {}, "Tab", "Previous tab", "tab"],
  ["pane.split", { direction: "right" }, "Pane", "Split right", "vertical"],
  ["pane.split", { direction: "down" }, "Pane", "Split down", "horizontal"],
  ["pane.close", {}, "Pane", "Close pane", "kill"],
  ["pane.zoom", {}, "Pane", "Zoom pane", "maximize fullscreen"],
  ...["left", "right", "up", "down"].map((d) => ["pane.go", { direction: d }, "Pane", `Focus pane ${d}`, "move go"]),
  ...["left", "right", "up", "down"].map((d) => ["pane.swap", { direction: d }, "Pane", `Swap pane ${d}`, "move"]),
  ["space.new", {}, "Space", "New space", "workspace"],
  ["space.rename", {}, "Space", "Rename space", "workspace"],
  ["space.close", {}, "Space", "Close space", "workspace kill"],
  ["space.next", {}, "Space", "Next space", "workspace"],
  ["space.prev", {}, "Space", "Previous space", "workspace"],
  ["layout.switch", {}, "Layout", "Next layout", "look switch"],
  ["layout.reset", {}, "Layout", "Reset layout", "new version default restore herdr's"],
  ["layout.clone", {}, "Layout", "Duplicate layout", "clone copy customize"],
  ["mod.clone", {}, "Layout", "Duplicate mod", "clone copy customize"],
  ["terminal.open", {}, "Terminal", "New terminal", "shell"],
  ["terminal.copy", {}, "Terminal", "Copy", "clipboard"],
  ["terminal.paste", {}, "Terminal", "Paste", "clipboard"],
  ["agent.next", {}, "Agent", "Next agent", ""],
  ["agent.prev", {}, "Agent", "Previous agent", ""],
  ["window.minimize", {}, "Window", "Minimize window", ""],
  ["window.maximize", {}, "Window", "Maximize window", "restore"],
  ["window.close", {}, "Window", "Close window", "quit"],
  ["app.settings", {}, "App", "Settings", "preferences options config look font theme sound"],
  ["app.help", {}, "App", "Keys", "keybindings shortcuts help"],
  ["app.welcome", {}, "App", "Welcome", "intro start onboarding"],
  ["app.whats_new", {}, "App", "What's new", "release changes news update"],
  ["app.reset", {}, "App", "Reset to the default screen", "escape hatch broken restore"],
  ["app.zoom_in", {}, "App", "Zoom in", "bigger font size"],
  ["app.zoom_out", {}, "App", "Zoom out", "smaller font size"],
  ["app.zoom_reset", {}, "App", "Reset zoom", "font size"],
  ["app.quit", {}, "App", "Quit Herdr", "exit"],
  ...["graphite", "ink", "moss", "atom-one-dark"].map((n) => ["app.theme", { name: n }, "App", `Theme: ${n.replace(/-/g, " ")}`, "colors look"]),
];

// Commands that act on something pointed at (ids, terminals): not for a list.
const HIDDEN = new Set([
  "space.move", "tab.move", "space.select", "tab.select", "pane.focus", "terminal.keep", "terminal.park", "terminal.kill",
  "terminal.show", "agent.focus", "layout.set", "layout.toggle", "layout.show", "layout.hide", "layout.resize",
  "layout.select", "mod.open", "overlay.close", "tab.settle", "tab.unsettle", "app.palette", "app.hide",
  "app.hide_others", "app.show_all", "app.theme", "app.settings",
]);

const FOUND = {
  pr: ["PR", "git-pull-request"], review: ["Review", "git-pull-request"], ticket: ["Ticket", "ticket"], step: ["Step", "book"],
  runbook: ["Runbook", "book"], block: ["Documents", "layers"], view: ["Documents", "layers"], todo: ["To-do", "list-todo"],
  meeting: ["Meeting", "calendar"], release: ["Release", "rocket"], app: ["Monitor", "activity"], routine: ["Routine", "clock"],
  changed: ["Changed", "history"], session: ["Session", "message"],
};

const STATUS = new Set(["working", "blocked", "done", "idle"]);

// The sidebar copy shows a block of the cockpit layout on `cockpit.go:<child>`.
const show = (child) => run(`cockpit.go:${child}`);

const titleOf = (name) => {
  const [noun, verb] = name.split(".");
  return { group: noun.charAt(0).toUpperCase() + noun.slice(1), title: `${verb.replace(/_/g, " ")} ${noun}`.replace(/^./, (c) => c.toUpperCase()) };
};

const SYMBOL = { cmd: "⌘", shift: "⇧", alt: "⌥", ctrl: "⌃", enter: "↵", escape: "Esc", backspace: "⌫", space: "Space", up: "↑", down: "↓", left: "←", right: "→" };

/** "cmd-shift-p" as the caps ⌘ ⇧ P. */
export function keyCaps(k) {
  const parts = k.endsWith("--") ? [...k.slice(0, -2).split("-"), "-"] : k.split("-");
  return parts.map((p) => SYMBOL[p] ?? (p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1)));
}

/** Letters of `q` in order in `text`: a score (word starts and runs count more) and where they matched; null: none. */
export function fuzzy(q, text) {
  const t = text.toLowerCase();
  let score = 0;
  let at = 0;
  let streak = 0;
  const hits = [];
  for (const ch of q) {
    const i = t.indexOf(ch, at);
    if (i < 0) return null;
    const start = i === 0 || /[\s:.\-_/›]/.test(t[i - 1]);
    streak = i === at && hits.length ? streak + 1 : 0;
    score += 1 + (start ? 6 : 0) + streak * 3 - Math.min(i - at, 6) * 0.3;
    hits.push(i);
    at = i + 1;
  }
  return { score: score - text.length * 0.02, hits };
}

const OPEN = { mod: "cockpit", props: { app: "palette" }, kind: "popup", frame: "none", width: 1, height: 1, at: { x: 0, y: 0 } };

/**
 * Opens the palette over the window, or closes it when it is open: `cockpit.palette` reaches the open palette too,
 * which closes itself and says so, so the sidebar opens one only when none answered.
 */
export function togglePalette(cx) {
  if (!cx) return openOverlay(OPEN);
  const asked = Date.now();
  cx.timer.after(80, () => {
    if ((shared("cockpit.palette.closed") ?? 0) < asked) openOverlay(OPEN);
  });
}

export default class Palette {
  init(_props, cx) {
    this.at = 0;
    this.scroll = 0;
    this.found = [];
    this.searching = false;
    this.asked = 0;
    this.page = null;
    this.busy = null;
    this.error = null;
    this.editor = "VS Code";
    this.recent = Array.isArray(shared("cockpit.palette.recent")) ? shared("cockpit.palette.recent") : [];
    this.over = this.agentBelow();
    this.place = this.placeBelow();
    this.handle = cx.focus_handle();
    this.input = InputState.new({ placeholder: "Search cockpit or type a command…" });
    this.input.on("change", (_e, cx) => this.changed(cx));
    this.input.on("submit", (_e, cx) => this.pick(this.rows()[this.at]?.e, cx));
    this.input.focus();
    cx.timer.after(400, () => this.input.focus());
    cx.spawn(async (cx) => {
      this.editor = await editorLabel();
      cx.notify();
    });
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  /** The agent focused when the palette opened: where found items can go, with its program as the hub names it. */
  agentBelow() {
    const f = focused();
    const address = f?.agent ?? f?.terminal;
    const a = address && agents().find((x) => x.address === address);
    if (!a) return null;
    // Herdr's program key, else what the agent reported ("claude-code", "codex"): the hub knows both spellings.
    const agent = a.program?.key ?? a.app ?? "";
    const t = terminals().find((x) => x.address === a.address);
    const labels = t?.labels ?? {};
    const session = Object.entries(labels).find(([k]) => /session/.test(k))?.[1] ?? null;
    // A terminal titled after its program says less than the place it runs in.
    const title = a.title && a.title !== a.program?.name ? a.title : `${a.space_name} › ${a.tab_name}`;
    return { terminal: a.address, title, cwd: t?.cwd ?? f.cwd ?? "", session, agent, name: a.program?.name ?? agent };
  }

  /** The folder of the focused terminal or space, and its machine, for Open in editor. */
  placeBelow() {
    const f = focused();
    const space = spaces().find((s) => s.id === f?.space);
    const folder = f?.cwd || space?.folder || null;
    return folder ? { folder, machine: space?.machine ?? null, name: folder.split("/").filter(Boolean).pop() ?? folder } : null;
  }

  changed(cx) {
    this.at = 0;
    this.scroll = 0;
    this.error = null;
    const query = this.input.value().trim();
    const asked = ++this.asked;
    if (query.length < 2) {
      this.found = [];
      this.searching = false;
      cx.notify();
      return;
    }
    this.searching = true;
    cx.notify();
    cx.spawn(async (cx) => {
      let found = [];
      try {
        found = await hub.search(query);
      } catch {}
      if (asked !== this.asked) return;
      this.found = found;
      this.searching = false;
      cx.notify();
    });
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Entries

  entries() {
    const known = new Set(commands().map((c) => c.name));
    const mac = win()?.platform === "macos";
    const list = [];
    const add = (e) => list.push({ args: {}, words: "", ...e, id: e.id ?? `${e.name}:${JSON.stringify(e.args ?? {})}` });

    if (this.over) {
      const o = this.over;
      add({ id: "fork-tab", group: "Session", icon: "git-fork", title: "Fork this session to a new tab", words: `${o.agent} ${o.title}`, act: () => this.agentsFork("tab") });
      add({ id: "fork-right", group: "Session", icon: "git-fork", title: "Fork this session to the right", words: `${o.agent} split ${o.title}`, act: () => this.agentsFork("right") });
    }
    if (this.place) {
      const p = this.place;
      add({ id: "editor", group: this.over ? "Session" : "Here", icon: "code", title: `Open ${p.name} in ${this.editor}`, words: `editor vscode code cursor ${p.folder}`, act: () => this.editorOpen() });
    }
    for (const [child, label, icon, bound] of APPS) {
      add({ id: `app:${child}`, group: "Cockpit", icon, title: label, name: bound ?? `cockpit.go:${child}`, words: "cockpit app go", act: () => (show(child), null) });
    }
    for (const [name, title, icon, words] of COCKPIT_COMMANDS) add({ id: name, group: "Cockpit", icon, title, name, words, act: () => (run(name), null) });
    add({ id: "refresh", group: "Cockpit", icon: "refresh", title: "Refresh the inbox", words: "fetch PRs reviews tickets CI now", act: () => hub.act("inbox", "request_refresh") });
    add({ id: "doctor", group: "Cockpit", icon: "shield-check", title: "Check cockpit", words: "doctor health report", page: "doctor" });
    add({ id: "brief", group: "Cockpit", icon: "newspaper", title: "Today's brief", words: "morning meetings reviews CI tickets", page: "brief" });
    add({ id: "kit", group: "Cockpit", icon: "palette", title: "Design kit", words: "components gallery ui", name: "cockpit.kit", act: () => (run("cockpit.kit"), null) });

    for (const s of spaces()) {
      add({ id: `space:${s.id}`, group: "Places", icon: "folder", title: s.name, hint: s.branch ?? "", status: s.status, words: `space ${s.branch ?? ""}`, act: () => (run("space.select", { id: s.id }), show("work"), null) });
      if (s.tabs.length > 1)
        for (const t of s.tabs) add({ id: `tab:${t.id}`, group: "Places", icon: "app-window", title: `${s.name} › ${t.name}`, status: t.status, words: "tab", act: () => (run("tab.select", { id: t.id }), show("work"), null) });
    }
    for (const a of agents()) {
      add({ id: `agent:${a.address}`, group: "Places", icon: "bot", title: a.title || a.program?.name || a.address, hint: `${a.space_name} › ${a.tab_name}`, status: a.status, words: `agent ${a.program?.name ?? ""} ${a.space_name}`, act: () => (run("agent.focus", { terminal: a.address }), show("work"), null) });
    }

    for (const [name, args, group, title, words] of NAMED) {
      if (!known.has(name) || (mac && name.startsWith("window."))) continue;
      add({ name, args, group, title, words });
    }
    for (const l of layouts()) {
      const t = { main: "Herdr", terminals: "Terminals", win98: "Herdr 98", cockpit: "Cockpit" }[l] ?? l.charAt(0).toUpperCase() + l.slice(1);
      add({ name: "layout.switch", args: { name: l }, group: "Layout", icon: "layout", title: `Layout: ${t}`, words: `look ${l}` });
    }
    const named = new Set(NAMED.map(([n]) => n));
    for (const c of commands()) {
      if (named.has(c.name) || HIDDEN.has(c.name) || c.args.some((a) => a.required && a.name !== "name")) continue;
      add({ name: c.name, ...titleOf(c.name), words: c.about });
    }
    return list;
  }

  /** The key bound to an entry: `name` or `name:value`, in keys.json's global or cockpit sections. */
  keyOf(e, bound) {
    if (!e.name) return null;
    const v = Object.values(e.args ?? {})[0];
    const want = v === undefined ? e.name : `${e.name}:${v}`;
    return bound.find((b) => (b.section === "global" || b.section === "layout:cockpit") && b.action === want)?.keys ?? null;
  }

  /** The rows: grouped when empty, ranked while typing, then what the hub found. */
  rows() {
    const all = this.entries();
    const q = this.input.value().trim().toLowerCase().replace(/\s+/g, "");
    if (!q) {
      const recent = this.recent.map((id) => all.find((e) => e.id === id)).filter(Boolean).slice(0, 5);
      const rest = GROUPS.slice(1).flatMap((g) => all.filter((e) => e.group === g && !recent.includes(e)));
      const others = all.filter((e) => !GROUPS.includes(e.group) && !recent.includes(e));
      return [...recent.map((e) => ({ e, head: "Recent" })), ...[...rest, ...others].map((e) => ({ e, head: e.group }))];
    }
    const rank = (e) => {
      const r = this.recent.indexOf(e.id);
      return r < 0 ? 99 : r;
    };
    const local = all
      .map((e) => {
        const title = fuzzy(q, e.title);
        const more = fuzzy(q, `${e.group} ${e.title} ${e.hint ?? ""} ${e.name ?? ""} ${e.words}`);
        // Letters strewn across the keywords are noise: a keyword match must keep them close together.
        const tight = more && more.hits[more.hits.length - 1] - more.hits[0] < q.length * 3;
        const best = title ?? (tight && { score: more.score - 4, hits: [] });
        return best && { e, score: best.score, hits: best.hits, head: null };
      })
      .filter(Boolean)
      .sort((a, b) => b.score - a.score || rank(a.e) - rank(b.e));
    const found = this.found.map((f, i) => {
      const [group, icon] = FOUND[f.kind] ?? FOUND[f.app] ?? ["Found", "search"];
      return { e: { id: `found:${i}`, group, icon, title: f.title, hint: f.help, found: f }, head: FOUND_HEAD };
    });
    return [...local, ...found];
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Doing

  remember(e) {
    if (e.found) return;
    this.recent = [e.id, ...this.recent.filter((r) => r !== e.id)].slice(0, 20);
    setShared("cockpit.palette.recent", this.recent);
  }

  /** Runs an entry; one that waits on the hub keeps the palette open until it answers, with its error in place. */
  pick(e, cx) {
    if (!e || this.busy) return;
    this.remember(e);
    if (e.page) return this.openPage(e.page, cx);
    if (e.found) return this.wait(e.id, () => hub.go(e.found.target), cx);
    if (e.act) return this.wait(e.id, e.act, cx);
    this.dismiss();
    if (e.name === "mod.clone")
      openOverlay({ mod: "prompt", kind: "dialog", width: 380, props: { title: "Duplicate mod", hint: "Mod to copy; creates <name>-copy", command: { name: "mod.clone", args: {} } } });
    else run(e.name, e.name === "layout.clone" ? { name: shared("layout") ?? "main" } : e.args);
  }

  wait(id, work, cx) {
    this.busy = id;
    this.error = null;
    cx.notify();
    cx.spawn(async (cx) => {
      let error = null;
      try {
        const answer = await work();
        error = typeof answer === "string" ? answer : (answer?.error ?? null);
      } catch (err) {
        error = err?.message ?? String(err);
      }
      this.busy = null;
      if (!error) return this.dismiss();
      this.error = error;
      cx.notify();
    });
  }

  /** ctrl-r, ctrl-a, ctrl-f: the selected found item into the agent below. */
  intoSession(action, cx) {
    const e = this.rows()[this.at]?.e;
    if (!this.over || !e?.found || this.busy) return false;
    this.wait(`${e.id}:${action}`, () => hub.act("palette", action, [this.over, e.found]), cx);
    return true;
  }

  agentsFork(place) {
    return hub.act("agents", "fork", [{ terminal: `app:${this.over.terminal}`, place }]);
  }

  editorOpen() {
    return openInEditor(this.place.folder, this.place.machine);
  }

  openPage(kind, cx) {
    this.page = { kind, title: kind === "doctor" ? "Cockpit doctor" : "Today's brief", rows: null, at: 0 };
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        const page = await hub.view("palette", { report: kind });
        if (this.page?.kind !== kind) return;
        this.page = { ...this.page, title: page.title ?? this.page.title, rows: page.rows ?? [] };
      } catch (err) {
        if (this.page) this.page.rows = [{ mark: "✗", tone: "bad", text: err?.message ?? String(err) }];
      }
      cx.notify();
    });
  }

  /** A key, from the window or `cockpit.key:` (tests): true when the palette used it. */
  key(e, cx, typed = false) {
    const key = e.keystroke;
    if (this.page) {
      const rows = this.page.rows ?? [];
      if (key === "escape") this.page = null;
      else if (key === "down") this.page.at = Math.min(this.page.at + 1, rows.length - 1);
      else if (key === "up") this.page.at = Math.max(this.page.at - 1, 0);
      else if (key === "enter" && rows[this.page.at]?.url) cx.spawn(async () => hub.openUrl(rows[this.page.at].url));
      else return false;
      return true;
    }
    const rows = this.rows();
    if (key === "escape") this.dismiss();
    else if (key === "down" || key === "ctrl-n") this.at = Math.min(this.at + 1, rows.length - 1);
    else if (key === "up" || key === "ctrl-p") this.at = Math.max(this.at - 1, 0);
    else if (key === "pagedown") this.at = Math.min(this.at + SHOWN, rows.length - 1);
    else if (key === "pageup") this.at = Math.max(this.at - SHOWN, 0);
    // A press of Enter in the search line is its submit; only a driven one comes here.
    else if (key === "enter" && !typed) this.pick(rows[this.at]?.e, cx);
    else if (key === "ctrl-r") return this.intoSession("insert", cx);
    else if (key === "ctrl-a") return this.intoSession("ask", cx);
    else if (key === "ctrl-f") return this.intoSession("fork", cx);
    else return false;
    return true;
  }

  /** `cockpit.palette` again closes it; `cockpit.palette:type:<text>` types into the search (tests drive it this way). */
  command(c, cx) {
    if (c === "palette") {
      setShared("cockpit.palette.closed", Date.now());
      this.dismiss();
    } else if (c.startsWith("palette:type:")) {
      this.input.set_value(c.slice(13));
      this.changed(cx);
    }
  }

  // ---------------------------------------------------------------------------------------------------------------
  // Drawing

  title(k, text, hits) {
    if (!hits?.length) return div().truncate().text_color(k.foreground).child(text);
    const parts = [];
    let cur = "";
    let lit = false;
    [...text].forEach((ch, i) => {
      const h = hits.includes(i);
      if (h !== lit && cur) {
        parts.push([cur, lit]);
        cur = "";
      }
      lit = h;
      cur += ch;
    });
    if (cur) parts.push([cur, lit]);
    return h_flex().overflow_hidden().children(parts.map(([s, h]) => div().flex_none().text_color(h ? k.foreground : k.mutedFg).when(h, (d) => d.font_semibold()).child(s.replace(/ /g, " "))));
  }

  sessionButtons(k) {
    const b = (id, label, key, action) =>
      h_flex()
        .id(`into-${id}`)
        .h(22)
        .px(6)
        .gap(4)
        .flex_none()
        .items_center()
        .rounded(RADIUS.sm)
        .border_1()
        .border_color(k.border)
        .bg(k.background)
        .text_size(TEXT.xs)
        .font_medium()
        .text_color(k.foreground)
        .cursor_pointer()
        .hover((s) => s.bg(k.accent))
        .on_click((_e, cx) => {
          cx.stop_propagation();
          this.intoSession(action, cx);
        })
        .tooltip(`${label}: into ${this.over.title}`)
        .child(label)
        .child(div().text_color(k.dim).child(key));
    return h_flex()
      .gap(4)
      .flex_none()
      .child(b("insert", "Insert", "⌃R", "insert"))
      .child(b("ask", "Ask", "⌃A", "ask"))
      .child(b("fork", "Fork", "⌃F", "fork"));
  }

  leading(k, e, on, busy) {
    let glyph = null;
    if (busy) glyph = Spinner({ size: 14, color: k.mutedFg });
    else if (STATUS.has(e.status)) glyph = e.status === "working" ? Spinner({ size: 12, color: k.working }) : div().size(8).rounded(4).bg(k[e.status]);
    else if (e.icon) glyph = Icon(e.icon, { size: 15, color: on ? k.foreground : k.mutedFg });
    return h_flex().size(16).flex_none().items_center().justify_center().when(Boolean(glyph), (el) => el.child(glyph));
  }

  list(k, rows) {
    const q = this.input.value().trim();
    const bound = keys();
    const tops = [];
    let y = 0;
    rows.forEach((r, i) => {
      const head = Boolean(r.head) && (!q || r.head === FOUND_HEAD) && (i === 0 || r.head !== rows[i - 1].head);
      tops.push({ head, title: y, row: y + (head ? HEAD_H : 0) });
      y += (head ? HEAD_H : 0) + ROW_H;
    });
    const fixed = y > LIST_H;
    this.maxScroll = Math.max(0, y - LIST_H);
    const sel = tops[this.at];
    if (sel) {
      const from = sel.head ? sel.title : sel.row;
      if (from < this.scroll) this.scroll = from;
      if (sel.row + ROW_H > this.scroll + LIST_H) this.scroll = sel.row + ROW_H - LIST_H;
    }
    this.scroll = Math.max(0, Math.min(this.scroll, this.maxScroll));
    // Each row sits at its edges from the model, not stacked: whole-pixel rounding would drift a long stack.
    const place = (el, start, end) => (fixed ? el.absolute().left(0).right(0).top(start - this.scroll).h(end - start) : el.flex_none().h(end - start));
    const visible = (start, end) => !fixed || (end > this.scroll && start < this.scroll + LIST_H);
    const out = [];
    rows.forEach((r, i) => {
      const { head, title, row } = tops[i];
      if (head && visible(title, row))
        out.push(
          place(h_flex(), title, row)
            .px(14)
            .pb(4)
            .gap(6)
            .items_end()
            .text_size(TEXT.xs)
            .font_medium()
            .text_color(k.dim)
            .child(r.head)
            .when(r.head === FOUND_HEAD && this.searching, (e) => e.child(Spinner({ size: 11, color: k.dim }))),
        );
      if (!visible(row, row + ROW_H)) return;
      const on = i === this.at;
      const e = r.e;
      const key = this.keyOf(e, bound);
      const busy = Boolean(this.busy) && (this.busy === e.id || this.busy.startsWith(`${e.id}:`));
      const into = on && Boolean(e.found) && Boolean(this.over);
      out.push(
        place(h_flex(), row, row + ROW_H)
          .id(`pl-${i}`)
          .mx(6)
          .px(10)
          .gap(10)
          .items_center()
          .rounded(RADIUS.md + 1)
          .cursor_pointer()
          .text_size(TEXT.base)
          .when(on, (el) => el.bg(k.selected))
          .when(!on, (el) => el.hover((s) => s.bg(k.accent)))
          .on_click((_ev, cx) => {
            this.at = i;
            this.pick(e, cx);
          })
          .child(this.leading(k, e, on, busy))
          .child(
            h_flex()
              .flex_1()
              .min_w(0)
              .gap(8)
              .items_center()
              .child(div().flex_none().max_w_full().child(this.title(k, e.title, r.hits)))
              .when(Boolean(e.hint), (el) => el.child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).text_color(k.dim).child(e.hint))),
          )
          .when(into, (el) => el.child(this.sessionButtons(k)))
          .when(!into && Boolean(q) && !key, (el) => el.child(div().flex_none().text_size(TEXT.xs).text_color(k.dim).child(e.group)))
          .when(!into && Boolean(key), (el) => el.child(Kbd(keyCaps(key).join("")))),
      );
    });
    return div()
      .id("palette-rows")
      .relative()
      .when(fixed, (el) => el.h(LIST_H).overflow_hidden())
      .on_scroll_wheel((ev, cx) => {
        const d = Math.abs(ev.delta.y) < 4 ? ev.delta.y * ROW_H : ev.delta.y;
        this.scroll = Math.max(0, Math.min(this.scroll - d, this.maxScroll));
        while (this.at < rows.length - 1 && tops[this.at].row < this.scroll) this.at++;
        while (this.at > 0 && tops[this.at].row + ROW_H > this.scroll + LIST_H) this.at--;
        cx.notify();
      })
      .children(fixed ? out : [v_flex().children(out)]);
  }

  pageView(k) {
    const p = this.page;
    const tone = { good: k.success, bad: k.destructive, warn: k.warning, dim: k.dim };
    const icon = { good: "circle-check", bad: "circle-x", warn: "alert", dim: "circle" };
    const head = h_flex()
      .h(HEAD_H)
      .pr(8)
      .gap(6)
      .items_center()
      .child(Button("page-back", { icon: "chevron-left", variant: "ghost", size: "icon-sm", tooltip: "Back  Esc", onClick: (_e, cx) => ((this.page = null), cx.notify()) }))
      .child(div().flex_1().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(p.title))
      .when(p.rows === null, (e) => e.child(Spinner({ size: 13, color: k.mutedFg })));
    const rows = p.rows ?? [];
    const body =
      p.rows === null
        ? div().px(14).py(12).text_size(TEXT.base).text_color(k.mutedFg).child(p.kind === "doctor" ? "Checking every part of cockpit…" : "Reading today's brief…")
        : v_flex()
            .id("page-rows")
            .h(LIST_H)
            .overflow_y_scrollbar()
            .children(
              rows.map((r, i) =>
                h_flex()
                  .id(`page-${i}`)
                  .mx(6)
                  .px(10)
                  .py(7)
                  .gap(10)
                  .items_start()
                  .rounded(RADIUS.md + 1)
                  .text_size(TEXT.base)
                  .when(i === p.at, (e) => e.bg(k.selected))
                  .when(Boolean(r.url), (e) => e.cursor_pointer().hover((s) => s.bg(k.accent)).on_click(() => hub.openUrl(r.url)))
                  .child(div().pt(1).child(Icon(icon[r.tone] ?? "circle", { size: 15, color: tone[r.tone] ?? k.mutedFg })))
                  .child(
                    v_flex()
                      .flex_1()
                      .min_w(0)
                      .child(div().text_color(k.foreground).child(r.text))
                      .when(Boolean(r.url), (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.dim).child(r.url ?? ""))),
                  )
                  .when(Boolean(r.url), (e) => e.child(Icon("external", { size: 13, color: k.dim }))),
              ),
            );
    return v_flex().h(PAGE_H).child(head).child(body);
  }

  render(cx) {
    const k = tokens();
    const rows = this.page ? [] : this.rows();
    this.at = Math.min(this.at, Math.max(0, rows.length - 1));
    const q = this.input.value().trim();
    const over = this.over;
    const searchLine = h_flex()
      .h(36)
      .flex_none()
      .px(10)
      .gap(10)
      .items_center()
      .rounded(RADIUS.lg + 1)
      .border_1()
      .border_color(k.ring)
      .bg(k.background)
      .child(Icon("command", { size: 16, color: k.dim }))
      .child(Input.new(this.input).flex_1().h_full().text_size(TEXT.base).text_color(k.foreground))
      .when(Boolean(over), (e) =>
        e.child(
          h_flex()
            .id("over")
            .h(22)
            .max_w(200)
            .px(7)
            .gap(5)
            .flex_none()
            .items_center()
            .rounded(RADIUS.sm)
            .bg(alpha(k.info, 0.12))
            .text_size(TEXT.xs)
            .font_medium()
            .text_color(k.info)
            .tooltip(`Found items go into this ${over?.name ?? "agent"} session: ⌃R insert, ⌃A ask, ⌃F fork`)
            .child(Icon("bot", { size: 12, color: k.info }))
            .child(div().truncate().child(over?.title ?? "")),
        ),
      )
      .when(!over, (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child("Esc")));
    const empty = div()
      .px(16)
      .py(14)
      .text_size(TEXT.base)
      .text_color(k.dim)
      .child(this.searching ? "Searching cockpit…" : `Nothing matches "${q}".`);
    const error = this.error
      ? h_flex()
          .mx(6)
          .px(10)
          .py(6)
          .gap(8)
          .items_center()
          .rounded(RADIUS.md)
          .bg(alpha(k.destructive, 0.12))
          .text_size(TEXT.sm)
          .text_color(k.destructive)
          .child(Icon("circle-x", { size: 14, color: k.destructive }))
          .child(div().flex_1().truncate().child(this.error))
      : null;
    const card = v_flex()
      .id("palette-card")
      .w(WIDTH)
      .max_w_full()
      .p(18)
      .gap(10)
      .rounded(RADIUS.xl)
      .border_1()
      .border_color(k.border)
      .bg(k.popover)
      .shadow_lg()
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(searchLine)
      .when(Boolean(error), (e) => e.child(error))
      .child(this.page ? this.pageView(k) : rows.length ? this.list(k, rows) : empty);
    return div()
      .id("cockpit-palette")
      .track_focus(this.handle)
      .size_full()
      .flex()
      .flex_col()
      .items_center()
      .pt(TOP - INSET)
      .pl(16 - INSET)
      .pr(16 + INSET)
      .on_mouse_down("left", () => this.dismiss())
      .on_key_down((e, cx) => {
        if (!this.key(e, cx, true)) return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(card);
  }
}
