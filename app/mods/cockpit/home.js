// Home, the first screen: today's numbers, the next move (the longest-waiting question, answered right there), the
// agents and PRs in four columns by whose move it is (Needs you, Working, Ready to land, Done), the agents a restart
// stopped, and the week's delivery. Built after jkuepker's Board; the agents come from the hub's `agents` view joined
// with Herdr's live list, the PRs and numbers from the `home` view.

import { div, svg, image } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { run, spaces, exec, knownAgents, setShared, layout, readTextFile } from "herdr";
import { show } from "./sidebar.js";
import { findNode } from "./status.js";
import * as hub from "./hub.js";
import { fleet, goToAgent, agentAct } from "./fleet.js";
import {
  tokens, TEXT, RADIUS, alpha, Icon, Spinner, Badge, Button, Kbd, TextField, Toaster, Flashes, Hover, AlertDialog, openMenu,
  Progress, Mono, dur, Chip, CountPill, StageHeading, Metric, Callout, ToneButton, Skeleton,
} from "./ui.js";

const DEFAULT_PINNED = ["inbox", "to-do", "runbook", "meetings"];
const many = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;

/** Home's overview: each pinned app in a few words, from the view the app itself draws. */
const OVERVIEW = [
  {
    view: "inbox", space: "inbox", label: "Inbox", icon: "inbox",
    line: (v) => {
      const c = v.counts ?? {};
      if (c.failing) return { text: `${many(c.failing, "PR")} failing`, tone: "destructive" };
      return { text: c.direct ? `${many(c.direct, "review")} for you` : "nothing for you" };
    },
  },
  { view: "meetings", space: "meetings", label: "Next", icon: "calendar", line: (v) => ({ text: v.banner?.title ? `${v.banner.title} · ${v.banner.when}` : "no meeting", tone: v.banner?.soon ? "warning" : null }) },
  { view: "runbook", space: "runbook", label: "Runbooks", icon: "book", line: (v) => (v.books?.length ? { text: `${many(v.left ?? 0, "step")} left` } : null) },
  { view: "todo", space: "to-do", label: "To-do", icon: "list-todo", line: (v) => ({ text: v.open ? `${v.open} open` : "all done" }) },
  { view: "monitor", space: "monitor", label: "Prod", icon: "activity", line: (v) => (v.headline ? { text: v.headline, tone: v.bad ? "destructive" : null } : null) },
  {
    view: "routines", space: "routines", label: "Routines", icon: "clock",
    line: (v) => {
      const t = v.tiles ?? {};
      if (t.unread) return { text: `${many(t.unread, "run")} to look at`, tone: t.failing ? "destructive" : "warning" };
      return { text: t.next ? `next ${t.next}` : "none scheduled" };
    },
  },
];

const COLUMNS = [
  { stage: "needs", label: "Needs you", tone: "blocked", pulse: true },
  { stage: "working", label: "Working", tone: "working", spin: true },
  { stage: "ready", label: "Ready to land", tone: "success" },
  { stage: "done", label: "Done", tone: "done" },
];

const plural = (n, word) => `${n} ${word}${n === 1 ? "" : "s"}`;
const money = (usd) => (usd === null || usd === undefined ? "–" : usd >= 100 ? `$${Math.round(usd)}` : `$${usd.toFixed(2)}`);
const hours = (h) => (h === null || h === undefined ? "–" : h < 1 ? `${Math.max(1, Math.round(h * 60))}m` : h < 48 ? `${h.toFixed(h < 10 ? 1 : 0)}h` : `${Math.round(h / 24)}d`);

function greeting(now) {
  const h = now.getHours();
  return h < 5 ? "Good night" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
}

export default class Home {
  init(_props, cx) {
    this.rows = null;
    this.home = null;
    this.queue = null;
    this.status = "connecting";
    this.filter = null;
    this.sel = null;
    this.focusMode = false;
    this.width = 1200;
    this.now = Date.now();
    this.busy = new Map();
    this.picks = new Map();
    this.confirm = null;
    this.showArchived = false;
    this.stages = new Map();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.hover = new Hover();
    this.handle = cx.focus_handle();
    this.reply = InputState.new({ placeholder: "Type your answer, ⏎ sends it to the agent" });
    this.reply.on("submit", (_e, cx) => this.sendReply(cx));
    this.replying = null;
    cx.timer.every(30000, (cx) => {
      this.now = Date.now();
      cx.notify();
    });
    this.apps = {};
    this.pinned = null;
    cx.spawn(async (cx) => {
      try {
        const { cache, home } = await hub.install();
        const text = (await readTextFile(`${cache}/config.json`)) ?? (await readTextFile(`${home}/.config/cockpit/config.json`));
        const spaces = text ? JSON.parse(text).spaces : null;
        if (Array.isArray(spaces)) this.pinned = spaces.map((s) => (typeof s === "string" ? s : s.name));
      } catch {}
      cx.notify();
    });
    this.sub = hub.subscribe(
      cx,
      { views: ["agents", "home", "queue", ...OVERVIEW.map((o) => o.view)] },
      (m, cx) => {
        if (OVERVIEW.some((o) => o.view === m.view)) this.apps[m.view] = m.value;
        if (m.view === "agents") this.rows = m.value;
        if (m.view === "home") this.home = m.value;
        if (m.view === "queue") this.queue = m.value;
        this.now = Date.now();
        cx.notify();
      },
      (state, cx) => {
        this.status = state;
        cx.notify();
      },
    );
  }

