// The inbox: the `inbox` view the hub pushes (the TUI inbox's own rows, built in Rust), with every TUI key and action
// plus the pointer's: a tree of sections with PR, review and ticket rows, details beside it (GitHub's merge readiness,
// the merge menu, review threads, failing tests, the Linear card with its states), hover cards fetched on demand,
// multi-select with shared actions, a right-click menu, a command list on ctrl+p, start dialogs that check first, and
// the outcomes of the requests the rows filed.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { settings } from "herdr";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Spinner, Badge, Button, Kbd, PageHeader, SearchField, TextField, KeyHints, Empty, ErrorState,
  Skeleton, Command, Toaster, Flashes, Scroller, Hover, HoverCard, openMenu, alpha, Chip, Mono, Metric, ToneButton, Pills,
  SegBar, Avatar, Separator, CountPill, Overline,
} from "./ui.js";

const H = { heading: 40, pr: 52, ticket: 40, group: 30, line: 26, text: 34 };
const KIND_NAME = { pr: ["PR", "PRs"], review: ["review request", "review requests"], ticket: ["ticket", "tickets"], alert: ["alert", "alerts"], brief: ["brief line", "brief lines"], muted: ["muted repo", "muted repos"], worktree: ["worktree", "worktrees"] };
const FRESH_MS = 120_000;

const words = (q) => q.toLowerCase().split(/\s+/).filter(Boolean);
const spansText = (spans) => (spans ?? []).map((s) => s.text ?? "").join(" ");

function haystack(row) {
  if (row.cells) return `${row.cells.ref} ${row.cells.title} ${row.cells.notes}`;
  return spansText(row.text) || row.name;
}

function kindCount(kind, n) {
  const [one, many] = KIND_NAME[kind] ?? ["row", "rows"];
  return `${n} ${n === 1 ? one : many}`;
}

/** What a row's state reads as, to tell when it changed: CI, review, draft for a PR; status for a ticket. */
function signature(row) {
  const it = row.item ?? {};
  if (row.kind === "pr" || row.kind === "review") return `${it.ci}|${it.review}|${it.isDraft}|${it.updatedAt}`;
  if (row.kind === "ticket") return row.cells?.ref ?? "";
  if (row.kind === "alert" || row.kind === "worktree") return spansText(row.text);
  return null;
}

function secsText(secs) {
  if (!secs) return null;
  const h = Math.floor(secs / 3600);
  const m = Math.floor(secs / 60) % 60;
  return h ? (m ? `${h}h ${m}m` : `${h}h`) : `${Math.max(1, m)}m`;
}

export default class Inbox {
  init(_props, cx) {
    this.model = null;
    this.agents = null;
    this.status = "connecting";
    this.group = "feature";
    this.sel = null;
    this.folds = new Map();
    this.unfolded = new Set();
    this.filter = "";
    this.filtering = false;
    this.picked = new Set();
    this.details = new Map();
    this.pending = new Map();
    this.outcomes = new Map();
    this.dialog = null;
    this.palette = null;
    this.shown = true;
    this.looked = null;
    this.seenNow = new Set();
    this.hover = new Hover();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();

    this.search = InputState.new({ placeholder: "Filter rows…  a repo, a ticket, a title" });
    this.search.on("change", (_e, cx) => {
      this.filter = this.search.value();
      this.unfolded.clear();
      this.selectFirstMatch();
      cx.notify();
    });
    this.search.on("submit", (_e, cx) => {
      this.filtering = false;
      if (!this.filter.trim()) this.clearFilter();
      this.handle.focus();
      cx.notify();
    });
    this.query = InputState.new({ placeholder: "Type a command or a row…" });
    this.query.on("change", (_e, cx) => {
      if (this.palette) this.palette.at = 0;
      cx.notify();
    });
    this.query.on("submit", (_e, cx) => this.runPalette(cx));
    this.issueTitle = InputState.new({ placeholder: "Title" });
    this.issueBody = InputState.new({ placeholder: "What and why, in a line or two" });
    this.issueTeam = InputState.new({ placeholder: "Team key (default: the first configured)" });
    this.issueTitle.on("submit", (_e, cx) => this.fileIssue(cx));
    this.issueBody.on("submit", (_e, cx) => this.fileIssue(cx));
    this.subscribe(cx);
  }

