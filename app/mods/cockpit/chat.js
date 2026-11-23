// The conversation with the chat's subject (the agent in front, or the one `chat:open` named), read rather than watched:
// what you said, one folded row for everything the agent did, the answer under a rule, and the box you answer in.

import { div, image } from "gpui-kit";
import { h_flex, v_flex, Scrollbar, Input, InputState, TextView } from "gpui-base";
import * as base from "gpui-base";

// A Herdr older than 0.1.15 has no ScrollHandle: the conversation still draws, it just does not follow its own end.
const ScrollHandle = base.ScrollHandle ?? null;
import { focused, run, exec, notify, shared, setShared, appHome, home, window as win } from "herdr";
import * as hub from "./hub.js";
import { take, testKeys } from "./person.js";
import { openUrl } from "./hub.js";
import { openPanel } from "./status.js";
import { AgentsModel, QuestionCard, UndoRow, oneTimeKey, pendView } from "./agentsui.js";
import { inFront } from "./front.js";
import { CHAT_SUBJECT, CHAT_PIN, CHAT_ASK, pinChat, frontAgent } from "./screens.js";
import { followInReview } from "./review.js";
import { showWhere } from "./projects.js";
import { windowsWords } from "./usage.js";
import { Started } from "./doctor.js";
import { ChatStart } from "./chatstart.js";
import { modelName, pickChips } from "./picks.js";
import { statusMark } from "./chat-top.js";
import { hint, keyText } from "./commands.js";
import { pasteImage, thumbnail, IMAGE_LINE } from "./paste.js";
import { linkify, imagesIn, pathOf, fileAndLine, expand } from "./textlinks.js";
import { openCode } from "./code.js";
import { TOOK, boxTakesBack } from "./browser.js";
import { tokens, TEXT, RADIUS, Icon, Spinner, Chip, Empty, Card, Button, HubDown, openMenu, AgentLogo } from "./ui.js";
import { NEXT_KEYS, NextSteps, nextKey, nextShown } from "./next-steps.js";

/** The permission modes the chip wears: what it says, how it looks, and whether the menu offers it. */
const MODES = {
  default: { label: "Asks first", icon: "shield-check", loud: false, offered: true },
  acceptEdits: { label: "Edits freely", icon: "triangle-alert", loud: true, offered: true },
  plan: { label: "Plan only", icon: "map", loud: false, offered: true },
  bypassPermissions: { label: "Full access", icon: "triangle-alert", loud: true, offered: true },
  manual: { label: "Asks first", icon: "shield-check", loud: false, offered: false },
  dontAsk: { label: "Never asks", icon: "triangle-alert", loud: true, offered: false },
  auto: { label: "Auto", icon: "zap", loud: true, offered: false },
};

/** The subject of a start box the palette opened: no agent, the box is Start work. */
const BLANK = "start:";

/** The modes in which tools run without asking first: each waits for the person's yes. */
const UNASKED = ["bypassPermissions", "auto"];

/** The modes that let it do more without asking: set only from the mode menu the person opened, or a confirm. */
const ASKS_LESS = ["acceptEdits", "bypassPermissions", "auto", "dontAsk"];

/** The modes a terminal's menu offers: its ⇧⇥ goes through them, Auto only where the agent has it on. */
const TERMINAL_MODES = ["default", "acceptEdits", "plan", "auto"];

/** ⇧⇥'s cycle, as the session's own (`session::CYCLE`). */
const CYCLE = ["default", "acceptEdits", "plan"];

/** The mode ⇧⇥ moves to from `now`. */
function nextMode(now) {
  return CYCLE[(Math.max(CYCLE.indexOf(now), 0) + 1) % CYCLE.length];
}

/**
 * The choice a digit the person pressed answers, or null: with the empty box focused for a question, with its card
 * focused (Tab puts it there) for a permission; never a key held down or sent from outside the window, nor while the
 * chat was put on its agent from outside (`pointed`) and the person has not pressed another key or clicked since.
 */
export function digitAnswers(event, { question, typed = "", focused = false, card = false, pointed = false } = {}) {
  if (event?.injected || event?.is_held || pointed) return null;
  const keystroke = String(event?.keystroke ?? "");
  if (!/^[1-9]$/.test(keystroke)) return null;
  if (question?.permission ? !card : !focused || String(typed).trim()) return null;
  const n = Number(keystroke.slice(-1));
  return question?.choices?.some((c) => c.n === n) ? n : null;
}

/**
 * The chat's own keys and the flag of its key context each one needs. GPUI gives a key to the deepest binding before any
 * `on_key_down`, so the box's own Tab, ⏎ and ⎋ and the window's focus moves would take them first.
 */
const OWN_KEYS = { tab: "chat_tab", "shift-tab": "chat_cycle", escape: "chat_escape", enter: "chat_complete" };

/** The flag of the keys that pick a next step (1, 2, 3) or put them away (0), while next steps show. */
const NEXT_FLAG = "chat_next";

/** The bindings that give the chat its keys: anywhere in it, and in its box, where they come before the box's own. */
export function chatBindings() {
  const own = Object.entries(OWN_KEYS).flatMap(([keystroke, flag]) => {
    const action = `cockpit_${flag}`;
    const box = { keystroke, action, context: `CockpitChat && ${flag} > Input` };
    return keystroke === "enter" ? [box] : [{ keystroke, action, context: `CockpitChat && ${flag}` }, box];
  });
  const next = NEXT_KEYS.flatMap((keystroke) => {
    const action = `cockpit_${NEXT_FLAG}_${keystroke}`;
    return [{ keystroke, action, context: `CockpitChat && ${NEXT_FLAG}` }, { keystroke, action, context: `CockpitChat && ${NEXT_FLAG} > Input` }];
  });
  return [...own, ...next];
}

/**
 * The chat's key context: a flag for each key it has a use for now, so any other key, or one it has nothing to do with,
 * keeps doing what it does elsewhere. Behind another screen, or while it does not hold the keyboard, the chat takes none.
 */
export function chatContext({ front = true, holds = true, hints = false, question = false, live = false, closes = false, confirm = false, next = false } = {}) {
  const flags = [];
  // A popup over the chat (the palette, a menu) has the keyboard: its keys are its own.
  if (!holds) return "CockpitChat";
  if (front && !confirm && (hints || question)) flags.push(OWN_KEYS.tab);
  if (front && !confirm && live) flags.push(OWN_KEYS["shift-tab"]);
  if ((front && (hints || closes || next)) || confirm) flags.push(OWN_KEYS.escape);
  // ⏎ is the chat's while it holds the box: it completes a hint, else sends, and ⇧⏎ stays the box's new line.
  if (front && !confirm) flags.push(OWN_KEYS.enter);
  // Next steps take 1, 2, 3 and 0 only while they show, which is with the box empty: typed digits stay digits.
  if (front && !confirm && !hints && next) flags.push(NEXT_FLAG);
  return ["CockpitChat", ...flags].join(" ");
}

/** How long a pinned agent the hub listed before, or one it has not listed yet, may be missing before the chat lets go. */
const PIN_GONE_MS = 5_000;
const PIN_NEW_MS = 30_000;

/** How long after a pin the focus may still move with it, as a layout switch that came with it settles. */
const PIN_SETTLE_MS = 2_000;

/** A key that repeats this soon after its last press, with no release seen between, is held down. */
const HELD_MS = 1000;

/** How long a model, an effort level or fast mode typed into a terminal shows before its transcript says so. */
const ASKED_MS = 10 * 60_000;


/** The most of a conversation's latest messages the hub's chat view sends, however many are asked for. */
const CEILING = 20_000;

/** The most rows drawn at once: a window of the conversation slides over the rest as the reader moves. */
export const DRAW = 300;

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
  // A fence still being written is code already, up to where the words stop.
  const rest = text.slice(last);
  const open = /```([^\n`]*)\n/.exec(rest);
  if (open) {
    out.push({ code: false, text: rest.slice(0, open.index) });
    out.push({ code: true, lang: open[1].trim(), text: rest.slice(open.index + open[0].length) });
  } else if (rest) out.push({ code: false, text: rest });
  return out.filter((s) => s.text.trim() !== "");
}

/** A short hash of the start of some words: part of a row's key, which must not move when older rows come in above. */
function hashOf(text) {
  let h = 5381;
  const s = String(text ?? "");
  for (let i = 0; i < Math.min(s.length, 200); i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0;
  return (h >>> 0).toString(36);
}

/** A message's identity, the same whichever page of the conversation it arrives in. */
export function msgId(m) {
  return m.id ?? `${m.role}:${m.at}:${hashOf(m.tool ? `${m.tool.name} ${m.tool.summary}` : m.text)}`;
}

/**
 * The latest page put after what the reader holds: messages that slid out of the page stay, and the page's own
 * replace the rest (a call's outcome may have come since). A page that does not reach what is held starts over.
 */
export function joinPages(held, page) {
  if (!held?.length || !page?.length) return page;
  const first = msgId(page[0]);
  const at = held.findIndex((m) => msgId(m) === first);
  return at < 0 ? page : [...held.slice(0, at), ...page];
}

/** The question a card shows, as an id: the card's keyboard belongs to that one. */
const askOf = (q) => (q ? (q.ask ?? q.asked ?? q.text ?? null) : null);

/** The line an agent's transcript records when the person stops a turn: a marker, not something they said. */
const INTERRUPTED = /^\[Request interrupted by user\b[^\]]*\]$/;

/**
 * The messages as the chat draws them. A turn is what you said, then everything the agent did and thought folded
 * into one row, then the one thing it answers with, or a "stopped" mark when you interrupted it or it failed.
 */