  close() {
    this.sub?.stop();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The model: cards per column

  cards() {
    const agents = fleet(this.rows, this.home);
    const settled = new Set(spaces().flatMap((s) => s.tabs.filter((t) => t.settled).map((t) => t.id)));
    const branches = new Set(agents.filter((a) => a.branch).map((a) => `${a.repo ?? ""}/${a.branch}`));
    const covered = (pr) => branches.has(`${pr.repo}/${pr.branch}`) || agents.some((a) => a.branch && a.branch === pr.branch);
    const agentCard = (a) => ({ kind: "agent", key: `a:${a.address}`, stage: a.stage, project: a.repo ?? a.space_name, agent: a, archived: settled.has(a.tab) });
    const prCard = (pr, stage) => ({ kind: "pr", key: `p:${pr.url}`, stage, project: pr.repo, pr, agent: agents.find((a) => a.branch && a.branch === pr.branch) ?? null });
    const all = [
      ...agents.map(agentCard),
      ...(this.home?.fixing ?? []).filter((pr) => !agents.some((a) => a.branch === pr.branch && a.stage === "needs")).map((pr) => prCard(pr, "needs")),
      ...(this.home?.ready ?? []).filter((pr) => !covered(pr)).map((pr) => prCard(pr, "ready")),
    ];
    for (const c of all) {
      const before = this.stages.get(c.key);
      if (before && before !== c.stage) this.flashLater(c.key, c.stage);
      this.stages.set(c.key, c.stage);
    }
    return all;
  }

  flashLater(key, stage) {
    const k = tokens();
    const color = { needs: k.blocked, working: k.working, ready: k.success, done: k.done }[stage];
    this.pendingFlash = [...(this.pendingFlash ?? []), [key, color]];
  }

  columns(all) {
    const shown = all.filter((c) => !this.filter || c.project === this.filter);
    const by = (stage) => shown.filter((c) => c.stage === stage && !(stage === "done" && c.archived && !this.showArchived));
    const since = (c) => c.agent?.since ?? 0;
    return {
      needs: by("needs").sort((a, b) => (a.kind === b.kind ? since(a) - since(b) : a.kind === "agent" ? -1 : 1)),
      working: by("working").sort((a, b) => (a.agent?.working ?? 0) - (b.agent?.working ?? 0)),
      ready: by("ready"),
      done: by("done").sort((a, b) => since(b) - since(a)),
      archived: shown.filter((c) => c.stage === "done" && c.archived).length,
    };
  }

  flat(cols) {
    return COLUMNS.flatMap((c) => cols[c.stage]);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async act(cx, key, label, fn) {
    this.busy.set(key, label);
    cx.notify();
    try {
      const r = await fn();
      if (r?.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r?.note) this.toasts.push(cx, r.note, { kind: "success" });
      return r;
    } catch (e) {
      this.toasts.push(cx, String(e?.message ?? e), { kind: "error" });
      return { error: String(e) };
    } finally {
      this.busy.delete(key);
      cx.notify();
    }
  }

  answer(cx, a, choice) {
    const q = a.question;
    cx.spawn(async (cx) => {
      const r = await this.act(cx, `a:${a.address}`, "Answering…", () => agentAct("answer", { terminal: a.handle, ...choice }));
      if (r?.error) this.openBeside(cx, a);
      else this.flashes.start(cx, `a:${a.address}`, tokens().success);
      this.picks.delete(a.address);
      if (q && this.replying === a.address) this.replying = null;
    });
  }

  sendReply(cx) {
    const a = this.agentByAddress(this.replying);
    const text = this.reply.value().trim();
    if (!a || !text) return;
    this.reply.set_value("");
    this.answer(cx, a, { text });
  }

  agentByAddress(address) {
    return fleet(this.rows, this.home).find((a) => a.address === address) ?? null;
  }

  openBeside(cx, a) {
    if (!a) return;
    run("agent.focus", { terminal: a.address });
    run("layout.show", { id: "home-agent" });
    this.focusMode = true;
    this.sel = `a:${a.address}`;
    cx.notify();
  }

  closeBeside(cx) {
    run("layout.hide", { id: "home-agent" });
    this.focusMode = false;
    this.handle.focus();
    cx.notify();
  }

  toggleFocus(cx, cols) {
    if (this.focusMode) return this.closeBeside(cx);
    const card = this.selected(cols);
    const a = card?.agent ?? cols.needs.find((c) => c.agent)?.agent;
    if (a) this.openBeside(cx, a);
    else this.toasts.push(cx, "No agent to open beside the board", { kind: "info" });
  }

  merge(cx, pr) {
    this.confirm = {
      title: `Merge ${pr.repo}#${pr.number}?`,
      description: `${pr.title} · squash into ${pr.base || "main"}`,
      label: "Merge",
      run: (cx) => cx.spawn(async (cx) => this.act(cx, `p:${pr.url}`, "Merging…", () => hub.act("inbox", "merge", [{ pr: pr.url, method: "squash" }]))),
    };
    cx.notify();
  }

  archive(cx, a) {
    run("tab.settle", { id: a.tab });
    this.toasts.push(cx, `Archived ${a.title}`, { kind: "success", description: "Its tab keeps running, folded away.", action: { label: "Undo", onClick: () => run("tab.unsettle", { id: a.tab }) } });
  }

  archiveAll(cx, cards) {
    const tabs = [...new Set(cards.filter((c) => c.agent && !c.archived).map((c) => c.agent.tab))];
    for (const id of tabs) run("tab.settle", { id });
    this.toasts.push(cx, `Archived ${plural(tabs.length, "agent")}`, { kind: "success", action: { label: "Undo", onClick: () => tabs.forEach((id) => run("tab.unsettle", { id })) } });
  }

  openUrl(cx, url) {
    if (url) cx.spawn(async () => exec("open", [url]));
  }

  newAgent(cx) {
    cx.spawn(async () => {
      const known = (await knownAgents().catch(() => [])).find((k) => k.installed);
      run("tab.new", known ? { argv: known.run } : {});
      run("layout.select", { stack: "center", child: "work" });
      setShared("cockpit.app", "work");
    });
  }

  menu(card) {
    const a = card.agent;
    const pr = card.pr ?? a?.pr;
    const id = card.key;
    openMenu([
      a && { label: "Go to agent", icon: "terminal", kbd: "⏎", command: `home:go:${id}` },
      a && { label: "Open beside the board", icon: "panel-right", kbd: "f", command: `home:beside:${id}` },
      a && { label: "Recap", icon: "sparkles", kbd: "r", command: `home:recap:${id}` },
      a && { label: "Queue a prompt", icon: "list-ordered", kbd: "q", command: `home:queue:${id}` },
      a && "separator",
      a && a.status === "blocked" && { label: "Snooze for an hour", icon: "moon", kbd: "s", command: `home:snooze:${id}` },
      a && { label: "Mark as seen", icon: "eye", command: `home:seen:${id}` },
      a && { label: "Fork to a new tab", icon: "git-fork", command: `home:fork:${id}` },
      pr?.url && { label: "Open the PR", icon: "git-pull-request", kbd: "o", command: `home:pr:${id}` },
      card.stage === "ready" && pr?.url && { label: "Merge…", icon: "git-merge", kbd: "m", command: `home:merge:${id}` },
      a && card.stage === "done" && "separator",
      a && card.stage === "done" && { label: "Archive", icon: "archive", kbd: "a", command: `home:archive:${id}` },
    ]);
  }

  menuAction(action, key, cx) {
    const card = this.flat(this.columns(this.cards())).find((c) => c.key === key) ?? this.cards().find((c) => c.key === key);
    if (!card) return;
    this.sel = key;
    this.run(action, card, cx);
  }

  run(action, card, cx) {
    const a = card.agent;
    const pr = card.pr ?? a?.pr;
    if (action === "go" && a) goToAgent(a.address);
    else if (action === "go" && pr?.url) this.openUrl(cx, pr.url);
    else if (action === "beside" && a) this.openBeside(cx, a);
    else if (action === "recap" && a) cx.spawn(async (cx) => this.act(cx, card.key, "Recap…", () => agentAct("recap", { terminal: a.handle })));
    else if (action === "snooze" && a) cx.spawn(async (cx) => this.act(cx, card.key, "Snoozing…", () => agentAct("snooze", { terminal: a.handle, until: "1h" })));
    else if (action === "seen" && a) cx.spawn(async (cx) => this.act(cx, card.key, "…", () => agentAct("seen", { terminal: a.handle })));
    else if (action === "fork" && a) cx.spawn(async (cx) => this.act(cx, card.key, "Forking…", () => agentAct("fork", { terminal: a.handle, place: "tab" })));
    else if (action === "queue" && a) {
      setShared("cockpit.queue.for", a.address);
      run("cockpit.queue", {});
    }
    else if (action === "pr" && pr?.url) this.openUrl(cx, pr.url);
    else if (action === "merge" && pr?.url) this.merge(cx, pr);
    else if (action === "archive" && a) this.archive(cx, a);
    cx.notify();
  }

  command(c, cx) {
    if (c === "show:home" || c === "focus") this.handle.focus();
    else if (c === "escape" && this.focusMode) this.closeBeside(cx);
    const [scope, action, ...rest] = c.split(":");
    if (scope === "home") this.menuAction(action, rest.join(":"), cx);
    cx.notify();
  }

  selected(cols) {
    const flat = this.flat(cols);
    return flat.find((c) => c.key === this.sel) ?? null;
  }

  move(cols, dy, dx) {
    const lists = COLUMNS.map((c) => cols[c.stage]);
    let ci = lists.findIndex((l) => l.some((c) => c.key === this.sel));
    if (ci < 0) {
      const first = lists.find((l) => l.length);
      this.sel = first?.[0]?.key ?? null;
      return;
    }
    let ri = lists[ci].findIndex((c) => c.key === this.sel);
    if (dx) {
      let next = ci + dx;
      while (next >= 0 && next < lists.length && !lists[next].length) next += dx;
      if (next < 0 || next >= lists.length) return;
      ci = next;
      ri = Math.min(ri, lists[ci].length - 1);
    } else ri = Math.max(0, Math.min(lists[ci].length - 1, ri + dy));
    this.sel = lists[ci][ri]?.key ?? this.sel;
  }

  key(e, cx) {
    const k = e.keystroke;
    if (this.confirm) {
      if (k === "enter" || k === "y") {
        const c = this.confirm;
        this.confirm = null;
        c.run(cx);
      } else if (k === "escape" || k === "n") this.confirm = null;
      else return false;
      cx.notify();
      return true;
    }
    if (this.replying) {
      if (k === "escape") {
        this.replying = null;
        this.handle.focus();
        cx.notify();
        return true;
      }
      return false;
    }
    const cols = this.columns(this.cards());
    // Nothing picked yet: the keys act on the NEXT MOVE agent, as its hints say.
    const card = this.selected(cols) ?? cols.needs.find((c) => c.agent) ?? null;
    const a = card?.agent;
    if (k === "j" || k === "down") this.move(cols, 1, 0);
    else if (k === "k" || k === "up") this.move(cols, -1, 0);
    else if (k === "l" || k === "right") this.move(cols, 0, 1);
    else if (k === "h" || k === "left") this.move(cols, 0, -1);
    else if (k === "f") this.toggleFocus(cx, cols);
    else if (k === "escape" && this.focusMode) this.closeBeside(cx);
    else if (k === "escape" && this.filter) this.filter = null;
    else if (k === "enter" && card) this.run("go", card, cx);
    else if (/^[1-9]$/.test(k) && (a?.question?.choices?.length ?? 0) >= Number(k)) this.pick(cx, a, Number(k) - 1);
    else if (k === "t" && a?.question) this.startReply(cx, a);
    else if (k === "m" && card?.stage === "ready") this.run("merge", card, cx);
    else if (k === "a" && card?.stage === "done") this.run("archive", card, cx);
    else if (k === "o" && card) this.run("pr", card, cx);
    else if (k === "r" && a) this.run("recap", card, cx);
    else if (k === "s" && a) this.run("snooze", card, cx);
    else if (k === "q" && a) this.run("queue", card, cx);
    else if (k === "n") this.newAgent(cx);
    else return false;
    cx.notify();
    return true;
  }

  pick(cx, a, i) {
    const q = a.question;
    const c = q.choices[i];
    if (!c) return;
    if (q.kind === "multi") {
      const set = new Set(this.picks.get(a.address) ?? q.choices.filter((x) => x.checked).map((x) => x.n));
      set.has(c.n) ? set.delete(c.n) : set.add(c.n);
      this.picks.set(a.address, set);
      cx.notify();
    } else if (c.free) this.startReply(cx, a);
    else this.answer(cx, a, { choice: c.n });
  }

  startReply(cx, a) {
    this.replying = a.address;
    this.sel = `a:${a.address}`;
    this.reply.focus();
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  render(cx) {
    const k = tokens();
    if (this.pendingFlash?.length) {
      const pending = this.pendingFlash;
      this.pendingFlash = [];
      cx.timer.after(1, (cx) => pending.forEach(([key, color]) => this.flashes.start(cx, key, color)));
    }
    const all = this.cards();
    const cols = this.columns(all);
    if (!this.sel || !this.flat(cols).some((c) => c.key === this.sel)) this.sel = this.flat(cols)[0]?.key ?? null;
    const beside = findNode(layout(), "home-agent");
    this.focusMode = Boolean(beside && !beside.hidden);
    const wide = this.width >= 1240;
    const narrow = this.width < 720;
    const loading = this.status !== "live" && !this.home;

    return div()
      .id("home-root")
      .size_full()
      .relative()
      .bg(k.background)
      .track_focus(this.handle)
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .on_mouse_down("left", () => {
        if (!this.replying) this.handle.focus();
      })
      .on_size_change((size, cx) => {
        if (Math.abs(size.width - this.width) > 1) {
          this.width = size.width;
          cx.notify();
        }
      })
      .child(
        v_flex()
          .id("home-scroll")
          .size_full()
          .overflow_y_scrollbar()
          .child(
            v_flex()
              .w_full()
              .pl(narrow ? 16 : 28)
              .pr((narrow ? 16 : 28) + ((this.queue?.count ?? 0) > 0 ? 44 : 0))
              .pt(22)
              .pb(28)
              .gap(18)
              .child(this.header(all, cols, narrow))
              .when(!loading, (e) => e.child(this.overview()))
              .when(loading, (e) => e.child(this.skeleton()))
              .when(Boolean(this.home?.lost?.length), (e) => e.child(this.lostBanner(narrow)))
              .child(this.nextMove(cols, narrow))
              .child(this.board(cols, wide, narrow)),
          ),
      )
      .when(Boolean(this.confirm), (e) =>
        e.child(
          AlertDialog("home-confirm", {
            title: this.confirm?.title,
            description: this.confirm?.description,
            confirmLabel: this.confirm?.label,
            onConfirm: (_e, cx) => {
              const c = this.confirm;
              this.confirm = null;
              c?.run(cx);
              cx.notify();
            },
            onCancel: (_e, cx) => {
              this.confirm = null;
              cx.notify();
            },
          }),
        ),
      )
      .child(this.toasts.render() ?? div());
  }

  header(all, cols, narrow) {
    const k = tokens();
    const now = new Date(this.now);
    const needs = cols.needs.length;
    const parts = [
      needs ? `${plural(needs, "thing")} ${needs === 1 ? "needs" : "need"} you` : "nothing needs you",
      `${cols.working.length} working`,
      cols.ready.length ? `${cols.ready.length} ready to land` : null,
    ].filter(Boolean);
    const date = now.toDateString().replace(/^(\w+) (\w+) 0?(\d+) \d+$/, "$1 $3 $2");
    return h_flex()
      .w_full()
      .gap(12)
      .items_end()
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(4)
          .child(h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Icon("sun", { size: 14 })).child(date))
          .child(div().text_size(narrow ? 20 : 24).font_semibold().text_color(k.foreground).child(greeting(now)))
          .child(div().truncate().text_size(TEXT.base).text_color(k.mutedFg).child(parts.join(" · "))),
      )
      .when(this.status.startsWith("error"), (e) => e.child(Badge("hub offline", "destructive", { dot: k.destructive })))
      .child(
        h_flex()
          .gap(8)
          .flex_none()
          .child(Button("home-new", { label: narrow ? null : "New agent", icon: "plus", variant: "outline", size: narrow ? "icon-sm" : "sm", kbd: narrow ? null : "n", tooltip: "Start an agent in a new tab  n", onClick: (_e, cx) => this.newAgent(cx) }))
          .child(
            Button("home-focus", {
              label: narrow ? null : this.focusMode ? "Close beside" : "Focus",
              icon: this.focusMode ? "x" : "scan-eye",
              variant: this.focusMode ? "default" : "outline",
              size: narrow ? "icon-sm" : "sm",
              kbd: narrow ? null : "f",
              tooltip: "The selected agent's terminal beside the board, the rest faded  f",
              onClick: (_e, cx) => this.toggleFocus(cx, cols),
            }),
          ),
      );
  }

  /** One short line per pinned app, a click away. */
  overview() {
    const k = tokens();
    const items = OVERVIEW.filter((o) => (this.pinned ?? DEFAULT_PINNED).includes(o.space))
      .map((o) => ({ ...o, line: this.apps[o.view] ? o.line(this.apps[o.view]) : null }))
      .filter((o) => o.line);
    return h_flex()
      .id("home-overview")
      .flex_wrap()
      .gap(8)
      .children(
        items.map((o) =>
          h_flex()
            .id(`ov-${o.view}`)
            .h(30)
            .px(10)
            .gap(7)
            .items_center()
            .rounded(RADIUS.md)
            .border_1()
            .border_color(k.border)
            .cursor_pointer()
            .hover((s) => s.bg(k.accent))
            .on_click(() => show(o.view))
            .tooltip(`Open ${o.label}`)
            .child(Icon(o.icon, { size: 14, color: o.line.tone ? k[o.line.tone] : k.mutedFg }))
            .child(div().text_size(TEXT.sm).font_medium().text_color(k.foreground).child(o.label))
            .child(div().text_size(TEXT.sm).text_color(o.line.tone ? k[o.line.tone] : k.mutedFg).child(o.line.text)),
        ),
      );
  }

  skeleton() {
    return h_flex().w_full().gap(12).children([0, 1, 2, 3, 4].map(() => v_flex().flex_1().gap(8).p(16).rounded(RADIUS.lg).child(Skeleton({ h: 12, w: 80 })).child(Skeleton({ h: 28, w: 48 }))));
  }

  lostBanner(narrow) {
    const k = tokens();
    const lost = this.home?.lost ?? [];
    const name = (e) => e.name || String(e.cwd ?? "").split("/").pop() || "agent";
    return Callout("home-lost", {
      tone: k.info,
      label: "Restart",
      icon: "rotate-ccw",
      stack: narrow,
      title: `${plural(lost.length, "agent")} stopped by the restart`,
      meta: "Herdr brought their terminals back as shells: resume each conversation where it was.",
      body: h_flex()
        .flex_wrap()
        .gap(6)
        .pt(4)
        .children(
          lost.map((e) =>
            Chip(`↻ ${name(e)}`, {
              id: `lost-${e.handle}`,
              color: k.info,
              tooltip: `${e.cwd}\nclaude --resume ${e.session}\nclick to resume, right-click to leave it stopped`,
              onClick: (_ev, cx) => cx.spawn(async (cx) => this.act(cx, `l:${e.handle}`, "Resuming…", () => hub.act("home", "resume", [e.handle]))),
            }).on_mouse_down("right", (_ev, cx) => cx.spawn(async () => hub.act("home", "dismiss", [e.handle]))),
          ),
        ),
      actions: [
        Button("lost-dismiss", { label: "Leave stopped", variant: "ghost", size: "sm", onClick: (_e, cx) => cx.spawn(async () => Promise.all(lost.map((e) => hub.act("home", "dismiss", [e.handle])))) }),
        ToneButton("lost-resume", { label: "Resume all", tone: k.info, kbd: "⌘⌥R", onClick: (_e, cx) => cx.spawn(async (cx) => this.act(cx, "lost", "Resuming…", () => hub.act("home", "resume", []))) }),
      ],
    });
  }

  nextMove(cols, narrow) {
    const k = tokens();
    const ask = cols.needs.find((c) => c.agent);
    if (ask) {
      const a = ask.agent;
      const q = a.question;
      const meta = [a.repo ?? a.space_name, a.branch, a.since ? `waiting ${dur(this.now - a.since)}` : null, a.ticket].filter(Boolean).join(" · ");
      const replying = this.replying === a.address;
      const choices = q?.choices ?? [];
      const body =
        replying
          ? h_flex().gap(8).pt(6).child(TextField(this.reply, { leading: Icon("message", { size: 14, color: k.mutedFg }) })).child(Button("reply-send", { label: "Send", icon: "send", size: "sm", onClick: (_e, cx) => this.sendReply(cx) }))
          : choices.length
            ? this.choiceRow(a, "next", 6)
            : null;
      return Callout("home-next", {
        tone: k.blocked,
        label: "Next move",
        icon: "hand",
        stack: narrow,
        title: q?.text ? `${a.title} asks: “${q.text}”` : `${a.title} is waiting for you`,
        meta,
        body: body ?? undefined,
        actions: [
          !replying && Button("next-reply", { label: narrow ? null : "Reply", icon: "message", variant: "outline", size: narrow ? "icon-sm" : "sm", kbd: narrow ? null : "t", tooltip: "Type an answer here  t", onClick: (_e, cx) => this.startReply(cx, a) }),
          Button("next-beside", { label: narrow ? null : "Open beside", icon: "panel-right", variant: "outline", size: narrow ? "icon-sm" : "sm", tooltip: "Its terminal beside the board  f", onClick: (_e, cx) => this.openBeside(cx, a) }),
          ToneButton("next-go", { label: "Go to agent", tone: k.blocked, kbd: "⏎", onClick: () => goToAgent(a.address) }),
        ].filter(Boolean),
      });
    }
    const fix = cols.needs.find((c) => c.kind === "pr");
    if (fix) {
      const pr = fix.pr;
      const why = pr.failed?.length ? `${pr.failed.join(", ")} failing` : pr.review === "CHANGES_REQUESTED" ? "changes requested" : "CI failing";
      return Callout("home-next", {
        tone: k.destructive,
        label: "Next move",
        icon: "wrench",
        stack: narrow,
        title: `Fix ${pr.repo}#${pr.number}: ${why}`,
        meta: `${pr.title} · ${pr.branch}`,
        actions: [
          Button("next-open", { label: "Open PR", icon: "external", variant: "outline", size: "sm", onClick: (_e, cx) => this.openUrl(cx, pr.url) }),
          ToneButton("next-fix", { label: "Fix with Claude", tone: k.destructive, onClick: (_e, cx) => cx.spawn(async (cx) => this.act(cx, fix.key, "Starting…", () => hub.act("inbox", "fix_ci", [pr.url]))) }),
        ],
      });
    }
    const ready = cols.ready[0];
    if (ready) {
      const pr = ready.pr ?? ready.agent?.pr;
      return Callout("home-next", {
        tone: k.success,
        label: "Next move",
        icon: "git-merge",
        stack: narrow,
        title: pr ? `Land ${pr.repo ?? ""}#${pr.number}: ${pr.title ?? ""}` : `${ready.agent?.title} is ready to land`,
        meta: "Approved, every check green.",
        actions: [pr?.url && Button("next-open", { label: "Open PR", icon: "external", variant: "outline", size: "sm", onClick: (_e, cx) => this.openUrl(cx, pr.url) }), pr?.url && ToneButton("next-merge", { label: "Merge", tone: k.success, kbd: "m", onClick: (_e, cx) => this.merge(cx, pr) })].filter(Boolean),
      });
    }
    return Callout("home-next", {
      tone: k.done,
      label: "All clear",
      icon: "coffee",
      stack: narrow,
      title: cols.working.length ? `${plural(cols.working.length, "agent")} on it, nothing waits for you` : "Nothing waits for you",
      meta: "New questions, failing checks and PRs to land show up here first.",
      actions: [Button("next-new", { label: "New agent", icon: "plus", variant: "outline", size: "sm", onClick: (_e, cx) => this.newAgent(cx) })],
    });
  }

  choiceRow(a, prefix, pad = 0) {
    const k = tokens();
    const q = a.question;
    const multi = q.kind === "multi";
    const picked = this.picks.get(a.address) ?? new Set(q.choices.filter((x) => x.checked).map((x) => x.n));
    return h_flex()
      .flex_wrap()
      .gap(6)
      .pt(pad)
      .children(
        q.choices.slice(0, 9).map((c, i) => {
          const on = multi ? picked.has(c.n) : i === q.cursor;
          return h_flex()
            .id(`${prefix}-choice-${a.address}-${i}`)
            .h(28)
            .px(10)
            .gap(6)
            .flex_none()
            .items_center()
            .rounded(RADIUS.md)
            .border_1()
            .border_color(on ? alpha(k.blocked, 0.6) : k.border)
            .bg(on ? alpha(k.blocked, 0.14) : k.background)
            .cursor_pointer()
            .hover((s) => s.bg(alpha(k.blocked, 0.18)).border_color(alpha(k.blocked, 0.6)))
            .when(Boolean(c.description), (e) => e.tooltip(c.description))
            .on_click((_e, cx) => {
              cx.stop_propagation();
              this.pick(cx, a, i);
            })
            .child(Mono(c.n, { color: k.blocked }))
            .when(multi, (e) => e.child(Icon(on ? "square-check" : "square", { size: 13, color: on ? k.blocked : k.mutedFg })))
            .child(div().max_w(260).truncate().text_size(TEXT.sm).text_color(k.foreground).child(c.label));
        }),
      )
      .when(multi, (e) =>
        e.child(ToneButton(`${prefix}-submit-${a.address}`, { label: `Submit · ${picked.size}`, tone: k.blocked, onClick: (_e, cx) => this.answer(cx, a, { choices: [...picked] }) })),
      );
  }

  board(cols, wide, narrow) {
    const k = tokens();
    const column = (c) => {
      const list = cols[c.stage];
      const dimmed = this.focusMode && c.stage !== "needs";
      const archive =
        c.stage === "done" && list.some((x) => x.agent && !x.archived)
          ? Button("archive-all", { label: "Archive all", variant: "ghost", size: "sm", tooltip: "Fold every done agent's tab away (they keep running)", onClick: (_e, cx) => this.archiveAll(cx, list) })
          : null;
      return v_flex()
        .id(`col-${c.stage}`)
        .flex_1()
        .min_w(0)
        .gap(10)
        .when(dimmed, (e) => e.opacity(0.28))
        .child(StageHeading(`head-${c.stage}`, c.label, { color: k[c.tone], count: list.length, pulse: Boolean(c.pulse) && list.length > 0, spin: Boolean(c.spin) && list.length > 0, action: archive ?? undefined }))
        .child(div().id(`cards-${c.stage}`).w_full().flex().flex_row().flex_wrap().gap(14).children(list.map((card) => div().w(narrow ? "100%" : 440).flex_none().child(this.card(card, c)))))
        .when(!list.length, (e) => e.child(this.emptyColumn(c)))
        .when(c.stage === "done" && cols.archived > 0, (e) =>
          e.child(
            Button("show-archived", { label: this.showArchived ? `Hide ${cols.archived} archived` : `${cols.archived} archived`, icon: "archive", variant: "ghost", size: "sm", onClick: (_e, cx) => { this.showArchived = !this.showArchived; cx.notify(); } }),
          ),
        );
    };
    // Stages with cards each get a row of them; the empty ones say so on one line, so no blank columns fill the screen.
    const full = COLUMNS.filter((c) => cols[c.stage].length || (c.stage === "done" && cols.archived > 0));
    const empty = COLUMNS.filter((c) => !full.includes(c));
    const quiet = { needs: "nothing needs you", working: "nothing working", ready: "nothing to land", done: "nothing finished lately" };
    return v_flex()
      .w_full()
      .gap(22)
      .children(full.map((c) => column(c).flex_none().w_full()))
      .when(empty.length > 0, (e) =>
        e.child(
          h_flex()
            .gap(14)
            .flex_wrap()
            .children(empty.map((c) => h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.dim).child(div().size(6).rounded(3).bg(alpha(k[c.tone], 0.6))).child(quiet[c.stage]))),
        ),
      );
  }

