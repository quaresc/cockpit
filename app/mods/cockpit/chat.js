// The conversation with the agent in front, read rather than watched: a turn is what you said, one folded row for
// everything the agent did, and the answer under a rule. The box below says what it may do and which model answers.
// The `chat` hub view reads the agent's own transcript, whichever agent wrote it; its terminal is ⌃` away.

import { div } from "gpui-kit";
import { h_flex, v_flex, Scrollbar, Input, InputState, TextView } from "gpui-base";
import * as base from "gpui-base";

// A Herdr older than 0.1.15 has no ScrollHandle: the conversation still draws, it just does not follow its own end.
const ScrollHandle = base.ScrollHandle ?? null;
import { focused, openOverlay, run, exec, notify, spaces, window as win } from "herdr";
import * as hub from "./hub.js";
import { openUrl } from "./hub.js";
import { openPanel } from "./status.js";
import { AgentsModel, QuestionCard } from "./agentsui.js";
import { tokens, TEXT, RADIUS, Icon, Spinner, Chip, Empty, openMenu } from "./ui.js";

/** The permission modes the chip wears: what it says, how it looks, and whether the menu offers it. */
const MODES = {
  default: { label: "Asks first", icon: "shield-check", loud: false, offered: true },
  acceptEdits: { label: "Edits freely", icon: "triangle-alert", loud: true, offered: true },
  plan: { label: "Plan only", icon: "map", loud: false, offered: true },
  bypassPermissions: { label: "Full access", icon: "triangle-alert", loud: true, offered: true },
  manual: { label: "Asks first", icon: "shield-check", loud: false, offered: false },
  dontAsk: { label: "Never asks", icon: "triangle-alert", loud: true, offered: false },
  auto: { label: "Auto", icon: "triangle-alert", loud: true, offered: false },
};

/** The models the picker offers, by the alias the agent takes. */
const MODELS = [
  { id: "opus", label: "Opus" },
  { id: "sonnet", label: "Sonnet" },
  { id: "haiku", label: "Haiku" },
  { id: "fable", label: "Fable" },
];

/** The reading column: wide enough for a table, narrow enough to read. */
const READ_W = 760;
const GUTTER = 24;