export function rowsOf(messages, { working = false, now = null, stopped = false, failed = false, error = null } = {}) {
  const rows = [];
  let turn = null;
  const open = (at, first) => (turn ??= { calls: [], notes: [], steps: [], from: at, to: at, first, ids: [] });
  const close = (live, cut = false) => {
    if (!turn) return;
    const answer = live || cut || !turn.notes.length ? null : turn.notes[turn.notes.length - 1];
    const aside = answer ? turn.notes.slice(0, -1) : turn.notes;
    if (turn.calls.length || aside.length) {
      const ids = answer ? turn.ids.filter((id) => id !== answer.mid) : turn.ids;
      // Everything it did and said on the way, in the order it came: the fold's list, its last one what it does now.
      const steps = answer ? turn.steps.filter((s) => s.note !== answer) : turn.steps;
      rows.push({ kind: "work", calls: turn.calls, notes: aside, steps, done: !live, from: turn.from, to: turn.to, secs: secondsBetween(turn.from, live ? now : turn.to), ids, base: `work:${turn.from}:${hashOf(turn.first)}` });
    }
    if (answer) rows.push({ kind: "answered", text: answer.text, at: answer.at, ids: [answer.mid], base: `answered:${answer.id ?? answer.at}:${hashOf(answer.text)}` });
    turn = null;
  };
  for (const m of messages ?? []) {
    if (m.tool) {
      const call = { ...m.tool, at: m.at };
      open(m.at, `${m.tool.name} ${m.tool.summary}`).calls.push(call);
      turn.steps.push({ call });
      turn.to = m.at;
      turn.ids.push(msgId(m));
      continue;
    }
    const text = String(m.text ?? "");
    if (!text.trim()) continue;
    if (m.role === "user") {
      const marker = INTERRUPTED.test(text.trim());
      close(false, marker);
      // Sent to a session cockpit drives and not replayed by it yet: it is still on its way.
      const id = m.id ?? m.at;
      const ids = [msgId(m)];
      rows.push(marker ? { kind: "stopped", at: m.at, ids, base: `stopped:${id}` } : { kind: "said", text, at: m.at, ids, base: `said:${id}:${hashOf(text)}`, ...(m.echo === true ? { pending: true } : {}) });
      continue;
    }
    const mid = msgId(m);
    const note = { text, at: m.at, id: m.id, mid };
    open(m.at, text).notes.push(note);
    turn.steps.push({ note });
    turn.to = m.at;
    turn.ids.push(mid);
  }
  // A driven session says how its last turn ended rather than writing a marker into its messages.
  const ended = !working && (stopped || failed);
  close(working, ended);
  if (ended && rows.at(-1)?.kind !== "stopped") rows.push(failed ? { kind: "stopped", failed: true, error: error || null, ids: [], base: "stopped:end" } : { kind: "stopped", ids: [], base: "stopped:end" });
  // A row's key comes from its own messages, not its place: folds and the reading position hold when earlier ones load.
  const seen = new Map();
  return rows.map(({ base, ...r }) => {
    const n = (seen.get(base) ?? 0) + 1;
    seen.set(base, n);
    return { ...r, key: n === 1 ? base : `${base}#${n}` };
  });
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

export { modelName };

/**
 * Whether to draw the hub as down: from the first error until it is live again, so retries in between do not blink.
 * Called as each state comes, not at drawing time, when the error may already have given way to the next retry.
 */
function hubDownNow(owner, state) {
  if (state === "live") owner.hubWasDown = false;
  else if (String(state ?? "").startsWith("error")) owner.hubWasDown = true;
  return Boolean(owner.hubWasDown);
}

/** How ready a PR is to merge, as the agents view knows it: its CI, then its reviews. */
function prReadiness(row) {
  const pr = row?.pr ?? {};
  const ci = String(row?.ci?.state ?? pr.ci ?? "").toLowerCase();
  const checks = { pass: "CI passed", success: "CI passed", fail: "CI failed", failure: "CI failed", error: "CI failed", running: "CI running", pending: "CI running" }[ci] ?? "No CI yet";
  const review = { APPROVED: "approved", CHANGES_REQUESTED: "changes requested", REVIEW_REQUIRED: "needs a review" }[pr.review] ?? "no review yet";
  return `${pr.draft ? "Draft · " : ""}${checks} · ${review}`;
}

/** How high an image an answer names is drawn under it. */
const PICTURE_H = 220;

export default class Chat {
  init(_props, cx) {
    this.model = new AgentsModel("chat");
    this.model.follow(cx);
    // The chat's own keyboard: a digit answers only from a key the person pressed here.
    this.handle = cx.focus_handle();
    // The question card's own keyboard: Tab puts it there, and a permission's digits answer only from it.
    this.cardFocus = cx.focus_handle();
    // The confirm's Cancel holds the keyboard while it is up, so ⏎ never confirms by itself.
    this.cancelFocus = cx.focus_handle();
    this.boxFocused = false;
    // When each of the chat's own keys last went down, until it is released: an action does not say it is held.
    this.down = new Map();
    // The keymap is the app's: binding again after a reload or another layout only repeats the same entries.
    try {
      cx.bind_keys(chatBindings());
    } catch {}
    // The chat was put on its agent, or the box focused, by a window command: digits answer nothing until the person
    // presses another key or clicks in the chat.
    this.pointed = false;
    // What a window command typed in the empty box, {target, text}: ⏎ does not start it, a click on ↑ does.
    this.typedOutside = null;
    // Full access waits for a yes: {mode} while the confirm is up.
    this.confirm = null;
    this.value = null;
    // Not null: null is a legitimate target (nothing in front), and the first turn of the loop must still subscribe.
    this.target = undefined;
    // Which turns the reader has folded open or shut by hand; the rest follow whether the turn is over.
    this.folds = new Map();
    this.scroll = ScrollHandle ? ScrollHandle.new() : null;
    // Pinned, the newest row stays in view as more come; else the reader stays put and what came after `seenMsg` is
    // counted, `top` saying how far down they are from the wheel and the rows' measured heights.
    this.pinned = true;
    this.top = 0;
    this.viewH = 0;
    this.heights = new Map();
    this.seenMsg = null;
    // A conversation's messages as the reader holds them while reading back: what slid out of the hub's latest page
    // stays, so nothing above the reader goes.
    this.kept = null;
    this.rows = [];
    this.all = [];
    // The drawn window's first row while the reader is away from the end ({key}), and the row a load from the hub
    // centres it on once it comes: null, the window holds the latest rows.
    this.windowTop = null;
    this.windowAround = null;
    // Older rows coming in above: the row that stays where it was, {anchor}.
    this.restoring = null;
    this.loadingEarlier = false;
    // A driven session's older pages, fetched as the reader reaches the top: {key, messages, from, to, more}.
    this.older = null;
    // How many of the latest messages the hub sends; "Show earlier" asks for more.
    this.limit = null;
    // Files to send with the next prompt, as paths, and why the last image pasted was not attached ({why, detail}).
    this.files = [];
    this.pasteNote = null;
    // The copies drawn of image files, by path: null while copying, {rel, w, h}, or false when the image is gone.
    this.thumbs = new Map();
    // What was typed for each subject, kept when another one comes in front: {text, files}.
    this.drafts = new Map();
    // Each subject's last send, {state: "sending" | "failed" | "unconfirmed" | "queued", why?, note?}, and the queued
    // prompt its box is editing, {id, draft}: a send that comes back lands on the subject it went to.
    this.sends = new Map();
    this.edits = new Map();
    // What was typed into a terminal for a pick, by terminal, until its transcript shows it: {setting: value, at}.
    this.typedPicks = new Map();
    // Whether the hub answers: "connecting", "live" or "error: …", and a start asked of it from here.
    this.hubState = "connecting";
    this.starting = false;
    this.started = new Started();
    // With no agent to read, the box starts work (chatstart.js).
    this.starter = new ChatStart();
    // Several lines: ⏎ sends (the chat's own binding), ⇧⏎ starts a new line; the box grows to eight of them.
    // A Herdr older than its multi-line box gets the one-line box rather than no chat at all.
    const placeholder = "Ask anything, @ to mention files, / for commands";
    // The state and the widget come together: a Herdr with only one of them gets the one-line box.
    this.multiLine = Boolean(base.Textarea && base.TextareaState);
    this.input = this.multiLine ? base.TextareaState.new({ placeholder, rows: 1 }) : InputState.new({ placeholder });
    this.input.set_auto_grow?.(1, 8);
    // The multi-line box says "submit" for ⇧⏎ too, after its new line: ⏎ sends through the chat's own binding instead.
    this.input.on("submit", (_e, cx) => this.multiLine || this.send(cx, "key"));
    // ⌘V with no text on the clipboard: its image, if any, rides with the prompt as the + menu's does.
    this.input.on("paste_other", (_e, cx) => this.pasteKey(cx));
    // Where the keyboard is decides which keys the chat binds (`chatContext`): each move draws them again.
    this.input.on("focus", (_e, cx) => ((this.boxFocused = true), cx?.notify?.()));
    this.input.on("blur", (_e, cx) => ((this.boxFocused = false), cx?.notify?.()));
    // Emptied, the box holds the person's words again. Each change draws the hints again, and with them the keys.
    this.input.on("change", (_e, cx) => {
      if (!this.input.value().trim()) this.typedOutside = null;
      cx.notify();
    });
    // The chat follows its subject: its subscriptions are remade when that changes.
    cx.spawn(async (cx) => {
      for (;;) {
        const want = this.wanted();
        if (want !== this.target) this.watch(want, cx);
        // Whether it is a session cockpit drives comes with the agents view: its session is followed once that says so.
        else if (this.driven(want) !== this.watchedDriven) this.watch(want, cx, true);
        this.inFrontNow(cx);
        this.asked(cx);
        this.keepBox();
        // With no agent to read, the box is Start work: what it holds is planned on this clock.
        if (this.value && !this.row() && !this.session()) this.starter.sync(this.input.value(), cx);
        await hub.sleep(cx, 400);
      }
    });
    this.watch(this.wanted(), cx);
  }

  /** The box takes the keyboard back from the browser the side panel shows beside the agent: the chat keeps it. */
  keepBox() {
    const took = shared(TOOK);
    if (!boxTakesBack({ took, answered: this.tookAnswered, now: Date.now() })) return;
    this.tookAnswered = took;
    this.input.focus();
  }

  /** The subject in front's last send. */
  get sent() {
    return this.sends?.get(this.target) ?? null;
  }

  /** The queued prompt the subject in front's box is editing: its id. */
  get editing() {
    return this.edits?.get(this.target)?.id ?? null;
  }

  /**
   * The chat's subject: the agent `chat:open` named, until another one comes in front; else the agent in view (the
   * focused pane's, else the one in the tab in front). A tab no agent runs in has none: the chat is empty there.
   */
  wanted() {
    const here = this.inView();
    const focus = focused();
    let pinned = shared(CHAT_PIN);
    const navigated = Boolean(pinned && ((pinned.tab && focus?.tab && pinned.tab !== focus.tab) || (pinned.space && focus?.space && pinned.space !== focus.space)));
    // The focus settling just after the pin (a layout switch that came with it) is where the pin was made.
    if (pinned && !navigated && pinned.focus !== here && Date.now() - (pinned.at ?? 0) < PIN_SETTLE_MS) {
      pinned = { ...pinned, focus: here };
      setShared(CHAT_PIN, pinned);
    }
    // Another tab, space or agent ends the pin; focus changes inside its tab and a hub restart keep it.
    const moved = navigated || (Boolean(pinned) && Boolean(here) && pinned.focus !== here);
    if (pinned?.key === BLANK && !moved) return BLANK;
    // An agent the chat was pinned to that the agents view no longer lists (a session that ended, a terminal closed)
    // goes, and the chat follows the agent in front again.
    if (pinned?.key && !moved && !this.pinGone(pinned.key)) return pinned.key;
    if (pinned) pinChat(null);
    return here;
  }

  /** The agent in view (`frontAgent`), Herdr's agents and the ones the agents view lists alike. */
  inView() {
    const rows = (this.model?.rows ?? []).map((r) => ({ address: String(r.address ?? r.terminal ?? "").replace(/^app:/, ""), tab: r.tab?.id ?? r.tab ?? null }));
    return frontAgent(focused(), rows);
  }

  /**
   * Whether the pinned agent is gone: the hub, answering, has not listed it for a while. A hub that is down, or back but
   * not listing it yet (a session just started, or one a restart is bringing back paused), keeps the pin.
   */
  pinGone(key, now = Date.now()) {
    const listed = Boolean(this.model.value) && !this.model.down && Boolean(this.model.row(key));
    if (listed || this.model.down || !this.model.value) {
      if (listed) this.pinSeen = key;
      this.pinMissing = null;
      return false;
    }
    if (this.pinMissing?.key !== key) this.pinMissing = { key, since: now };
    // One the hub listed before goes soon after it stops listing it; one it has not listed yet gets longer to show up.
    return now - this.pinMissing.since > (this.pinSeen === key ? PIN_GONE_MS : PIN_NEW_MS);
  }

  /** Follows this agent (a terminal's address or a driven session's key) until another one comes in front. */
  open(key, cx) {
    pinChat(key);
    if (key !== this.target) this.watch(key, cx);
  }

  /** Whether the agents view lists this key as a session cockpit drives: null before it says. */
  driven(key) {
    if (!key) return false;
    // While the hub is down its rows are Herdr's own, which know no session: the subject stays what it was.
    if (this.model.down && key === this.target && this.watchedDriven !== undefined) return this.watchedDriven;
    const row = this.model.row(key);
    return row ? row.driven === true : this.model.value ? false : null;
  }

  /** A question the Cockpit layout left for this session: put once the chat is on it. */
  asked(cx) {
    const ask = shared(CHAT_ASK);
    if (!ask?.key) return;
    // A question nobody came to the chat for within a minute is not put later.
    if (Date.now() - (ask.at ?? 0) > 60_000) return setShared(CHAT_ASK, null);
    if (ask.key !== this.target) return;
    setShared(CHAT_ASK, null);
    if (ask.what === "handover") this.handing = { key: ask.key };
    cx.notify();
  }

  /** The review panel follows the chat's subject only while the chat is the screen in front. */
  inFrontNow(cx) {
    const front = inFront("chat");
    if (front === this.front) return;
    this.front = front;
    if (front) {
      this.reviewed = null;
      this.follows(cx);
    } else {
      followInReview(null);
      showWhere(null);
    }
  }

  close() {
    this.watchGeneration = (this.watchGeneration ?? 0) + 1;
    this.chat?.stop();
    this.starter.close();
    this.model.stop();
    this.scroll?.release();
    followInReview(null);
    showWhere(null);
  }

  /** Whether older messages come before the first one shown. */
  hasEarlier() {
    const live = this.session();
    // The hub sends at most CEILING of a conversation's latest messages: past it, nothing earlier is offered.
    if (!live) return Boolean(this.value?.more) && (this.limit ?? 400) < CEILING;
    return Boolean(this.older?.key === live.key ? this.older.more : live.cursor?.more);
  }

  /**
   * Reads further back, once at a time, keeping the first row shown where it is: a conversation asks the hub for more
   * of its latest messages, a driven session for the page before the oldest it has.
   */
  earlier(cx) {
    const first = this.topMessage();
    if (this.loadingEarlier || this.restoring || !first) return;
    const start = this.all.findIndex((r) => r.key === this.rows[0]?.key);
    // Rows read already but not drawn come first: the window slides up over them.
    if (start > 0) {
      this.windowTop = this.all[Math.max(0, start - DRAW / 2)].ids?.[0] ?? null;
      this.restoring = { anchor: first };
      this.pinned = false;
      this.seenMsg ??= this.lastMessage();
      return cx.notify();
    }
    if (!this.hasEarlier()) return;
    this.loadingEarlier = true;
    this.restoring = { anchor: first };
    this.windowAround = first;
    // Reading back: the newest rows no longer pull the view down.
    this.pinned = false;
    this.seenMsg ??= this.lastMessage();
    const live = this.session();
    if (!live) {
      this.limit = Math.min((this.limit ?? 400) * 4, CEILING);
      this.watch(this.target, cx, true);
      return cx.notify();
    }
    const key = live.key;
    const had = this.older?.key === key ? this.older : null;
    cx.spawn(async (cx) => {
      try {
        const page = (await hub.view("session", { key, before: had ? had.from : live.cursor?.from, limit: 400 }))?.session;
        if (page?.key === key && this.session()?.key === key) {
          const c = page.cursor ?? {};
          this.older = { key, messages: [...(page.messages ?? []), ...(had?.messages ?? [])], from: c.from ?? 0, to: had ? had.to : c.to, more: Boolean(c.more) };
        } else this.restoring = null;
      } catch {
        this.restoring = null;
      }
      this.loadingEarlier = false;
      cx.notify();
    });
    cx.notify();
  }

  /**
   * The live page of a driven session moves on as messages come; what slid out of it between the older pages and it
   * is fetched, so nothing goes missing in the middle.
   */
  fillGap(cx) {
    const live = this.session();
    const o = this.older;
    const from = live?.cursor?.from;
    if (!o || o.key !== live?.key || this.filling || typeof from !== "number" || from <= o.to) return;
    this.filling = true;
    cx.spawn(async (cx) => {
      try {
        const page = (await hub.view("session", { key: o.key, before: from, limit: from - o.to }))?.session;
        if (page && this.older === o) this.older = { ...o, messages: [...o.messages, ...(page.messages ?? [])], to: page.cursor?.to ?? from };
      } catch {}
      this.filling = false;
      // The live page may have moved again while this was on its way.
      this.fillGap(cx);
      cx.notify();
    });
  }

  /**
   * A driven session's new live page: what slid out of the last one goes after the older pages, so no message goes
   * missing between them; a session that holds fewer messages than they say (a restart) starts them over.
   */
  slide(next) {
    const prev = this.sessions?.session;
    const o = this.older;
    if (!o || !next || o.key !== next.key) return;
    const total = next.cursor?.total;
    if (typeof total === "number" && total < o.to) {
      this.older = null;
      return;
    }
    const pf = prev?.key === next.key ? prev.cursor?.from : null;
    const nf = next.cursor?.from;
    if (typeof pf !== "number" || typeof nf !== "number" || nf <= o.to || pf > o.to) return;
    const end = Math.min(nf, prev.cursor?.to ?? pf + (prev.messages?.length ?? 0));
    const add = (prev.messages ?? []).slice(o.to - pf, end - pf);
    if (add.length) this.older = { ...o, messages: [...o.messages, ...add], to: o.to + add.length };
  }

  /** The messages drawn: a conversation's, or a driven session's older pages then its live one. */
  messagesOf(live) {
    if (!live) return this.kept ?? this.value?.messages;
    const o = this.older?.key === live.key ? this.older : null;
    if (!o) return live.messages;
    const memo = this.joined;
    if (memo?.older === o && memo.live === live.messages) return memo.all;
    // Older pages end where the live one starts; any overlap is the live page's.
    const from = live.cursor?.from ?? o.to;
    const all = [...o.messages.slice(0, Math.max(0, Math.min(o.messages.length, from - o.from))), ...(live.messages ?? [])];
    this.joined = { older: o, live: live.messages, all };
    return all;
  }

  /** The prompts waiting to be sent to this agent. */
  queued() {
    const handle = this.subjectHandle();
    return (handle && this.queues?.queues?.[handle]) || [];
  }

  /**
   * Follows one subject in place of the one before: its conversation, its queue, and its session when cockpit drives
   * it. What was typed for the last one is kept for when it comes back; the side panel's review follows along.
   */
  watch(subject, cx, keep = false) {
    const generation = this.watchGeneration = (this.watchGeneration ?? 0) + 1;
    this.chat?.stop();
    if (!keep) {
      if (this.target !== undefined && this.target !== null) {
        // An edit left half done gives the box back what was being written before it.
        const edit = this.edits.get(this.target);
        this.edits.delete(this.target);
        this.drafts.set(this.target, edit ? edit.draft : { text: this.input.value(), files: this.files });
      }
      const draft = this.drafts.get(subject);
      this.input.set_value(draft?.text ?? "");
      this.files = draft?.files ?? [];
      this.limit = null;
      this.reviewed = null;
      this.value = null;
      this.sessions = null;
      this.older = null;
      this.restoring = null;
      this.loadingEarlier = false;
      this.heights = new Map();
      this.pinned = true;
      this.lost = false;
      this.seenMsg = null;
      this.kept = null;
      this.folds = new Map();
      this.cardAsk = null;
      this.pasteNote = null;
      this.ending = null;
      this.windowTop = null;
      this.windowAround = null;
    }
    this.target = subject;
    // The blank subject is a start box of its own, with no agent to follow until it starts one.
    if (subject === BLANK) {
      setShared(CHAT_SUBJECT, null);
      this.value = { agent: null, messages: [] };
      this.chat = null;
      return;
    }
    setShared(CHAT_SUBJECT, subject);
    this.follows(cx);
    const views = [{ name: "chat", args: { terminal: subject, frontend: "app", limit: this.limit } }, "queue"];
    this.watchedDriven = this.driven(subject);
    if (this.watchedDriven) views.push({ name: "session", args: { key: subject } });
    this.chat = hub.subscribe(
      cx,
      { views },
      (m, cx) => {
        if (this.watchGeneration !== generation) return;
        if (m.view === "chat") {
          // Reading back, the messages held stay, and the page's new ones go after them.
          this.kept = !this.pinned && this.value ? joinPages(this.kept ?? this.value.messages, m.value?.messages ?? []) : null;
          this.value = m.value;
          this.loadingEarlier = false;
        }
        if (m.view === "queue") this.queues = m.value;
        if (m.view === "session") {
          this.slide(m.value?.session);
          this.sessions = m.value;
          this.fillGap(cx);
        }
        this.follows(cx);
        cx.notify();
      },
      (state, cx) => {
        if (this.watchGeneration !== generation) return;
        this.hubState = state;
        if (String(state).startsWith("error")) this.lastHubError = state;
        hubDownNow(this, state);
        if (state === "live") this.starting = false;
        cx.notify();
      },
    );
  }

  /** Tells the review panel which agent and folder the chat is on, once per change of either. */
  follows(_cx) {
    const handle = this.subjectHandle();
    const cwd = this.session()?.cwd || this.value?.session?.cwd || this.value?.agent?.cwd || null;
    // The repositories its agent works in, the latest first: Files, Changes and Review show the one it changed last.
    const repos = this.value?.session?.repos ?? [];
    const said = `${handle} ${cwd} ${repos.map((r) => r.path).join(" ")}`;
    if (!handle || said === this.reviewed || this.front === false) return;
    this.reviewed = said;
    showWhere(cwd ? { cwd, repos } : null);
    followInReview(handle, repos[0]?.path ?? cwd);
  }

  /** The subject's handle, as the hub's actions take it: `app:t3`, or a driven session's key. */
  subjectHandle() {
    const t = this.target;
    if (!t) return null;
    return this.value?.agent?.terminal ?? (this.driven(t) || t.startsWith("app:") ? t : `app:${t}`);
  }

  /**
   * The session cockpit drives, when it is the subject. Its words arrive as protocol events rather than from a
   * transcript, so they are already in the shape the chat draws.
   */
  session() {
    // Only a driven subject's session is followed (`watch`), so there is none for a terminal.
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

  /**
   * A window command, `key` the one-time key a menu, the palette or a click sent it with (person.js). Without one it only
   * moves the chat and its box around: it stops, sets, attaches and starts nothing, and arms no digit.
   */
  command(c, cx, key = null) {
    if (c.startsWith("chat:open:")) {
      this.open(c.slice(10), cx);
      this.input.focus();
      this.pointed = !take(c, key);
    }
    else if (c === "reply") (this.input.focus(), (this.pointed = !take(c, key)));
    else if (c === "chat:earlier") this.earlier(cx);
    else if (c === "chat:handover") this.handover(cx);
    else if (c === "chat:end") this.toEnd(cx);
    // The palette's ⇧⇥ and ⎋ entries reach the session the chat shows, and only while the chat is in front; without
    // the palette's key they only close the confirm. ⇧⇥ moves into a mode that asks less only through the confirm.
    else if (c === "chat:mode" || c === "mode") this.ownKey(cx) && (this.confirm ? (this.confirm = null) : take(c, key) && this.cycleMode(cx));
    else if (c === "chat:stop" || c === "stop") this.ownKey(cx) && (this.confirm ? (this.confirm = null) : take(c, key) && this.sessionAct("interrupt", {}, cx));
    else if (c.startsWith("mode:")) {
      const [mode, modeKey = null] = c.slice(5).split(":");
      this.askMode(mode, modeKey, cx);
    }
    else if (c.startsWith("pick:")) take(c, key) && this.choose(c.slice(5), cx);
    else if (c === "attach:file") this.pick("file", cx);
    else if (c === "attach:clipboard") take(c, key) && this.pick("clipboard", cx);
    // The empty box: a window command may type in it, never start it; ⏎ leaves what it typed for a click on ↑.
    else if (c.startsWith("chat:start:type:")) {
      if (!this.row() && !this.session()) {
        this.input.set_value(c.slice("chat:start:type:".length));
        this.typedOutside = { target: this.target ?? null, text: this.input.value() };
      }
    } else if (c.startsWith("chat:start:new:")) {
      // A start box of its own, holding what the palette had; the person's pick starts with ⏎, a bare command does not.
      const text = c.slice("chat:start:new:".length);
      this.open(BLANK, cx);
      this.input.set_value(text);
      this.input.focus();
      this.typedOutside = take(c, key) || !text.trim() ? null : { target: BLANK, text };
    } else if (c === "chat:start:send") {
      const start = (cx) => !this.row() && !this.session() && (this.send(cx, "command"), cx.notify());
      if (take(c, key)) start(cx);
      else cx.spawn(async (cx) => (await testKeys()) && start(cx));
    } else if (c.startsWith("chatstart:") && take(c, key)) this.starter.command(c, cx);
    else if (c.startsWith("chat:merge:") && take(c, key)) this.askMerge(Number(c.slice("chat:merge:".length)), cx);
    else if (c === "chat:pr:open") this.row()?.pr?.url && openUrl(this.row().pr.url);
    cx.notify();
  }

  /** Whether a layout key is the chat's to act on: it is the screen in front, showing a session cockpit drives. */
  ownKey() {
    return inFront("chat") && Boolean(this.session());
  }

  /** ⇧⇥: the next mode of the cycle, through the confirm when it asks less than the one before. */
  cycleMode(cx) {
    const next = nextMode(this.session()?.mode ?? "default");
    this.askMode(next, null, cx);
  }

  /**
   * The mode menu's items, keyed for this opening of it: a command naming no key, or an older one, picks nothing. A
   * terminal's offers the modes its ⇧⇥ goes through.
   */
  modeItems(terminal = false) {
    this.modeKey = oneTimeKey();
    const ids = terminal ? TERMINAL_MODES : Object.keys(MODES).filter((id) => MODES[id].offered);
    return ids.map((id) => ({ label: MODES[id].label, icon: MODES[id].icon, command: `mode:${id}:${this.modeKey}` }));
  }

  /**
   * A mode, picked from the menu the person opened (`key` its one-time key) or asked for otherwise. One that runs tools
   * without asking is set only once the person confirms it, for this session; one that asks less is set at once only
   * from that menu, else through the same confirm.
   */
  askMode(mode, key, cx) {
    const live = this.session();
    // An agent in a terminal changes its mode by its own keys, typed there by the hub.
    const row = live ? null : this.value?.session?.switch?.mode ? this.row() : null;
    const fromMenu = Boolean(key) && key === this.modeKey;
    this.modeKey = null;
    if (!live && !row) return cx.notify();
    if (UNASKED.includes(mode) || (ASKS_LESS.includes(mode) && !fromMenu)) {
      const first = live ? (live.title ?? live.messages?.find((m) => m.role === "user")?.text?.split("\n")[0] ?? "") : (row.title ?? "");
      const asked = first.length > 60 ? `${first.slice(0, 59).trimEnd()}…` : first;
      const cwd = live ? live.cwd : row.cwd;
      this.confirm = { mode, key: live ? live.key : row.terminal, ...(live ? {} : { terminal: true }), title: asked || "This session", folder: String(cwd ?? "").split("/").pop() };
      this.cancelFocus.focus();
    } else if (live) this.sessionAct("mode", { mode }, cx);
    else this.terminalAct("mode", { mode }, cx, row.terminal);
    cx.notify();
  }

  /** The confirm's answer: a yes sets the mode on the session or terminal it was asked for, and only that one. */
  confirmed(yes, cx) {
    const c = this.confirm;
    this.confirm = null;
    if (yes && c?.terminal) this.terminalAct("mode", { mode: c.mode }, cx, c.key);
    else if (yes && c) this.sessionAct("mode", { mode: c.mode, confirm: true }, cx, c.key);
    cx.notify();
  }

  /** One of the agents view's actions typed into a terminal agent (its mode, its focus view), its refusal said. */
  terminalAct(action, arg, cx, terminal = this.row()?.terminal) {
    if (!terminal) return;
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("agents", action, [{ terminal, ...arg }]);
        if (r?.error) notify({ title: "Its terminal did not take it", body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 5000 });
      } catch (e) {
        notify({ title: "Its terminal did not take it", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /**
   * A key with the keyboard in the chat: Tab and ⏎ complete a `/` or `@` hint, else Tab puts the keyboard on the question
   * card, whose digits answer only its question (`digitAnswers`), End goes to the newest message and ⇧⇥ moves a driven
   * session to its next mode. ⎋ closes what is open first (the hints, a question under the header, a confirm) and stops
   * the turn only when nothing is, while a menu or the palette keeps its own keys.
   */
  pressed(e, cx) {
    const key = e.keystroke;
    // A chat hidden behind another screen keeps its copy, and may keep the keyboard: its keys answer nothing then.
    if (!this.confirm && !inFront("chat")) return false;
    const hints = this.hints().items;
    if (key === "escape") return this.escape(hints, cx);
    if (this.confirm) return false;
    if ((key === "tab" || key === "enter") && hints.length > 0) {
      this.complete(hints[0].put, cx);
      return true;
    }
    if (key === "end" && (!this.boxFocused || !this.input.value().trim())) {
      this.toEnd(cx);
      return true;
    }
    // From outside the window, a key only moves around: it answers, stops and changes nothing.
    if (e.injected) return false;
    // A next step only fills the box, for the person to read and send: 1, 2, 3 pick one, 0 puts them away.
    const offer = this.nextOffer();
    const pick = nextKey(key, offer, { held: e.is_held, pointed: this.pointed });
    if (pick?.fill) return this.fillNext(pick.fill, cx);
    if (pick?.dismiss) return this.dismissNext(offer);
    // The person's own key other than a digit: the chat is theirs again, whoever put it on this agent.
    if (!/^[1-9]$/.test(key)) this.pointed = false;
    const row = this.row();
    const q = row ? this.model.shown(row) : null;
    if (key === "tab" && q) {
      this.cardAsk = askOf(q);
      this.cardFocus.focus();
      return true;
    }
    const live = this.session();
    if (key === "shift-tab" && live) {
      if (!e.is_held) this.sessionAct("mode", {}, cx);
      return true;
    }
    // The card holds the keyboard for the question it was put on: a newer one starts without it.
    const card = this.cardFocus.is_focused() && this.cardAsk !== null && this.cardAsk === askOf(q);
    const n = digitAnswers(e, { question: q, typed: this.input.value(), focused: this.boxFocused || this.handle.is_focused(), card, pointed: this.pointed });
    if (n === null) return false;
    this.model.answer(cx, row, n, "key");
    return true;
  }

  /** ⎋: the hints, then a confirm, then a question under the header, then the next steps, and only then the turn, stopped. */
  escape(hints, cx) {
    if (hints.length > 0) {
      this.hintsOff = this.input.value();
      return true;
    }
    if (this.confirm) {
      this.confirm = null;
      return true;
    }
    if (this.ending || this.handing) {
      this.ending = null;
      this.handing = null;
      return true;
    }
    const offer = this.nextOffer();
    if (offer) return this.dismissNext(offer);
    if (this.stoppable()) {
      this.stop(cx);
      return true;
    }
    return false;
  }

  /** The next steps the strip shows now, or null (`nextShown`): none while a confirm or a question under the header is up. */
  nextOffer() {
    if (this.confirm || this.ending || this.handing) return null;
    const live = this.session();
    const row = this.row();
    return nextShown(this.value?.next, {
      working: live ? live.working === true : row?.status === "working",
      question: Boolean(row?.question),
      typed: this.input?.value?.() ?? "",
      sending: this.sent?.state === "sending",
      dismissed: this.nextOff ?? null,
    });
  }

  /** Whether 1, 2, 3 and 0 pick next steps now; while a window command pointed the chat they type, as anywhere in the box. */
  nextKeys() {
    return this.nextOffer()?.state === "ready" && !this.pointed;
  }

  /** Puts a next step in the box, focused for the person to edit and send: nothing is sent from here. */
  fillNext(prompt, cx) {
    this.input.set_value(prompt);
    this.input.focus();
    cx.notify();
    return true;
  }

  /** Puts this turn's next steps away; the next turn brings its own. */
  dismissNext(offer) {
    this.nextOff = offer?.turn ?? null;
    return true;
  }

  /** Whether the turn can be stopped from here: a driven session's while it works, a terminal's where its agent says how. */
  stoppable() {
    const live = this.session();
    return live ? live.working === true : this.row()?.stoppable === true;
  }

  /** A key from outside the window (a test's): the same path as a pressed one, which answers and changes nothing for it. */
  key(e, cx) {
    return this.pressed(e, cx);
  }

  /**
   * One of the chat's own keys, pressed for real (its binding): the same path as `on_key_down`, a repeat before its release
   * counted as held. Its context said the chat had a use for it; when that changed since, it does what it does elsewhere.
   */
  action(keystroke, cx) {
    const key = keystroke.split("-").pop();
    const now = Date.now();
    const held = this.down.has(keystroke) && now - this.down.get(keystroke) < HELD_MS;
    this.down.set(keystroke, now);
    if (this.pressed({ keystroke, key, is_held: held }, cx)) return cx.notify();
    if (keystroke === "tab") globalThis.window?.focus_next?.();
    else if (keystroke === "shift-tab") globalThis.window?.focus_prev?.();
    // A held ⏎ never sends: only a press does.
    else if (keystroke === "enter" && inFront("chat") && !this.confirm && !held) this.send(cx, "key");
  }

  /** A key let go in the chat: its next press, with any modifier, is a press again. */
  released(e) {
    for (const keystroke of this.down.keys()) if (keystroke.split("-").pop() === e?.key) this.down.delete(keystroke);
    if (e?.key === "v") this.pasteDown = null;
  }


  /** Adds a file to the next prompt, if it is not already there. */
  attach(path, cx, subject = this.target) {
    const clean = String(path ?? "").trim();
    if (subject !== this.target) {
      if (clean) {
        const draft = this.drafts.get(subject) ?? { text: "", files: [] };
        this.drafts.set(subject, { ...draft, files: [...new Set([...(draft.files ?? []), clean])] });
      }
      return cx.notify();
    }
    if (clean && !this.files.includes(clean)) this.files.push(clean);
    if (clean) this.pasteNote = null;
    cx.notify();
  }

  /** ⌘V with no text on the clipboard: its image, once per press; its repeats while held paste nothing more. */
  pasteKey(cx) {
    const now = Date.now();
    const held = this.pasteDown != null && now - this.pasteDown < HELD_MS;
    this.pasteDown = now;
    if (!held) this.pasteFromClipboard(cx, false);
  }

  /**
   * The clipboard's image as a file of the next prompt (paste.js), from ⌘V or the + menu. Nothing to paste does nothing
   * for ⌘V and says so when asked from the menu; a paste that fails says why above the box.
   */
  pasteFromClipboard(cx, asked) {
    const subject = this.target;
    cx.spawn(async (cx) => {
      const got = await pasteImage();
      if (this.target === subject) this.pasteNote = got.error ? { why: got.error, detail: got.detail } : got.none && asked ? { why: "the clipboard holds no image", detail: "" } : null;
      if (got.path) this.attach(got.path, cx, subject);
      cx.notify();
    });
  }

  /**
   * A mod has no file picker, so choosing one is osascript, macOS's own open panel; elsewhere, dropping a file on the box
   * is the way. The clipboard's image is read wherever a tool for it is there.
   */
  pick(what, cx) {
    if (what === "clipboard") return this.pasteFromClipboard(cx, true);
    const mac = win()?.platform === "macos";
    if (!mac) return notify({ title: "Drop the file on the box", body: "Choosing goes through macOS; a drop works anywhere.", level: "info" });
    const script = ["try", 'return POSIX path of (choose file with prompt "Send a file to the agent")', "on error", 'return ""', "end try"];
    const subject = this.target;
    cx.spawn(async (cx) => {
      try {
        const r = await exec("osascript", script.flatMap((line) => ["-e", line]));
        const path = String(r?.stdout ?? "").trim();
        if (path) this.attach(path, cx, subject);
      } catch (e) {
        notify({ title: "Could not read that", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** Above the box, why an image was not attached, with a way to put the note away. */
  pasteLine(k) {
    return h_flex()
      .id("chat-paste-note")
      .w_full()
      .max_w(READ_W)
      .px(6)
      .gap(6)
      .items_center()
      .text_size(TEXT.sm)
      .text_color(k.foreground)
      .child(Icon("circle-x", { size: 13 }))
      .child(div().flex_1().min_w(0).truncate().tooltip(this.pasteNote.detail || this.pasteNote.why).child(`Image not attached — ${this.pasteNote.why}`))
      .child(Button("chat-paste-ok", { label: "Dismiss", variant: "ghost", size: "sm", onClick: (_e, cx) => ((this.pasteNote = null), cx.notify()) }));
  }

  /** What the box offers under what has been typed, over the agent's commands and the folder's files. */
  hints() {
    const live = this.session();
    const typed = this.input.value();
    // ⎋ put them away for what is typed now; another key brings them back.
    if (this.hintsOff !== null && this.hintsOff !== undefined) {
      if (this.hintsOff === typed) return { kind: null, items: [] };
      this.hintsOff = null;
    }
    const out = hintsFor(typed, { commands: live?.commands, files: this.fileList });
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

  /**
   * A model, an effort level or fast mode the person picked in the box's menus (`model:opus`, `effort:high`, `fast:on`),
   * for the agent the chat shows: a driven session takes it itself; a terminal gets its agent's own command typed.
   */
  choose(spec, cx) {
    const split = spec.lastIndexOf("|");
    if (split < 0) return;
    let subject;
    try {
      subject = decodeURIComponent(spec.slice(split + 1));
    } catch {
      return;
    }
    if (!subject || subject !== this.target) {
      notify({ title: "The chat changed", body: "Open the model, effort or speed menu again for this agent.", level: "warning" });
      return;
    }
    spec = spec.slice(0, split);
    const at = spec.indexOf(":");
    const [setting, word] = [spec.slice(0, at), spec.slice(at + 1)];
    if (!["model", "effort", "fast"].includes(setting) || !word) return;
    const value = setting === "fast" ? word === "on" : word;
    const live = this.session();
    const terminal = live ? null : this.row()?.terminal;
    if (!live && !terminal) return;
    cx.spawn(async (cx) => {
      let r;
      try {
        r = live
          ? await hub.act("session", setting, [{ key: live.key, [setting]: value }])
          : await hub.act("agents", "pick", [{ terminal, setting, value, source: "chat:menu" }]);
      } catch (e) {
        r = { error: String(e?.message ?? e) };
      }
      if (r?.error) notify({ title: "It did not take it", body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 5000 });
      else if (terminal) this.typedPicks.set(terminal, { ...(this.typedPicks.get(terminal) ?? {}), [setting]: value, at: Date.now() });
      cx.notify();
    });
  }

  /** What was typed into the terminal `row` shows and its transcript does not say yet. */
  askedFor(row, ran) {
    const asked = row?.terminal ? this.typedPicks.get(row.terminal) : null;
    if (!asked || Date.now() - asked.at > ASKED_MS) return {};
    const shown = (s) => asked[s] === ran?.[s] || (s === "model" && modelName(ran?.model).toLowerCase().startsWith(String(asked.model).toLowerCase()));
    return Object.fromEntries(["model", "effort", "fast"].filter((s) => s in asked && !shown(s)).map((s) => [s, asked[s]]));
  }

  /** A pick chip: a menu where the agent takes a change now, else its value with the reason in its tooltip. */
  pickChip(c, k) {
    if (!c.disabled) {
      const subject = encodeURIComponent(this.target ?? "");
      const items = c.items.map((item) => item?.command ? { ...item, command: item.command + "|" + subject } : item);
      return this.picker(c.id, { label: c.label, icon: c.icon, tooltip: c.tooltip, items }, k);
    }
    return h_flex()
      .id(c.id)
      .h(26)
      .px(7)
      .gap(5)
      .flex_none()
      .items_center()
      .text_size(TEXT.sm)
      .text_color(k.dim)
      .tooltip(c.tooltip)
      .when(Boolean(c.icon), (e) => e.child(Icon(c.icon, { size: 14, color: k.dim })))
      .child(div().child(c.label));
  }

  /** One of the session's own actions, on the session the chat is showing. */
  sessionAct(action, arg, cx, key = this.session()?.key) {
    if (!key) return;
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("session", action, [{ key, ...arg }]);
        // A refusal (a mode that needs a yes, a session gone) is said, not swallowed.
        if (r?.error) notify({ title: "The session did not take it", body: String(r.error).replace(/^[a-z]+: /, ""), level: "warning", timeout: 5000 });
      } catch (e) {
        notify({ title: "The session did not take it", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** Back to the newest message, which then stays in view as more come. */
  toEnd(cx) {
    this.pinned = true;
    this.lost = false;
    this.windowTop = null;
    this.windowAround = null;
    // Back at the end, a load from further back that comes in later moves nothing.
    this.restoring = null;
    this.seenMsg = null;
    this.kept = null;
    this.top = this.maxTop();
    this.jump(cx, [this.rows.length]);
    cx.notify();
  }

  /** How far down the rows can go: their measured heights past the height of the view. */
  maxTop() {
    let h = 0;
    for (const r of this.rows) h += this.heights.get(r.key) ?? 0;
    return Math.max(0, h - this.viewH);
  }

  /**
   * Scroll requests in turn, each after the layout before it: a request is consumed once, after the next layout, and
   * the last is asked again a few frames on for rows still measuring their text.
   */
  jump(cx, targets) {
    if (!this.scroll) return;
    targets.forEach((t, i) => cx.timer.after(16 + i * 40, () => this.scroll.scroll_to_item(t)));
    const last = targets.at(-1);
    for (const wait of [160, 400]) cx.timer.after(wait + targets.length * 40, () => this.pinned && !this.restoring && this.scroll.scroll_to_item(last));
  }

  /**
   * The wheel moved the conversation: leaving the end freezes the drawn window, so new rows never take rows away above
   * the reader, and reaching the top with more before it reads further back. Ticks that come while a slide or a load
   * settles (a trackpad's momentum) move nothing.
   */
  wheeled(e, cx) {
    const dy = Number(e?.delta?.y ?? 0);
    if (!dy || this.restoring || this.loadingEarlier) return;
    const max = this.maxTop();
    if (this.pinned) this.top = max;
    this.top = Math.min(max, Math.max(0, this.top - dy));
    const atEnd = !this.lost && this.top >= max - 4;
    const last = this.all.at(-1);
    // At the bottom of a window that is not the end: the window slides down, the reader stays where they are.
    if (atEnd && dy < 0 && this.windowTop && last && !this.rows.some((r) => r.key === last.key)) return this.later(cx);
    if (atEnd !== this.pinned) {
      this.pinned = atEnd;
      this.windowTop = atEnd ? null : (this.rows[0]?.ids?.[0] ?? null);
      this.seenMsg = atEnd ? null : this.lastMessage();
      if (atEnd) this.kept = null;
      cx.notify();
    }
    if (this.top <= 0 && dy > 0) this.earlier(cx);
  }

  /** Slides the drawn window down over later rows, the message at the top of the view kept where it is. */
  later(cx) {
    if (this.restoring) return;
    const start = this.all.findIndex((r) => r.key === this.rows[0]?.key);
    const next = Math.max(0, Math.min(this.all.length - DRAW, start + DRAW / 2));
    this.restoring = { anchor: this.topMessage() };
    this.windowTop = this.all[next]?.ids?.[0] ?? null;
    cx.notify();
  }

  /** The index of the row holding this message, -1 when none does. */
  static at(rows, id) {
    return id ? rows.findIndex((r) => r.ids?.includes(id)) : -1;
  }

  /**
   * The window of rows drawn: from the row holding `windowTop` while the reader reads back, the latest ones otherwise.
   * Each row's text views take a place from a pool kept by row, the least recently drawn given up first, so a row
   * keeps its views while the window moves and the pool never holds more than a window.
   */
  drawn(all) {
    if (this.windowAround) {
      const i = Chat.at(all, this.windowAround);
      if (i >= 0) {
        this.windowTop = all[Math.max(0, i - DRAW / 2)].ids?.[0] ?? null;
        this.windowAround = null;
      }
    }
    let start = all.length - DRAW;
    if (this.windowTop) {
      const i = Chat.at(all, this.windowTop);
      if (i >= 0) start = Math.min(i, Math.max(0, all.length - 1));
      else this.windowTop = null;
    }
    start = Math.max(0, start);
    const rows = all.slice(start, start + DRAW);
    const pool = (this.pool ??= new Map());
    const keys = new Set(rows.map((r) => r.key));
    const used = new Set();
    for (const r of rows) if (pool.has(r.key)) used.add(pool.get(r.key));
    const free = [...pool.entries()].filter(([k]) => !keys.has(k));
    let next = 0;
    const placed = rows.map((r) => {
      let slot = pool.get(r.key);
      if (slot === undefined) {
        const given = free.shift();
        if (given) {
          pool.delete(given[0]);
          slot = given[1];
        } else {
          while (used.has(next)) next++;
          slot = next;
        }
        used.add(slot);
      }
      // Drawn now: the most recently used, last in the pool's order.
      pool.delete(r.key);
      pool.set(r.key, slot);
      return { ...r, slot };
    });
    while (pool.size > DRAW) pool.delete(pool.keys().next().value);
    return placed;
  }

  /** A row measured: the view follows the newest when it is pinned there, as the last row grows while it streams. */
  measured(key, height, cx) {
    if (this.heights.get(key) === height) return;
    const fitted = this.maxTop() <= 0;
    this.heights.set(key, height);
    // Whether it all fits decides the "Earlier messages" pill: drawn again once, when that changes.
    if (fitted !== this.maxTop() <= 0 && !this.redraw) {
      this.redraw = true;
      cx.timer.after(30, (later) => {
        this.redraw = false;
        (later ?? cx).notify();
      });
    }
    if (this.pinned && !this.restoring && !this.following && this.scroll) {
      this.following = true;
      cx.timer.after(16, () => {
        this.following = false;
        if (this.pinned) this.scroll.scroll_to_item(this.rows.length);
      });
    }
  }

  /** The row at the top of the view, from how far down the reader is and the rows' heights. */
  rowAtTop() {
    if (this.pinned) return this.rows[0] ?? null;
    let h = 0;
    for (const r of this.rows) {
      h += this.heights.get(r.key) ?? 0;
      if (h > this.top) return r;
    }
    return this.rows[0] ?? null;
  }

  /** The first message of the row at the top of the view: what reading back keeps in place. */
  topMessage() {
    return this.rowAtTop()?.ids?.[0] ?? null;
  }

  /** The newest message drawn, as an id. */
  lastMessage() {
    const all = this.messagesNow ?? [];
    return all.length ? msgId(all.at(-1)) : null;
  }

  /** How many messages with words came since the reader left the end: never 0 while an answer waits unread. */
  unseen() {
    if (this.pinned || !this.seenMsg) return 0;
    const all = this.messagesNow ?? [];
    let at = all.length - 1;
    while (at >= 0 && msgId(all[at]) !== this.seenMsg) at--;
    if (at < 0) return 0;
    return all.slice(at + 1).filter((m) => !m.tool && String(m.text ?? "").trim()).length;
  }

  /**
   * After the rows changed: the newest kept in view when the reader is at the end, and the message that was at the top
   * kept there when rows came in above it, whatever row now holds it.
   */
  settle(rows, cx) {
    const last = rows.at(-1)?.key ?? null;
    const first = rows[0]?.key ?? null;
    if (last === this.lastKey && first === this.firstKey && rows.length === this.rows.length) return;
    this.lastKey = last;
    this.firstKey = first;
    if (this.restoring && this.pinned) this.restoring = null;
    if (this.restoring) {
      const at = Chat.at(rows, this.restoring.anchor);
      if (at > 0) {
        // A row below the anchor first, a view's height down, then the anchor: revealed from below, it lands at the
        // top where it was, and the newest rows never flash by.
        let h = 0;
        let below = rows.length;
        for (let i = at; i < rows.length; i++) {
          h += this.heights.get(rows[i].key) ?? 0;
          if (h >= this.viewH) {
            below = i + 1;
            break;
          }
        }
        this.jump(cx, [Math.min(below, rows.length), at]);
        const anchor = this.restoring.anchor;
        cx.timer.after(700, () => {
          this.restoring = null;
          // The anchor sits at the top: the reader is as far down as the rows above it are tall.
          const i = Chat.at(this.rows, anchor);
          this.top = this.rows.slice(0, Math.max(0, i)).reduce((sum, r) => sum + (this.heights.get(r.key) ?? 0), 0);
        });
        return;
      }
      if (at < 0) this.restoring = null;
    }
    if (this.pinned && rows.length > 0) this.jump(cx, [rows.length]);
  }


  /**
   * ⏎ in the box: an answer when it waits on a question, else a prompt — sent when the agent is free, queued after the
   * others while it works. The words leave the box once the hub took them; refused, they stay with "Not sent · Retry".
   */
  send(cx, how = "click") {
    // ⏎ with a hint showing completes it: a half-typed command is never sent.
    if (how === "key" && this.hints().items.length > 0) {
      this.complete(this.hints().items[0].put, cx);
      return;
    }
    const subject = this.target;
    const live = this.session();
    const row = this.row();
    const typed = this.input.value().trim();
    const text = [...this.files, typed].filter(Boolean).join("\n");
    if (!row && !live) {
      // What a window command typed starts from a click on ↑ only: ⏎ is the person's key, whatever it was meant for.
      const outside = this.typedOutside;
      if (outside && outside.target === (this.target ?? null) && this.input.value().trim()) {
        if (how === "key") return notify({ title: "Typed from outside the window", body: "Click ↑ to start it.", level: "info", timeout: 4000 });
      }
      // No agent to send it to: the box starts one (with no prompt too), and the chat follows it once it is there.
      this.starter.start(cx, (key, cx) => (this.clearBox(), key && this.open(key, cx)), this.input.value());
      return cx.notify();
    }
    if (!text || this.sent?.state === "sending") return cx.notify();
    // An edited queued prompt goes back in its place, a driven session's as well as a terminal's.
    const edit = this.edits.get(subject) ?? null;
    // A question takes words: files stay in the box for the next prompt.
    if (!edit && !live && row?.question) {
      if (typed) return this.answerFromBox(cx, row, typed, how);
      this.sends.set(subject, { state: "failed", why: "it waits on an answer: type it, the files go with the next prompt" });
      return cx.notify();
    }
    const handle = live?.key ?? row?.terminal;
    // Everything else goes through the queue: now when the agent is free, after the prompts already waiting otherwise.
    const ask = edit ? (cx) => hub.act("queue", "edit", [handle, edit.id, text], cx) : (cx) => hub.act("queue", "tell", [handle, text], cx);
    this.sends.set(subject, { state: "sending" });
    cx.notify();
    cx.spawn(async (cx) => {
      let r;
      try {
        r = await ask(cx);
      } catch (e) {
        r = { error: String(e?.message ?? e), unconfirmed: hub.isUnconfirmed(e) };
      }
      const why = String(r?.error ?? "").replace(/^[a-z_]+: /, "");
      if (r?.unconfirmed) this.sends.set(subject, { state: "unconfirmed", why });
      else if (r?.error) this.sends.set(subject, { state: "failed", why });
      else {
        const queued = String(r?.note ?? "").startsWith("queued") ? { state: "queued", note: r.note } : null;
        if (queued) this.sends.set(subject, queued);
        else this.sends.delete(subject);
        if (queued) cx.timer.after(6000, (cx) => this.sends.get(subject) === queued && (this.sends.delete(subject), cx.notify()));
        if (edit) this.edits.delete(subject);
        // Only what was sent leaves the box: words typed meanwhile stay, and a subject left meanwhile keeps no copy.
        if (subject === this.target) {
          const unchanged = this.input.value().trim() === typed;
          if (unchanged) this.clearBox();
          if (edit && unchanged) this.restoreDraft(edit.draft);
        } else if (this.drafts.get(subject)?.text?.trim() === typed) this.drafts.delete(subject);
      }
      cx.notify();
    });
  }

  /**
   * The box's words as the answer to the question the agent waits on, files left in the box. They leave it once the
   * hub took them, wherever the reader is by then; one that went with no answer back is not offered to send again.
   */
  answerFromBox(cx, row, typed, how) {
    const subject = this.target;
    this.sends.set(subject, { state: "sending" });
    this.model.startReply(cx, row, null);
    this.model.reply.set_value(typed);
    cx.spawn(async (cx) => {
      let r = { ok: false };
      try {
        r = (await this.model.sendReply(cx, how)) ?? { ok: false };
      } catch {}
      this.sends.delete(subject);
      if (r.ok || r.unconfirmed) this.forget(subject, typed);
      if (r.unconfirmed) this.sends.set(subject, { state: "unconfirmed", answer: true });
      else if (!r.ok) this.sends.set(subject, { state: "failed", why: "the answer did not go", answer: true });
      cx.notify();
    });
    cx.notify();
  }

  /** What was sent leaves the box it went from: the one in front if unchanged, else that subject's saved draft. */
  forget(subject, typed) {
    if (subject === this.target) {
      if (this.input.value().trim() === typed) this.input.set_value("");
    } else if (this.drafts.get(subject)?.text?.trim() === typed) this.drafts.delete(subject);
  }

  /** Retry after a send that failed: an answer only while its question still waits, never as a prompt. */
  retry(cx) {
    const sent = this.sent;
    if (sent?.answer && !this.row()?.question) {
      this.sends.set(this.target, { state: "failed", why: "its question is gone: look at the conversation before answering again" });
      return cx.notify();
    }
    this.send(cx);
  }

  /** What the box held before an edit, back in it. */
  restoreDraft(draft) {
    this.input.set_value(draft?.text ?? "");
    this.files = draft?.files ?? [];
  }

  clearBox() {
    this.typedOutside = null;
    this.input.set_value("");
    this.files = [];
    this.drafts.delete(this.target);
  }

  /** Stops the turn it is taking: a session cockpit drives through its own interrupt, a terminal through its keys. */
  stop(cx) {
    const live = this.session();
    if (live) return this.sessionAct("interrupt", {}, cx);
    const row = this.row();
    if (!row?.terminal) return;
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("agents", "interrupt", [{ terminal: row.terminal }]);
        if (r?.error) notify({ title: "It did not stop", body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 5000 });
      } catch (e) {
        notify({ title: "It did not stop", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  /** One of the queue's actions on the subject's queue, its refusal said. */
  queueAct(action, args, cx) {
    const handle = this.subjectHandle();
    if (!handle) return;
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("queue", action, [handle, ...args]);
        if (r?.error) notify({ title: "The queue did not take it", body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 5000 });
      } catch (e) {
        notify({ title: "The queue did not take it", body: String(e?.message ?? e), level: "warning" });
      }
      cx.notify();
    });
  }

  // Drawing -------------------------------------------------------------------------------------------------------

  /**
   * The subject in one line over the conversation: what it is doing, its name, where it works, its branch, and its PR
   * with CI. A session cockpit drives can also be resumed in a terminal of its own.
   */
  header(k, row, live) {
    const cwd = live?.cwd || this.value?.session?.cwd || row.cwd || "";
    const place = row.repo || String(cwd).split("/").filter(Boolean).pop() || null;
    const branch = row.branch || this.value?.session?.branch || null;
    const pr = row.pr?.number ? row.pr : null;
    const ci = row.ci?.state ?? null;
    // Colour is for what the agent does: CI says how it went in a shape (a dot passed, a ring still runs, a cross failed).
    const ciMark = ci === "fail" ? Icon("x", { size: 11 }) : ci === "pass" ? div().size(7).rounded(4).bg(k.mutedFg) : div().size(7).rounded(4).border_1().border_color(k.mutedFg);
    // Narrow, the header's buttons keep their icon and say the rest on hover.
    const narrow = (this.width ?? 1000) < 820;
    const remote = Boolean(hub.remoteMachine());
    const sep = () => div().flex_none().text_color(k.dim).child("·");
    return h_flex()
      .id("chat-head")
      .w_full()
      .h(40)
      .flex_none()
      .px(GUTTER)
      .justify_center()
      .child(
        h_flex()
          .w_full()
          .max_w(READ_W)
          .gap(8)
          .items_center()
          .text_size(TEXT.sm)
          .text_color(k.mutedFg)
          .child(div().flex_none().child(statusMark(row.status, k)))
          .child(AgentLogo(row, { size: 14, tooltip: row.agent_name, id: "chat-head-logo" }))
          .child(div().flex_shrink(1).min_w(120).truncate().font_medium().text_color(k.foreground).child(row.title || row.tab?.name || this.target || ""))
          .when(Boolean(place), (e) => e.child(sep()).child(div().flex_none().max_w(140).truncate().child(place)))
          .when(Boolean(branch), (e) => e.child(sep()).child(div().flex_shrink(1).min_w(0).truncate().font_family("JetBrains Mono").text_size(TEXT.xs).child(branch)))
          .when(Boolean(pr), (e) =>
            e.child(sep()).child(
              h_flex()
                .id("chat-head-pr")
                .flex_none()
                .gap(5)
                .items_center()
                .cursor_pointer()
                .hover((s) => s.text_color(k.foreground))
                .tooltip([pr.title, row.ci?.label, "Merge or open it"].filter(Boolean).join(" · "))
                .on_click((_e, cx) => this.prMenu(cx, row))
                .child(div().child(`#${pr.number}`))
                .when(Boolean(ci), (el) => el.child(ciMark).child(div().child("CI")))
                .when(this.prChecking === pr.url, (el) => el.child(Spinner({ size: 10, color: k.mutedFg }))),
            ),
          )
          .child(div().flex_1())
          .when(Boolean(live), (e) =>
            e.child(
              Button("chat-head-end", {
                label: narrow ? undefined : "End",
                icon: "circle-x",
                variant: "ghost",
                size: "sm",
                tooltip: "End this conversation",
                onClick: (_e, cx) => this.end(cx),
              }),
            ),
          )
          .when(Boolean(live) && !remote, (e) =>
            e.child(
              Button("chat-head-terminal", {
                label: narrow ? undefined : "Open in a terminal",
                icon: "terminal",
                variant: "ghost",
                size: "sm",
                tooltip: "Hand this session over to a terminal of its own",
                onClick: (_e, cx) => this.handover(cx),
              }),
            ),
          ),
      );
  }

  /**
   * The PR chip's menu: how ready it is, from the row's CI and reviews then GitHub's own answer, and the merges that
   * apply. Its items carry the menu's key; a merge picked there is asked once more under the header.
   */
  prMenu(cx, row) {
    const pr = row?.pr;
    if (!pr?.url || this.prChecking) return;
    this.prChecking = pr.url;
    cx.notify();
    const arg = { url: pr.url, repo: row.repo ?? pr.repo ?? "", number: pr.number };
    cx.spawn(async (cx) => {
      let got;
      try {
        got = await hub.fetchDetails("pr_merge", arg);
      } catch (e) {
        got = { error: String(e?.message ?? e) };
      }
      this.prChecking = null;
      cx.notify();
      // Moved on to another agent meanwhile: its PR is not the one asked about.
      if (this.row()?.pr?.url !== pr.url) return;
      const choices = got?.error ? [] : (got?.choices ?? []);
      this.prMerges = { url: pr.url, arg, base: got?.base ?? null, choices };
      const why = got?.error ? `GitHub did not answer: ${String(got.error).replace(/^[a-z_]+: /, "")}` : got?.why;
      openMenu(
        [
          { label: prReadiness(row), icon: "info", info: true },
          got?.ready?.text && { label: got.ready.text, icon: got.ready.tone === "good" ? "check" : got.ready.tone === "bad" ? "x" : "clock", info: true },
          "separator",
          ...choices.map((c, i) => ({ label: c.label, icon: c.do === "auto" || c.do === "auto-off" ? "clock" : c.do === "dequeue" ? "x" : "git-merge", command: `chat:merge:${i}` })),
          !choices.length && why && { label: why, icon: "info", info: true },
          choices.length > 0 && "separator",
          { label: "Open on GitHub", icon: "external", command: "chat:pr:open" },
        ],
        { width: 320 },
      );
    });
  }

  /** A merge picked from the PR chip's menu, asked under the header first; the PR must still be the chat's. */
  askMerge(i, cx) {
    const m = this.prMerges;
    const c = m?.choices?.[i];
    if (!c || this.row()?.pr?.url !== m.url) return;
    const n = `#${m.arg.number}`;
    const base = m.base ?? "its base";
    const call = {
      merge: { fn: "merge", args: [m.arg, c.method], question: `${c.label}: ${n} into ${base} now?`, verb: "Merge" },
      enqueue: { fn: "merge", args: [m.arg], question: `Add ${n} to the merge queue of ${base}? It merges when its turn comes.`, verb: "Add to queue" },
      auto: { fn: "automerge", args: [m.arg, c.method], question: `Turn on auto-merge for ${n}? It merges once checks and reviews allow.`, verb: "Turn on" },
      "auto-off": { fn: "automerge", args: [m.arg, "off"], question: `Turn off auto-merge for ${n}?`, verb: "Turn off" },
      dequeue: { fn: "dequeue", args: [m.arg], question: `Take ${n} out of the merge queue?`, verb: "Take out" },
    }[c.do];
    if (call) this.merging = { url: m.url, ...call };
    cx.notify();
  }

  /** The merge asked under the header, once the person says yes: the hub's inbox act, its note said when it answers. */
  merge(cx) {
    const m = this.merging;
    if (!m || m.busy) return;
    this.merging = { ...m, busy: true };
    cx.notify();
    cx.spawn(async (cx) => {
      const r = await hub.act("inbox", m.fn, m.args).catch((e) => ({ error: String(e?.message ?? e) }));
      this.merging = null;
      if (r?.error) notify({ title: `${m.verb}: it did not go through`, body: String(r.error).replace(/^[a-z_]+: /, ""), level: "warning", timeout: 6000 });
      else notify({ title: r?.note || "GitHub took it", level: "info", timeout: 4000 });
      cx.notify();
    });
  }

  /** Ends the driven session; while it works, only once the person says so under the header. */
  end(cx, sure = false) {
    const live = this.session();
    if (!live) return;
    if (live.working === true && !sure) {
      this.ending = { key: live.key };
      return cx.notify();
    }
    this.ending = null;
    this.sessionAct("stop", sure ? { confirm: true } : {}, cx, live.key);
    cx.notify();
  }

  /**
   * Hands the driven session over to a terminal of its own: asked first, since it then leaves the chat, and refused by
   * the hub while it works or waits on an answer, so one process only ever writes it.
   */
  handover(cx, sure = false) {
    const live = this.session();
    if (!live || hub.remoteMachine()) return;
    if (!sure) {
      this.handing = { key: live.key };
      return cx.notify();
    }
    this.handing = null;
    this.sessionAct("terminal", { confirm: true, frontend: "app" }, cx, live.key);
    cx.notify();
  }

  /** A question under the header that a session's own action waits on: what it does, Keep, and the one action. */
  askLine(k, { id, text, keep, go, onKeep, onGo }) {
    return h_flex()
      .w_full()
      .px(GUTTER)
      .justify_center()
      .child(
        h_flex()
          .id(id)
          .w_full()
          .max_w(READ_W)
          .gap(8)
          .px(12)
          .py(6)
          .items_center()
          .rounded(RADIUS.lg)
          .bg(k.muted)
          .text_size(TEXT.sm)
          .text_color(k.foreground)
          .child(Icon("triangle-alert", { size: 13 }))
          .child(div().flex_1().min_w(0).child(text))
          .child(Button(`${id}-keep`, { label: keep, variant: "ghost", size: "sm", onClick: (_e, cx) => (onKeep(), cx.notify()) }))
          .child(Button(`${id}-go`, { label: go, variant: "outline", size: "sm", onClick: (_e, cx) => onGo(cx) })),
      );
  }

  /** The hub does not answer: say so where the conversation goes, with the way to start it. */
  hubDown(k) {
    return HubDown("chat-hub-down", this, String(this.lastHubError ?? this.hubState).replace(/^error: /, ""), k, { maxW: READ_W });
  }

  /** Every row is laid in the same reading column, whatever it holds. */
  column(child) {
    return h_flex().w_full().px(GUTTER).justify_center().child(v_flex().w_full().max_w(READ_W).child(child));
  }

  said(row, k) {
    // The images sent with it show as themselves above its words rather than as their paths.
    const lines = String(row.text).split("\n");
    const pics = lines.filter((l) => IMAGE_LINE.test(l) && this.thumbs.get(l) !== false);
    const text = lines.filter((l) => !pics.includes(l)).join("\n").trim();
    return this.column(
      v_flex()
        .w_full()
        .py(10)
        .gap(4)
        .items_end()
        .when(pics.length > 0, (e) => e.child(h_flex().flex_wrap().justify_end().gap(6).children(pics.map((p) => this.picture(p, 120, k)))))
        .when(Boolean(text), (e) => e.child(
          v_flex()
            .max_w(READ_W * 0.78)
            .px(16)
            .py(11)
            .rounded(20)
            .bg(k.muted)
            .text_size(TEXT.base)
            .text_color(k.foreground)
            .child(TextView.markdown(`chat-s${row.slot}`, linkify(text)).selectable(true).text_size(TEXT.base).on_link_click((url) => this.openLink(url))),
        ))
        // Sent to a session cockpit drives, not yet replayed by it.
        .when(Boolean(row.pending), (e) => e.child(h_flex().gap(5).items_center().text_size(TEXT.xs).text_color(k.dim).child(Spinner({ size: 10, color: k.dim })).child(div().child("Sending…")))),
    );
  }

  answered(row, k) {
    const parts = segments(row.text).map((s, i) =>
      s.code
        ? this.codeBox(`chat-s${row.slot}-c${i}`, s, k)
        : TextView.markdown(`chat-s${row.slot}-t${i}`, linkify(s.text)).selectable(true).text_size(TEXT.base).on_link_click((url) => this.openLink(url)),
    );
    // The images it names show as themselves under its words, each opened full size with a click.
    const pics = imagesIn(row.text, home()).filter((p) => this.thumbs.get(p) !== false);
    const picture = (p, i) => this.picture(p, PICTURE_H, k).id(`chat-s${row.slot}-p${i}`).cursor_pointer().tooltip(p).on_click(() => openUrl(p));
    return this.column(
      v_flex()
        .w_full()
        .pb(10)
        .gap(2)
        .text_color(k.foreground)
        .children(parts)
        .when(pics.length > 0, (e) => e.child(h_flex().flex_wrap().gap(8).pt(6).children(pics.map(picture)))),
    );
  }

  /**
   * A link the conversation holds: a path opens in the side panel beside the chat (a folder or an image in the system's
   * own app), anything else in the browser.
   */
  openLink(url) {
    const path = pathOf(url);
    if (!path) return openUrl(url);
    const { file, line } = fileAndLine(expand(path, home()));
    if (IMAGE_LINE.test(file)) return openUrl(file);
    exec("test", ["-d", file])
      .then((r) => (r?.code === 0 ? openUrl(file) : openCode(file, { line: line ?? undefined })))
      .catch(() => openCode(file, { line: line ?? undefined }));
  }

  /**
   * A fenced block in a box of its own: its language and a copy button over the lines, kept as written and scrolled
   * sideways rather than wrapped.
   */
  codeBox(id, s, k) {
    const text = s.text.replace(/\n$/, "");
    return v_flex()
      .id(id)
      .w_full()
      .my(6)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .bg(k.card)
      .overflow_hidden()
      .child(
        h_flex()
          .h(28)
          .px(12)
          .gap(8)
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .text_size(TEXT.xs)
          .text_color(k.dim)
          .child(div().flex_1().child(s.lang || "code"))
          .child(
            div()
              .id(`${id}-copy`)
              .cursor_pointer()
              .hover((st) => st.text_color(k.foreground))
              .tooltip("Copy")
              .on_click((_e, cx) => {
                cx.write_to_clipboard(text);
                notify({ title: "Copied", level: "info", timeout: 1500 });
              })
              .child(Icon("copy", { size: 12 })),
          ),
      )
      .child(
        div()
          .id(`${id}-lines`)
          .w_full()
          .overflow_x_scroll()
          .px(12)
          .py(10)
          .font_family("JetBrains Mono")
          .text_size(TEXT.sm)
          .text_color(k.foreground)
          .child(v_flex().children(text.split("\n").map((line) => div().whitespace_nowrap().child(line.replace(/\t/g, "    ") || " ")))),
      );
  }

  /** Whether a turn's fold is shut: a running one then shows only what it does now, a finished one only how long it took. */
  folded(row) {
    // Kept by a message the row holds: the fold survives the row growing when earlier messages come.
    for (const id of row.ids ?? []) if (this.folds.has(id)) return this.folds.get(id);
    return true;
  }

  /**
   * What the agent did for one turn, as one line you can open. While it works, under it, the step it is on as its
   * terminal words it ("Reading the config"); open, it lists every call and everything it said on the way.
   */
  work(row, k) {
    const open = !this.folded(row);
    const steps = row.steps ?? [...row.notes.map((note) => ({ note })), ...row.calls.map((call) => ({ call }))];
    const toggle = (_e, cx) => {
      for (const id of row.ids ?? []) this.folds.delete(id);
      if (row.ids?.[0]) this.folds.set(row.ids[0], open);
      cx.notify();
    };
    const mark = (c) => (c.ok === false ? Icon("x", { size: 12, color: k.destructive }) : c.ok === true ? Icon("check", { size: 12, color: k.success }) : Spinner({ size: 11, color: k.mutedFg }));
    const call = (c) =>
      h_flex()
        .h(26)
        .gap(10)
        .items_center()
        .text_size(TEXT.sm)
        .child(div().flex_none().w(78).truncate().text_color(k.foreground).child(c.name))
        .child(
          c.about
            ? div().flex_1().min_w(0).truncate().text_color(k.mutedFg).child(c.about)
            : div().flex_1().min_w(0).truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(c.summary || "—"),
        )
        .child(div().flex_none().child(mark(c)));
    const note = (n, i) =>
      div()
        .w_full()
        .py(4)
        .text_size(TEXT.sm)
        .text_color(k.mutedFg)
        .child(TextView.markdown(`chat-s${row.slot}-n${i}`, n.text).selectable(true).text_size(TEXT.sm));
    const step = (s, i) => (s.call ? call(s.call) : note(s.note, i));
    // The step it is on as its terminal shows it, else the last thing its transcript holds: what for, then the call.
    const shown = !row.done ? this.value?.session : null;
    const latest = shown?.task ? { step: { label: shown.task, detail: shown.detail } } : steps.at(-1);
    const line = (label, detail) =>
      v_flex()
        .w_full()
        .min_w(0)
        .child(div().w_full().truncate().text_size(TEXT.sm).text_color(k.foreground).child(label))
        .when(Boolean(detail), (e) => e.child(div().w_full().truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(detail)));
    const now = (s) =>
      s.step
        ? line(s.step.label, s.step.detail)
        : s.call
        ? h_flex()
            .w_full()
            .gap(10)
            .items_start()
            .child(
              v_flex()
                .flex_1()
                .min_w(0)
                .child(div().w_full().truncate().text_size(TEXT.sm).text_color(k.foreground).child(s.call.about || s.call.name))
                .child(div().w_full().truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(`${s.call.about ? `${s.call.name} · ` : ""}${s.call.summary || "—"}`)),
            )
            .child(div().flex_none().pt(3).child(mark(s.call)))
        : div().w_full().text_size(TEXT.sm).text_color(k.mutedFg).child(TextView.markdown(`chat-s${row.slot}-now`, s.note.text).selectable(true).text_size(TEXT.sm));
    // The rest of the list, folded beside the last one.
    const more = latest?.step ? steps.length : steps.length - 1;
    const expander = h_flex()
      .id(`work-more-${row.key}`)
      .flex_none()
      .h(22)
      .px(6)
      .gap(3)
      .items_center()
      .rounded(RADIUS.md)
      .cursor_pointer()
      .text_size(TEXT.xs)
      .text_color(k.dim)
      .hover((st) => st.bg(k.accent).text_color(k.foreground))
      .tooltip("Everything it did this turn")
      .on_click(toggle)
      .child(div().child(`${more} more`))
      .child(Icon("chevron-down", { size: 12, color: k.dim }));
    const label = workedFor(row.done ? row.secs : secondsBetween(row.from, new Date().toISOString()), row.done);
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
            .on_click(toggle)
            .when(!row.done, (e) => e.child(Spinner({ size: 13, color: k.mutedFg })))
            .child(div().min_w(0).truncate().text_size(TEXT.base).child(label))
            .child(Icon(open ? "chevron-down" : "chevron-right", { size: 15, color: k.dim })),
        )
        .when(open && steps.length > 0, (e) => e.child(v_flex().w_full().pt(6).children(steps.map(step))))
        .when(!open && !row.done && Boolean(latest), (e) =>
          e.child(
            h_flex()
              .w_full()
              .pt(6)
              .pl(20)
              .gap(8)
              .items_start()
              .child(div().flex_1().min_w(0).child(now(latest)))
              .when(more > 0, (e) => e.child(expander)),
          ),
        )
        // The rule sits close under the row and the answer stands well clear of it.
        .when(row.done, (e) => e.child(div().w_full().h(1).mt(10).mb(22).bg(k.border))),
    );
  }

  /** Where the turn stopped, by you or on an error: one quiet line, never a bubble of yours. */
  stopped(row, k) {
    return this.column(
      h_flex()
        .w_full()
        .py(8)
        .gap(6)
        .items_start()
        .text_size(TEXT.sm)
        .text_color(row.failed ? k.destructive : k.dim)
        .child(div().pt(2).child(Icon(row.failed ? "circle-x" : "circle-stop", { size: 13 })))
        .child(div().flex_1().min_w(0).whitespace_normal().child(row.failed ? `Stopped with an error${row.error ? `: ${row.error}` : ""}` : "Stopped by you")),
    );
  }

  rowEl(row, k) {
    if (row.kind === "said") return this.said(row, k);
    if (row.kind === "stopped") return this.stopped(row, k);
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

  /**
   * The prompts waiting their turn, under the conversation: each can be edited in the box, moved up, sent now or taken
   * back. A queue that stopped draining says why, with the way to let it go on.
   */
  queuedRows(k) {
    const queued = this.queued();
    const paused = this.queues?.paused?.[this.subjectHandle()] ?? null;
    if (queued.length === 0 && !paused) return [];
    const action = (id, icon, tooltip, onClick) =>
      div()
        .id(id)
        .size(20)
        .flex()
        .items_center()
        .justify_center()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_color(k.dim)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip(tooltip)
        .on_click((_e, cx) => {
          onClick(cx);
          cx.notify();
        })
        .child(Icon(icon, { size: 11 }));
    const one = (q, i) =>
      h_flex()
        .w_full()
        .px(14)
        .py(8)
        .gap(6)
        .items_start()
        .rounded(16)
        .border_1()
        .border_color(this.editing === q.id ? k.ring : k.border)
        .bg(k.canvas)
        .child(div().pt(1).child(Icon("list-ordered", { size: 13, color: k.mutedFg })))
        .child(div().flex_1().min_w(0).text_size(TEXT.base).text_color(k.mutedFg).child(q.text))
        .child(
          action(`queued-edit-${q.id}`, "pencil", "Edit it in the box", () => {
            const held = this.edits.get(this.target);
            this.edits.set(this.target, { id: q.id, draft: held ? held.draft : { text: this.input.value(), files: this.files } });
            this.input.set_value(q.text);
            this.files = [];
            this.input.focus();
          }),
        )
        .when(i > 0, (e) => e.child(action(`queued-up-${q.id}`, "arrow-up", "Move it up", (cx) => this.queueAct("move", [q.id, i - 1], cx))))
        .child(action(`queued-send-${q.id}`, "send", "Send it now", (cx) => this.queueAct("send", [q.id], cx)))
        .child(action(`queued-${q.id}`, "x", "Take it back", (cx) => this.queueAct("remove", [q.id], cx)));
    const pausedRow = paused
      ? h_flex()
          .w_full()
          .px(14)
          .py(8)
          .gap(8)
          .items_center()
          .rounded(16)
          .bg(k.muted)
          .text_size(TEXT.sm)
          .text_color(k.mutedFg)
          .child(Icon("circle-pause", { size: 13, color: k.mutedFg }))
          .child(div().flex_1().min_w(0).child(`Paused — ${paused.why ?? "it stopped sending"}`))
          .child(Button("queued-resume", { label: "Resume", variant: "ghost", size: "sm", tooltip: "Send the next prompt when it is free", onClick: (_e, cx) => this.queueAct("resume", [], cx) }))
      : null;
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
            .when(queued.length > 0, (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(`${queued.length} waiting to be sent`)))
            .when(Boolean(pausedRow), (e) => e.child(pausedRow))
            .children(queued.map(one)),
        ),
    ];
  }

  /** Copies image files into the app's folder, the only place a picture draws from, once each. */
  thumbsFor(paths, cx) {
    for (const path of paths) {
      if (!IMAGE_LINE.test(path) || this.thumbs.has(path)) continue;
      this.thumbs.set(path, null);
      cx.spawn(async (cx) => {
        this.thumbs.set(path, (await thumbnail(path, appHome())) ?? false);
        cx.notify();
      });
    }
  }

  /** An image `height` high, its width cropped past three times that; an empty frame holds its place while it is copied. */
  picture(path, height, k) {
    const t = this.thumbs.get(path);
    const w = t ? Math.round((height * t.w) / t.h) : height;
    return div()
      .w(Math.min(w, height * 3))
      .h(height)
      .flex_none()
      .rounded(RADIUS.md)
      .overflow_hidden()
      .border_1()
      .border_color(k.border)
      .bg(k.muted)
      .when(Boolean(t), (e) => e.child(image(t.rel).w(w).h(height)));
  }

  /** The files riding with the next prompt: an image as a small picture of itself, any other file by its name. */
  fileChips(k) {
    return this.files.map((path, i) => {
      const off = (_e, cx) => {
        cx.stop_propagation();
        this.files = this.files.filter((f) => f !== path);
        cx.notify();
      };
      if (IMAGE_LINE.test(path) && this.thumbs.get(path) !== false)
        return this.picture(path, 44, k).id(`chat-file-${i}`).cursor_pointer().tooltip(`${path} — click to take it off`).on_click(off);
      return Chip(String(path).split("/").pop(), { id: `chat-file-${i}`, icon: "file-text", tooltip: `${path} — click to take it off`, onClick: off });
    });
  }

  /**
   * Under the box, how the last send went: "Sending…" while the hub has it, "Not sent · Retry" with the reason when it
   * was refused (the words are still in the box), and which queued prompt the box is editing.
   */
  sendLine(k) {
    const failed = this.sent?.state === "failed" || this.sent?.state === "unconfirmed";
    return h_flex()
      .id("chat-sent")
      .w_full()
      .max_w(READ_W)
      .px(6)
      .gap(6)
      .items_center()
      .text_size(TEXT.sm)
      .text_color(failed ? k.foreground : k.dim)
      .when(this.sent?.state === "sending", (e) => e.child(Spinner({ size: 12, color: k.dim })).child(div().child("Sending…")))
      .when(this.sent?.state === "failed", (e) =>
        e
          .child(Icon("circle-x", { size: 13 }))
          .child(div().flex_1().min_w(0).truncate().tooltip(this.sent.why ?? "").child(`Not sent${this.sent.why ? ` — ${this.sent.why}` : ""}`))
          .child(Button("chat-retry", { label: "Retry", variant: "ghost", size: "sm", onClick: (_e, cx) => this.retry(cx) })),
      )
      // It went and no answer came: sending again could say it twice, so the person looks first.
      .when(this.sent?.state === "unconfirmed", (e) =>
        e
          .child(Icon("circle-help", { size: 13 }))
          .child(div().flex_1().min_w(0).truncate().tooltip(this.sent.why ?? "").child(this.sent.answer ? "Not confirmed — look at the question again" : "Not confirmed — check before sending again"))
          .child(Button("chat-unconfirmed-ok", { label: "Dismiss", variant: "ghost", size: "sm", onClick: (_e, cx) => (this.sends.delete(this.target), cx.notify()) })),
      )
      .when(this.sent?.state === "queued", (e) => e.child(Icon("list-ordered", { size: 12 })).child(div().flex_1().min_w(0).truncate().child(`Queued — ${String(this.sent.note).replace(/^queued:? ?/, "")}`)))
      .when(!this.sent && !this.editing && Boolean(this.session()?.paused), (e) => e.child(Icon("circle-pause", { size: 12 })).child(div().flex_1().child("Paused · picks up when you write")))
      .when(!this.sent && Boolean(this.editing), (e) =>
        e
          .child(Icon("pencil", { size: 12 }))
          .child(div().flex_1().child("Editing a queued prompt: sending puts it back in its place"))
          .child(
            Button("chat-edit-cancel", {
              label: "Cancel",
              variant: "ghost",
              size: "sm",
              onClick: (_e, cx) => {
                const edit = this.edits.get(this.target);
                this.edits.delete(this.target);
                this.restoreDraft(edit?.draft);
                cx.notify();
              },
            }),
          ),
      );
  }

  /**
   * The mode an agent in a terminal is in, and its focus view, as its screen and transcript say: a menu that changes the
   * mode by its own ⇧⇥ and a click that types its /focus, where the agent has them; else shown only.
   */
  shownModes(k) {
    const ran = this.value?.session ?? {};
    const can = ran.switch ?? {};
    const out = [];
    const mode = ran.mode ? (MODES[ran.mode] ?? { label: ran.mode, icon: "shield-check", loud: false }) : null;
    if (mode && can.mode)
      out.push(this.picker("chat-mode-shown", { label: mode.label, icon: mode.icon, color: mode.loud ? k.blocked : null, tooltip: "What it may do without asking", items: () => this.modeItems(true) }, k));
    else if (mode) out.push(this.chip("chat-mode-shown", mode.icon, mode.label, "What it may do without asking — ⇧⇥ in its terminal changes it", mode.loud ? k.blocked : null, k));
    if (typeof ran.focus === "boolean" && (ran.focus || can.focus)) {
      const tip = ran.focus ? "Focus view: its terminal shows only its last words" : "Focus view is off: its terminal shows every step";
      out.push(
        this.chip("chat-focus-shown", ran.focus ? "scan-eye" : "eye-off", "Focus", can.focus ? `${tip} — click to turn it ${ran.focus ? "off" : "on"}` : tip, ran.focus ? null : k.dim, k, can.focus ? (cx) => this.terminalAct("focus_mode", {}, cx) : null),
      );
    }
    return out;
  }

  /** A word in the bottom bar with its icon, that does something when clicked or only says something. */
  chip(id, icon, label, tooltip, color, k, onClick = null) {
    return h_flex()
      .id(id)
      .h(26)
      .px(7)
      .gap(5)
      .flex_none()
      .items_center()
      .rounded(RADIUS.md)
      .text_size(TEXT.sm)
      .text_color(color ?? k.mutedFg)
      .tooltip(tooltip)
      .when(Boolean(onClick), (e) => e.cursor_pointer().hover((s) => s.bg(k.accent)).on_click((_e, cx) => (onClick(cx), cx.notify())))
      .child(Icon(icon, { size: 14, color: color ?? k.mutedFg }))
      .child(div().child(label));
  }

  /** A word in the bottom bar that opens a menu: the permission mode, the model. */
  picker(id, { label, icon, logo, color, tooltip, items }, k) {
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
      .when(Boolean(logo), (e) => e.child(AgentLogo(logo, { size: 14, color: color ?? k.mutedFg })))
      .child(div().child(label))
      .child(Icon("chevron-down", { size: 13, color: k.dim }))
      .on_click((_e, cx) => {
        openMenu(typeof items === "function" ? items() : items, { width: 240 });
        cx.notify();
      });
  }

  /** The box: what you type, and under it what the agent may do, which model answers, and the way to send it. */
  composer(k) {
    const live = this.session();
    const row = this.row();
    // Where its work lives, built once a drawing.
    const cited = [...this.documents(k), ...this.links(k)];
    const waiting = Boolean(row?.question);
    const working = live ? live.working === true : row?.status === "working";
    // A driven session stops through its own interrupt; a terminal only where its agent says which keys stop it.
    const stoppable = this.stoppable();
    const empty = !this.input.value().trim();
    const hints = this.hints();
    // Not over the hints: a "/" typed is the box's own.
    const offer = hints.items.length ? null : this.nextOffer();
    const mode = MODES[live?.mode] ?? MODES.default;
    // Its cost is cockpit's estimate from the transcript, as everywhere, not the figure the agent reports.
    const seat = { ...(live ? { cwd: live.cwd } : (this.value?.session ?? {})), cost: row?.cost };
    const agent = this.value?.agent ?? {};
    // What the model picker says it is working with, rather than four more chips in the bar.
    const about = [
      seat.cwd ? `in ${seat.cwd}` : null,
      agent.branch ? `on ${agent.branch}` : null,
      typeof seat.context === "number" ? `${Math.round(seat.context)}% of its context` : null,
      typeof seat.cost === "number" && seat.cost > 0 ? `~$${seat.cost.toFixed(2)} so far, estimated` : null,
    ].filter(Boolean).join(" · ");
    // What the model answers with, where it works, and how much of the agent's limits are used, as its agent reports them.
    const limits = windowsWords(this.model.value?.quota, row?.agent || row?.program);
    // The model, its effort level and fast mode, as its agent offers them; a terminal's from what its transcript says.
    const ran = live ? null : (this.value?.session ?? null);
    const subject = row ?? (live ? {} : null);
    const picks = subject ? pickChips(subject, { live, ran, asked: this.askedFor(subject, ran), about, limits }) : [];
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
      .when(Boolean(this.pasteNote), (e) => e.child(this.pasteLine(k)))
      .when(Boolean(this.sent) || Boolean(this.editing) || Boolean(this.session()?.paused), (e) => e.child(this.sendLine(k)))
      .when(cited.length > 0, (e) => e.child(h_flex().w_full().max_w(READ_W).gap(1).items_center().justify_end().children(cited)))
      .when(Boolean(offer), (e) =>
        e.child(div().w_full().max_w(READ_W).px(4).child(NextSteps(offer, k, { onPick: (prompt, cx) => this.fillNext(prompt, cx), onDismiss: (cx) => (this.dismissNext(offer), cx.notify()) }))),
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
          .child(div().w_full().min_h(46).child((this.multiLine ? base.Textarea : Input).new(this.input).w_full().text_size(TEXT.base).text_color(k.foreground)))
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
                      tooltip: `What it may do without asking  ${keyText("shift-tab")}`,
                      items: () => this.modeItems(),
                    },
                    k,
                  ),
                ),
              )
              .when(!live && Boolean(row), (e) => e.children(this.shownModes(k)))
              .when(!row && !live, (e) => e.children(this.starter.chips((id, o) => this.picker(id, o, k))))
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
              .children(picks.map((c) => this.pickChip(c, k)))
              .when(stoppable, (e) => e.child(bare("chat-stop", "square", `Stop this turn  ${keyText("escape")}`, (_e, cx) => this.stop(cx))))
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
                  .tooltip(!row && !live ? "Start work  ⏎" : waiting ? "Answer it  ⏎" : row?.waits ? "Queue it: it waits at its terminal, the queue sends it once it is free  ⏎" : working ? "Queue it  ⏎" : "Send  ⏎")
                  .on_click((_e, cx) => this.send(cx))
                  .child(Icon("arrow-up", { size: 15 })),
              ),
          ),
      );
  }

  render(cx) {
    const k = tokens();
    const live = this.session();
    // The confirm was asked for one session or terminal: another one in front takes it away.
    if (this.confirm && (live?.key ?? this.row()?.terminal) !== this.confirm.key) this.confirm = null;
    const row = this.row();
    const working = live ? live.working === true : row?.status === "working";
    if (this.wantFiles) this.loadFiles(cx);
    const failed = live ? Boolean(live.failed) : row?.status === "failed";
    const end = { stopped: live ? live.interrupted === true : false, failed, error: live ? live.failed : row?.error };
    // The rows of the conversation are made again only when its messages or its end changed, not on every redraw.
    const messages = this.messagesOf(live) ?? [];
    this.messagesNow = messages;
    const shape = `${working}|${end.stopped}|${end.failed}|${end.error ?? ""}`;
    const memo = this.made;
    const all = memo?.messages === messages && memo.length === messages.length && memo.shape === shape ? memo.rows : rowsOf(messages, { working, ...end });
    this.made = { messages, length: messages.length, shape, rows: all };
    const rows = this.drawn(all);
    const named = (r) => (r.kind === "said" ? String(r.text).split("\n") : r.kind === "answered" ? imagesIn(r.text, home()) : []);
    this.thumbsFor([...this.files, ...rows.flatMap(named)], cx);
    // A turn still running counts its own seconds, so the row is redrawn while nothing else moves: one redraw a second,
    // however often something else draws it. `ticking` is when it was armed: one that never fired is armed again.
    if (working && !(Date.now() - (this.ticking ?? 0) < 2000)) {
      const at = (this.ticking = Date.now());
      // The timer's own context redraws: the drawing's is gone by the time it fires.
      cx.timer.after(1000, (cx) => {
        if (this.ticking === at) this.ticking = 0;
        cx.notify();
      });
    }
    this.settle(rows, cx);
    this.rows = rows;
    this.all = all;
    const unseen = this.unseen();
    const down = Boolean(this.hubWasDown);
    // No agent to read: the empty state, and what is still to set up when cockpit is new here.
    const nobody = () => {
      // The setup checklist is read the first time this empty state shows, not before; the box starts work meanwhile.
      this.started.want(cx);
      return this.starter.body(k, { width: READ_W, gutter: GUTTER, below: this.started.el({ compact: true }) });
    };
    // The hub down, an agent followed draws the hub's state rather than the start form its missing row would give.
    const waiting = (!live && !this.value) || (down && !row && Boolean(this.target) && this.target !== BLANK);
    const body = waiting
      ? v_flex().flex_1().min_h(0).items_center().justify_center().when(!down || !(live || this.value), (e) => e.child(down ? this.hubDown(k) : Spinner({ size: 18, color: k.mutedFg })))
      : !row
        ? nobody()
        : rows.length === 0
          ? v_flex().flex_1().min_h(0).items_center().justify_center().child(Empty({ icon: "message-square", title: "Nothing said yet", description: "Its transcript is empty, or it has only just started." }))
          : div()
              .id("chat-body")
              .relative()
              .flex_1()
              .min_h(0)
              .pt(8)
              .on_scroll_wheel((e, cx) => this.wheeled(e, cx))
              // A press on the scrollbar's strip: the reader takes the view, and where it ends up is not known.
              .on_mouse_down("left", (e, cx) => {
                if (!(e?.local_position && e?.bounds && e.local_position.x > e.bounds.width - 16)) return;
                this.lost = true;
                if (this.pinned) {
                  this.pinned = false;
                  this.windowTop = rows[0]?.ids?.[0] ?? null;
                  this.seenMsg = this.lastMessage();
                  cx.notify();
                }
              })
              .child(
                ((area) => (this.scroll ? area.track_scroll(this.scroll) : area))(
                  v_flex()
                    .id("chat-rows")
                    .size_full()
                    .overflow_y_scrollbar()
                    .on_size_change((size) => (this.viewH = size.height)),
                )
                  .children(rows.map((r) => div().id(`row-${r.key}`).w_full().on_size_change((size, cx) => this.measured(r.key, size.height, cx)).child(this.rowEl(r, k))))
                  // The end of the conversation: revealing it shows the bottom of the last row, however tall that is.
                  .child(div().id("chat-end").w_full().h(1)),
              )
              .child(Scrollbar.vertical("chat-rows").absolute().inset_0())
              .when((this.hasEarlier() || all[0]?.key !== rows[0]?.key) && (this.loadingEarlier || this.maxTop() <= 0 || (!this.pinned && this.top <= 40)), (e) => e.child(this.earlierPill(k)))
              .when(!this.pinned, (e) => e.child(this.newPill(k, unseen)));
    return v_flex()
      .id("chat")
      .relative()
      .size_full()
      .on_size_change((size, cx) => {
        if ((size.width < 820) !== ((this.width ?? 1000) < 820)) cx.notify();
        this.width = size.width;
      })
      .bg(k.canvas)
      .text_color(k.foreground)
      .track_focus(this.handle)
      .key_context(
        chatContext({
          front: inFront("chat"),
          holds: this.boxFocused || this.handle.is_focused() || this.cardFocus.is_focused() || this.cancelFocus.is_focused(),
          hints: this.hints().items.length > 0,
          question: Boolean(row && this.model.shown(row)),
          live: Boolean(live),
          closes: Boolean(this.ending || this.handing || this.stoppable()),
          confirm: Boolean(this.confirm),
          next: this.nextKeys(),
        }),
      )
      .on_action("cockpit_chat_tab", (_e, cx) => this.action("tab", cx))
      .on_action("cockpit_chat_cycle", (_e, cx) => this.action("shift-tab", cx))
      .on_action("cockpit_chat_escape", (_e, cx) => this.action("escape", cx))
      .on_action("cockpit_chat_complete", (_e, cx) => this.action("enter", cx))
      .on_action("cockpit_chat_next_1", (_e, cx) => this.action("1", cx))
      .on_action("cockpit_chat_next_2", (_e, cx) => this.action("2", cx))
      .on_action("cockpit_chat_next_3", (_e, cx) => this.action("3", cx))
      .on_action("cockpit_chat_next_0", (_e, cx) => this.action("0", cx))
      .on_key_up((e) => this.released(e))
      .when(Boolean(row), (e) => e.child(this.header(k, row, live)))
      .when(Boolean(this.ending) && this.ending.key === live?.key, (e) =>
        e.child(this.askLine(k, { id: "chat-ending", text: "It is working: ending it stops this turn too.", keep: "Keep it", go: "End it", onKeep: () => (this.ending = null), onGo: (cx) => this.end(cx, true) })),
      )
      .when(Boolean(this.handing) && this.handing.key === live?.key, (e) =>
        e.child(
          this.askLine(k, {
            id: "chat-handing",
            text: live?.working ? "It is working: stop its turn before it moves to a terminal." : "It leaves this chat and goes on in a terminal of its own.",
            keep: "Keep it here",
            go: "Open it in a terminal",
            onKeep: () => (this.handing = null),
            onGo: (cx) => this.handover(cx, true),
          }),
        ),
      )
      .when(Boolean(this.merging) && this.merging.url === row?.pr?.url, (e) =>
        e.child(
          this.askLine(k, {
            id: "chat-merging",
            text: this.merging.busy ? `${this.merging.verb}…` : this.merging.question,
            keep: "Keep it",
            go: this.merging.verb,
            onKeep: () => (this.merging = null),
            onGo: (cx) => this.merge(cx),
          }),
        ),
      )
      .when(down && Boolean(live || this.value), (e) => e.child(h_flex().w_full().px(GUTTER).pt(6).justify_center().child(div().w_full().max_w(READ_W).child(this.hubDown(k)))))
      .on_mouse_down("left", (_e, cx) => ((this.pointed = false), cx.notify()))
      .on_key_down((e, cx) => {
        if (this.pressed(e, cx)) {
          cx.stop_propagation();
          cx.notify();
        }
      })
      .child(body)
      .children(this.queuedRows(k))
      // A dismissal's Undo, above the card it came from: the chat keeps key hints out of it, as everywhere here.
      .when(Boolean(UndoRow("chat-undo", this.model)), (e) => e.child(h_flex().w_full().px(GUTTER).pt(8).justify_center().child(div().w_full().max_w(READ_W).child(UndoRow("chat-undo", this.model)))))
      .when(Boolean(row?.question?.view), (e) => (this.planInPanel(row, cx), e))
      .when(Boolean(row?.question || row?.waits), (e) =>
        e.child(h_flex().w_full().px(GUTTER).pt(8).justify_center().child(div().w_full().max_w(READ_W).child(QuestionCard(`chat-card-${row.terminal}-${row.question?.ask ?? row.question?.text?.length ?? ""}`, row, this.model, { wide: true, focus: this.cardFocus })))),
      )
      .child(this.composer(k))
      .when(Boolean(this.confirm), (e) => e.child(this.fullAccess(k, this.confirm)));
  }

  /**
   * A plan the agent asks about opens once in the side panel, where it reads whole and stays after it is carried out;
   * the card above the box keeps it to one line.
   */
  planInPanel(row, cx) {
    const view = row.question.view;
    if (this.planOpened === view) return;
    this.planOpened = view;
    cx.timer.after(0, () => {
      pendView(row, view);
      openPanel("show");
      run(`cockpit.show:go:${view}`);
    });
  }

  /** At the top of the conversation when there is more before it: reads further back, the first row staying put. */
  earlierPill(k) {
    return h_flex()
      .absolute()
      .top(6)
      .left(0)
      .right(0)
      .justify_center()
      .child(
        h_flex()
          .id("chat-earlier")
          .gap(6)
          .items_center()
          .px(12)
          .py(4)
          .rounded(13)
          .cursor_pointer()
          .text_size(TEXT.xs)
          .text_color(k.mutedFg)
          .bg(k.card)
          .border_1()
          .border_color(k.border)
          .shadow_sm()
          .hover((s) => s.text_color(k.foreground))
          .tooltip("Read further back in this conversation")
          .on_click((_e, cx) => this.earlier(cx))
          .child(this.loadingEarlier ? Spinner({ size: 11, color: k.mutedFg }) : Icon("arrow-up", { size: 12 }))
          .child(div().child(this.loadingEarlier ? "Reading further back…" : "Earlier messages")),
      );
  }

  /** Away from the end: how many rows came since, and the way back down. */
  newPill(k, unseen) {
    return h_flex()
      .absolute()
      .bottom(10)
      .left(0)
      .right(0)
      .justify_center()
      .child(
        h_flex()
          .id("chat-to-end")
          .h(30)
          .gap(6)
          .items_center()
          .justify_center()
          .when(unseen > 0, (e) => e.px(12))
          .when(unseen === 0, (e) => e.w(30))
          .rounded(15)
          .cursor_pointer()
          .bg(k.card)
          .border_1()
          .border_color(k.border)
          .shadow_sm()
          .text_size(TEXT.xs)
          .text_color(unseen > 0 ? k.foreground : k.mutedFg)
          .hover((s) => s.text_color(k.foreground))
          .tooltip("The newest message")
          .on_click((_e, cx) => this.toEnd(cx))
          .when(unseen > 0, (e) => e.child(div().child(`${unseen} new`)))
          .child(Icon("arrow-down", { size: 13 })),
      );
  }

  /** The confirm before Full access: what it lets the agent do, Cancel, and the one action. Keys in the tooltips. */
  fullAccess(k, c) {
    const full = c.mode === "bypassPermissions";
    const edits = c.mode === "acceptEdits";
    return div()
      .id("chat-confirm")
      .absolute()
      .inset_0()
      .bg("#00000066")
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(
        v_flex()
          .size_full()
          .items_center()
          .justify_center()
          .child(
            Card({
              id: "chat-confirm-card",
              title: full ? "Give it full access?" : edits ? "Let it edit files without asking?" : "Let it run tools without asking?",
              description: `“${c.title}”${c.folder ? `, in ${c.folder},` : ""} will ${edits ? "change files" : "run commands and change files"} without asking you first, until you set it back.`,
              padding: 24,
              gap: 16,
              footer: h_flex()
                .justify_end()
                .gap(8)
                .child(Button("chat-confirm-cancel", { label: "Cancel", variant: "ghost", tooltip: "Keep it as it is  esc", onClick: (_e, cx) => this.confirmed(false, cx) }).track_focus(this.cancelFocus))
                .child(Button("chat-confirm-ok", { label: full ? "Give full access" : edits ? "Let it edit" : "Let it run", tooltip: "It stops asking", onClick: (_e, cx) => this.confirmed(true, cx) })),
            })
              .w(420)
              .bg(k.popover)
              .shadow_lg(),
          ),
      );
  }
}
