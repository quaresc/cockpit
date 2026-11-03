// The show view: what a Claude session shows beside itself through the MCP `show` tool, block by block, drawn natively
// (markdown, sortable tables, charts with hover, flow and Mermaid diagrams, sequence charts, pictures, before/after
// compares, diffs with line comments, live logs, CI runs, questions, the session's living blueprint). As a pane (`props.name`) it shows that view; in
// the right panel it follows the focused agent's views, one tab each. Answers go back through the hub.

import { div, image, PathBuilder } from "gpui-kit";
import { h_flex, v_flex, v_virtual_list, Scrollbar, InputState, Input, TextView } from "gpui-base";
import { exec, appHome, focused, panes, run, layout, shared, setShared, props, settings } from "herdr";
import * as hub from "./hub.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Badge, Button, Kbd, Empty, Toaster, Progress, Spinner, openMenu, dur } from "./ui.js";
import { Chart, zoomed, compact } from "./plot.js";
import { parseFlowchart, flowGraph, laidOut, Graph, graphSize, Sequence, sequenceSize, fit } from "./diagram.js";
import { DiffView } from "./diff.js";

const ICON = {
  markdown: "file-text", table: "table", chart: "chart-column", flow: "workflow", mermaid: "workflow", blueprint: "map", sequence: "list-ordered",
  image: "image", compare: "square-split", diff: "git-compare", log: "scroll-text", ci: "activity", choice: "circle-help", confirm: "shield-check",
};
const ROW = 32;
const hhmm = (ms) => {
  if (!ms) return "";
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
};
const ago = (ms) => (!ms ? "" : Date.now() - ms < 45000 ? "just now" : `${dur(Date.now() - ms)} ago`);
// Answers a block takes again and again rather than once.
const REPEATED = ["comment", "review", "blueprint"];
const csvField = (v) => {
  const t = v === null || v === undefined ? "" : typeof v === "boolean" ? (v ? "True" : "False") : String(v);
  return /[",\n\r]/.test(t) ? `"${t.replaceAll('"', '""')}"` : t;
};

/** The blueprint canvas's dot grid, a dot every 16 px behind the boxes; paths take raw pixels, hence the zoom. */
function dots(k, height) {
  const z = settings().zoom ?? 1, s = 16;
  let grid = PathBuilder.stroke(1.6 * z).dash_array([1.6 * z, (s - 1.6) * z]);
  for (let y = s / 2; y < height; y += s) grid = grid.move_to("0%", `${(y / height) * 100}%`).line_to("100%", `${(y / height) * 100}%`);
  return window.paint_path(grid.build(), alpha(k.foreground, 0.1)).absolute().inset_0();
}

/** Whether the layout shows the block holding `app` (a pane copy, outside the layout, always is). */
function shownInLayout(app) {
  const walk = (node, ok) => {
    if (!node || typeof node !== "object") return null;
    if (node.content?.mod === "cockpit" && node.content?.props?.app === app) return ok && !node.hidden;
    for (const child of node.children ?? []) {
      const visible = ok && !node.hidden && !child.hidden && (node.type !== "stack" || (node.active ?? node.children?.[0]?.id) === child.id);
      const found = walk(child, visible);
      if (found !== null) return found;
    }
    return null;
  };
  return walk(layout(), true) ?? true;
}

export default class Show {
  init(p, cx) {
    const pr = props() ?? p ?? {};
    this.name = pr.name ?? null;
    this.panel = !this.name;
    this.data = null;
    this.status = "connecting";
    this.pick = this.remembered();
    this.terminal = null;
    this.first = 0;
    this.focus = null;
    this.ui = new Map();
    this.live = new Map();
    this.diffs = new Map();
    this.images = new Map();
    this.width = 480;
    this.palette = null;
    this.toasts = new Toaster();
    this.handle = cx.focus_handle();
    this.input = InputState.new({ placeholder: "Leave a comment…" });
    this.input.on("submit", (_e, cx) => {
      this.activeDiff()?.commit();
      this.handle.focus();
      cx.notify();
    });
    // A blueprint's change request: the box clicked (`asking`) and what should change about it.
    this.asking = null;
    this.ask = InputState.new({ placeholder: "What should change in this step?" });
    this.ask.on("submit", (_e, cx) => this.sendAsk(cx));
    this.other = InputState.new({ placeholder: "Something else…" });
    this.other.on("submit", (_e, cx) => {
      const n = this.focus;
      const text = this.other.value().trim();
      if (n !== null && text) this.answer(cx, n, "choice", text, text);
    });
    this.subscribe(cx);
    cx.timer.every(1000, (cx) => this.tick(cx));
    if (this.name) {
      // A pane split from the agent's is focused for a moment: that is how this copy learns which pane it is.
      const learn = () => {
        const f = focused();
        const mine = f?.pane && panes().find((x) => x.pane === f.pane && x.mod === "cockpit");
        if (mine && !this.pane) this.pane = f.pane;
      };
      learn();
      cx.timer.after(150, () => learn());
    }
  }

  close() {
    this.sub?.stop();
  }

  /** The panel's picked view survives a reload of the mod. */
  remembered() {
    try {
      return localStorage.getItem("show.pick") || null;
    } catch {
      return null;
    }
  }

  subscribe(cx) {
    try {
      if (this.panel) localStorage.setItem("show.pick", this.pick ?? "");
    } catch {}
    this.sub?.stop();
    const args = this.name ? { name: this.name } : { terminal: this.terminal, pick: this.pick };
    this.sub = hub.subscribe(
      cx,
      { views: [{ name: "show", args }] },
      (m, cx) => {
        if (m.view === "show") this.take(m.value, cx);
        if (m.focus === "show") this.goTo(m.target?.key ?? m.target, cx);
        cx.notify();
      },
      (state, cx) => {
        this.status = state;
        cx.notify();
      },
    );
  }

  take(value, cx) {
    const before = this.data?.view;
    this.data = value;
    const view = value?.view;
    if (!view) return;
    if (!before || before.name !== view.name || before.at !== view.at) {
      this.first = 0;
      if (before?.name !== view.name) [this.focus, this.asking] = [null, null];
      if (this.pendingGo?.startsWith(`${view.name}#`)) {
        const go = this.pendingGo;
        this.pendingGo = null;
        this.goTo(go, cx);
      }
      if (before && before.name === view.name) this.toasts.push(cx, "The view was updated", { kind: "info", ms: 1800 });
    }
    if (this.focus !== null && this.focus >= view.blocks.length) this.focus = null;
    for (const b of view.blocks) {
      if (b.type === "image" && b.src) this.ensure(b.src, cx);
      if (b.type === "compare") [b.before, b.after].forEach((s) => s && this.ensure(s, cx));
    }
    if (this.panel) {
      const asks = (value.views ?? []).reduce((n, v) => n + (v.asks ?? 0), 0);
      const badge = asks ? { count: asks, tone: "warning" } : null;
      if (JSON.stringify(shared("badge.show") ?? null) !== JSON.stringify(badge)) setShared("badge.show", badge);
    }
  }

  view() {
    return this.data?.view ?? null;
  }

  /** A block's own UI state, kept while the view's call (its `at`) stays the same. */
  state(b, init = {}) {
    const key = `${this.view()?.name}:${this.view()?.at}:${b.n}`;
    if (!this.ui.has(key)) this.ui.set(key, { ...init });
    return this.ui.get(key);
  }

  liveKey(b) {
    return `${this.view()?.name}:${this.view()?.at}:${b.n}`;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Live blocks: a diff read, a log run, a command's rows, each on its own clock while the view is shown

  tick(cx) {
    const view = this.view();
    if (!view || !shownInLayout(this.panel ? "show" : "__pane__")) return;
    const now = Date.now();
    // A blueprint's "updated 2m ago" moves on.
    if (now - (this.aged ?? 0) >= 15000 && view.blocks.some((b) => b.type === "blueprint")) {
      this.aged = now;
      cx.notify();
    }
    for (const b of view.blocks) {
      if (!b.live) continue;
      const key = this.liveKey(b);
      const l = this.live.get(key) ?? { next: 0 };
      this.live.set(key, l);
      if (l.busy || now < l.next) continue;
      if (b.type === "diff" && b.patch && l.data) continue;
      l.busy = true;
      cx.spawn(async (cx) => {
        try {
          const data = await hub.view("show-block", { name: view.name, n: b.n, since: l.data?.hash ?? null });
          if (!data?.same) {
            l.data = data;
            if (b.type === "diff") this.diff(b).set(data?.error ? null : data);
            if (b.type === "log" && this.state(b, { follow: true }).follow) cx.timer.after(30, () => this.logScroll?.get(key)?.scroll_to_bottom());
            cx.notify();
          }
        } catch (e) {
          l.data = { error: String(e?.message ?? e) };
          cx.notify();
        }
        l.busy = false;
        l.next = Date.now() + 1000 * (b.type === "diff" ? 3 : b.every ?? 5);
      });
    }
  }

  diff(b) {
    const key = this.liveKey(b);
    if (!this.diffs.has(key)) this.diffs.set(key, new DiffView(`show-diff-${b.n}`, this.input));
    return this.diffs.get(key);
  }

  activeDiff() {
    const view = this.view();
    if (!view) return null;
    for (const b of view.blocks) if (b.type === "diff" && this.diffs.get(this.liveKey(b))?.composer) return this.diffs.get(this.liveKey(b));
    return null;
  }

  /** A picture copied into the app's folder (it draws pictures only from there), with its size. */
  ensure(pic, cx) {
    if (!pic?.key || this.images.has(pic.key)) return;
    this.images.set(pic.key, { state: "loading" });
    const rel = `data/cockpit/show/${pic.key}`;
    const fetch = pic.remote ? 'curl -fsSL --max-time 20 -o "$1" "$2"' : 'cp "$2" "$1"';
    const script = `mkdir -p "$(dirname "$1")" && { [ -s "$1" ] || ${fetch}; } && sips -g pixelWidth -g pixelHeight "$1"`;
    cx.spawn(async (cx) => {
      try {
        const r = await exec("sh", ["-c", script, "sh", `${appHome()}/${rel}`, pic.src]);
        const w = Number(/pixelWidth: (\d+)/.exec(r.stdout)?.[1]), h = Number(/pixelHeight: (\d+)/.exec(r.stdout)?.[1]);
        this.images.set(pic.key, r.code === 0 ? { state: "ready", rel, w: w || 800, h: h || 500 } : { state: "failed", error: r.stderr.trim() || "could not read the picture" });
      } catch (e) {
        this.images.set(pic.key, { state: "failed", error: String(e?.message ?? e) });
      }
      cx.notify();
    });
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Answers and actions

  answer(cx, n, type, value, label) {
    const view = this.view();
    if (!view) return;
    const b = view.blocks[n];
    const st = this.state(b);
    if (st.sent && !REPEATED.includes(type)) return;
    if (!REPEATED.includes(type)) st.sent = label;
    cx.spawn(async (cx) => {
      try {
        const r = await hub.act("show", "answer", [view.name, n, type, value, label]);
        if (r.error) {
          st.sent = null;
          this.toasts.push(cx, r.error, { kind: "error" });
        } else this.toasts.push(cx, r.note || "Sent to the session", { kind: "success" });
      } catch (e) {
        st.sent = null;
        this.toasts.push(cx, String(e?.message ?? e), { kind: "error" });
      }
      cx.notify();
    });
    cx.notify();
  }

  sendComments(cx, b) {
    const d = this.diff(b);
    const comments = d.review();
    if (!comments.length) return;
    const label = comments.map((c, i) => `${i + 1}) ${c.path}:${c.line}: ${c.text.replace(/\s+/g, " ")}`).join(" ");
    if (comments.length === 1) this.answer(cx, b.n, "comment", { path: comments[0].path, line: comments[0].line, text: comments[0].text }, `${comments[0].path}:${comments[0].line}`);
    else this.answer(cx, b.n, "review", { comments }, label);
    d.comments = [];
  }

  /** Opens the change request composer on a blueprint's box. */
  askAbout(b, node, cx) {
    this.asking = { n: b.n, id: node.id, title: node.title, sub: node.sub, status: node.status };
    this.focus = b.n;
    this.ask.set_value("");
    this.ask.focus();
    cx.notify();
  }

  /** `<block>:<box id>`: the composer on that box, as a click on it opens it. */
  askById(arg, cx) {
    const [n, ...id] = arg.split(":");
    const b = this.view()?.blocks?.[Number(n)];
    const node = b?.type === "blueprint" ? parseFlowchart(b.text).nodes.get(id.join(":")) : null;
    if (node) this.askAbout(b, node, cx);
  }

  sendAsk(cx) {
    const a = this.asking;
    const text = this.ask.value().trim();
    if (!a || !text) return;
    this.answer(cx, a.n, "blueprint", { node: a.id, label: a.title, text }, a.title);
    this.asking = null;
    this.ask.set_value("");
    this.handle.focus();
    cx.notify();
  }

  openUrl(url) {
    if (url) exec("open", [url]).catch(() => {});
  }

  hide() {
    if (this.pane) run("pane.close", { pane: this.pane });
    else run("pane.close");
  }

  copyCsv(cx, b) {
    const rows = this.rowsOf(b);
    const text = [b.columns, ...rows.map((r) => r.values)].map((r) => r.map(csvField).join(",")).join("\r\n") + "\r\n";
    cx.write_to_clipboard(text);
    this.toasts.push(cx, `Copied ${b.title ?? "the table"} as CSV`, { kind: "success" });
  }

  goTo(key, cx) {
    if (typeof key !== "string") return;
    const [name, block] = key.split("#");
    if (this.panel && name && name !== this.view()?.name) {
      // The block is found once that view arrives.
      this.pick = name;
      this.pendingGo = block ? key : null;
      this.subscribe(cx);
      return;
    }
    if (this.name && name !== this.name) return;
    const view = this.view();
    const n = view && block ? Math.max(0, view.blocks.findIndex((b) => String(b.n) === block || (b.id ?? null) === block || b.title === block)) : 0;
    this.first = n;
    this.focus = view && this.interactive(view.blocks[n] ?? {}) ? n : null;
    this.handle.focus();
  }

  command(c, cx) {
    if (c === "focus" || c === "show:show") this.handle.focus();
    const [scope, action, ...rest] = c.split(":");
    if (scope !== "show") return;
    const arg = rest.join(":");
    if (action === "hide" && arg === this.name) this.hide();
    else if (action === "go") this.goTo(arg, cx);
    else if (action === "copy") {
      const b = this.view()?.blocks?.[Number(arg)];
      if (b) this.copyCsv(cx, b);
    } else if (action === "first") this.first = Number(arg) || 0;
    else if (action === "ask") this.askById(arg, cx);
    else if (action === "page") this.openUrl(this.view()?.url);
    cx.notify();
  }

  menu(b) {
    openMenu([
      { label: "Show from here", icon: "arrow-up", command: `show:first:${b.n}` },
      b.type === "table" && { label: "Copy as CSV", icon: "copy", command: `show:copy:${b.n}` },
      this.view()?.url && { label: "Open the view's page", icon: "external", kbd: "o", command: "show:page:" },
    ]);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

  interactive(b) {
    return ["table", "chart", "diff", "log", "compare", "choice", "confirm", "flow", "mermaid", "blueprint", "sequence"].includes(b.type);
  }

  key(e, cx) {
    const view = this.view();
    const key = e.keystroke;
    if (this.palette) return this.paletteKey(e, cx);
    const b = this.focus !== null ? view?.blocks?.[this.focus] : null;
    if (b && this.blockKey(b, e, cx)) return true;
    const n = view?.blocks?.length ?? 0;
    switch (key) {
      case "ctrl-p": this.palette = { at: 0 }; return true;
      case "j": case "down": case "space": case "f": case "pagedown": this.first = Math.min(Math.max(0, n - 1), this.first + 1); return true;
      case "k": case "up": case "b": case "pageup": this.first = Math.max(0, this.first - 1); return true;
      case "g": case "home": this.first = 0; return true;
      case "shift-g": case "end": this.first = Math.max(0, n - 1); return true;
      case "tab": case "n": return this.moveFocus(1), true;
      case "shift-tab": case "shift-n": return this.moveFocus(-1), true;
      case "escape": if (this.focus !== null) { this.focus = null; return true; } return false;
      case "o": this.openUrl(view?.url); return true;
      case "r": this.openReview(); return true;
      case "q": if (this.name) this.hide(); return Boolean(this.name);
    }
    return false;
  }

  moveFocus(by) {
    const blocks = this.view()?.blocks ?? [];
    const stops = blocks.filter((b) => this.interactive(b)).map((b) => b.n);
    if (!stops.length) return;
    const at = stops.indexOf(this.focus);
    this.focus = stops[(at < 0 ? (by > 0 ? 0 : stops.length - 1) : at + by + stops.length) % stops.length];
    if (this.focus < this.first) this.first = this.focus;
  }

  openReview() {
    run("layout.show", { id: "panel" });
    run("layout.select", { stack: "panel-body", child: "review" });
  }

  blockKey(b, e, cx) {
    const key = e.keystroke;
    const st = this.state(b);
    switch (b.type) {
      case "diff": {
        if (key === "shift-s" || key === "ctrl-enter") return this.sendComments(cx, b), true;
        return this.diff(b).key(e);
      }
      case "table": {
        const rows = this.rowsOf(b);
        if (key === "j" || key === "down") st.cursor = Math.min(rows.length - 1, (st.cursor ?? -1) + 1);
        else if (key === "k" || key === "up") st.cursor = Math.max(0, (st.cursor ?? 0) - 1);
        else if (/^[1-9]$/.test(key) && Number(key) <= b.columns.length) this.sortBy(st, Number(key) - 1);
        else return false;
        if (st.cursor !== undefined) this.tableScroll?.get(this.liveKey(b))?.scroll_to_item(st.cursor, "center");
        return true;
      }
      case "chart": {
        const data = this.chartData(b);
        const total = data?.labels?.length ?? 0;
        if (key === "left" || key === "h") st.hover = Math.max(0, (st.hover ?? total) - 1);
        else if (key === "right" || key === "l") st.hover = Math.min(total - 1, (st.hover ?? -1) + 1);
        else if (key === "+" || key === "=" || key === "shift-=") st.window = zoomed(st.window, total, st.hover, true);
        else if (key === "-") st.window = zoomed(st.window, total, st.hover, false);
        else if (key === "0") st.window = null;
        else return false;
        return true;
      }
      case "log": {
        if (key === "shift-g" || key === "end") st.follow = true;
        else if (key === "k" || key === "up" || key === "g") st.follow = false;
        else return false;
        const scroll = this.logScroll?.get(this.liveKey(b));
        if (key === "g") scroll?.scroll_to_item(0, "top");
        if (st.follow) scroll?.scroll_to_bottom();
        return true;
      }
      case "compare": {
        if (key === "h" || key === "left") st.split = Math.max(0, (st.split ?? 50) - 5);
        else if (key === "l" || key === "right") st.split = Math.min(100, (st.split ?? 50) + 5);
        else if (key === "s") st.side = !st.side;
        else return false;
        return true;
      }
      case "blueprint": {
        if (this.asking?.n === b.n) {
          // The composer has the keys; escape closes it.
          if (key === "escape") {
            this.asking = null;
            this.handle.focus();
          }
          return true;
        }
        const scale = st.scale ?? st.fitted ?? 1;
        if (key === "+" || key === "=" || key === "shift-=") st.scale = Math.min(2, scale * 1.2);
        else if (key === "-") st.scale = Math.max(0.3, scale / 1.2);
        else if (key === "0" || key === "f") st.scale = null;
        else return false;
        return true;
      }
      case "flow": case "mermaid": case "sequence": {
        if (key === "+" || key === "=" || key === "shift-=") st.scale = Math.min(2, (st.scale ?? 1) * 1.2);
        else if (key === "-") st.scale = Math.max(0.3, (st.scale ?? 1) / 1.2);
        else if (key === "0") st.scale = null;
        else return false;
        return true;
      }
      case "choice": {
        if (st.sent || b.answered) return false;
        const opts = b.options ?? [];
        if (key === "j" || key === "down") st.at = Math.min(opts.length - 1, (st.at ?? -1) + 1);
        else if (key === "k" || key === "up") st.at = Math.max(0, (st.at ?? 0) - 1);
        else if (key === "enter" && st.at !== undefined) this.answer(cx, b.n, "choice", opts[st.at].label, opts[st.at].label);
        else if (/^[1-9]$/.test(key) && Number(key) <= opts.length) this.answer(cx, b.n, "choice", opts[Number(key) - 1].label, opts[Number(key) - 1].label);
        else return false;
        return true;
      }
      case "confirm": {
        if (st.sent || b.answered) return false;
        if (key === "left" || key === "right") st.yes = !st.yes;
        else if (key === "enter") this.answer(cx, b.n, "confirm", Boolean(st.yes), st.yes ? b.confirm_label : "cancelled");
        else if (key === "y") this.answer(cx, b.n, "confirm", true, b.confirm_label);
        else return false;
        return true;
      }
    }
    return false;
  }

  paletteItems() {
    const view = this.view();
    const items = [];
    for (const b of view?.blocks ?? []) {
      items.push({ label: `Go to: ${b.title ?? b.type}`, run: () => { this.first = b.n; this.focus = this.interactive(b) ? b.n : null; } });
      if (b.type === "table" && !b.live) items.push({ label: `Copy as CSV: ${b.title ?? "table"}`, run: (cx) => this.copyCsv(cx, b) });
      if (b.type === "blueprint") for (const node of parseFlowchart(b.text).nodes.values()) items.push({ label: `Ask for a change: ${node.title}`, run: (cx) => this.askAbout(b, node, cx) });
    }
    if (view?.url) items.push({ label: "Open the view's page in the browser", run: () => this.openUrl(view.url) });
    return items;
  }

  paletteKey(e, cx) {
    const items = this.paletteItems();
    const key = e.keystroke;
    if (key === "escape" || key === "ctrl-p") this.palette = null;
    else if (key === "down" || key === "j") this.palette.at = Math.min(items.length - 1, this.palette.at + 1);
    else if (key === "up" || key === "k") this.palette.at = Math.max(0, this.palette.at - 1);
    else if (key === "enter") {
      items[this.palette.at]?.run(cx);
      this.palette = null;
    }
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  render(cx) {
    const k = tokens();
    if (this.panel) {
      const f = focused();
      if (f?.terminal && f.terminal !== this.terminal) {
        this.terminal = f.terminal;
        cx.timer.after(0, (cx) => this.subscribe(cx));
      }
    }
    const view = this.view();
    const narrow = this.width < 520;
    let root = v_flex().id("show-root").size_full().relative().bg(k.background).text_color(k.foreground).track_focus(this.handle)
      .on_size_change((size, cx) => {
        if (Math.abs(size.width - this.width) > 1 || Math.abs(size.height - (this.height ?? 0)) > 1) {
          [this.width, this.height] = [size.width, size.height];
          cx.notify();
        }
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) {
          cx.stop_propagation();
          cx.notify();
        }
      });
    root = root.child(this.header(k, view, narrow));
    if (!view) {
      const waiting = this.status !== "live" && !this.data;
      root = root.child(waiting
        ? v_flex().flex_1().items_center().justify_center().gap(8).child(Spinner({ size: 18 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Connecting to the cockpit hub…"))
        : Empty({ icon: "layers", title: this.name ? `Waiting for the view ${this.name}` : "Nothing shown yet", description: this.name ? "It appears as soon as the session writes it." : "When an agent calls the show tool (a chart, a diagram, a diff, a question), its view lands here." }));
    } else {
      root = root.child(this.body(k, view, cx, narrow)).child(this.footer(k, view));
    }
    if (this.palette) root = root.child(this.paletteCard(k));
    const toasts = this.toasts.render();
    return toasts ? root.child(toasts) : root;
  }

  header(k, view, narrow) {
    const tabs = this.panel ? this.data?.views ?? [] : [];
    const title = view?.title ?? this.name ?? "Show";
    const head = h_flex().h(44).flex_none().px(12).gap(8).items_center().border_b_1().border_color(k.border)
      .child(h_flex().size(24).flex_none().rounded(RADIUS.md).items_center().justify_center().bg(alpha(k.ring, 0.14)).child(Icon("sparkles", { size: 14, color: k.ring })))
      .child(v_flex().flex_1().min_w(0)
        .child(div().truncate().text_size(TEXT.base).font_semibold().child(title))
        .when(Boolean(view), (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(`${view.name} · ${hhmm(view.at)} · ${view.blocks.length} block${view.blocks.length === 1 ? "" : "s"}`))))
      .when(Boolean(view?.url), (e) => e.child(Button("show-page", { icon: "external", label: narrow ? undefined : "Page", size: narrow ? "icon-sm" : "sm", variant: "ghost", kbd: narrow ? undefined : "o", tooltip: `Open ${view.url}`, onClick: () => this.openUrl(view.url) })))
      .child(Button("show-review", { icon: "git-compare", size: "icon-sm", variant: "ghost", tooltip: "Review the agent's changes (r)", onClick: () => this.openReview() }))
      .when(Boolean(this.name), (e) => e.child(Button("show-hide", { icon: "x", size: "icon-sm", variant: "ghost", tooltip: "Hide (q): its chip in the session's footer brings it back", onClick: () => this.hide() })));
    if (tabs.length < 2) return head;
    const strip = h_flex().id("show-tabs").h(36).flex_none().px(8).gap(4).items_center().border_b_1().border_color(k.border).overflow_x_scrollbar()
      .children(tabs.map((t) => {
        const on = t.name === view?.name;
        return h_flex().id(`show-tab-${t.name}`).h(26).px(8).gap(6).flex_none().items_center().rounded(RADIUS.md).cursor_pointer().text_size(TEXT.sm)
          .text_color(on ? k.foreground : k.mutedFg).when(on, (e) => e.bg(k.muted).font_medium()).when(!on, (e) => e.hover((s) => s.bg(k.accent)))
          .tooltip(`${t.name} · ${hhmm(t.at)}`)
          .on_click((_e, cx) => {
            this.pick = t.name;
            this.subscribe(cx);
            cx.notify();
          })
          .child(div().max_w(160).truncate().child(t.title ?? t.name))
          .when(t.asks > 0, (e) => e.child(div().size(6).rounded_full().bg(k.warning)));
      }));
    return v_flex().flex_none().child(head).child(strip);
  }

  body(k, view, cx, narrow) {
    const blocks = view.blocks.slice(this.first);
    const inner = Math.max(200, this.width - (narrow ? 24 : 32) - 2);
    let col = v_flex().id(`show-body-${view.name}-${this.first}`).flex_1().min_h(0).overflow_y_scrollbar().p(narrow ? 8 : 12).gap(12);
    if (this.first > 0) {
      col = col.child(h_flex().id("show-above").h(28).px(10).gap(6).items_center().rounded(RADIUS.md).cursor_pointer().bg(k.muted).text_size(TEXT.sm).text_color(k.mutedFg)
        .hover((s) => s.text_color(k.foreground)).on_click((_e, cx) => { this.first = Math.max(0, this.first - 1); cx.notify(); })
        .child(Icon("chevron-up", { size: 14 })).child(`${this.first} block${this.first === 1 ? "" : "s"} above`).child(div().flex_1()).child(Kbd("k")));
    }
    return col.children(blocks.map((b) => this.card(k, b, cx, inner - (narrow ? 16 : 32), narrow)));
  }

  card(k, b, cx, room, narrow) {
    const focused = this.focus === b.n;
    const live = this.live.get(this.liveKey(b));
    const refreshed = b.refreshed_at ?? live?.data?.at;
    // A blueprint draws its own header: title, age, zoom.
    const head = b.type === "blueprint" ? null : h_flex().gap(8).items_center().min_h(24)
      .child(Icon(ICON[b.type] ?? "layers", { size: 14, color: k.mutedFg }))
      .child(div().flex_1().min_w(0).truncate().text_size(TEXT.base).font_semibold().child(b.title ?? this.typeName(b)))
      .when(Boolean(refreshed), (e) => e.child(div().flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(`refreshed ${hhmm(refreshed)}`)))
      .when(Boolean(b.live) && b.type !== "diff", (e) => e.child(div().id(`show-live-${b.n}`).size(6).rounded_full().bg(k.success).loop("opacity", { from: 0.35, to: 1, period: 2000 })))
      .children(this.actions(k, b, cx, narrow));
    let body;
    try {
      body = this.blockBody(k, b, cx, room, narrow);
    } catch (e) {
      body = div().text_size(TEXT.sm).text_color(k.destructive).child(`could not draw this block: ${e?.message ?? e}`);
    }
    return v_flex().id(`show-card-${b.n}`).flex_none().gap(10).p(narrow ? 10 : 14).rounded(RADIUS.lg).border_1()
      .border_color(focused ? k.ring : k.border).bg(k.card).when(focused, (e) => e.shadow_sm())
      .on_mouse_down("left", (_e, cx) => {
        if (this.interactive(b) && this.focus !== b.n) {
          this.focus = b.n;
          this.handle.focus();
          cx.notify();
        }
      })
      .on_mouse_down("right", (_e, cx) => {
        cx.stop_propagation();
        this.menu(b);
      })
      .when(Boolean(head), (e) => e.child(head))
      .when(Boolean(b.error), (e) => e.child(this.callout(k, "alert", k.destructive, b.error)))
      .child(body);
  }

  typeName(b) {
    return { markdown: "Note", table: "Table", chart: "Chart", flow: "Diagram", mermaid: "Diagram", blueprint: "Blueprint", sequence: "Sequence", image: "Picture", compare: "Before and after", diff: "Changes", log: "Log", ci: "CI", choice: "Question", confirm: "Confirm" }[b.type] ?? b.type;
  }

  callout(k, icon, color, text) {
    return h_flex().gap(8).p(10).items_start().rounded(RADIUS.md).bg(alpha(color, 0.1)).border_1().border_color(alpha(color, 0.3)).text_size(TEXT.sm)
      .child(Icon(icon, { size: 14, color })).child(div().flex_1().min_w(0).text_color(k.foreground).child(String(text)));
  }

  actions(k, b, cx, narrow) {
    const st = this.state(b);
    const btn = (id, icon, tooltip, onClick, label) => Button(`show-${id}-${b.n}`, { icon, label: narrow ? undefined : label, size: narrow || !label ? "icon-sm" : "sm", variant: "ghost", tooltip, onClick });
    switch (b.type) {
      case "table": return b.live && !this.live.get(this.liveKey(b))?.data?.columns ? [] : [btn("csv", "copy", "Copy as CSV", (_e, cx) => this.copyCsv(cx, this.tableBlock(b)))];
      case "chart": {
        const data = this.chartData(b);
        const total = data?.labels?.length ?? 0;
        return total > 3 ? [
          btn("zout", "zoom-out", "Zoom out (−)", (_e, cx) => { st.window = zoomed(st.window, total, st.hover, false); cx.notify(); }),
          btn("zin", "zoom-in", "Zoom in (+)", (_e, cx) => { st.window = zoomed(st.window, total, st.hover, true); cx.notify(); }),
        ].concat(st.window ? [btn("zall", "maximize", "Every point (0)", (_e, cx) => { st.window = null; cx.notify(); })] : []) : [];
      }
      case "flow": case "mermaid": case "sequence":
        return [
          btn("dout", "zoom-out", "Smaller (−)", (_e, cx) => { st.scale = Math.max(0.3, (st.scale ?? this.fitted ?? 1) / 1.2); cx.notify(); }),
          btn("din", "zoom-in", "Larger (+)", (_e, cx) => { st.scale = Math.min(2, (st.scale ?? this.fitted ?? 1) * 1.2); cx.notify(); }),
          btn("dfit", "scan", "Fit to the pane (0)", (_e, cx) => { st.scale = null; cx.notify(); }),
        ];
      case "compare": return [btn("side", st.side ? "square-split" : "columns", st.side ? "Overlay with a slider (s)" : "Side by side (s)", (_e, cx) => { st.side = !st.side; cx.notify(); })];
      case "diff": {
        const d = this.diff(b);
        const n = d.review().length;
        return [
          btn("split", "columns", d.split ? "Unified (s)" : "Side by side (s)", (_e, cx) => { d.split = !d.split; cx.notify(); }),
          btn("fold", "fold-vertical", "Fold every file (z)", (_e, cx) => { d.foldAll(); cx.notify(); }),
        ].concat(n ? [Button(`show-send-${b.n}`, { icon: "send", label: `Send · ${n}`, size: "sm", tooltip: "Send the comments to the session (shift-s)", onClick: (_e, cx) => this.sendComments(cx, b) })] : []);
      }
      case "log": return [btn("follow", st.follow === false ? "arrow-down" : "circle-pause", st.follow === false ? "Follow the end (G)" : "Stop following", (_e, cx) => { st.follow = st.follow === false; if (st.follow) this.logScroll?.get(this.liveKey(b))?.scroll_to_bottom(); cx.notify(); })];
      case "image": return b.src?.remote ? [btn("open", "external", "Open the picture", () => this.openUrl(b.src.src))] : [];
      case "ci": return b.pr?.url ? [btn("pr", "git-pull-request", `Open #${b.pr.number}`, () => this.openUrl(b.pr.url))] : [];
    }
    return [];
  }

  blockBody(k, b, cx, room, narrow) {
    switch (b.type) {
      case "markdown": return TextView.markdown(`show-md-${this.liveKey(b)}`, String(b.text ?? "").slice(0, 24000)).selectable(true).text_size(TEXT.base).on_link_click((url) => this.openUrl(url));
      case "table": return this.table(k, this.tableBlock(b), cx);
      case "chart": return this.chart(k, b, cx, room);
      case "flow": return this.graph(k, b, flowGraph(b), room);
      case "mermaid":
        if (b.graph === "flowchart") return this.graph(k, b, parseFlowchart(b.text), room);
        if (b.graph === "sequence") return this.sequence(k, b, room);
        return this.callout(k, "info", k.mutedFg, b.error ?? "This Mermaid diagram type is not drawn here: flowchart and sequenceDiagram are.");
      case "blueprint": return this.blueprint(k, b, room, narrow);
      case "sequence": return this.sequence(k, b, room);
      case "image": return this.picture(k, b.src, room, b.caption);
      case "compare": return this.compare(k, b, room);
      case "diff": return this.diffBody(k, b, cx, narrow);
      case "log": return this.log(k, b);
      case "ci": return this.ci(k, b);
      case "choice": return this.choice(k, b, cx);
      case "confirm": return this.confirm(k, b, cx);
    }
    return div().text_size(TEXT.sm).text_color(k.mutedFg).child(`${b.unknown ?? b.type}: this block type is not drawn here`);
  }

  pending(k, b, what) {
    const data = this.live.get(this.liveKey(b))?.data;
    if (data?.error) return this.callout(k, "alert", k.destructive, data.error);
    return h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 14 })).child(what);
  }

  // Tables -------------------------------------------------------------------------------------------------------------

  tableBlock(b) {
    if (!b.live) return b;
    const data = this.live.get(this.liveKey(b))?.data;
    return data?.columns ? { ...data, n: b.n, title: b.title, live: true, fed: true } : { ...b, pendingRows: true };
  }

  sortBy(st, i) {
    st.sort = st.sort?.i === i ? { i, desc: !st.sort.desc } : { i, desc: false };
  }

  rowsOf(b) {
    const st = this.state(b);
    const rows = (b.cells ?? []).map((cells, i) => ({ i, cells, values: b.rows?.[i] ?? cells }));
    if (st.sort) {
      const { i, desc } = st.sort;
      rows.sort((x, y) => {
        const [a, c] = [x.values[i], y.values[i]];
        const order = a === null || a === undefined ? 1 : c === null || c === undefined ? -1 : typeof a === "number" && typeof c === "number" ? a - c : String(a).localeCompare(String(c));
        return desc ? -order : order;
      });
    }
    return rows;
  }

  table(k, b, cx) {
    if (b.pendingRows) return this.pending(k, b, `running ${b.command}…`);
    const st = this.state(b);
    const rows = this.rowsOf(b);
    const cols = b.columns ?? [];
    const cell = (i, content, head) => {
      let el = h_flex().flex_1().min_w(0).px(8).items_center();
      if (b.numeric?.[i]) el = el.justify_end();
      return el.child(div().truncate().when(Boolean(b.numeric?.[i]) && !head, (e) => e.font_family("JetBrains Mono").text_size(TEXT.sm)).child(content));
    };
    const header = h_flex().h(34).border_b_1().border_color(k.border).children(cols.map((c, i) => {
      const on = st.sort?.i === i;
      return h_flex().id(`show-th-${b.n}-${i}`).flex_1().min_w(0).px(8).gap(4).items_center().cursor_pointer().when(Boolean(b.numeric?.[i]), (e) => e.justify_end())
        .text_size(TEXT.sm).font_medium().text_color(on ? k.foreground : k.mutedFg).hover((s) => s.text_color(k.foreground))
        .tooltip(`Sort by ${c} (${i + 1})`)
        .on_click((_e, cx) => { this.sortBy(st, i); cx.notify(); })
        .child(div().truncate().child(c))
        .child(Icon(on ? (st.sort.desc ? "chevron-down" : "chevron-up") : "chevrons-up-down", { size: 12 }));
    }));
    if (!this.tableScroll) this.tableScroll = new Map();
    const key = this.liveKey(b);
    const list = v_virtual_list(`show-rows-${key}`, rows.length, ROW, (i) => String(rows[i].i), (range) =>
      rows.slice(range.start, range.end).map((r, j) => {
        const at = range.start + j;
        return h_flex().h(ROW).border_b_1().border_color(alpha(k.border, 0.6)).text_size(TEXT.base)
          .when(st.cursor === at, (e) => e.bg(k.selected)).when(st.cursor !== at && at % 2 === 1, (e) => e.bg(alpha(k.muted, 0.35)))
          .children(r.cells.map((c, i) => cell(i, c, false)));
      })).size_full().on_item_click((id, cx) => {
        st.cursor = rows.findIndex((r) => String(r.i) === id);
        cx.notify();
      });
    const height = Math.min(rows.length, 12) * ROW;
    return v_flex().w_full().rounded(RADIUS.md).border_1().border_color(k.border).overflow_hidden()
      .child(header)
      .child(div().relative().h(Math.max(ROW, height)).child(list).child(Scrollbar.vertical(`show-rows-${key}`).viewport_from_layout().absolute().inset_0()))
      .child(h_flex().h(28).px(8).items_center().bg(alpha(k.muted, 0.5)).text_size(TEXT.xs).text_color(k.mutedFg)
        .child(`${rows.length} row${rows.length === 1 ? "" : "s"}${st.sort ? ` · by ${cols[st.sort.i]} ${st.sort.desc ? "↓" : "↑"}` : ""}`));
  }

  // Charts and diagrams -------------------------------------------------------------------------------------------------

  chartData(b) {
    if (!b.live) return b;
    return this.live.get(this.liveKey(b))?.data ?? null;
  }

  chart(k, b, cx, room) {
    const data = this.chartData(b);
    if (!data?.series) return this.pending(k, b, `running ${b.command}…`);
    const st = this.state(b, { hidden: new Set() });
    return Chart(`show-chart-${b.n}`, data, {
      height: 180,
      width: room,
      hover: st.hover ?? null,
      hidden: st.hidden,
      window: st.window,
      onHover: (i, cx) => {
        if (st.hover !== i) {
          st.hover = i;
          cx.notify();
        }
      },
      onToggle: (name, cx) => {
        if (!st.hidden.delete(name)) st.hidden.add(name);
        cx.notify();
      },
    }).when(Boolean(st.window), (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`rows ${st.window.start + 1}–${st.window.start + st.window.len} of ${data.labels.length} · 0 shows them all`)));
  }

  /** A graph's layout and the scale it draws at: fitted to `room` until zoomed. */
  laid(b, g, st, room, min, turns) {
    const key = `${b.type}:${JSON.stringify(b.text ?? [b.nodes, b.edges, b.direction])}`;
    let L = laidOut(key, g);
    let size = graphSize(L);
    // A wide left-to-right chart in a narrow pane reads better top to bottom than shrunk.
    if (g.dir === "LR" && turns && size.w * 0.7 > room) {
      L = laidOut(`${key}:TB`, { ...g, dir: "TB" });
      size = graphSize(L);
    }
    return { L, size, scale: st.scale ?? fit(size.w, room, min) };
  }

  graph(k, b, g, room) {
    if (!g.nodes.size) return this.callout(k, "info", k.mutedFg, "This diagram has no nodes.");
    const st = this.state(b);
    const { L, scale } = this.laid(b, g, st, room, 0.45, st.scale == null);
    const drawn = Graph(`show-g-${b.n}`, L, {
      scale,
      hovered: st.hovered ?? null,
      onHover: (id, cx) => {
        if (st.hovered !== id && (id !== null || st.hovered)) {
          st.hovered = id;
          cx.notify();
        }
      },
    });
    const done = [...g.nodes.values()].filter((n) => n.status === "done").length;
    return v_flex().gap(6).w_full()
      .when(Boolean(g.title), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(g.title)))
      .child(div().id(`show-gbox-${b.n}`).w_full().overflow_x_scrollbar().child(div().p(4).child(drawn)))
      .when(done > 0, (e) => e.child(h_flex().gap(8).items_center().child(Progress(done / g.nodes.size, { color: k.done, w: 120 })).child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${done} of ${g.nodes.size} done`))));
  }

  sequence(k, b, room) {
    const st = this.state(b);
    const size = sequenceSize(b);
    const scale = st.scale ?? fit(size.w, room, 0.5);
    return div().id(`show-seq-${b.n}`).w_full().overflow_x_scrollbar().child(Sequence(`show-s-${b.n}`, b, { scale }));
  }

  // Blueprint: the session's plan, which it keeps current; a click on a box asks it for a change there -----------------

  blueprint(k, b, room, narrow) {
    const g = parseFlowchart(b.text);
    if (!g.nodes.size) return this.callout(k, "info", k.mutedFg, "This blueprint has no boxes yet.");
    const st = this.state(b);
    // The canvas fits the pane's height (header, legend and foot stay in view); Fit fits the plan into it both ways.
    const tallest = Math.max(200, (this.height ?? 800) - (this.asking?.n === b.n ? (narrow ? 390 : 360) : 250));
    const { L, size } = this.laid(b, g, st, room, 0.3, true);
    st.fitted = Math.max(0.3, Math.min(fit(size.w, room, 0.3), (tallest - 24) / size.h));
    const scale = st.scale ?? st.fitted;
    const nodes = [...g.nodes.values()];
    const done = nodes.filter((n) => n.status === "done").length;
    const active = nodes.filter((n) => n.status === "active" || n.status === "current");
    const asking = this.asking?.n === b.n ? this.asking : null;
    const zoom = (to) => (_e, cx) => {
      st.scale = to;
      cx.notify();
    };
    const facts = [`updated ${ago(b.updated ?? this.view()?.at)}`, `${done} of ${nodes.length} done`, active.length ? `now: ${active.map((n) => n.title).join(", ")}` : null].filter(Boolean);
    const controls = h_flex().flex_none().gap(2).items_center().p(2).rounded(RADIUS.md).border_1().border_color(k.border)
        .child(Button(`show-bp-fit-${b.n}`, { label: "Fit", size: "sm", variant: st.scale == null ? "secondary" : "ghost", tooltip: "Fit to the pane (0)", onClick: zoom(null) }))
        .child(Button(`show-bp-out-${b.n}`, { icon: "minus", size: "icon-sm", variant: "ghost", tooltip: "Smaller (−)", onClick: zoom(Math.max(0.3, scale / 1.2)) }))
        .child(div().w(42).flex_none().text_center().font_family("JetBrains Mono").text_size(TEXT.xs).text_color(k.mutedFg).child(`${Math.round(scale * 100)}%`))
        .child(Button(`show-bp-in-${b.n}`, { icon: "plus", size: "icon-sm", variant: "ghost", tooltip: "Larger (+)", onClick: zoom(Math.min(2, scale * 1.2)) }));
    // A narrow pane gives the title the whole row: the zoom goes to the foot.
    const head = h_flex().gap(8).items_center().min_h(32)
      .child(h_flex().size(24).flex_none().rounded(RADIUS.md).items_center().justify_center().bg(alpha(k.ring, 0.14)).child(Icon("map", { size: 14, color: k.ring })))
      .child(v_flex().flex_1().min_w(0)
        .child(div().truncate().text_size(TEXT.base).font_semibold().child(g.title ?? b.title ?? this.view()?.title ?? "Blueprint"))
        .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(facts.join(" · "))))
      .when(!narrow, (e) => e.child(controls));
    const swatch = (mark, label) => h_flex().gap(6).items_center().flex_none().child(mark).child(label);
    const legend = h_flex().gap(14).items_center().flex_wrap().text_size(TEXT.xs).text_color(k.mutedFg)
      .child(swatch(div().size(12).rounded_full().bg(k.done).flex().items_center().justify_center().text_size(8).font_semibold().text_color(k.background).child("✓"), "Done"))
      .child(swatch(div().size(9).rounded_full().bg(k.working), "In progress"))
      .child(swatch(div().w(14).h(9).rounded(3).border_1().border_color(k.input).bg(k.card), "To do"))
      .child(div().flex_1())
      .child(h_flex().gap(6).items_center().child(Progress(done / nodes.length, { color: k.done, w: narrow ? 64 : 110 })).child(`${Math.round((100 * done) / nodes.length)}%`));
    const drawn = Graph(`show-bp-${b.n}`, L, {
      scale,
      hovered: asking?.id ?? st.hovered ?? null,
      onHover: (id, cx) => {
        if (st.hovered !== id && (id !== null || st.hovered)) {
          st.hovered = id;
          cx.notify();
        }
      },
      onClick: (node, cx) => this.askAbout(b, node, cx),
    });
    const W = size.w * scale, H = size.h * scale;
    const composer = asking ? this.askCard(k, asking, narrow) : null;
    // Zoomed in, a taller plan scrolls inside the canvas.
    const height = Math.min(tallest, Math.max(140, H + 24));
    const canvas = div().id(`show-bp-canvas-${b.n}`).relative().w_full().h(height).rounded(RADIUS.md).border_1().border_color(k.border).bg(alpha(k.muted, 0.3)).overflow_hidden()
      .child(dots(k, height))
      .child(div().id(`show-bp-scroll-${b.n}`).absolute().inset_0().overflow_scrollbar()
        .child(div().w(Math.max(W + 24, room)).h(Math.max(H + 24, height)).flex().items_center().justify_center().child(drawn)));
    const hint = composer ? null : narrow ? "Click a box to ask for a change" : "Click a box to ask the session for a change there";
    return v_flex().gap(10).w_full()
      .child(head)
      .child(legend)
      .child(canvas)
      .when(Boolean(composer), (e) => e.child(composer))
      .when(Boolean(hint) || narrow, (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.xs).text_color(k.dim)
        .when(Boolean(hint), (e) => e.child(Icon("hand", { size: 12, color: k.dim })).child(div().flex_1().min_w(0).truncate().child(hint ?? "")))
        .when(!hint, (e) => e.child(div().flex_1()))
        .when(narrow, (e) => e.child(controls))));
  }

  /** The change request composer, under the canvas so the box it is about stays in view. */
  askCard(k, a, narrow) {
    const stop = (el) => el.on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_mouse_up("left", (_e, cx) => cx.stop_propagation());
    const state = a.status === "done" ? Badge("done", "success") : a.status === "active" || a.status === "current" ? Badge("in progress", "info") : null;
    const card = v_flex().id("show-bp-ask").gap(8).p(12).rounded(RADIUS.lg).border_1().border_color(k.ring).bg(k.popover).shadow_lg()
      .child(h_flex().gap(8).items_center()
        .child(Icon("message-square-plus", { size: 14, color: k.ring }))
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).font_semibold().child(a.title))
        .when(Boolean(state), (e) => e.child(state)))
      .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(`A change request about this step${a.sub ? ` · ${a.sub}` : ""}`))
      .child(Input.new(this.ask).h(32).px(8).rounded(RADIUS.md).border_1().border_color(k.input).bg(k.background).text_size(TEXT.sm))
      .child(h_flex().gap(6).items_center()
        .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(narrow ? "" : "Reaches the session as your next message"))
        .child(Button("show-bp-cancel", { label: "Cancel", variant: "outline", size: "sm", onClick: (_e, cx) => { this.asking = null; this.handle.focus(); cx.notify(); } }))
        .child(Button("show-bp-send", { label: "Send", icon: "send", size: "sm", kbd: narrow ? undefined : "⏎", onClick: (_e, cx) => this.sendAsk(cx) })));
    return stop(card);
  }

  // Pictures ------------------------------------------------------------------------------------------------------------

  picture(k, src, room, caption) {
    const pic = src && this.images.get(src.key);
    if (!pic || pic.state === "loading") return h_flex().h(120).w_full().items_center().justify_center().rounded(RADIUS.md).bg(k.muted).child(Spinner({ size: 16 }));
    if (pic.state === "failed") return this.callout(k, "image", k.destructive, `${src.src}: ${pic.error}`);
    const w = Math.min(room, pic.w), h = (w * pic.h) / pic.w;
    return v_flex().gap(6).items_center()
      .child(image(pic.rel).w(w).h(h).rounded(RADIUS.md))
      .when(Boolean(caption), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(caption ?? "")));
  }

  compare(k, b, room) {
    const st = this.state(b, { split: 50 });
    const [a, c] = [b.before && this.images.get(b.before.key), b.after && this.images.get(b.after.key)];
    if (!a || !c || a.state === "loading" || c.state === "loading") return h_flex().h(120).w_full().items_center().justify_center().rounded(RADIUS.md).bg(k.muted).child(Spinner({ size: 16 }));
    if (a.state === "failed" || c.state === "failed") return this.callout(k, "image", k.destructive, (a.state === "failed" ? a : c).error);
    const label = (text) => div().px(6).py(1).rounded(RADIUS.sm).bg(alpha(k.background, 0.85)).text_size(TEXT.xs).font_medium().text_color(k.foreground).child(text);
    if (st.side) {
      const w = (room - 8) / 2;
      const one = (pic, text) => v_flex().gap(4).w(w).child(label(text)).child(image(pic.rel).w(w).h((w * pic.h) / pic.w).rounded(RADIUS.sm));
      return h_flex().gap(8).items_start().child(one(a, "Before")).child(one(c, "After"));
    }
    const w = Math.min(room, Math.max(a.w, c.w)), h = (w * c.h) / c.w;
    const split = st.split ?? 50;
    return v_flex().gap(6)
      .child(div().id(`show-cmp-${b.n}`).relative().w(w).h(h).rounded(RADIUS.md).overflow_hidden().cursor_col_resize()
        .on_mouse_down("left", (e, cx) => {
          st.drag = true;
          if (e.local_position) st.split = Math.max(0, Math.min(100, (e.local_position.x / w) * 100));
          cx.notify();
        })
        .on_mouse_up("left", (_e, cx) => { st.drag = false; cx.notify(); })
        .on_hover((on, cx) => { if (!on) st.drag = false; cx.notify(); })
        .on_mouse_move((e, cx) => {
          if (!st.drag) return;
          st.split = Math.max(0, Math.min(100, (e.local_position.x / w) * 100));
          cx.notify();
        })
        .child(image(c.rel).absolute().left(0).top(0).w(w).h(h))
        .child(div().absolute().left(0).top(0).h(h).w((w * split) / 100).overflow_hidden().child(image(a.rel).absolute().left(0).top(0).w(w).h((w * a.h) / a.w)))
        .child(div().absolute().top(0).h(h).w(2).ml(-1).left((w * split) / 100).bg(k.background))
        .child(h_flex().absolute().size(24).ml(-12).left((w * split) / 100).top(h / 2 - 12).rounded_full().items_center().justify_center().bg(k.background).border_1().border_color(k.border).shadow_md().child(Icon("chevrons-left-right", { size: 12 })))
        .child(div().absolute().left(8).top(8).child(label("Before")))
        .child(div().absolute().right(8).top(8).child(label("After"))))
      .when(Boolean(b.caption), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(b.caption ?? "")));
  }

  // Diffs, logs, CI ----------------------------------------------------------------------------------------------------

  diffBody(k, b, cx, narrow) {
    const data = this.live.get(this.liveKey(b))?.data;
    if (!data) return this.pending(k, b, b.repo ? `reading the changes in ${b.repo}…` : "reading the patch…");
    if (data.error) return this.callout(k, "alert", k.destructive, data.error);
    const d = this.diff(b);
    if (!d.files().length) return this.callout(k, "check", k.success, "No changes.");
    const sent = this.state(b).sent;
    return v_flex().gap(8)
      .child(h_flex().gap(8).items_center().text_size(TEXT.xs).text_color(k.mutedFg)
        .child(`${d.files().length} file${d.files().length === 1 ? "" : "s"}`)
        .child(div().text_color(k.success).child(`+${data.added ?? 0}`))
        .child(div().text_color(k.destructive).child(`−${data.removed ?? 0}`))
        .child(div().flex_1())
        .child(sent ? "comments sent" : "click a line to comment"))
      .child(div().rounded(RADIUS.md).border_1().border_color(k.border).overflow_hidden().child(d.render(cx, { height: 460, narrow })));
  }

  log(k, b) {
    const data = this.live.get(this.liveKey(b))?.data;
    if (!data?.lines) return this.pending(k, b, `running ${b.command}…`);
    if (!this.logScroll) this.logScroll = new Map();
    const key = this.liveKey(b);
    if (!this.logScroll.has(key)) this.logScroll.set(key, null);
    const lines = data.lines;
    const color = (level) => (level === "error" ? k.destructive : level === "warn" ? k.warning : k.foreground);
    const list = v_virtual_list(`show-log-${key}`, lines.length, 18, (i) => String(i), (range) =>
      lines.slice(range.start, range.end).map(([text, level]) => div().h(18).px(10).whitespace_nowrap().text_size(TEXT.sm).font_family("JetBrains Mono").text_color(color(level)).child(text || " "))).size_full();
    return v_flex().gap(6)
      .child(div().relative().h(Math.min(Math.max(lines.length, 1), 18) * 18 + 8).py(4).rounded(RADIUS.md).bg(alpha(k.muted, 0.6)).child(list).child(Scrollbar.vertical(`show-log-${key}`).viewport_from_layout().absolute().inset_0()))
      .child(h_flex().gap(8).items_center().text_size(TEXT.xs).text_color(k.mutedFg)
        .child(Badge(data.code === 0 ? "exit 0" : `exit ${data.code}`, data.code === 0 ? "success" : "destructive"))
        .child(div().font_family("JetBrains Mono").truncate().flex_1().child(`$ ${b.command}`))
        .child(`every ${b.every}s · ${hhmm(data.at)}`));
  }

  ci(k, b) {
    const head = h_flex().gap(8).items_center().flex_wrap()
      .child(Badge(b.repo ?? "?", "outline", { icon: "folder-git" }))
      .child(Badge(b.branch ?? "?", "secondary", { icon: "git-branch" }))
      .when(Boolean(b.pr), (e) => e.child(div().min_w(0).max_w_full().overflow_hidden().child(Badge(`#${b.pr.number} ${b.pr.title ?? ""}`, "outline", { id: `show-ci-pr-${b.n}`, icon: "git-pull-request", onClick: () => this.openUrl(b.pr.url), tooltip: b.pr.url }))));
    if (!b.run) return v_flex().gap(8).child(head).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("No CI run known for this branch yet."));
    const r = b.run;
    const tone = r.status === "running" ? k.warning : r.status === "passed" ? k.success : k.destructive;
    return v_flex().gap(10).child(head)
      .child(h_flex().gap(10).items_center()
        .child(r.status === "running" ? Spinner({ size: 14, color: tone }) : Icon(r.status === "passed" ? "circle-check" : "circle-x", { size: 16, color: tone }))
        .child(div().flex_1().child(Progress(r.done / r.total, { color: tone })))
        .child(div().text_size(TEXT.sm).font_medium().child(`${r.done}/${r.total} jobs · ${r.status === "failed" ? r.outcome || "failed" : r.status}`)))
      .when(Boolean(r.failed?.job), (e) => e.child(v_flex().gap(4).p(10).rounded(RADIUS.md).bg(alpha(k.destructive, 0.08))
        .child(h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.destructive).child(Icon("circle-x", { size: 14 })).child(`${r.failed.job}${r.failed.step ? ` · ${r.failed.step}` : ""}`))
        .children((r.failed.tests ?? []).slice(0, 12).map((t) => div().pl(20).text_size(TEXT.xs).font_family("JetBrains Mono").text_color(k.mutedFg).child(String(t))))));
  }

  // Questions ----------------------------------------------------------------------------------------------------------

  answeredRow(k, label, good = true) {
    return h_flex().gap(8).items_center().p(10).rounded(RADIUS.md).bg(alpha(good ? k.success : k.muted, good ? 0.1 : 0.6)).text_size(TEXT.sm)
      .child(Icon(good ? "circle-check" : "circle-dashed", { size: 14, color: good ? k.success : k.mutedFg })).child(`Answered: ${label}`);
  }

  choice(k, b, cx) {
    const st = this.state(b);
    const done = st.sent ?? b.answered;
    const opts = b.options ?? [];
    const pick = (o) => (_e, cx) => this.answer(cx, b.n, "choice", o.label, o.label);
    const preview = st.at !== undefined ? opts[st.at]?.preview : null;
    return v_flex().gap(8)
      .when(Boolean(b.prompt), (e) => e.child(TextView.markdown(`show-q-${this.liveKey(b)}`, String(b.prompt ?? "")).text_size(TEXT.base)))
      .when(Boolean(done), (e) => e.child(this.answeredRow(k, done)))
      .when(!done, (e) => e
        .children(opts.map((o, i) => {
          const on = st.at === i;
          return h_flex().id(`show-opt-${b.n}-${i}`).min_h(36).px(10).py(6).gap(10).items_center().rounded(RADIUS.md).border_1().cursor_pointer()
            .border_color(on ? k.ring : k.border).bg(on ? alpha(k.ring, 0.08) : k.card).hover((s) => s.bg(k.accent))
            .on_hover((h, cx) => { if (h && st.at !== i) { st.at = i; cx.notify(); } })
            .on_click(pick(o))
            .child(i < 9 ? Kbd(String(i + 1)) : div().w(20))
            .child(div().flex_1().min_w(0).text_size(TEXT.base).child(o.label))
            .child(Icon("corner-down-left", { size: 14, color: on ? k.ring : k.dim }));
        }))
        .when(Boolean(preview), (e) => e.child(div().p(10).rounded(RADIUS.md).bg(k.muted).child(TextView.markdown(`show-pv-${b.n}-${st.at}`, String(preview ?? "")).text_size(TEXT.sm))))
        .child(h_flex().gap(8).items_center()
          .child(Input.new(this.other).flex_1().h(32).px(8).rounded(RADIUS.md).border_1().border_color(k.input).bg(k.background).text_size(TEXT.sm))
          .child(Button(`show-other-${b.n}`, { label: "Send", size: "sm", variant: "outline", onClick: (_e, cx) => {
            const text = this.other.value().trim();
            if (text) this.answer(cx, b.n, "choice", text, text);
          } }))));
  }

  confirm(k, b, cx) {
    const st = this.state(b);
    const done = st.sent ?? b.answered;
    return v_flex().gap(10)
      .child(TextView.markdown(`show-c-${this.liveKey(b)}`, String(b.summary ?? "")).text_size(TEXT.base))
      .when(Boolean(b.consequence), (e) => e.child(this.callout(k, "triangle-alert", k.destructive, b.consequence)))
      .when(Boolean(done), (e) => e.child(this.answeredRow(k, done, done !== "cancelled")))
      .when(!done, (e) => e.child(h_flex().gap(8).justify_end()
        .child(Button(`show-no-${b.n}`, { label: "Cancel", variant: "outline", onClick: (_e, cx) => this.answer(cx, b.n, "confirm", false, "cancelled") }))
        .child(Button(`show-yes-${b.n}`, { label: b.confirm_label ?? "Confirm", variant: "destructive", kbd: "y", onClick: (_e, cx) => this.answer(cx, b.n, "confirm", true, b.confirm_label ?? "Confirm") }))));
  }

  // Footer and palette -------------------------------------------------------------------------------------------------

  footer(k, view) {
    const b = this.focus !== null ? view.blocks[this.focus] : null;
    const typing = Boolean(this.activeDiff()) || Boolean(this.asking);
    const by = {
      table: [["↑↓", "rows"], ["1-9", "sort"]],
      chart: [["←→", "point"], ["+/−", "zoom"], ["0", "all"]],
      diff: [["j/k", "line"], ["[ ]", "file"], ["c", "comment"], ["s", "split"], ["⇧S", "send"]],
      log: [["k", "scroll"], ["G", "follow"]],
      compare: [["h/l", "split"], ["s", "side by side"]],
      flow: [["+/−", "size"], ["0", "fit"]],
      mermaid: [["+/−", "size"], ["0", "fit"]],
      blueprint: [["click", "ask for a change"], ["+/−", "zoom"], ["0", "fit"]],
      sequence: [["+/−", "size"], ["0", "fit"]],
      choice: [["↑↓", "move"], ["⏎", "pick"], ["1-9", "pick"]],
      confirm: [["y", "confirm"], ["⏎", "press"]],
    };
    const hints = typing ? [["⏎", this.asking ? "send to the session" : "add the comment"], ["esc", "cancel"]]
      : b ? [...(by[b.type] ?? []), ["esc", "leave"]]
      : [["j/k", "blocks"], ["tab", "focus"], ["^P", "commands"], ...(view.url ? [["o", "page"]] : []), ...(this.name ? [["q", "hide"]] : [])];
    return h_flex().h(30).flex_none().px(12).gap(12).items_center().border_t_1().border_color(k.border).overflow_hidden().text_size(TEXT.xs).text_color(k.mutedFg)
      .children(hints.map(([key, what]) => h_flex().flex_none().gap(4).items_center().child(Kbd(key)).child(what)));
  }

  paletteCard(k) {
    const items = this.paletteItems();
    return div().absolute().inset_0().bg(alpha(k.background, 0.5)).flex().justify_center().pt(48).child(
      v_flex().w(Math.min(420, this.width - 24)).p(4).rounded(RADIUS.lg).border_1().border_color(k.border).bg(k.popover).shadow_lg()
        .child(div().px(8).py(6).text_size(TEXT.xs).text_color(k.mutedFg).child("Commands · ↑↓ ⏎ · esc"))
        .children(items.map((it, i) => h_flex().id(`show-cmd-${i}`).h(30).px(8).items_center().rounded(RADIUS.md).cursor_pointer().text_size(TEXT.base)
          .when(i === this.palette.at, (e) => e.bg(k.selected)).hover((s) => s.bg(k.accent))
          .on_click((_e, cx) => { it.run(cx); this.palette = null; cx.notify(); })
          .child(div().truncate().child(it.label)))),
    );
  }
}

export { compact };