/** A message's words split on fences, so code is drawn in a box of its own: Herdr's Markdown draws it square. */
export function segments(text) {
  const out = [];
  const fence = /```([^\n`]*)\n([\s\S]*?)```/g;
  let last = 0;
  let m;
  while ((m = fence.exec(text))) {
    if (m.index > last) out.push({ code: false, text: text.slice(last, m.index) });
    out.push({ code: true, lang: m[1].trim(), text: m[2] });
    last = fence.lastIndex;
  }
  if (last < text.length) out.push({ code: false, text: text.slice(last) });
  return out.filter((s) => s.text.trim() !== "");
}

/**
 * The messages as the chat draws them. A turn is what you said, then everything the agent did and thought folded
 * into one row, then the one thing it answers with. A turn still running keeps its words in the fold: the answer is
 * whatever it says last.
 */
export function rowsOf(messages, { working = false, now = null } = {}) {
  const rows = [];
  let turn = null;
  const open = (at) => (turn ??= { calls: [], notes: [], from: at, to: at });
  const close = (live) => {
    if (!turn) return;
    const answer = live || !turn.notes.length ? null : turn.notes[turn.notes.length - 1];
    const aside = answer ? turn.notes.slice(0, -1) : turn.notes;
    if (turn.calls.length || aside.length) {
      rows.push({ kind: "work", calls: turn.calls, notes: aside, done: !live, secs: secondsBetween(turn.from, live ? now : turn.to) });
    }
    if (answer) rows.push({ kind: "answered", text: answer.text, at: answer.at });
    turn = null;
  };
  for (const m of messages ?? []) {
    if (m.tool) {
      open(m.at).calls.push({ ...m.tool, at: m.at });
      turn.to = m.at;
      continue;
    }
    const text = String(m.text ?? "");
    if (!text.trim()) continue;
    if (m.role === "user") {
      close(false);
      rows.push({ kind: "said", text, at: m.at });
      continue;
    }
    open(m.at).notes.push({ text, at: m.at });
    turn.to = m.at;
  }
  close(working);
  return rows.map((r, i) => ({ ...r, key: `r${i}` }));
}

/** Seconds between two RFC 3339 stamps, or null when either is missing or unreadable. */
function secondsBetween(from, to) {
  const a = Date.parse(from ?? "");
  const b = Date.parse(to ?? "");
  return Number.isNaN(a) || Number.isNaN(b) || b < a ? null : Math.round((b - a) / 1000);
}

/** "Worked for 2m 14s", or "Working for 12s" while it still is. */
export function workedFor(secs, done) {
  const verb = done ? "Worked" : "Working";
  if (secs === null) return verb;
  const mins = Math.floor(secs / 60);
  const hours = Math.floor(mins / 60);
  const span = hours ? `${hours}h ${mins % 60}m` : mins ? `${mins}m ${secs % 60}s` : `${secs}s`;
  return `${verb} for ${span}`;
}

/**
 * What the box offers under what has been typed: the agent's own slash commands after a "/", the folder's files
 * after an "@". Both are the real thing — the agent publishes its commands, the folder is read with git.
 */
export function hintsFor(typed, { commands, files } = {}) {
  const text = String(typed ?? "");
  if (text.startsWith("/") && !text.includes(" ")) {
    const want = text.slice(1).toLowerCase();
    const items = (commands ?? [])
      .filter((c) => String(c.name ?? "").toLowerCase().startsWith(want))
      .slice(0, 8)
      .map((c) => ({ label: `/${c.name}`, hint: c.description ?? "", put: `/${c.name} ` }));
    return { kind: "command", items };
  }
  const at = /(?:^|\s)@([^\s]*)$/.exec(text);
  if (!at) return { kind: null, items: [] };
  const want = at[1].toLowerCase();
  const before = text.slice(0, at.index + at[0].length - at[1].length - 1);
  const items = (files ?? [])
    .filter((f) => f.toLowerCase().includes(want))
    .slice(0, 8)
    .map((f) => ({ label: f.split("/").pop(), hint: f, put: `${before}@${f} ` }));
  return { kind: "file", items };
}

/** "claude-opus-5-5" as the picker says it: "Opus 5.5". */
export function modelName(id) {
  const parts = /^claude-([a-z]+)-(\d+)-(\d+)/.exec(String(id ?? ""));
  if (!parts) return String(id ?? "");
  return `${parts[1][0].toUpperCase()}${parts[1].slice(1)} ${parts[2]}.${parts[3]}`;
}

export default class Chat {
  init(_props, cx) {
    this.model = new AgentsModel();
    this.model.follow(cx);
    this.value = null;
    // Not null: null is a legitimate target (nothing in front), and the first turn of the loop must still subscribe.
    this.target = undefined;
    // Which turns the reader has folded open or shut by hand; the rest follow whether the turn is over.
    this.folds = new Map();
    this.sending = false;
    this.scroll = ScrollHandle ? ScrollHandle.new() : null;
    // The end is where a conversation is read from; a new message brings it back up.
    this.seen = 0;
    // True once the wheel has moved: the end is no longer where the reader is.
    this.away = false;
    // How many of the latest messages the hub sends; "Show earlier" asks for more.
    this.limit = null;
    // Files to send with the next prompt, as paths.
    this.files = [];
    this.input = InputState.new({ placeholder: "Ask anything, @ to mention files, / for commands" });
    this.input.on("submit", (_e, cx) => this.send(cx));
    // The chat follows whoever is in front: its subscription is remade when that changes.
    cx.spawn(async (cx) => {
      for (;;) {
        const f = focused();
        const want = f?.agent ?? f?.terminal ?? null;
        if (want !== this.target) this.watch(want, cx);
        await hub.sleep(cx, 400);
      }
    });
    this.watch(focused()?.agent ?? focused()?.terminal ?? null, cx);
  }

  close() {
    this.chat?.stop();
    this.model.stop();
    this.scroll?.release();
  }

  /** Reads further back: the hub sends the latest 400 until it is asked for more. */
  earlier(cx) {
    this.limit = (this.limit ?? 400) * 4;
    this.watch(this.target, cx, true);
    cx.notify();
  }

  /** The prompts waiting to be sent to this agent. */
  queued() {
    const handle = this.value?.agent?.terminal;
    return (handle && this.queues?.queues?.[handle]) || [];
  }

  /** Follows one agent's conversation, in place of the one followed before. */
  watch(terminal, cx, keep = false) {
    this.chat?.stop();
    this.target = terminal;
    if (!keep) {
      this.limit = null;
      this.files = [];
    }
    this.value = null;
    this.chat = hub.subscribe(cx, { views: [{ name: "chat", args: { terminal, frontend: "app", limit: this.limit } }, "queue", "session"], }, (m, cx) => {
      if (m.view === "chat") this.value = m.value;
      if (m.view === "queue") this.queues = m.value;
      if (m.view === "session") this.sessions = m.value;
      cx.notify();
    });
  }

  /**
   * The session cockpit drives itself, when there is one. Its words arrive as protocol events rather than from a
   * transcript, so they are already in the shape the chat draws.
   */
  session() {
    return this.sessions?.session ?? null;
  }

  /** The row the question card and the actions are about, in the shape the agents view gives. */
  row() {
    // A driven session is a row of the agents view too, keyed by its session key, and it is the one in front.
    const live = this.session();
    if (live?.key) return this.model.row(live.key) ?? null;
    const agent = this.value?.agent;
    if (!agent) return null;
    return this.model.row(agent.terminal) ?? agent;
  }

  command(c, cx) {
    if (c === "terminal") this.openTerminal();
    else if (c === "reply") this.input.focus();
    else if (c === "session") this.startSession(cx);
    else if (c === "mode") this.sessionAct("mode", {}, cx);
    else if (c.startsWith("mode:")) this.sessionAct("mode", { mode: c.slice(5) }, cx);
    else if (c.startsWith("model:")) this.sessionAct("model", { model: c.slice(6) }, cx);
    else if (c === "stop") this.sessionAct("interrupt", {}, cx);
    else if (c === "attach:file") this.pick("file", cx);
    else if (c === "attach:clipboard") this.pick("clipboard", cx);
    cx.notify();
  }

  key(e, cx) {
    const row = this.row();
    const digit = Number(e.keystroke);
    if (row?.question && digit >= 1 && digit <= 9 && !this.input.value().trim()) {
      this.model.answer(cx, row, digit);
      return true;
    }
    if (e.keystroke === "ctrl-`") {
      this.openTerminal();
      return true;
    }
    const first = this.hints().items[0];
    if (first && (e.keystroke === "tab" || e.keystroke === "enter")) {
      this.complete(first.put, cx);
      return true;
    }
    if (e.keystroke === "end") {
      this.toEnd(cx);
      return true;
    }
    return false;
  }

  /** Adds a file to the next prompt, if it is not already there. */
  attach(path, cx) {
    const clean = String(path ?? "").trim();
    if (clean && !this.files.includes(clean)) this.files.push(clean);
    cx.notify();
  }

  /**
   * A mod has no file picker, and `read_from_clipboard` gives text alone, so both of these are osascript: macOS's own
   * open panel, and the clipboard's image written to a file. Elsewhere, dropping a file on the box is the way.
   */
  pick(what, cx) {
    const mac = win()?.platform === "macos";
    if (!mac) return notify({ title: "Drop the file on the box", body: "Choosing and pasting go through macOS; a drop works anywhere.", level: "info" });
    const script =
      what === "clipboard"
        ? [
            "try",
            'set img to the clipboard as «class PNGf»',
            'set p to (POSIX path of (path to temporary items)) & "cockpit-paste-" & (do shell script "date +%s") & ".png"',
            "set f to open for access (POSIX file p) with write permission",
            "write img to f",
            "close access f",
            "return p",
            "on error",
            'return ""',
            "end try",
          ]
        : ["try", 'return POSIX path of (choose file with prompt "Send a file to the agent")', "on error", 'return ""', "end try"];
    cx.spawn(async (cx) => {
      try {
        const r = await exec("osascript", script.flatMap((line) => ["-e", line]));
        const path = String(r?.stdout ?? "").trim();
        if (path) this.attach(path, cx);
        else if (what === "clipboard") notify({ title: "No image on the clipboard", level: "info", timeout: 2500 });
      } catch (e) {
        notify({ title: "Could not read that", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** What the box offers under what has been typed, over the agent's commands and the folder's files. */
  hints() {
    const live = this.session();
    const out = hintsFor(this.input.value(), { commands: live?.commands, files: this.fileList });
    if (out.kind === "file") this.wantFiles = true;
    return out;
  }

  /** The folder's files, for "@". Read once per session: git's list where there is one, else everything not hidden. */
  loadFiles(cx) {
    const cwd = this.session()?.cwd || this.value?.session?.cwd;
    if (!cwd || this.filesFor === cwd) return;
    this.filesFor = cwd;
    cx.spawn(async (cx) => {
      try {
        const where = JSON.stringify(cwd);
        const r = await exec("sh", ["-c", `cd ${where} && { git ls-files 2>/dev/null || find . -type f -not -path '*/.*' | sed 's|^\./||'; } | head -5000`]);
        this.fileList = String(r?.stdout ?? "").split("\n").filter(Boolean);
      } catch {
        this.fileList = [];
      }
      cx.notify();
    });
  }

  /** Puts a command or a file in the box, ready for the rest of the sentence. */
  complete(put, cx) {
    this.input.set_value(put);
    this.input.focus();
    cx.notify();
  }

  /** One of the session's own actions, on the session the chat is showing. */
  sessionAct(action, arg, cx) {
    const live = this.session();
    if (!live) return;
    cx.spawn(async (cx) => {
      try {
        await hub.act("session", action, [{ key: live.key, ...arg }]);
      } catch (e) {
        notify({ title: "The session did not take it", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** Starts a session cockpit drives itself, in the folder of the space in front. */
  startSession(cx) {
    const here = spaces().find((s) => s.active) ?? spaces()[0];
    const cwd = here?.folder || this.value?.session?.cwd;
    if (!cwd) return notify({ title: "Nowhere to start it", body: "The space in front has no folder.", level: "info" });
    cx.spawn(async (cx) => {
      try {
        await hub.act("session", "start", [{ agent: "claude", cwd, mode: "default" }]);
      } catch (e) {
        notify({ title: "Could not start it", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** Back to the newest message. */
  toEnd(cx) {
    this.away = false;
    if (this.rowCount > 0) this.scroll?.scroll_to_item(this.rowCount - 1);
    cx.notify();
  }

  /** The agent's real terminal over the window, for what a reader cannot show. */
  openTerminal() {
    const agent = this.value?.agent;
    if (!agent) return;
    openOverlay({
      kind: "window",
      key: "cockpit-chat-terminal",
      width: 0.7,
      height: 0.7,
      frame: "none",
      layout: {
        type: "column",
        background: "background",
        radius: 12,
        border: { width: 1, color: "border" },
        shadow: "lg",
        children: [
          { type: "block", size: { px: 34 }, content: { mod: "cockpit", props: { app: "agent-head", place: "overlay" } } },
          { type: "block", content: { terminal: agent.address } },
        ],
      },
    });
  }

  /** ⏎ in the box: an answer when it waits on a question, else a prompt — queued while it works, sent when it is idle. */
  send(cx) {
    const live = this.session();
    if (live) {
      const text = [...this.files, this.input.value().trim()].filter(Boolean).join("\n");
      if (!text || this.sending) return cx.notify();
      this.input.set_value("");
      this.files = [];
      cx.spawn(async (cx) => {
        try {
          await hub.act("session", "send", [{ key: live.key, text }]);
        } catch (e) {
          notify({ title: "The session did not take it", body: String(e?.message ?? e), level: "warning" });
        }
        cx.notify();
      });
      return cx.notify();
    }
    const row = this.row();
    const typed = this.input.value().trim();
    const text = [...this.files, typed].filter(Boolean).join("\n");
    if (!row || !text || this.sending) return cx.notify();
    if (row.question) {
      this.model.startReply(cx, row, null);
      this.model.reply.set_value(text);
      this.model.sendReply(cx);
      this.input.set_value("");
      this.files = [];
      return cx.notify();
    }
    this.sending = true;
    cx.notify();
    cx.spawn(async (cx) => {
      try {
        await hub.act("queue", "add", [row.terminal, text]);
        if (row.status !== "working") await hub.act("queue", "send", [row.terminal, "all"]);
        this.input.set_value("");
        this.files = [];
      } catch {}
      this.sending = false;
      cx.notify();
    });
  }

  // Drawing -------------------------------------------------------------------------------------------------------

  /** Every row is laid in the same reading column, whatever it holds. */
  column(child) {
    return h_flex().w_full().px(GUTTER).justify_center().child(v_flex().w_full().max_w(READ_W).child(child));
  }

  said(row, k) {
    return this.column(
      h_flex()
        .w_full()
        .py(10)
        .justify_end()
        .child(
          v_flex()
            .max_w(READ_W * 0.78)
            .px(16)
            .py(11)
            .rounded(20)
            .bg(k.muted)
            .text_size(TEXT.base)
            .text_color(k.foreground)
            .child(TextView.markdown(`chat-${row.key}`, row.text).selectable(true).text_size(TEXT.base)),
        ),
    );
  }

  answered(row, k) {
    const parts = segments(row.text).map((s, i) =>
      s.code
        ? div()
            .w_full()
            .my(4)
            .font_family("JetBrains Mono")
            .text_size(TEXT.sm)
            .text_color(k.foreground)
            .child(TextView.markdown(`chat-${row.key}-c${i}`, `\`\`\`${s.lang}\n${s.text}\`\`\``).selectable(true).text_size(TEXT.sm))
        : TextView.markdown(`chat-${row.key}-t${i}`, s.text).selectable(true).text_size(TEXT.base).on_link_click((url) => openUrl(url)),
    );
    return this.column(v_flex().w_full().pb(10).gap(2).text_color(k.foreground).children(parts));
  }

  /** Whether a turn's fold is open: everything it did is in view while it works, out of the way once it answers. */
  folded(row) {
    return this.folds.has(row.key) ? this.folds.get(row.key) : row.done;
  }

  /**
   * What the agent did for one turn, as one line you can open. Closed it is only how long it took, with a rule under
   * it; open it is every call it made and everything it said on the way.
   */
  work(row, k) {
    const open = !this.folded(row);
    const mark = (c) => (c.ok === false ? Icon("x", { size: 12, color: k.destructive }) : c.ok === true ? Icon("check", { size: 12, color: k.success }) : Spinner({ size: 11, color: k.mutedFg }));
    const call = (c, i) =>
      h_flex()
        .h(26)
        .gap(10)
        .items_center()
        .text_size(TEXT.sm)
        .child(div().flex_none().w(78).truncate().text_color(k.foreground).child(c.name))
        .child(div().flex_1().min_w(0).truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(c.summary || "—"))
        .child(div().flex_none().child(mark(c, i)));
    const note = (n, i) =>
      div()
        .w_full()
        .py(4)
        .text_size(TEXT.sm)
        .text_color(k.mutedFg)
        .child(TextView.markdown(`chat-${row.key}-n${i}`, n.text).selectable(true).text_size(TEXT.sm));
    return this.column(
      v_flex()
        .w_full()
        .pt(12)
        .child(
          h_flex()
            .id(`work-${row.key}`)
            .gap(7)
            .items_center()
            .cursor_pointer()
            .text_color(k.mutedFg)
            .hover((s) => s.text_color(k.foreground))
            .tooltip(open ? "Fold it away" : "What it did")
            .on_click((_e, cx) => {
              this.folds.set(row.key, open);
              cx.notify();
            })
            .when(!row.done, (e) => e.child(Spinner({ size: 13, color: k.mutedFg })))
            .child(div().text_size(TEXT.base).child(workedFor(row.secs, row.done)))
            .child(Icon(open ? "chevron-down" : "chevron-right", { size: 15, color: k.dim })),
        )
        .when(open && (row.calls.length > 0 || row.notes.length > 0), (e) =>
          e.child(v_flex().w_full().pt(6).children([...row.notes.map(note), ...row.calls.map(call)])),
        )
        .when(row.done, (e) => e.child(div().w_full().h(1).mt(14).bg(k.border))),
    );
  }

  rowEl(row, k) {
    if (row.kind === "said") return this.said(row, k);
    if (row.kind === "work") return this.work(row, k);
    return this.answered(row, k);
  }

  /**
   * Where the agent's work lives: its ticket, PR, CI run and whatever else its conversation cites. Their names repeat
   * (two things on GitHub are both "GitHub"), so they are icons in a row, each saying what it is on hover.
   */
  links(k) {
    const links = this.value?.agent?.links ?? [];
    if (links.length === 0) return [];
    const one = (l, i) =>
      div()
        .id(`chat-link-${i}`)
        .size(24)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_color(k.mutedFg)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip([l.name, l.detail].filter(Boolean).join(" · ") || l.url)
        .on_click(() => openUrl(l.url))
        .child(Icon(l.icon || "link", { size: 14 }));
    return links.map(one);
  }

  /** The views this agent showed beside itself, opened in the side panel. */
  documents(k) {
    const docs = this.value?.agent?.documents ?? [];
    if (docs.length === 0) return [];
    const open = (name) => {
      openPanel("show");
      run(`cockpit.show:go:${name}`);
    };
    return [
      h_flex()
        .id("chat-docs")
        .h(24)
        .px(7)
        .gap(5)
        .items_center()
        .flex_none()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_size(11.5)
        .text_color(k.mutedFg)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip("What this agent showed beside itself")
        .on_click((_e, cx) => {
          if (docs.length === 1) open(docs[0].name);
          else openMenu(docs.map((d) => ({ label: d.title || d.name, icon: "layers", command: `doc:${d.name}` })), { width: 280 });
          cx.notify();
        })
        .child(Icon("layers", { size: 13 }))
        .child(div().child(String(docs.length))),
    ];
  }

  /** The prompts waiting their turn, under the conversation; each can be taken back. */
  queuedRows(k) {
    const queued = this.queued();
    if (queued.length === 0) return [];
    const row = this.row();
    const one = (q) =>
      h_flex()
        .w_full()
        .px(14)
        .py(8)
        .gap(8)
        .items_start()
        .rounded(16)
        .border_1()
        .border_color(k.border)
        .bg(k.canvas)
        .child(div().pt(1).child(Icon("list-ordered", { size: 13, color: k.mutedFg })))
        .child(div().flex_1().min_w(0).text_size(TEXT.base).text_color(k.mutedFg).child(q.text))
        .child(
          div()
            .id(`queued-${q.id}`)
            .size(20)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS.md)
            .cursor_pointer()
            .text_color(k.dim)
            .hover((s) => s.bg(k.accent).text_color(k.foreground))
            .tooltip("Take it back")
            .on_click((_e, cx) => {
              cx.spawn(async () => hub.act("queue", "remove", [row?.terminal, q.id]).catch(() => undefined));
              cx.notify();
            })
            .child(Icon("x", { size: 11 })),
        );
    return [
      h_flex()
        .w_full()
        .px(GUTTER)
        .pt(4)
        .justify_center()
        .child(
          v_flex()
            .w_full()
            .max_w(READ_W)
            .gap(4)
            .child(div().text_size(TEXT.xs).text_color(k.dim).child(`${queued.length} waiting to be sent`))
            .children(queued.map(one)),
        ),
    ];
  }

  /** The files riding with the next prompt. */
  fileChips(k) {
    return this.files.map((path, i) =>
      Chip(String(path).split("/").pop(), {
        id: `chat-file-${i}`,
        icon: "file-text",
        tooltip: `${path} — click to take it off`,
        onClick: (_e, cx) => {
          this.files = this.files.filter((f) => f !== path);
          cx.notify();
        },
      }),
    );
  }

  /** A word in the bottom bar that opens a menu: the permission mode, the model. */
  picker(id, { label, icon, color, tooltip, items }, k) {
    return h_flex()
      .id(id)
      .h(26)
      .px(7)
      .gap(5)
      .flex_none()
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_size(TEXT.sm)
      .text_color(color ?? k.mutedFg)
      .hover((s) => s.bg(k.accent))
      .tooltip(tooltip)
      .when(Boolean(icon), (e) => e.child(Icon(icon, { size: 14, color: color ?? k.mutedFg })))
      .child(div().child(label))
      .child(Icon("chevron-down", { size: 13, color: k.dim }))
      .on_click((_e, cx) => {
        openMenu(items, { width: 240 });
        cx.notify();
      });
  }

  /** The box: what you type, and under it what the agent may do, which model answers, and the way to send it. */
  composer(k) {
    const live = this.session();
    const row = this.row();
    const waiting = Boolean(row?.question);
    const working = live ? live.working === true : row?.status === "working";
    const empty = !this.input.value().trim();
    const hints = this.hints();
    const mode = MODES[live?.mode] ?? MODES.default;
    const seat = live ? { cwd: live.cwd, cost: live.cost } : (this.value?.session ?? {});
    const agent = this.value?.agent ?? {};
    const model = modelName(live?.model || this.value?.session?.model);
    // What the model picker says it is working with, rather than four more chips in the bar.
    const about = [
      seat.cwd ? `in ${seat.cwd}` : null,
      agent.branch ? `on ${agent.branch}` : null,
      typeof seat.context === "number" ? `${Math.round(seat.context)}% of its context` : null,
      typeof seat.cost === "number" && seat.cost > 0 ? `$${seat.cost.toFixed(2)} so far` : null,
    ].filter(Boolean).join(" · ");
    const bare = (id, icon, tooltip, onClick) =>
      div()
        .id(id)
        .size(26)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(13)
        .cursor_pointer()
        .text_color(k.dim)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip(tooltip)
        .on_click(onClick)
        .child(Icon(icon, { size: 15 }));
    return v_flex()
      .w_full()
      .px(GUTTER)
      .pb(14)
      .pt(6)
      .gap(6)
      .items_center()
      .when(hints.items.length > 0, (e) =>
        e.child(
          v_flex()
            .w_full()
            .max_w(READ_W)
            .p(4)
            .gap(1)
            .rounded(RADIUS.lg)
            .bg(k.card)
            .border_1()
            .border_color(k.border)
            .shadow_sm()
            .children(
              hints.items.map((c, i) =>
                h_flex()
                  .id(`chat-hint-${i}`)
                  .h(28)
                  .px(8)
                  .gap(8)
                  .items_center()
                  .rounded(RADIUS.md)
                  .cursor_pointer()
                  .text_size(TEXT.sm)
                  .when(i === 0, (e) => e.bg(k.selected))
                  .hover((s) => s.bg(k.accent))
                  .on_click((_e, cx) => this.complete(c.put, cx))
                  .child(div().flex_none().font_medium().text_color(k.foreground).child(c.label))
                  .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(c.hint)),
              ),
            ),
        ),
      )
      .when(this.files.length > 0, (e) =>
        e.child(h_flex().w_full().max_w(READ_W).gap(5).items_center().overflow_x_scroll().children(this.fileChips(k))),
      )
      .when(this.links(k).length + this.documents(k).length > 0, (e) =>
        e.child(h_flex().w_full().max_w(READ_W).gap(1).items_center().justify_end().children([...this.documents(k), ...this.links(k)])),
      )
      .child(
        v_flex()
          .id("chat-box")
          .w_full()
          .max_w(READ_W)
          .px(16)
          .pt(14)
          .pb(10)
          .gap(8)
          .rounded(22)
          .bg(k.card)
          .border_1()
          .border_color(k.border)
          .shadow_sm()
          .on_drop((e, cx) => {
            for (const path of e.paths ?? []) this.attach(path, cx);
            cx.notify();
          })
          .child(div().w_full().min_h(46).child(Input.new(this.input).w_full().text_size(TEXT.base).text_color(k.foreground)))
          .child(
            h_flex()
              .w_full()
              .gap(4)
              .items_center()
              .child(
                bare("chat-attach", "plus", "Send a file with it — or drop one here", (_e, cx) => {
                  openMenu(
                    [
                      { label: "Choose a file…", icon: "file-text", command: "attach:file" },
                      { label: "The image on the clipboard", icon: "file-plus", command: "attach:clipboard" },
                    ],
                    { width: 240 },
                  );
                  cx.notify();
                }),
              )
              .when(Boolean(live), (e) =>
                e.child(
                  this.picker(
                    "chat-mode",
                    {
                      label: mode.label,
                      icon: mode.icon,
                      color: mode.loud ? k.blocked : null,
                      tooltip: "What it may do without asking  ⇧⇥",
                      items: Object.entries(MODES).filter(([, m]) => m.offered).map(([id, m]) => ({ label: m.label, icon: m.icon, command: `mode:${id}` })),
                    },
                    k,
                  ),
                ),
              )
              .when(Boolean(seat.cwd), (e) =>
                e.child(
                  h_flex()
                    .id("chat-where")
                    .h(26)
                    .px(7)
                    .gap(5)
                    .flex_none()
                    .items_center()
                    .text_size(TEXT.sm)
                    .text_color(k.dim)
                    .tooltip(about)
                    .child(Icon("folder", { size: 13, color: k.dim }))
                    .child(div().max_w(160).truncate().child(String(seat.cwd).split("/").pop())),
                ),
              )
              .child(div().flex_1().min_w(0))
              .when(Boolean(model), (e) =>
                e.child(
                  live
                    ? this.picker("chat-model", { label: model, tooltip: about ? `The model it answers with — ${about}` : "The model it answers with", items: MODELS.map((m) => ({ label: m.label, command: `model:${m.id}` })) }, k)
                    : div().flex_none().px(7).text_size(TEXT.sm).text_color(k.dim).child(model),
                ),
              )
              .when(working, (e) => e.child(bare("chat-stop", "square", "Stop this turn  ⎋", (_e, cx) => this.sessionAct("interrupt", {}, cx))))
              .child(
                div()
                  .id("chat-send")
                  .size(30)
                  .flex_none()
                  .flex()
                  .items_center()
                  .justify_center()
                  .rounded(15)
                  .cursor_pointer()
                  .bg(empty ? k.muted : k.primary)
                  .text_color(empty ? k.mutedFg : k.primaryFg)
                  .hover((s) => (empty ? s.bg(k.accent).text_color(k.foreground) : s.opacity(0.85)))
                  .tooltip(waiting ? "Answer it  ⏎" : working ? "Queue it  ⏎" : "Send  ⏎")
                  .on_click((_e, cx) => this.send(cx))
                  .child(Icon("arrow-up", { size: 15 })),
              ),
          ),
      );
  }

  render(cx) {
    const k = tokens();
    const live = this.session();
    const row = this.row();
    const working = live ? live.working === true : row?.status === "working";
    if (this.wantFiles) this.loadFiles(cx);
    const rows = rowsOf(live ? live.messages : this.value?.messages, { working, now: new Date().toISOString() });
    this.rowCount = rows.length;
    // A turn still running counts its own seconds, so the row is redrawn while nothing else moves.
    if (working) cx.timer.after(1000, () => cx.notify());
    if (rows.length !== this.seen) {
      this.seen = rows.length;
      this.away = false;
      for (const wait of [16, 120, 400]) {
        if (rows.length > 0 && this.scroll) cx.timer.after(wait, () => this.scroll.scroll_to_item(rows.length - 1));
      }
    }
    const body = !live && !this.value
      ? v_flex().flex_1().min_h(0).items_center().justify_center().child(Spinner({ size: 18, color: k.mutedFg }))
      : !row
        ? v_flex().flex_1().min_h(0).items_center().justify_center().child(Empty({ icon: "message-square", title: "No agent here", description: "Start one in a tab, and its conversation shows here." }))
        : rows.length === 0
          ? v_flex().flex_1().min_h(0).items_center().justify_center().child(Empty({ icon: "message-square", title: "Nothing said yet", description: "Its transcript is empty, or it has only just started." }))
          : div()
              .id("chat-body")
              .relative()
              .flex_1()
              .min_h(0)
              .pt(8)
              .on_scroll_wheel((_e, cx) => {
                if (this.away) return;
                this.away = true;
                cx.notify();
              })
              .child(
                ((area) => (this.scroll ? area.track_scroll(this.scroll) : area))(
                  v_flex().id("chat-rows").size_full().overflow_y_scrollbar(),
                ).children(rows.map((r) => this.rowEl(r, k))),
              )
              .child(Scrollbar.vertical("chat-rows").absolute().inset_0());
    return v_flex()
      .id("chat")
      .size_full()
      .bg(k.canvas)
      .text_color(k.foreground)
      .child(body)
      .when(this.away && rows.length > 0, (e) =>
        e.child(
          h_flex()
            .w_full()
            .px(GUTTER)
            .justify_center()
            .child(
              div()
                .w_full()
                .max_w(READ_W)
                .flex()
                .justify_end()
                .child(
                  div()
                    .id("chat-to-end")
                    .size(30)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(15)
                    .cursor_pointer()
                    .bg(k.card)
                    .border_1()
                    .border_color(k.border)
                    .shadow_sm()
                    .text_color(k.mutedFg)
                    .hover((s) => s.text_color(k.foreground))
                    .tooltip("The newest message  End")
                    .on_click((_e, cx) => this.toEnd(cx))
                    .child(Icon("arrow-down", { size: 14 })),
                ),
            ),
        ),
      )
      .when(Boolean(this.value?.more), (e) =>
        e.child(
          h_flex()
            .w_full()
            .justify_center()
            .py(4)
            .child(
              div()
                .id("chat-earlier")
                .px(12)
                .py(4)
                .rounded(13)
                .cursor_pointer()
                .text_size(TEXT.xs)
                .text_color(k.mutedFg)
                .bg(k.card)
                .border_1()
                .border_color(k.border)
                .hover((s) => s.text_color(k.foreground))
                .tooltip("Read further back in this session")
                .on_click((_e, cx) => this.earlier(cx))
                .child("Show earlier"),
            ),
        ),
      )
      .children(this.queuedRows(k))
      .when(Boolean(row?.question), (e) =>
        e.child(h_flex().w_full().px(GUTTER).pt(8).justify_center().child(div().w_full().max_w(READ_W).child(QuestionCard(`chat-card-${row.terminal}-${row.question?.ask ?? row.question?.text?.length ?? ""}`, row, this.model, { wide: true })))),
      )
      .child(this.composer(k));
  }
}