  emptyColumn(c) {
    const k = tokens();
    const text = { needs: "No question waits for you", working: "No agent is working", ready: "Nothing to land yet", done: "Nothing finished lately" }[c.stage];
    return h_flex().h(56).px(14).items_center().justify_center().rounded(RADIUS.lg).border_1().border_color(alpha(k.border, 0.8)).text_size(TEXT.sm).text_color(k.dim).child(text);
  }

  programIcon(a) {
    const k = tokens();
    const p = a?.program;
    if (p?.icon) return p.tint ? svg(p.icon).size(14).flex_none().text_color(k.mutedFg) : image(p.icon).w(14).h(14).flex_none();
    return Icon(a ? "bot" : "git-pull-request", { size: 14, color: k.mutedFg });
  }

  card(card, col) {
    const k = tokens();
    const tone = k[col.tone];
    const a = card.agent;
    const pr = card.pr ?? a?.pr ?? null;
    const sel = card.key === this.sel;
    const busy = this.busy.get(card.key);
    const flash = this.flashes.of(card.key);
    const showing = this.hover.is(card.key) || sel;
    const isAgentCard = card.kind === "agent";

    const timer = !isAgentCard
      ? card.stage === "ready"
        ? "ready"
        : pr?.failed?.length
          ? "failing"
          : "changes asked"
      : card.stage === "needs"
        ? a.since
          ? `waiting ${dur(this.now - a.since)}`
          : "waiting"
        : card.stage === "working"
          ? a.working
            ? dur(this.now - a.working)
            : "working"
          : card.stage === "ready"
            ? "ready"
            : a.landed
              ? "merged"
              : a.since
                ? `${dur(this.now - a.since)} ago`
                : a.status;
    const title = isAgentCard ? a.title : pr?.title ?? "";
    const facts = [];
    const ticket = a?.ticket ?? pr?.ticket;
    if (ticket) facts.push(Chip(ticket, { mono: true, icon: "ticket" }));
    const checks = pr?.checks ?? a?.checks;
    if (pr?.number) facts.push(Chip(`#${pr.number}`, { id: `${card.key}-pr`, mono: true, icon: "git-pull-request", tooltip: pr.title ?? "Open the PR", onClick: (_e, cx) => this.openUrl(cx, pr.url) }));
    const ci = pr?.ci ?? a?.ci;
    if (ci === "SUCCESS") facts.push(Chip(checks?.total ? `${checks.done ?? checks.total}/${checks.total} checks` : "checks", { color: k.success, icon: "check" }));
    else if (ci === "FAILURE" || ci === "ERROR") facts.push(Chip(pr?.failed?.length ? pr.failed.slice(0, 2).join(", ") : "checks", { color: k.destructive, icon: "x" }));
    else if (ci === "PENDING") facts.push(Chip("checks running", { color: k.warning, icon: "loader" }));
    if (pr?.review === "APPROVED") facts.push(Chip("approved", { color: k.success }));
    else if (pr?.review === "CHANGES_REQUESTED") facts.push(Chip("changes asked", { color: k.destructive }));
    const ch = a?.changes;
    if (ch && (ch.added || ch.removed)) facts.push(h_flex().gap(4).child(Mono(`+${ch.added ?? 0}`, { color: k.success })).child(Mono(`−${ch.removed ?? 0}`, { color: k.destructive })));
    if (ch?.files) facts.push(Mono(plural(ch.files, "file")));
    if (a?.cost) facts.push(Mono(money(a.cost)));
    const queued = this.queue?.queues?.[a ? a.handle : ""]?.length ?? 0;
    if (queued) facts.push(Chip(`${queued} queued`, { color: k.info, icon: "list-ordered" }));

    const actions = [];
    if (busy) actions.push(h_flex().gap(6).items_center().child(Spinner({ size: 12, color: tone })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(busy)));
    else if (card.stage === "needs" && isAgentCard && a.status !== "blocked") {
      actions.push(ToneButton(`${card.key}-fix`, { label: "Ask it to fix", tone, onClick: (_e, cx) => this.answer(cx, a, { text: a.ci === "FAILURE" ? "The PR's checks fail: look at the failing jobs and fix them." : "Changes were requested on the PR: read the review comments and address them." }) }));
      actions.push(Button(`${card.key}-go`, { label: "Go to agent", variant: "outline", size: "sm", onClick: () => goToAgent(a.address) }));
      if (pr?.url) actions.push(Button(`${card.key}-open`, { label: "Open PR", variant: "ghost", size: "sm", onClick: (_e, cx) => this.openUrl(cx, pr.url) }));
    } else if (card.stage === "needs" && isAgentCard) {
      if (!a.question?.choices?.length) actions.push(ToneButton(`${card.key}-reply`, { label: "Reply", tone, kbd: "t", onClick: (_e, cx) => this.startReply(cx, a) }));
      actions.push(Button(`${card.key}-beside`, { label: "Open beside", variant: "outline", size: "sm", onClick: (_e, cx) => this.openBeside(cx, a) }));
      actions.push(Button(`${card.key}-go`, { label: "Go to agent", variant: "ghost", size: "sm", onClick: () => goToAgent(a.address) }));
    } else if (card.stage === "needs") {
      actions.push(ToneButton(`${card.key}-fix`, { label: "Fix with Claude", tone: k.destructive, onClick: (_e, cx) => cx.spawn(async (cx) => this.act(cx, card.key, "Starting…", () => hub.act("inbox", "fix_ci", [pr.url]))) }));
      actions.push(Button(`${card.key}-open`, { label: "Open PR", variant: "outline", size: "sm", onClick: (_e, cx) => this.openUrl(cx, pr.url) }));
    } else if (card.stage === "ready") {
      if (pr?.url) actions.push(ToneButton(`${card.key}-merge`, { label: "Merge", tone: k.success, kbd: "m", onClick: (_e, cx) => this.merge(cx, pr) }));
      if (pr?.url) actions.push(Button(`${card.key}-open`, { label: "Open PR", variant: "outline", size: "sm", onClick: (_e, cx) => this.openUrl(cx, pr.url) }));
      if (a) actions.push(Button(`${card.key}-go`, { label: "Go to agent", variant: "ghost", size: "sm", onClick: () => goToAgent(a.address) }));
    } else if (showing && isAgentCard) {
      actions.push(Button(`${card.key}-go`, { label: "Go to agent", variant: "outline", size: "sm", onClick: () => goToAgent(a.address) }));
      if (card.stage === "done") actions.push(Button(`${card.key}-archive`, { label: "Archive", icon: "archive", variant: "ghost", size: "sm", kbd: "a", onClick: (_e, cx) => this.archive(cx, a) }));
      if (card.stage === "working") actions.push(Button(`${card.key}-queue`, { label: "Queue a prompt", icon: "list-ordered", variant: "ghost", size: "sm", onClick: (_e, cx) => this.run("queue", card, cx) }));
    }

    const recap = a?.recap && card.stage !== "needs" ? String(a.recap).split("\n").find((l) => l.trim()) : null;
    const replying = isAgentCard && this.replying === a.address;
    let body = v_flex()
      .flex_1()
      .min_w(0)
      .px(12)
      .py(10)
      .gap(7)
      .child(
        h_flex()
          .w_full()
          .gap(6)
          .items_center()
          .child(this.programIcon(a))
          .when(Boolean(card.project), (e) => e.child(Chip(card.project, { color: k.t.ansi4 })))
          .child(div().flex_1().min_w(0).truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.dim).child(a?.branch ?? pr?.branch ?? a?.space_name ?? ""))
          .child(Mono(timer, { color: card.stage === "needs" ? tone : card.stage === "ready" ? k.success : k.mutedFg })),
      )
      .child(div().w_full().truncate().text_size(13.5).font_medium().text_color(k.foreground).child(title))
      .when(card.stage === "needs" && isAgentCard, (e) =>
        e.child(
          v_flex()
            .w_full()
            .px(10)
            .py(8)
            .gap(4)
            .rounded(RADIUS.md)
            .bg(alpha(tone, 0.1))
            .child(div().text_size(10.5).font_semibold().text_color(tone).child(a.status === "blocked" ? "ASKS YOU" : "TO FIX"))
            .child(div().w_full().whitespace_normal().text_size(TEXT.sm).text_color(k.foreground).child(a.question?.text || (a.status === "blocked" ? "It stopped and waits for you." : a.ci === "FAILURE" ? "Its PR's checks fail: tell it to fix them." : "Changes were asked on its PR.")))
            .when(Boolean(a.question?.choices?.length), (e) => e.child(this.choiceRow(a, card.key, 4))),
        ),
      )
      .when(replying, (e) => e.child(h_flex().gap(6).child(TextField(this.reply)).child(Button(`${card.key}-send`, { icon: "send", size: "icon-sm", onClick: (_e, cx) => this.sendReply(cx) }))))
      .when(Boolean(recap), (e) => e.child(div().w_full().truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(recap ?? "")))
      .when(facts.length > 0, (e) => e.child(h_flex().w_full().flex_wrap().gap(6).items_center().children(facts)))
      .when(actions.length > 0, (e) => e.child(h_flex().w_full().flex_wrap().gap(6).items_center().children(actions)));

    let el = div()
      .id(`card-${card.key}`)
      .w_full()
      .flex_none()
      .flex()
      .flex_row()
      .relative()
      .rounded(RADIUS.lg)
      .overflow_hidden()
      .border_1()
      .border_color(sel ? alpha(tone, 0.7) : k.border)
      .bg(card.stage === "needs" ? alpha(tone, 0.04) : k.card)
      .cursor_pointer()
      .hover((s) => s.border_color(alpha(tone, 0.5)))
      .on_click((e, cx) => {
        this.sel = card.key;
        if (e.click_count > 1) this.run("go", card, cx);
        cx.notify();
      })
      .on_mouse_down("right", (_e, cx) => {
        this.sel = card.key;
        this.menu(card);
        cx.notify();
      })
      .child(div().w(3).flex_none().bg(tone))
      .child(body);
    if (flash) el = el.child(div().id(`card-${card.key}-flash`).absolute().inset_0().bg(alpha(flash.color ?? tone, 0.25)).opacity(flash.on ? 1 : 0).transition("opacity", { duration: 1400, easing: "ease-out" }));
    return this.hover.track(el, card.key);
  }
}