  subscribe(cx) {
    this.sub?.stop();
    this.sub = hub.subscribe(
      cx,
      { views: [{ name: "inbox", args: { group: this.group } }, { name: "agents", args: {} }] },
      (m, cx) => {
        if (m.view === "inbox") this.take(m.value, cx);
        if (m.view === "agents") this.agents = m.value;
        if (m.focus === "inbox") this.goTo(m.target?.key, cx);
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
  // The model

  take(model, cx) {
    const before = this.model;
    this.model = model;
    const k = tokens();
    const was = new Map();
    if (before) for (const r of this.all(before)) was.set(r.key, signature(r));
    for (const r of this.all(model)) {
      const sig = signature(r);
      if (sig === null || !before) continue;
      if (!was.has(r.key)) this.flashes.start(cx, r.key, k.ring);
      else if (was.get(r.key) !== sig) {
        const bad = (r.item?.ci === "FAILURE" || r.item?.ci === "ERROR") && !String(was.get(r.key)).startsWith("FAILURE");
        this.flashes.start(cx, r.key, bad ? k.destructive : k.success);
        this.forget(`pr_merge ${r.key}`);
      }
    }
    if (!this.looked && model.state === "ok") this.looked = new Map(this.all(model).map((r) => [r.key, signature(r)]));
    for (const key of [...this.picked]) if (!this.all(model).some((r) => r.key === key)) this.picked.delete(key);
    if (!this.sel || !this.visible().some((v) => v.row.path === this.sel)) this.sel = this.visible()[0]?.row.path ?? null;
    this.settle(cx);
  }

  /** Every row of the tree, depth first. */
  all(model = this.model) {
    const out = [];
    const walk = (row) => {
      out.push(row);
      for (const kid of row.children ?? []) walk(kid);
    };
    for (const top of model?.sections ?? []) walk(top);
    return out;
  }

  isOpen(row) {
    if (!row.children?.length) return false;
    return this.folds.has(row.path) ? this.folds.get(row.path) : row.expand;
  }

  /** The rows that show, with their depth: the filter keeps matches, their ancestors, and all under an unfolded row. */
  visible() {
    const q = words(this.filter);
    const out = [];
    if (!q.length) {
      const walk = (row, depth) => {
        out.push({ row, depth });
        if (this.isOpen(row)) for (const kid of row.children) walk(kid, depth + 1);
      };
      for (const top of this.model?.sections ?? []) walk(top, 0);
      return out;
    }
    const hit = (row) => {
      const text = haystack(row).toLowerCase();
      return q.every((w) => text.includes(w));
    };
    const keep = (row) => hit(row) || (row.children ?? []).some(keep);
    const walk = (row, depth, all) => {
      if (!all && !keep(row)) return;
      out.push({ row, depth, context: !hit(row) });
      const every = all || this.unfolded.has(row.path);
      for (const kid of row.children ?? []) walk(kid, depth + 1, every);
    };
    for (const top of this.model?.sections ?? []) walk(top, 0, false);
    return out;
  }

  current() {
    return this.visible().find((v) => v.row.path === this.sel)?.row ?? null;
  }

  rowByKey(key) {
    return this.all().find((r) => r.key === key);
  }

  selectAt(i) {
    const vis = this.visible();
    const at = Math.max(0, Math.min(i, vis.length - 1));
    if (vis[at]) this.select(vis[at].row);
  }

  select(row) {
    this.sel = row.path;
    this.seenNow.add(row.key);
    for (const r of this.all().filter((r) => r.path.startsWith(`${row.path}/`))) if (!this.isOpen(row)) this.seenNow.add(r.key);
  }

  selIndex() {
    return Math.max(0, this.visible().findIndex((v) => v.row.path === this.sel));
  }

  selectFirstMatch() {
    const first = this.visible().find((v) => !v.context);
    if (first) this.sel = first.row.path;
  }

  clearFilter() {
    this.filter = "";
    this.search.set_value("");
    this.unfolded.clear();
  }

  toggleFold(row) {
    if (!row?.children?.length) return;
    if (this.filter.trim()) {
      if (!this.unfolded.delete(row.path)) this.unfolded.add(row.path);
    } else this.folds.set(row.path, !this.isOpen(row));
  }

  goTo(key, cx) {
    const row = key && this.rowByKey(key);
    if (!row) {
      if (key) this.toasts.push(cx, `${key} is not in the inbox any more`);
      return;
    }
    this.clearFilter();
    const parts = row.path.split("/").filter(Boolean);
    for (let i = 1; i < parts.length; i++) this.folds.set(`/${parts.slice(0, i).join("/")}`, true);
    this.select(row);
    this.flashes.start(cx, row.key, tokens().ring);
    this.handle.focus();
  }

  marked(row) {
    if (!this.looked || this.seenNow.has(row.key)) return false;
    const sig = signature(row);
    if (sig === null) return false;
    const hidden = !this.isOpen(row) ? (row.children ?? []).some((kid) => this.marked(kid)) : false;
    return (this.looked.has(row.key) ? this.looked.get(row.key) !== sig : true) || hidden;
  }

  changedCount() {
    return this.all().filter((r) => signature(r) !== null && this.looked && !this.seenNow.has(r.key) && this.looked.get(r.key) !== signature(r)).length;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Details on demand

  detail(kind, args, key, cx) {
    const id = `${kind} ${key}`;
    const known = this.details.get(id);
    if (known && (known.loading || Date.now() - known.at < (known.error ? 30_000 : FRESH_MS))) return known;
    const entry = { loading: true, at: Date.now() };
    this.details.set(id, entry);
    cx.spawn(async (cx) => {
      try {
        const value = await hub.fetchDetails(kind, args);
        this.details.set(id, { at: Date.now(), value, error: value?.error ?? null });
      } catch (e) {
        this.details.set(id, { at: Date.now(), error: String(e?.message ?? e) });
      }
      cx.notify();
    });
    return entry;
  }

  forget(id) {
    this.details.delete(id);
  }

  prArgs(row) {
    const it = row.item ?? {};
    return { url: it.url, repo: it.repo, number: it.number, branch: it.branch, run: null };
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async call(cx, call, { armed = null, row = null, quiet = false } = {}) {
    if (!call) return;
    try {
      const r = await hub.request("act", { app: "inbox", action: call.fn, args: call.args ?? [], armed });
      if (r.error) this.toasts.push(cx, r.error, { kind: "error", ms: 6000 });
      else if (r.note && !quiet) {
        const url = r.note.match(/https?:\/\/\S+/)?.[0];
        this.toasts.push(cx, r.note.replace(/\s*https?:\/\/\S+/, ""), {
          kind: "success",
          ms: url ? 8000 : 3500,
          action: url ? { label: "Open", onClick: (_e, cx) => cx.spawn(async (cx) => this.call(cx, { fn: "open_url", args: [url] }, { quiet: true })) } : this.cleanupAction(call),
        });
      }
      for (const [key, job] of Object.entries(r.pending ?? {})) this.pending.set(key, job);
      return r;
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    } finally {
      cx.notify();
    }
  }

  /** After a merge, the Clean up a merged branch's worktree asks for. */
  cleanupAction(call) {
    if (call.fn !== "merge") return undefined;
    const url = call.args?.[0]?.url ?? call.args?.[0];
    const tree = (this.model?.worktrees ?? []).find((w) => w.pr === url && !w.changes);
    if (!tree) return undefined;
    return { label: "Remove worktree", onClick: (_e, cx) => cx.spawn(async (cx) => this.call(cx, { fn: "remove_worktree", args: [tree.path] }, { armed: `remove-${tree.path}` })) };
  }

  /** Request outcomes the mod wrote: on the row a few seconds, as a toast, then forgotten. */
  settle(cx) {
    const results = this.model?.results ?? {};
    for (const [row, job] of [...this.pending]) {
      const outcome = results[job.file];
      if (!outcome) continue;
      this.pending.delete(row);
      this.outcomes.set(row, outcome);
      this.toasts.push(cx, `${job.label}: ${outcome.message}`, { kind: outcome.ok ? "success" : "error" });
      cx.timer.after(10_000, (cx) => {
        this.outcomes.delete(row);
        cx.notify();
      });
      cx.spawn(async () => hub.request("act", { app: "inbox", action: "forget_result", args: [job.file] }).catch(() => {}));
    }
  }

  /** A row's action: a choice asks which (the merge menu asks GitHub), a change asks first, the rest runs at once. */
  run(cx, row, act) {
    if (!act) return;
    if (act.call?.fn === "start") return this.openStart(cx, row, act);
    if (act.label === "merge" && (row.kind === "pr" || row.kind === "review")) return this.openMerge(cx, row);
    if (act.choices?.length) {
      this.dialog = { kind: "pick", title: `${act.label[0].toUpperCase()}${act.label.slice(1)}`, description: row.name, choices: act.choices.map((c) => ({ label: c.label, run: (cx) => this.call(cx, c.call, { row }) })) };
    } else if (act.confirm) {
      this.dialog = { kind: "confirm", title: `${act.label[0].toUpperCase()}${act.label.slice(1)}?`, description: row.name, confirm: act.label, destructive: /remove|stop|close/.test(act.label), run: (cx) => this.call(cx, act.call, { armed: act.confirm, row }) };
    } else cx.spawn(async (cx) => this.call(cx, act.call, { row }));
    cx.notify();
  }

  /** The picked rows' shared action: asked once for all of them, then run on each. */
  runShared(cx, label, key) {
    const rows = this.pickedRows();
    const acts = rows.map((r) => [r, r.acts.find((a) => a.label === label && a.key === key)]).filter(([, a]) => a);
    if (!acts.length) return;
    const go = async (cx, choice) => {
      for (const [r, a] of acts) {
        const call = choice !== undefined ? a.choices[choice]?.call : a.call;
        await this.call(cx, call, { armed: a.confirm, row: r, quiet: true });
      }
      this.toasts.push(cx, `${label}: ${acts.length} sent`, { kind: "success" });
      this.picked.clear();
      cx.notify();
    };
    const what = this.describe(rows);
    const first = acts[0][1];
    if (first.choices?.length) this.dialog = { kind: "pick", title: label, description: what, choices: first.choices.map((c, i) => ({ label: c.label, run: (cx) => go(cx, i) })) };
    else this.dialog = { kind: "confirm", title: `${label} ${what}?`, description: rows.map((r) => r.name).slice(0, 5).join(" · "), confirm: label, run: (cx) => go(cx) };
    cx.notify();
  }

  pickedRows() {
    const seen = new Set();
    return this.all().filter((r) => this.picked.has(r.key) && !seen.has(r.key) && seen.add(r.key));
  }

  shared(rows) {
    if (!rows.length) return [];
    return rows[0].acts.filter((a) => rows.every((r) => r.acts.some((b) => b.label === a.label && b.key === a.key)));
  }

  describe(rows) {
    const kinds = new Map();
    for (const r of rows) kinds.set(r.kind, (kinds.get(r.kind) ?? 0) + 1);
    return [...kinds].map(([kind, n]) => kindCount(kind, n)).join(", ");
  }

  pick(row) {
    if (!row || row.kind === "plain") return false;
    if (!this.picked.delete(row.key)) this.picked.add(row.key);
    return true;
  }

  pickAlike(row) {
    if (!row || row.kind === "plain") return;
    const keys = this.visible().map((v) => v.row).filter((r) => r.kind === row.kind).map((r) => r.key);
    if (keys.every((k) => this.picked.has(k))) keys.forEach((k) => this.picked.delete(k));
    else keys.forEach((k) => this.picked.add(k));
  }

  /** Enter: the main action, else what a click does (a PR's details are already beside it), else the fold. */
  primary(cx, row) {
    if (!row) return;
    const main = row.acts.find((a) => a.main);
    if (main) return this.run(cx, row, main);
    if (row.primary && row.primary.fn !== "details") return cx.spawn(async (cx) => this.call(cx, row.primary, { row }));
    if (row.children?.length) this.toggleFold(row);
  }

  regroup(cx) {
    this.group = this.group === "feature" ? "status" : "feature";
    this.subscribe(cx);
    cx.notify();
  }

  refresh(cx) {
    cx.spawn(async (cx) => this.call(cx, { fn: "request_refresh", args: [] }));
  }

  openMerge(cx, row) {
    this.dialog = { kind: "merge", row, step: null };
    this.detail("pr_merge", this.prArgs(row), row.key, cx);
    cx.notify();
  }

  mergeChoice(cx, row, choice) {
    const pr = row.item;
    const n = `#${pr.number}`;
    const base = this.details.get(`pr_merge ${row.key}`)?.value?.base ?? pr.base ?? "its base";
    const call = {
      merge: { fn: "merge", args: [pr, choice.method], question: `${choice.label}: ${n} into ${base} now?`, verb: "Merge" },
      enqueue: { fn: "merge", args: [pr], question: `Add ${n} to the merge queue of ${base}? It merges when its turn comes.`, verb: "Add to queue" },
      auto: { fn: "automerge", args: [pr, choice.method], question: `Turn on auto-merge for ${n}? It merges once checks and reviews allow.`, verb: "Turn on" },
      "auto-off": { fn: "automerge", args: [pr, "off"], question: `Turn off auto-merge for ${n}?`, verb: "Turn off" },
      dequeue: { fn: "dequeue", args: [pr], question: `Take ${n} out of the merge queue?`, verb: "Take out" },
    }[choice.do];
    if (!call) return;
    this.dialog = { kind: "confirm", title: call.question, description: pr.title, confirm: call.verb, run: (cx) => this.call(cx, call, { row }) };
    cx.notify();
  }

  openStart(cx, row, act) {
    const target = row.kind === "ticket" ? { id: row.key } : row.item;
    this.dialog = { kind: "start", row, act, plan: null };
    cx.spawn(async (cx) => {
      try {
        const plan = await hub.fetchDetails("start_plan", { target });
        if (this.dialog?.kind === "start" && this.dialog.row === row) this.dialog.plan = plan;
      } catch (e) {
        if (this.dialog?.kind === "start") this.dialog.plan = { ok: false, steps: [], blockers: [String(e?.message ?? e)] };
      }
      cx.notify();
    });
    cx.notify();
  }

  openFile(cx) {
    this.dialog = { kind: "file" };
    this.issueTitle.set_value("");
    this.issueBody.set_value("");
    this.issueTitle.focus();
    cx.notify();
  }

  fileIssue(cx) {
    const title = this.issueTitle.value().trim();
    if (!title) return;
    const spec = { title, description: this.issueBody.value().trim(), team: this.issueTeam.value().trim() };
    this.closeDialog(cx);
    cx.spawn(async (cx) => this.call(cx, { fn: "file_issue", args: [spec] }));
  }

  closeDialog(cx) {
    this.dialog = null;
    this.handle.focus();
    cx.notify();
  }

  statusMenu(row) {
    const card = this.details.get(`ticket_card ${row.key}`)?.value;
    const names = card?.states?.length ? card.states.map((s) => s.name) : row.acts.find((a) => a.label === "status")?.choices.map((c) => c.label) ?? [];
    openMenu(names.map((name) => ({ label: name, icon: card?.state?.name === name ? "check" : undefined, command: `inbox:state:${row.key}:${name}` })));
  }

  menu(row) {
    if (!row) return;
    this.sel = row.path;
    const items = row.acts.map((a, i) => ({ label: a.label[0].toUpperCase() + a.label.slice(1), kbd: a.key, icon: ICON[a.label] ?? ICON[a.call?.fn], destructive: /remove|stop/.test(a.label), command: `inbox:act:${i}` }));
    if (row.kind !== "plain") items.push("separator", { label: this.picked.has(row.key) ? "Unpick" : "Pick", kbd: "v", command: "inbox:pick" }, { label: `Pick all ${KIND_NAME[row.kind]?.[1] ?? "alike"}`, kbd: "V", command: "inbox:alike" });
    if (row.children?.length) items.push({ label: this.isOpen(row) ? "Fold" : "Unfold", kbd: "space", command: "inbox:fold" });
    openMenu(items);
  }

  command(c, cx) {
    if (c === "focus" || c === "show:inbox") {
      this.shown = true;
      this.handle.focus();
    } else if (c.startsWith("show:") && this.shown) {
      // Leaving the inbox: what it shows now is what you saw.
      this.shown = false;
      if (this.model?.state === "ok") this.looked = new Map(this.all().map((r) => [r.key, signature(r)]));
      this.seenNow.clear();
    }
    const [scope, action, ...rest] = c.split(":");
    if (scope === "inbox") {
      const row = this.current();
      if (action === "act" && row) this.run(cx, row, row.acts[Number(rest[0])]);
      else if (action === "pick") this.pick(row);
      else if (action === "alike") this.pickAlike(row);
      else if (action === "fold") this.toggleFold(row);
      else if (action === "state") {
        const [id, ...name] = rest;
        cx.spawn(async (cx) => {
          await this.call(cx, { fn: "set_state", args: [{ id }, name.join(":")] });
          this.forget(`ticket_card ${id}`);
        });
      } else if (action === "file") this.openFile(cx);
      else if (action === "refresh") this.refresh(cx);
    }
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p)

  paletteGroups(cx) {
    const q = words(this.query.value());
    const hit = (text) => q.every((w) => text.toLowerCase().includes(w));
    const own = [
      { id: "p-refresh", label: "Refresh the inbox", shortcut: "R", icon: "refresh-cw", run: () => this.refresh(cx) },
      { id: "p-group", label: `Group work by ${this.group === "feature" ? "status" : "feature"}`, shortcut: "w", icon: "layers", run: () => this.regroup(cx) },
      { id: "p-filter", label: "Filter rows", shortcut: "/", icon: "search", run: () => this.startFilter(cx) },
      { id: "p-file", label: "File an issue", shortcut: "I", icon: "file-plus", run: () => this.openFile(cx) },
    ];
    const acts = [];
    const gos = [];
    for (const row of this.all()) {
      row.acts.forEach((a, i) => acts.push({ id: `p-${row.path}-${i}`, label: `${a.label}  ${row.name}`, hint: row.section, shortcut: a.key, icon: ICON[a.label] ?? ICON[a.call?.fn], main: a.main, run: () => this.run(cx, row, a) }));
      if (row.name && row.kind !== "plain") gos.push({ id: `p-go-${row.path}`, label: `Go to  ${row.name}`, hint: row.section, icon: "chevron-right", run: () => this.goTo(row.key, cx) });
    }
    return [
      { heading: "Inbox", items: own.filter((i) => hit(i.label)) },
      { heading: "Actions", items: (q.length ? acts : acts.filter((a) => a.main)).filter((i) => hit(`${i.label} ${i.hint}`)).slice(0, 60) },
      { heading: "Go to", items: q.length ? gos.filter((i) => hit(i.label)).slice(0, 40) : [] },
    ];
  }

  openPalette(cx) {
    this.palette = { at: 0 };
    this.query.set_value("");
    this.query.focus();
    cx.notify();
  }

  closePalette(cx) {
    this.palette = null;
    this.handle.focus();
    cx.notify();
  }

  runPalette(cx, picked) {
    const all = this.paletteGroups(cx).flatMap((g) => g.items);
    const item = picked ?? all[this.palette?.at ?? 0];
    this.closePalette(cx);
    if (item) item.run();
  }

  startFilter(cx) {
    this.filtering = true;
    this.search.focus();
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys: the TUI's (j k g G pages [ ] space enter / v V w R ctrl+p, each row action's own letter), and the dialogs'

  key(e, cx) {
    const k = e.keystroke;
    if (this.palette) {
      const n = this.paletteGroups(cx).flatMap((g) => g.items).length;
      if (k === "escape") this.closePalette(cx);
      else if (k === "down" || k === "ctrl-n") this.palette.at = Math.min(n - 1, this.palette.at + 1);
      else if (k === "up" || k === "ctrl-p") this.palette.at = Math.max(0, this.palette.at - 1);
      else return false;
      cx.notify();
      return true;
    }
    if (this.dialog) return this.dialogKey(k, cx);
    if (this.filtering) {
      if (k === "escape") {
        this.filtering = false;
        this.clearFilter();
        this.handle.focus();
        cx.notify();
        return true;
      }
      if (k === "down" || k === "up") {
        this.selectAt(this.selIndex() + (k === "down" ? 1 : -1));
        cx.notify();
        return true;
      }
      return false;
    }
    const row = this.current();
    const picking = this.picked.size > 0;
    switch (k) {
      case "ctrl-p":
      case "cmd-shift-p":
        this.openPalette(cx);
        return true;
      case "j":
      case "down":
        this.selectAt(this.selIndex() + 1);
        break;
      case "k":
      case "up":
        this.selectAt(this.selIndex() - 1);
        break;
      case "pagedown":
      case "ctrl-d":
        this.selectAt(this.selIndex() + 12);
        break;
      case "pageup":
      case "ctrl-u":
        this.selectAt(this.selIndex() - 12);
        break;
      case "g":
      case "home":
        this.selectAt(0);
        break;
      case "shift-g":
      case "end":
        this.selectAt(this.visible().length - 1);
        break;
      case "[":
      case "]": {
        const vis = this.visible();
        const at = this.selIndex();
        const tops = vis.map((v, i) => [v, i]).filter(([v]) => v.depth === 0).map(([, i]) => i);
        const to = k === "]" ? tops.find((i) => i > at) : [...tops].reverse().find((i) => i < at);
        if (to !== undefined) this.selectAt(to);
        break;
      }
      case "/":
        this.startFilter(cx);
        break;
      case "escape":
        if (picking) this.picked.clear();
        else if (this.filter) this.clearFilter();
        else return false;
        break;
      case "space":
        this.toggleFold(row);
        break;
      case "enter":
        this.primary(cx, row);
        break;
      case "v":
        this.pick(row);
        break;
      case "shift-v":
        this.pickAlike(row);
        break;
      case "shift-f10":
        this.menu(row);
        break;
      default: {
        const letter = e.key?.length === 1 ? (k.startsWith("shift-") ? e.key.toUpperCase() : e.key) : null;
        if (letter && !k.includes("cmd-") && !k.includes("ctrl-") && !k.includes("alt-")) {
          if (picking) {
            const a = this.shared(this.pickedRows()).find((a) => a.key === letter);
            if (a) {
              this.runShared(cx, a.label, a.key);
              break;
            }
          } else {
            const a = row?.acts.find((a) => a.key === letter);
            if (a) {
              this.run(cx, row, a);
              break;
            }
          }
          if (letter === "w") this.regroup(cx);
          else if (letter === "R") this.refresh(cx);
          else if (letter === "I") this.openFile(cx);
          else return false;
          break;
        }
        return false;
      }
    }
    cx.notify();
    return true;
  }

  dialogKey(k, cx) {
    const d = this.dialog;
    if (k === "escape") {
      this.closeDialog(cx);
      return true;
    }
    if (d.kind === "file") return false;
    const choices = d.kind === "pick" ? d.choices : d.kind === "merge" ? this.mergeChoices(d.row) : [];
    if (/^[1-9]$/.test(k) && choices[Number(k) - 1]) {
      const c = choices[Number(k) - 1];
      if (d.kind === "merge") this.mergeChoice(cx, d.row, c);
      else {
        this.closeDialog(cx);
        cx.spawn(async (cx) => c.run(cx));
      }
      return true;
    }
    if (k === "enter") {
      if (d.kind === "confirm") {
        this.closeDialog(cx);
        cx.spawn(async (cx) => d.run(cx));
      } else if (d.kind === "start" && d.plan?.ok) this.startNow(cx);
      return true;
    }
    return true;
  }

  startNow(cx) {
    const d = this.dialog;
    this.closeDialog(cx);
    cx.spawn(async (cx) => this.call(cx, d.act.call, { armed: d.act.confirm, row: d.row }));
  }

  mergeChoices(row) {
    const got = this.details.get(`pr_merge ${row.key}`);
    if (got?.value?.choices) return got.value.choices;
    if (got?.error || got?.value?.error) return (row.acts.find((a) => a.label === "merge")?.choices ?? []).map((c) => ({ do: "merge", method: c.call.args[1], label: c.label[0].toUpperCase() + c.label.slice(1) }));
    return [];
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: pieces

  tone(t, k) {
    return { bad: k.destructive, warn: k.warning, good: k.success, dim: k.mutedFg, accent: k.ring }[t] ?? null;
  }

  /** A line of spans, cut at the edge, or wrapped (`wrap`, the details panel). */
  spans(spans, k, { size = TEXT.base, grow = false, wrap = false } = {}) {
    return h_flex()
      .when(grow, (e) => e.flex_1())
      .min_w(0)
      .gap(6)
      .items_center()
      .when(!wrap, (e) => e.overflow_hidden().whitespace_nowrap())
      .when(wrap, (e) => e.items_start())
      .text_size(size)
      .children(
        (spans ?? []).map((s) =>
          s.icon
            ? Icon(s.icon, { size: 14, color: this.tone(s.tone, k) ?? k.mutedFg })
            : div()
                .when(!wrap, (e) => e.flex_none())
                .when(wrap, (e) => e.flex_shrink(1).min_w(0))
                .text_color(this.tone(s.tone, k) ?? k.foreground)
                .when(Boolean(s.bold), (e) => e.font_semibold())
                .child(s.text),
        ),
      );
  }

  progress(row, k) {
    const job = row.status && this.pending.get(row.status);
    if (job) return h_flex().gap(6).items_center().flex_none().child(Spinner({ size: 12, color: k.warning })).child(div().text_size(TEXT.xs).text_color(k.warning).child(job.working));
    const out = row.status && this.outcomes.get(row.status);
    if (out) return Chip(out.message, { color: out.ok ? k.success : k.destructive, icon: out.ok ? "check" : "x" });
    return null;
  }

  readyChip(row, k) {
    const ready = (this.model?.prs ?? []).find((p) => p.url === row.key)?.ready;
    const exact = this.details.get(`pr_merge ${row.key}`)?.value?.ready;
    const r = exact ?? ready;
    if (!r) return null;
    const icon = { ready: "git-merge", blocked: "circle-x", waiting: "hourglass", draft: "git-pull-request-draft", queued: "layers", merged: "git-merge" }[r.state] ?? "circle";
    return Chip(r.text.replace(/ · you can't merge.*$/, ""), { color: this.tone(r.tone, k) ?? k.mutedFg, icon });
  }

  timeChip(id, k) {
    const week = this.agents?.tickets?.find((t) => t.key === id)?.week ?? (this.model?.tickets ?? []).find((t) => t.id === id)?.secs;
    const text = secsText(week);
    return text ? Chip(text, { color: k.info, icon: "timer", id: `time-${id}`, tooltip: "Agents' time on this ticket this week" }) : null;
  }

  rowActions(row, k, cx) {
    return row.acts.slice(0, 3).map((a, i) =>
      Button(`ra-${row.path}-${i}`, {
        icon: ICON[a.label] ?? ICON[a.call?.fn] ?? "chevron-right",
        variant: "ghost",
        size: "icon-sm",
        tooltip: `${a.label}${a.key ? `  ${a.key}` : ""}`,
        onClick: (_e, cx) => {
          cx.stop_propagation();
          this.select(row);
          this.run(cx, row, a);
        },
      }),
    );
  }

  /** A PR's hover card: GitHub's facts, fetched when the pointer rests on the row. */
  prCard(row, k, cx) {
    const got = this.hover.is(`row-${row.path}`) ? this.detail("pr_merge", this.prArgs(row), row.key, cx) : this.details.get(`pr_merge ${row.key}`);
    const it = row.item ?? {};
    const card = got?.value?.card;
    const head = v_flex()
      .gap(4)
      .child(h_flex().gap(6).items_center().child(Mono(`${it.repo}#${it.number}`)).child(div().flex_1()).child(Avatar(it.authorName || it.author || "?", { size: 18 })))
      .child(div().text_size(TEXT.base).font_semibold().child(it.title ?? ""));
    if (!card) return head.child(got?.error ? div().text_size(TEXT.sm).text_color(k.mutedFg).child(got.error) : v_flex().gap(6).child(Skeleton({ w: 260 })).child(Skeleton({ w: 200 })));
    return head
      .child(h_flex().gap(6).flex_wrap().child(Mono(`+${card.additions}`, { color: k.success })).child(Mono(`−${card.deletions}`, { color: k.destructive })).child(Mono(`${card.files} files`)).children((card.labels ?? []).map((l) => Chip(l.name, { color: l.color ? `#${l.color}` : undefined }))))
      .when(Boolean(card.body), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(card.body.slice(0, 280))))
      .child(this.readyLine(got.value.ready, k));
  }

  readyLine(r, k) {
    const c = this.tone(r?.tone, k) ?? k.mutedFg;
    return h_flex().gap(6).items_center().child(div().size(8).rounded(4).bg(c)).child(div().text_size(TEXT.sm).text_color(c).child(r?.text ?? ""));
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: rows

  rowEl(v, k, cx) {
    const { row, depth, context } = v;
    const id = `row-${row.path}`;
    const selected = row.path === this.sel;
    const flash = this.flashes.of(row.key);
    const picked = this.picked.has(row.key);
    const indent = depth * 18;
    const hasKids = Boolean(row.children?.length);
    const open = this.filter.trim() ? this.unfolded.has(row.path) || (row.children ?? []).length > 0 : this.isOpen(row);
    const chevron = hasKids
      ? div()
          .id(`fold-${row.path}`)
          .w(16)
          .flex_none()
          .cursor_pointer()
          .on_click((_e, cx) => {
            cx.stop_propagation();
            this.toggleFold(row);
            cx.notify();
          })
          .child(Icon(open ? "chevron-down" : "chevron-right", { size: 14, color: k.mutedFg }))
      : div().w(16).flex_none();
    const height = this.height(v);
    let el = h_flex()
      .id(id)
      .h(height)
      .pl(8 + indent)
      .pr(8)
      .gap(8)
      .items_center()
      .rounded(RADIUS.md)
      .relative()
      .cursor_pointer()
      .when(selected, (e) => e.bg(k.selected))
      .when(!selected, (e) => e.hover((s) => s.bg(k.accent)))
      .when(picked, (e) => e.border_1().border_color(k.ring).bg(alpha(k.ring, selected ? 0.16 : 0.08)))
      .when(Boolean(context), (e) => e.opacity(0.55));
    if (flash) el = el.child(div().id(`${id}-flash`).absolute().inset_0().rounded(RADIUS.md).bg(alpha(flash.color ?? k.ring, 0.28)).opacity(flash.on ? 1 : 0).transition("opacity", { duration: 1600, easing: "ease-out" }));
    el = el.child(chevron);
    if (this.marked(row)) el = el.child(div().id(`${id}-mark`).size(6).flex_none().rounded(3).bg(k.ring).tooltip("Changed since you last looked"));
    el = el.child(this.body(v, k, cx));
    const prog = this.progress(row, k);
    if (prog) el = el.child(prog);
    if (this.hover.is(id) || selected) el = el.child(h_flex().gap(2).flex_none().children(this.rowActions(row, k, cx)));
    el = this.hover.track(el, id);
    el = el
      .on_click((e, cx) => {
        this.select(row);
        this.handle.focus();
        if (e.click_count > 1) this.primary(cx, row);
        else if (hasKids && (row.kind === "plain" || depth === 0)) this.toggleFold(row);
        cx.notify();
      })
      .on_mouse_down("right", (_e, cx) => {
        this.select(row);
        this.menu(row);
        cx.notify();
      });
    if (row.kind === "pr" || row.kind === "review") return HoverCard(`card-${row.path}`, el, this.prCard(row, k, cx), { width: 360, openDelay: 650 });
    return el;
  }

  height({ row, depth }) {
    if (depth === 0) return H.heading;
    if (row.kind === "pr" || row.kind === "review") return H.pr;
    if (row.kind === "ticket") return H.ticket;
    if (row.kind === "plain") return row.children?.length ? H.group : H.line;
    return H.text;
  }

  body(v, k, cx) {
    const { row, depth } = v;
    if (depth === 0) {
      const count = row.children?.length ?? 0;
      return h_flex()
        .flex_1()
        .min_w(0)
        .gap(8)
        .items_center()
        .child(Icon(row.icon ?? "circle", { size: 15, color: row.key === "alerts" ? k.warning : k.foreground }))
        .child(div().flex_none().text_size(TEXT.base).font_semibold().child(row.title ?? row.name))
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(/^\d+$/.test(row.extra ?? "") ? "" : row.extra ?? ""))
        .when(row.key === "today", (e) =>
          e.child(Button(`dismiss-${row.path}`, { label: "Dismiss", variant: "ghost", size: "sm", kbd: "x", onClick: (_e, cx) => (cx.stop_propagation(), this.run(cx, row, row.acts[0])) })),
        )
        .when(row.key === "work", (e) => e.child(Pills("group-pills", [["feature", "By feature"], ["status", "By status"]], this.group, (_v, cx) => this.regroup(cx))))
        .when(row.key !== "work" && row.key !== "today", (e) => e.child(CountPill(count, { on: count > 0 })));
    }
    if (row.cells && (row.kind === "pr" || row.kind === "review")) return this.prBody(row, k);
    if (row.cells && row.kind === "ticket") return this.ticketBody(row, k);
    if (row.kind === "plain") return this.spans(row.text, k, { size: TEXT.sm, grow: true });
    return this.spans(row.text, k, { grow: true });
  }

  prBody(row, k) {
    const c = row.cells;
    const it = row.item ?? {};
    const lead = h_flex().gap(4).flex_none().children(c.lead.map((l) => Icon(l.icon, { size: 15, color: this.tone(l.tone, k) ?? k.mutedFg })));
    const chips = [];
    if (row.kind === "pr") {
      if (it.branch) chips.push(Chip(it.branch, { mono: true, icon: "git-branch" }));
      const ready = this.readyChip(row, k);
      if (ready) chips.push(ready);
    } else {
      chips.push(Chip(it.isDirect ? "asked of you" : "asked of your team", { icon: it.isDirect ? "user" : "users", color: it.isDirect ? k.warning : undefined }));
      if (it.authorName || it.author) chips.push(h_flex().gap(4).items_center().child(Avatar(it.authorName || it.author, { size: 16 })).child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(it.authorName || it.author)));
    }
    if (c.notes && row.kind === "pr") chips.push(div().text_size(TEXT.xs).text_color(this.tone(c.notes_tone, k) ?? k.mutedFg).child(c.notes));
    return h_flex()
      .flex_1()
      .min_w(0)
      .gap(10)
      .items_center()
      .child(lead)
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(4)
          .child(h_flex().gap(8).items_center().min_w(0).child(Mono(c.ref)).child(div().flex_1().min_w(0).truncate().text_color(c.title_tone === "dim" ? k.mutedFg : k.foreground).child(c.title)))
          .child(h_flex().gap(6).items_center().overflow_hidden().children(chips)),
      )
      .child(div().w(36).flex_none().text_right().text_size(TEXT.xs).text_color(k.mutedFg).child(c.age ?? ""));
  }

  ticketBody(row, k) {
    const c = row.cells;
    const [id, ...status] = c.ref.split(/\s+/);
    const statusText = status.join(" ");
    const lead = c.lead[0];
    const t = (this.model?.tickets ?? []).find((x) => x.id === row.key);
    return h_flex()
      .flex_1()
      .min_w(0)
      .gap(10)
      .items_center()
      .child(Icon(lead?.icon ?? "circle", { size: 15, color: this.tone(lead?.tone, k) ?? k.mutedFg }))
      .child(Mono(id, { size: TEXT.sm, color: k.foreground }))
      .child(div().flex_1().min_w(0).truncate().child(c.title))
      .when(Boolean(statusText), (e) => e.child(Badge(statusText, lead?.tone === "warn" ? "warning" : "outline")))
      .when(Boolean(this.timeChip(row.key, k)), (e) => e.child(this.timeChip(row.key, k)))
      .when(Boolean(t?.cost), (e) => e.child(Mono(`$${t.cost.toFixed(2)}`)));
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: the details beside the list

  panel(k, cx, { narrow }) {
    const rows = this.pickedRows();
    let title;
    let description;
    let children = [];
    let footer = null;
    if (rows.length) {
      title = `${this.describe(rows)} picked`;
      description = "Their shared actions run on each";
      const shared = this.shared(rows);
      children = [
        v_flex().gap(4).children(rows.slice(0, 8).map((r) => div().truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(r.name))),
        rows.length > 8 ? div().text_size(TEXT.sm).text_color(k.dim).child(`and ${rows.length - 8} more`) : null,
        shared.length ? null : div().text_size(TEXT.sm).text_color(k.mutedFg).child("No action is shared by all of them."),
      ].filter(Boolean);
      footer = this.buttons(shared.map((a) => ({ ...a, run: (cx) => this.runShared(cx, a.label, a.key) })), k, "shared").child(Button("unpick", { label: "Drop picks", variant: "ghost", size: "sm", kbd: "esc", onClick: (_e, cx) => (this.picked.clear(), cx.notify()) }));
    } else {
      const row = this.current();
      if (!row) return null;
      title = row.cells ? `${row.cells.ref.split(/\s+/)[0]}  ${row.cells.title}` : row.title ?? row.name;
      description = row.section || null;
      if (row.kind === "pr" || row.kind === "review") children = this.prPanel(row, k, cx);
      else if (row.kind === "ticket") children = this.ticketPanel(row, k, cx);
      else if (row.kind === "plain" && row.children?.length) children = [this.childList(row, k)];
      else children = [this.infoBlock(row, k)];
      footer = row.acts.length ? this.buttons(row.acts.map((a) => ({ ...a, run: (cx) => this.run(cx, row, a) })), k, "row") : null;
    }
    const body = v_flex().id("inbox-panel-body").flex_1().min_h(0).p(16).gap(14).overflow_y_scrollbar().children(children);
    return v_flex()
      .id("inbox-panel")
      .when(!narrow, (e) => e.w(400).h_full().border_l_1())
      .when(narrow, (e) => e.w_full().h(300).border_t_1())
      .flex_none()
      .border_color(k.border)
      .bg(k.background)
      .child(
        v_flex()
          .px(16)
          .py(12)
          .gap(2)
          .flex_none()
          .border_b_1()
          .border_color(k.border)
          .child(div().truncate().text_size(TEXT.base).font_semibold().child(title ?? ""))
          .when(Boolean(description), (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(description))),
      )
      .child(body)
      .when(Boolean(footer), (e) => e.child(h_flex().p(12).gap(6).flex_wrap().flex_none().border_t_1().border_color(k.border).child(footer)));
  }

  buttons(acts, k, id) {
    return h_flex()
      .gap(6)
      .flex_wrap()
      .children(acts.map((a, i) => Button(`${id}-act-${i}`, { label: a.label[0].toUpperCase() + a.label.slice(1), icon: ICON[a.label] ?? ICON[a.call?.fn], kbd: a.key, size: "sm", variant: a.main ? "default" : "outline", onClick: (_e, cx) => a.run(cx) })));
  }

  /** A heading's or group's rows, to jump to. */
  childList(row, k) {
    const kids = row.children.filter((r) => r.kind !== "plain" || r.children?.length);
    if (!kids.length) return div().text_size(TEXT.sm).text_color(k.mutedFg).child("Nothing here.");
    return v_flex().gap(2).children(
      kids.slice(0, 30).map((r) =>
        h_flex()
          .id(`kid-${r.path}`)
          .h(28)
          .px(6)
          .gap(8)
          .items_center()
          .rounded(RADIUS.md)
          .cursor_pointer()
          .hover((s) => s.bg(k.accent))
          .on_click((_e, cx) => (this.goTo(r.key, cx), cx.notify()))
          .child(r.cells ? h_flex().gap(6).flex_1().min_w(0).items_center().child(Mono(r.cells.ref.split(/\s+/)[0])).child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).child(r.cells.title)) : this.spans(r.text, k, { size: TEXT.sm, grow: true }))
          .when(Boolean(r.children?.length) && r.kind === "plain", (e) => e.child(CountPill(r.children.length))),
      ),
    );
  }

  infoBlock(row, k) {
    return v_flex().gap(6).children((row.info ?? []).map((line) => this.spans(line, k, { size: TEXT.sm, wrap: true })));
  }

  facts(pairs, k) {
    return v_flex().gap(8).children(
      pairs
        .filter(([, v]) => v !== null && v !== undefined && v !== "")
        .map(([label, value]) => h_flex().gap(12).items_center().child(div().w(84).flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(label)).child(typeof value === "string" ? div().flex_1().min_w(0).truncate().text_size(TEXT.sm).child(value) : h_flex().flex_1().min_w(0).gap(6).flex_wrap().child(value))),
    );
  }

  section(label, children, k) {
    return v_flex().gap(8).child(Overline(label)).children(children);
  }

  prPanel(row, k, cx) {
    const it = row.item ?? {};
    const got = this.detail("pr_merge", this.prArgs(row), row.key, cx);
    const d = got.value && !got.value.error ? got.value : null;
    const failing = it.ci === "FAILURE" || it.ci === "ERROR";
    const threads = this.detail("review_threads", this.prArgs(row), row.key, cx);
    const out = [];
    const ready = d?.ready ?? (this.model?.prs ?? []).find((p) => p.url === row.key)?.ready;
    if (row.kind === "pr" && ready) {
      const c = this.tone(ready.tone, k) ?? k.mutedFg;
      const canMerge = (d?.choices ?? []).some((x) => x.do === "merge" || x.do === "enqueue");
      out.push(
        v_flex()
          .gap(10)
          .p(12)
          .rounded(RADIUS.lg)
          .border_1()
          .border_color(alpha(c, 0.45))
          .bg(alpha(c, 0.08))
          .child(h_flex().gap(8).items_center().child(Icon(ready.state === "ready" ? "git-merge" : ready.state === "blocked" ? "circle-x" : "hourglass", { size: 15, color: c })).child(div().flex_1().text_size(TEXT.sm).font_medium().text_color(c).child(ready.text)).when(Boolean(got.loading), (e) => e.child(Spinner({ size: 12, color: k.mutedFg }))))
          .when(!d && Boolean(got.error), (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`GitHub: ${got.error ?? got.value?.error}`)))
          .when(Boolean(d?.why) && !canMerge, (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(d.why)))
          .child(
            h_flex()
              .gap(6)
              .flex_wrap()
              .child(canMerge ? ToneButton("panel-merge", { label: d.choices[0].do === "enqueue" ? "Add to merge queue" : "Merge…", tone: k.success, kbd: "M", icon: "git-merge", onClick: (_e, cx) => this.openMerge(cx, row) }) : Button("panel-merge", { label: "Merge…", icon: "git-merge", size: "sm", variant: "outline", kbd: "M", onClick: (_e, cx) => this.openMerge(cx, row) }))
              .when(Boolean(d?.auto), (e) => e.child(Chip(`auto-merge · ${String(d.auto).toLowerCase()}`, { color: k.info, icon: "zap" }))),
          ),
      );
    }
    const card = d?.card;
    out.push(
      this.facts(
        [
          ["Branch", it.branch ? h_flex().gap(4).items_center().child(Mono(it.branch, { color: k.foreground })).child(Icon("arrow-up", { size: 12, color: k.dim })).child(Mono(it.base ?? "")) : null],
          ["Author", row.kind === "review" ? h_flex().gap(6).items_center().child(Avatar(it.authorName || it.author || "?", { size: 18 })).child(div().text_size(TEXT.sm).child(it.authorName || it.author || "")) : null],
          ["Changes", card ? h_flex().gap(6).child(Mono(`+${card.additions}`, { color: k.success })).child(Mono(`−${card.deletions}`, { color: k.destructive })).child(Mono(`${card.files} files`)) : null],
          ["Checks", card ? h_flex().w_full().gap(8).items_center().child(div().flex_1().child(SegBar("checks", [{ n: card.checks.pass, color: k.success }, { n: card.checks.fail, color: k.destructive }, { n: card.checks.pending, color: k.warning }, { n: card.checks.skip, color: k.border }]))).child(Mono(`${card.checks.pass}/${card.checks.pass + card.checks.fail + card.checks.pending}`)) : null],
          ["Reviews", card?.reviewers?.length ? h_flex().gap(4).children(card.reviewers.slice(0, 6).map((r) => h_flex().id(`rv-${r.login}`).gap(4).items_center().tooltip(`${r.login}: ${String(r.state).toLowerCase().replace("_", " ")}`).child(Avatar(r.login, { size: 20 })).child(Icon(r.state === "APPROVED" ? "check" : r.state === "CHANGES_REQUESTED" ? "message-square" : "clock", { size: 12, color: r.state === "APPROVED" ? k.success : r.state === "CHANGES_REQUESTED" ? k.destructive : k.mutedFg })))) : null],
          ["Labels", card?.labels?.length ? h_flex().gap(4).flex_wrap().children(card.labels.map((l) => Chip(l.name, { color: l.color ? `#${l.color}` : undefined }))) : null],
          ["Updated", it.updatedAt ? (row.cells?.age ? `${row.cells.age} ago` : "") : null],
        ],
        k,
      ),
    );
    const t = threads.value?.threads ?? [];
    if (t.length) out.push(this.section(`${t.length} unresolved thread${t.length === 1 ? "" : "s"}`, t.slice(0, 6).map((th) => v_flex().gap(2).p(8).rounded(RADIUS.md).bg(k.muted).child(h_flex().gap(6).items_center().child(Avatar(th.author ?? "?", { size: 16 })).child(div().text_size(TEXT.xs).font_medium().child(th.author ?? "")).child(div().flex_1()).child(Mono(`${th.path}${th.line ? `:${th.line}` : ""}`))).child(div().text_size(TEXT.sm).text_color(k.foreground).child(String(th.body ?? "").split("\n").find((l) => l.trim()) ?? ""))), k));
    else if (threads.loading) out.push(Skeleton({ w: 220 }));
    if (failing) {
      const tests = this.detail("failed_tests", this.prArgs(row), row.key, cx).value;
      const log = this.detail("ci_log", this.prArgs(row), row.key, cx).value;
      const names = (tests?.tests ?? []).map((x) => (typeof x === "string" ? x : x.name ?? x.title ?? ""));
      if (names.length) out.push(this.section(`${names.length} failing test${names.length === 1 ? "" : "s"}`, names.slice(0, 5).map((n) => h_flex().gap(6).items_center().child(Icon("circle-x", { size: 12, color: k.destructive })).child(div().truncate().text_size(TEXT.sm).child(n))), k));
      const lines = log?.lines ?? [];
      if (lines.length || log?.message)
        out.push(this.section("Failing job's log", [v_flex().p(8).rounded(RADIUS.md).bg(k.muted).children((lines.length ? lines.slice(-6) : [log.message]).map((l) => div().truncate().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(String(l).replace(/\x1b\[[0-9;]*m/g, ""))))], k));
    }
    if (card?.body) out.push(this.section("Description", [div().text_size(TEXT.sm).text_color(k.mutedFg).child(card.body.slice(0, 600))], k));
    out.push(this.section("From the inbox", [this.infoBlock(row, k)], k));
    return out;
  }

  ticketPanel(row, k, cx) {
    const t = (this.model?.tickets ?? []).find((x) => x.id === row.key) ?? {};
    const got = this.detail("ticket_card", { id: row.key }, row.key, cx);
    const card = got.value && !got.value.error ? got.value : null;
    const states = card?.states ?? [];
    const out = [];
    out.push(
      this.facts(
        [
          ["Status", h_flex().gap(6).items_center().child(Badge(card?.state?.name ?? t.status ?? "", "outline", { id: "status-badge", onClick: () => this.statusMenu(row), tooltip: "Change the status  i" })).child(Button("status-menu", { icon: "chevron-down", variant: "ghost", size: "icon-sm", tooltip: "Change the status  i", onClick: () => this.statusMenu(row) }))],
          ["Project", t.project ?? null],
          ["Assignee", card ? (card.assignee ? h_flex().gap(6).items_center().child(Avatar(card.assignee, { size: 18 })).child(div().text_size(TEXT.sm).child(card.mine ? `${card.assignee} (you)` : card.assignee)) : Button("assign-me", { label: "Assign me", icon: "user-plus", size: "sm", variant: "outline", onClick: (_e, cx) => cx.spawn(async (cx) => (await this.call(cx, { fn: "assign", args: [{ id: row.key }, "me"] }), this.forget(`ticket_card ${row.key}`))) })) : null],
          ["Priority", card?.priority && card.priority !== "No priority" ? card.priority : null],
          ["Branch", card?.branch ? Mono(card.branch, { color: k.foreground }) : null],
          ["Time", this.timeChip(row.key, k)],
          ["Claude", t.cost ? Mono(`$${t.cost.toFixed(2)}`, { color: k.foreground }) : null],
          ["Labels", card?.labels?.length ? h_flex().gap(4).flex_wrap().children(card.labels.map((l) => Chip(l.name, { color: l.color }))) : null],
        ],
        k,
      ),
    );
    if (states.length) out.push(this.section("Move to", [Pills("ticket-states", states.map((s) => [s.name, s.name]), card.state?.name, (name, cx) => this.command(`inbox:state:${row.key}:${name}`, cx))], k));
    if (got.error || got.value?.error) out.push(div().text_size(TEXT.xs).text_color(k.dim).child(got.error ?? got.value.error));
    if (card?.description) out.push(this.section("Description", [div().text_size(TEXT.sm).text_color(k.mutedFg).child(card.description.slice(0, 700))], k));
    const prs = (row.children ?? []).filter((r) => r.kind === "pr");
    if (prs.length) out.push(this.section(`${prs.length} PR${prs.length === 1 ? "" : "s"}`, prs.map((p) => h_flex().id(`tpr-${p.path}`).gap(8).items_center().p(6).rounded(RADIUS.md).cursor_pointer().hover((s) => s.bg(k.accent)).on_click((_e, cx) => (this.goTo(p.key, cx), cx.notify())).children(p.cells.lead.map((l) => Icon(l.icon, { size: 13, color: this.tone(l.tone, k) ?? k.mutedFg }))).child(Mono(p.cells.ref)).child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).child(p.cells.title))), k));
    out.push(this.section("From the inbox", [this.infoBlock(row, k)], k));
    return out;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: the dialogs

  dialogEl(k, cx) {
    const d = this.dialog;
    let title = d.title;
    let description = d.description;
    let body = null;
    let actions = [Button("dlg-cancel", { label: "Cancel", variant: "outline", kbd: "esc", onClick: (_e, cx) => this.closeDialog(cx) })];
    const choiceList = (choices, onPick) =>
      v_flex().gap(4).children(
        choices.map((c, i) =>
          h_flex()
            .id(`dlg-choice-${i}`)
            .h(36)
            .px(10)
            .gap(10)
            .items_center()
            .rounded(RADIUS.md)
            .border_1()
            .border_color(k.border)
            .cursor_pointer()
            .hover((s) => s.bg(k.accent))
            .on_click((_e, cx) => onPick(c, cx))
            .child(Kbd(String(i + 1)))
            .child(div().flex_1().text_size(TEXT.base).child(c.label)),
        ),
      );
    if (d.kind === "confirm") {
      actions.push(Button("dlg-ok", { label: d.confirm ? d.confirm[0].toUpperCase() + d.confirm.slice(1) : "Continue", variant: d.destructive ? "destructive" : "default", kbd: "⏎", onClick: (_e, cx) => (this.closeDialog(cx), cx.spawn(async (cx) => d.run(cx))) }));
    } else if (d.kind === "pick") {
      body = choiceList(d.choices, (c, cx) => (this.closeDialog(cx), cx.spawn(async (cx) => c.run(cx))));
    } else if (d.kind === "merge") {
      const it = d.row.item;
      const got = this.details.get(`pr_merge ${d.row.key}`);
      title = `Merge ${it.repo}#${it.number}`;
      description = it.title;
      const choices = this.mergeChoices(d.row);
      body = v_flex()
        .gap(12)
        .child(got?.value?.ready ? this.readyLine(got.value.ready, k) : got?.loading ? h_flex().gap(8).items_center().child(Spinner({ size: 12 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Asking GitHub whether it can merge…")) : div().text_size(TEXT.sm).text_color(k.mutedFg).child(got?.error ? `GitHub did not say (${got.error}): these are the usual methods` : ""))
        .when(choices.length > 0, (e) => e.child(choiceList(choices, (c, cx) => this.mergeChoice(cx, d.row, c))))
        .when(!choices.length && Boolean(got?.value?.why), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(got.value.why)));
    } else if (d.kind === "start") {
      const p = d.plan;
      title = d.row.kind === "ticket" ? `Start ${d.row.key}` : `Work on ${d.row.cells?.ref ?? d.row.name}`;
      description = d.row.cells?.title ?? d.row.name;
      body = !p
        ? h_flex().gap(8).items_center().child(Spinner({ size: 12 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Checking the ticket, the PR and your checkouts…"))
        : v_flex()
            .gap(8)
            .children((p.blockers ?? []).map((b) => h_flex().gap(8).items_start().child(Icon("circle-x", { size: 14, color: k.destructive })).child(div().flex_1().text_size(TEXT.sm).text_color(k.destructive).child(b))))
            .children((p.steps ?? []).map((s, i) => h_flex().gap(8).items_start().child(h_flex().size(18).flex_none().rounded(9).bg(k.muted).text_size(TEXT.xs).text_color(k.mutedFg).items_center().justify_center().child(String(i + 1))).child(div().flex_1().text_size(TEXT.sm).child(s))));
      actions.push(Button("dlg-start", { label: p?.ok ? "Start" : "Can't start", icon: "play", kbd: "⏎", disabled: !p?.ok, onClick: (_e, cx) => p?.ok && this.startNow(cx) }));
    } else if (d.kind === "file") {
      title = "File an issue";
      description = "A new issue, assigned to you";
      body = v_flex().gap(8).child(TextField(this.issueTitle)).child(TextField(this.issueBody)).child(TextField(this.issueTeam));
      actions.push(Button("dlg-file", { label: "File it", icon: "file-plus", kbd: "⏎", onClick: (_e, cx) => this.fileIssue(cx) }));
    }
    return div()
      .id("inbox-dialog")
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
            v_flex()
              .id("inbox-dialog-card")
              .w(460)
              .p(20)
              .gap(14)
              .rounded(RADIUS.lg)
              .border_1()
              .border_color(k.border)
              .bg(k.popover)
              .shadow_lg()
              .child(v_flex().gap(4).child(div().text_size(TEXT.lg).font_semibold().child(title ?? "")).when(Boolean(description), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(description))))
              .when(Boolean(body), (e) => e.child(body))
              .child(h_flex().justify_end().gap(8).children(actions)),
          ),
      );
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: the page

  /** The feed's error lines, the same one said twice (by the hub and the plugin) once. */
  errorsEl(errors, k) {
    const seen = new Set();
    const lines = errors.filter((x) => {
      const key = x.toLowerCase().replace(/[^a-z0-9]/g, "");
      return !seen.has(key) && seen.add(key);
    });
    return v_flex().mx(24).mt(12).px(12).py(8).gap(2).flex_none().rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).children(lines.map((x) => h_flex().gap(8).items_center().child(Icon("triangle-alert", { size: 13, color: k.destructive })).child(div().truncate().text_size(TEXT.sm).text_color(k.destructive).child(x))));
  }

  tiles(k) {
    const c = this.model?.counts ?? {};
    const jump = (pred) => (_e, cx) => {
      const row = this.all().find(pred);
      if (row) this.goTo(row.key, cx);
      cx.notify();
    };
    const prs = this.model?.prs ?? [];
    return h_flex()
      .px(24)
      .pt(16)
      .gap(12)
      .flex_none()
      .child(Metric("m-needs", { label: "Needs you", value: c.needs_you ?? 0, tone: c.needs_you ? k.destructive : k.foreground, tint: Boolean(c.needs_you), icon: "triangle-alert", tooltip: "failing, sent back or asked of you", onClick: jump((r) => (r.kind === "pr" && prs.find((p) => p.url === r.key)?.needs_you) || (r.kind === "review" && r.item?.isDirect)) }))
      .child(Metric("m-reviews", { label: "Reviews", value: c.reviews ?? 0, icon: "eye", tooltip: `${c.direct ?? 0} for you`, onClick: jump((r) => r.kind === "review") }))
      .child(Metric("m-ready", { label: "Ready to land", value: c.ready ?? 0, tone: c.ready ? k.success : k.foreground, icon: "git-merge", tooltip: "approved and green", onClick: jump((r) => r.kind === "pr" && prs.find((p) => p.url === r.key)?.ready?.state === "ready") }))
      .child(Metric("m-work", { label: "In progress", value: c.in_progress ?? 0, icon: "briefcase", tooltip: `${c.tickets ?? 0} tickets · ${c.prs ?? 0} PRs`, onClick: jump((r) => r.kind === "ticket") }))
      .child(Metric("m-clean", { label: "To clean up", value: c.cleanup ?? 0, tone: c.cleanup ? k.warning : k.foreground, icon: "folder-git", tooltip: `${c.worktrees ?? 0} worktrees`, onClick: jump((r) => r.kind === "worktree") }));
  }

  list(k, cx) {
    const m = this.model;
    if (!m) {
      if (this.status.startsWith("error")) return ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) });
      return v_flex().p(16).gap(12).children([0, 1, 2, 3, 4, 5].map((i) => h_flex().gap(10).items_center().child(Skeleton({ w: 16, h: 16, radius: RADIUS.sm })).child(Skeleton({ w: 200 + ((i * 89) % 240) }))));
    }
    if (m.state !== "ok") return Empty({ icon: "inbox", title: m.state === "empty" ? "No inbox yet" : "Refreshing", description: m.message, action: Button("empty-refresh", { label: "Refresh", icon: "refresh-cw", kbd: "R", onClick: (_e, cx) => this.refresh(cx) }) });
    const vis = this.visible();
    if (this.filter.trim() && !vis.length) return Empty({ icon: "search", title: "No row matches", description: `Nothing has every word of “${this.filter}”.` });
    const at = vis.findIndex((v) => v.row.path === this.sel);
    // Rows are built only when the scroller draws them: a getter per row.
    const self = this;
    const rows = vis.map((v) => ({ height: this.height(v), get el() { return self.rowEl(v, k, cx); } }));
    return this.scroller.view("inbox-rows", rows, at);
  }

  render(cx) {
    const k = tokens();
    const m = this.model;
    const narrow = (this.width ?? 1200) < 980;
    const changed = m?.state === "ok" ? this.changedCount() : 0;
    const parts = [];
    if (m?.updated) parts.push(`updated ${m.updated}`);
    if (m?.counts?.needs_you) parts.push(`${m.counts.needs_you} need you`);
    if (m?.counts?.ready) parts.push(`${m.counts.ready} ready to land`);
    if (changed) parts.push(`${changed} changed since you looked`);
    const root = v_flex()
      .id("inbox")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        const w = size.width / (settings().zoom ?? 1);
        if (Math.abs(w - (this.width ?? 0)) > 1) {
          this.width = w;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => {
        if (!this.filtering && !this.palette && !this.dialog) this.handle.focus();
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "inbox",
          title: "Inbox",
          description: m ? parts.join(" · ") || "Nothing waiting" : "Loading…",
          actions: [
            Button("file-issue", { label: "File issue", icon: "file-plus", variant: "outline", size: "sm", kbd: "I", onClick: (_e, cx) => this.openFile(cx) }),
            Button("commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
            Button("refresh", { label: "Refresh", icon: "refresh-cw", variant: "outline", size: "sm", kbd: "R", onClick: (_e, cx) => this.refresh(cx) }),
          ],
        }),
      )
      .when(m?.state === "ok" && !narrow, (e) => e.child(this.tiles(k)))
      .when(Boolean(m?.errors?.length), (e) => e.child(this.errorsEl(m.errors, k)))
      .child(
        h_flex()
          .px(24)
          .py(12)
          .gap(8)
          .flex_none()
          .items_center()
          .child(h_flex().id("filter").flex_1().on_mouse_down("left", (_e, cx) => this.startFilter(cx)).child(SearchField(this.search, { active: this.filtering, kbd: "/" })))
          .when(this.picked.size > 0, (e) => e.child(Badge(`${this.picked.size} picked`, "info", { id: "picked-badge", onClick: (_e, cx) => (this.picked.clear(), cx.notify()), tooltip: "Drop the picks  esc" })))
          .when(changed > 0, (e) => e.child(Badge(`${changed} changed`, "secondary", { dot: k.ring })))
          .when(this.status !== "live", (e) => e.child(Badge(this.status.startsWith("error") ? "offline" : "connecting", this.status.startsWith("error") ? "destructive" : "secondary", { dot: this.status.startsWith("error") ? k.destructive : k.warning }))),
      )
      .child(
        (narrow ? v_flex() : h_flex())
          .flex_1()
          .min_h(0)
          .child(v_flex().flex_1().min_w(0).min_h(0).h_full().px(16).child(this.list(k, cx)))
          .when(m?.state === "ok" && Boolean(this.current() || this.picked.size), (e) => {
            const panel = this.panel(k, cx, { narrow });
            return panel ? e.child(panel) : e;
          }),
      )
      .child(KeyHints([["j k", "move"], ["⏎", "main"], ["space", "fold"], ["v V", "pick"], ["/", "filter"], ["w", "group"], ["R", "refresh"], ["ctrl p", "commands"]]));
    let out = root;
    const toasts = this.toasts.render();
    if (toasts) out = out.child(toasts);
    if (this.dialog) out = out.child(this.dialogEl(k, cx));
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id("inbox-palette-backdrop")
          .absolute()
          .inset_0()
          .bg("#00000055")
          .on_mouse_down("left", (_e, cx) => this.closePalette(cx))
          .child(
            v_flex()
              .absolute()
              .top(72)
              .left(0)
              .right(0)
              .items_center()
              .child(v_flex().id("inbox-palette").w(600).max_h(440).rounded(RADIUS.lg).border_1().border_color(k.border).bg(k.popover).shadow_lg().overflow_hidden().on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(Command("inbox-commands", { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id }))),
          ),
      );
    }
    return out;
  }
}

/** The icon of an action, by its label or its call. */
const ICON = {
  review: "eye",
  open: "external",
  open_url: "external",
  "mute repo": "volume-x",
  unmute: "volume-x",
  "fix ci": "wrench",
  rerun: "refresh-cw",
  redeploy: "rocket",
  preview: "globe",
  deploy: "rocket",
  ArgoCD: "gauge",
  Dashboard: "gauge",
  Vercel: "gauge",
  merge: "git-merge",
  "auto-merge": "zap",
  "to draft": "git-pull-request-draft",
  "mark ready": "git-pull-request",
  "work on it": "play",
  start: "play",
  plan: "map",
  continue: "play",
  talk: "message-circle",
  status: "circle-dot",
  done: "check",
  dismiss: "x",
  "extend 2 days": "clock",
  stop: "square",
  "close space": "x",
  "close tab": "x",
  "remove worktree": "trash",
  remove: "trash",
  go: "chevron-right",
};
