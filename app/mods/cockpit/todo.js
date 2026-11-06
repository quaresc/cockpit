// The to-do list (the reference app view): the `todo` view the hub pushes, edited in place through the hub's `todo`
// actions, with every key and action of the TUI app (move, add, edit, toggle, reorder, mark several, delete, undo,
// fold the done ones, filter, open a ticket or link, a command list on ctrl+p) and the pointer's: buttons for each
// action, hover actions, a right-click menu, drag to reorder, double-click to edit, Undo on the toast of a delete.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Badge, Button, Checkbox, Kbd, ListRow, PageHeader, SearchField, TextField, KeyHints, Empty,
  ErrorState, Skeleton, Command, Toaster, Flashes, Scroller, Hover, Reorder, Progress, openMenu, alpha,
} from "./ui.js";

const ADDER = { kind: "adder" };
const FOLD = { kind: "fold" };
const same = (a, b) => a.kind === b.kind && a.key === b.key;

export default class Todo {
  init(_props, cx) {
    this.model = null;
    this.status = "connecting";
    this.sel = ADDER;
    this.showDone = false;
    this.filter = "";
    this.filtering = false;
    this.marks = new Set();
    this.editing = null; // { key } for an item, {} for the adder
    this.palette = null; // { at }
    this.hover = new Hover();
    this.reorder = new Reorder((key, to, cx) => cx.spawn(async (cx) => this.act(cx, "place", [key, to])));
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();

    this.editor = InputState.new({ placeholder: "What needs doing? A ticket key or a URL becomes a link." });
    this.editor.on("submit", (_e, cx) => this.commitEdit(cx));
    this.search = InputState.new({ placeholder: "Filter to-dos…" });
    this.search.on("change", (_e, cx) => {
      this.filter = this.search.value();
      cx.notify();
    });
    this.search.on("submit", (_e, cx) => {
      this.filtering = false;
      this.handle.focus();
      cx.notify();
    });
    this.query = InputState.new({ placeholder: "Type a command or search…" });
    this.query.on("change", (_e, cx) => {
      if (this.palette) this.palette.at = 0;
      cx.notify();
    });
    this.query.on("submit", (_e, cx) => this.runPalette(cx));

    this.sub = hub.subscribe(
      cx,
      { views: ["todo"] },
      (m, cx) => {
        if (m.view === "todo") this.take(m.value, cx);
        if (m.focus === "todo") this.goTo(m.target?.key, cx);
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

  /** Keyboard commands from keys.json (`cockpit.<command>`) and pane.go's "focus". */
  command(c, cx) {
    if (c === "focus" || c === "show:todo") this.handle.focus();
    // A right-click menu's choice: todo:<action>:<item>.
    const [scope, action, ...rest] = c.split(":");
    if (scope === "todo") this.menuAction(action, rest.join(":"), cx);
    cx.notify();
  }

  menuAction(action, key, cx) {
    const item = this.item(key);
    if (!item) return;
    this.sel = { kind: "item", key };
    if (action === "toggle") this.toggle(cx, [key]);
    else if (action === "edit") this.startEdit(cx, key);
    else if (action === "open") this.openLink(cx, item.parts.find((p) => p.token)?.text);
    else if (action === "up") this.shift(cx, -1);
    else if (action === "down") this.shift(cx, 1);
    else if (action === "top") cx.spawn(async (cx) => this.act(cx, "place", [key, 0]));
    else if (action === "mark") this.mark(cx);
    else if (action === "delete") this.remove(cx, [key]);
  }

  menu(item) {
    const link = item.parts.find((p) => p.token);
    openMenu([
      { label: item.done ? "Reopen" : "Done", icon: item.done ? "undo" : "check", kbd: "x", command: `todo:toggle:${item.key}` },
      { label: "Edit", icon: "pencil", kbd: "e", command: `todo:edit:${item.key}` },
      link && { label: `Open ${link.label}`, icon: "external", kbd: "o", command: `todo:open:${item.key}` },
      "separator",
      !item.done && { label: "Move to top", icon: "arrow-up", command: `todo:top:${item.key}` },
      !item.done && { label: "Move up", icon: "arrow-up", kbd: "K", command: `todo:up:${item.key}` },
      !item.done && { label: "Move down", icon: "arrow-down", kbd: "J", command: `todo:down:${item.key}` },
      { label: this.marks.has(item.key) ? "Unmark" : "Mark", kbd: "v", command: `todo:mark:${item.key}` },
      "separator",
      { label: "Delete", icon: "trash", kbd: "D", destructive: true, command: `todo:delete:${item.key}` },
    ]);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The model and what the selection can stop on

  take(model, cx) {
    const before = this.model;
    const at = this.selIndex();
    this.model = model;
    if (!before) {
      const first = this.items().find((i) => !i.done);
      this.sel = first ? { kind: "item", key: first.key } : ADDER;
      return;
    }
    const known = new Map(before.items.map((i) => [i.key, i]));
    const k = tokens();
    for (const item of model.items) {
      const was = known.get(item.key);
      const wasDone = before.items.find((i) => !i.done && i.undated === item.undated && item.done);
      if (wasDone) this.flashes.start(cx, this.unfolded() ? item.key : "\u0000fold", k.done);
      else if (!was) this.flashes.start(cx, item.key, k.ring);
    }
    // The selection follows its item through a toggle (its text gains or loses the date).
    if (this.sel.kind === "item" && !model.items.some((i) => i.key === this.sel.key)) {
      const old = before.items.find((i) => i.key === this.sel.key);
      const now = old && model.items.find((i) => i.undated === old.undated);
      if (now && (!now.done || this.unfolded())) this.sel = { kind: "item", key: now.key };
      else this.selectAt(at);
    }
    for (const key of [...this.marks]) if (!model.items.some((i) => i.key === key)) this.marks.delete(key);
  }

  items() {
    return this.model?.items ?? [];
  }

  matches(item) {
    const text = item.text.toLowerCase();
    return this.filter.toLowerCase().split(/\s+/).filter(Boolean).every((w) => text.includes(w));
  }

  unfolded() {
    return this.showDone || this.filter.trim() !== "";
  }

  shown() {
    const items = this.items().filter((i) => this.matches(i));
    return { open: items.filter((i) => !i.done), done: items.filter((i) => i.done) };
  }

  selectable() {
    const { open, done } = this.shown();
    const all = [ADDER, ...open.map((i) => ({ kind: "item", key: i.key }))];
    if (done.length) {
      all.push(FOLD);
      if (this.unfolded()) all.push(...done.map((i) => ({ kind: "item", key: i.key })));
    }
    return all;
  }

  selIndex() {
    return Math.max(0, this.selectable().findIndex((s) => same(s, this.sel)));
  }

  selectAt(i) {
    const all = this.selectable();
    this.sel = all[Math.max(0, Math.min(i, all.length - 1))] ?? ADDER;
  }

  item(key) {
    return this.items().find((i) => i.key === key);
  }

  selected() {
    return this.sel.kind === "item" ? this.item(this.sel.key) : null;
  }

  /** The marked items in list order, else the selected one. */
  targets() {
    if (this.marks.size) return this.items().filter((i) => this.marks.has(i.key)).map((i) => i.key);
    const item = this.selected();
    return item ? [item.key] : [];
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async act(cx, action, args) {
    try {
      const r = await hub.act("todo", action, args);
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note) this.toasts.push(cx, r.note, { kind: action === "open_link" ? "info" : "success" });
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    }
    cx.notify();
  }

  startEdit(cx, key) {
    this.editing = key ? { key } : {};
    this.sel = key ? { kind: "item", key } : ADDER;
    this.editor.set_value(key ? this.item(key)?.text ?? "" : "");
    this.editor.focus();
    cx.notify();
  }

  commitEdit(cx) {
    const editing = this.editing;
    if (!editing) return;
    const text = this.editor.value().trim();
    this.editing = null;
    this.handle.focus();
    if (editing.key) {
      if (text && text !== editing.key) {
        this.sel = { kind: "item", key: text };
        cx.spawn(async (cx) => this.act(cx, "edit", [editing.key, text]));
      }
    } else if (text) {
      this.sel = { kind: "item", key: text };
      cx.spawn(async (cx) => this.act(cx, "add", [text]));
    }
    cx.notify();
  }

  cancel(cx) {
    this.editing = null;
    this.handle.focus();
    cx.notify();
  }

  toggle(cx, keys = this.targets()) {
    if (!keys.length) return;
    this.marks.clear();
    cx.spawn(async (cx) => this.act(cx, "toggle", [keys]));
  }

  /** Deletes at once; the toast's Undo (or u) brings them back. */
  remove(cx, keys = this.targets()) {
    if (!keys.length) return;
    this.marks.clear();
    cx.spawn(async (cx) => {
      const r = await hub.act("todo", "delete", [keys]).catch((e) => ({ error: String(e?.message ?? e) }));
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else
        this.toasts.push(cx, keys.length === 1 ? "To-do deleted" : `${keys.length} to-dos deleted`, {
          kind: "success",
          description: keys.length === 1 ? keys[0] : undefined,
          ms: 6000,
          action: { label: "Undo", onClick: (_e, cx) => cx.spawn(async (cx) => this.act(cx, "undo", [])) },
        });
      cx.notify();
    });
  }

  shift(cx, by) {
    const item = this.selected();
    if (!item || item.done || this.marks.size) return;
    cx.spawn(async (cx) => this.act(cx, "move", [item.key, by]));
  }

  openLink(cx, token) {
    const item = this.selected();
    const part = token ?? item?.parts.find((p) => p.token)?.text;
    if (part) cx.spawn(async (cx) => this.act(cx, "open_link", [part]));
  }

  mark(cx) {
    const item = this.selected();
    if (!item) return;
    if (!this.marks.delete(item.key)) this.marks.add(item.key);
    this.selectAt(this.selIndex() + 1);
    cx.notify();
  }

  goTo(key, cx) {
    const item = key && (this.item(key) ?? this.items().find((i) => i.undated === key.replace(/\s*\(done \d{4}-\d{2}-\d{2}\)$/, "")));
    if (!item) {
      if (key) this.toasts.push(cx, `Not on the list any more: ${key}`);
      return;
    }
    if (!this.matches(item)) {
      this.filter = "";
      this.search.set_value("");
    }
    if (item.done) this.showDone = true;
    this.sel = { kind: "item", key: item.key };
    this.handle.focus();
  }

  /** The selection's actions, as the action bar and the command list show them. */
  acts(cx) {
    if (this.marks.size) {
      const n = this.targets().length;
      return [
        { id: "toggle", label: `Toggle ${n}`, kbd: "x", icon: "check", run: () => this.toggle(cx) },
        { id: "delete", label: `Delete ${n}`, kbd: "D", icon: "trash", variant: "outline", run: () => this.remove(cx) },
        { id: "unmark", label: "Unmark all", kbd: "esc", variant: "ghost", run: () => (this.marks.clear(), cx.notify()) },
      ];
    }
    if (this.sel.kind === "adder") return [{ id: "add", label: "Add a to-do", kbd: "a", icon: "plus", run: () => this.startEdit(cx) }];
    if (this.sel.kind === "fold") return [{ id: "fold", label: this.unfolded() ? "Hide done" : "Show done", kbd: "z", run: () => this.fold(cx) }];
    const item = this.selected();
    if (!item) return [];
    const link = item.parts.find((p) => p.token);
    return [
      { id: "toggle", label: item.done ? "Reopen" : "Done", kbd: "x", icon: item.done ? "undo" : "check", run: () => this.toggle(cx) },
      { id: "edit", label: "Edit", kbd: "e", icon: "pencil", variant: "outline", run: () => this.startEdit(cx, item.key) },
      ...(link ? [{ id: "open", label: `Open ${link.label}`, kbd: "o", icon: "external", variant: "outline", run: () => this.openLink(cx, link.text) }] : []),
      ...(!item.done
        ? [
            { id: "up", label: "Up", kbd: "K", icon: "arrow-up", variant: "ghost", run: () => this.shift(cx, -1) },
            { id: "down", label: "Down", kbd: "J", icon: "arrow-down", variant: "ghost", run: () => this.shift(cx, 1) },
          ]
        : []),
      { id: "mark", label: "Mark", kbd: "v", variant: "ghost", run: () => this.mark(cx) },
      { id: "delete", label: "Delete", kbd: "D", icon: "trash", variant: "ghost", run: () => this.remove(cx) },
    ];
  }

  fold(cx) {
    this.showDone = !this.showDone;
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p): the list's own actions, then every item's and every link's

  paletteGroups(cx) {
    const q = this.query.value().toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (text) => q.every((w) => text.toLowerCase().includes(w));
    const list = [{ id: "p-add", label: "Add a to-do", shortcut: "a", icon: "plus", run: () => this.startEdit(cx) }];
    if (this.model?.can_undo) list.push({ id: "p-undo", label: "Undo the last change", shortcut: "u", icon: "undo", run: () => this.act(cx, "undo", []) });
    const done = this.items().filter((i) => i.done).length;
    if (done) list.push({ id: "p-fold", label: `${this.showDone ? "Hide" : "Show"} the ${done} done`, shortcut: "z", run: () => this.fold(cx) });
    const items = [];
    const links = [];
    for (const item of this.items()) {
      for (const p of item.parts.filter((p) => p.token)) links.push({ id: `p-open-${item.key}-${p.text}`, label: `Open ${p.label}`, hint: item.text, icon: "external", run: () => this.act(cx, "open_link", [p.text]) });
      const status = item.done ? "done" : "open";
      items.push({ id: `p-toggle-${item.key}`, label: `${item.done ? "Reopen" : "Done"}  ${item.text}`, hint: status, icon: item.done ? "undo" : "check", run: () => this.toggle(cx, [item.key]) });
      items.push({ id: `p-edit-${item.key}`, label: `Edit  ${item.text}`, hint: status, icon: "pencil", run: () => this.startEdit(cx, item.key) });
      if (!item.done) {
        items.push({ id: `p-up-${item.key}`, label: `Move up  ${item.text}`, hint: status, icon: "arrow-up", run: () => ((this.sel = { kind: "item", key: item.key }), this.shift(cx, -1)) });
        items.push({ id: `p-down-${item.key}`, label: `Move down  ${item.text}`, hint: status, icon: "arrow-down", run: () => ((this.sel = { kind: "item", key: item.key }), this.shift(cx, 1)) });
      }
      items.push({ id: `p-delete-${item.key}`, label: `Delete  ${item.text}`, hint: status, icon: "trash", run: () => this.remove(cx, [item.key]) });
      items.push({ id: `p-go-${item.key}`, label: `Go to  ${item.text}`, hint: status, icon: "chevron-right", run: () => this.goTo(item.key, cx) });
    }
    const groups = [
      { heading: "To-do", items: list.filter((i) => hit(i.label)) },
      { heading: "Links", items: (q.length ? links : links.filter((l) => !this.item(l.hint)?.done)).filter((i) => hit(`${i.label} ${i.hint}`)) },
      { heading: "Items", items: q.length ? items.filter((i) => hit(i.label)) : [] },
    ];
    return groups;
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
    if (item) cx.spawn(async (cx) => item.run());
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

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
    if (this.editing) {
      if (k === "escape") this.cancel(cx);
      return k === "escape";
    }
    if (this.filtering) {
      if (k === "escape") {
        this.filtering = false;
        this.filter = "";
        this.search.set_value("");
        this.handle.focus();
        cx.notify();
        return true;
      }
      return false;
    }
    const item = this.selected();
    const n = this.selectable().length;
    switch (k) {
      case "ctrl-p":
      case "cmd-shift-p":
        this.openPalette(cx);
        break;
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
        this.selectAt(this.selIndex() + 10);
        break;
      case "pageup":
      case "ctrl-u":
        this.selectAt(this.selIndex() - 10);
        break;
      case "g":
      case "home":
        this.selectAt(0);
        break;
      case "shift-g":
      case "end":
        this.selectAt(n - 1);
        break;
      case "a":
        this.startEdit(cx);
        break;
      case "u":
        cx.spawn(async (cx) => this.act(cx, "undo", []));
        break;
      case "z":
        this.fold(cx);
        break;
      case "/":
        this.filtering = true;
        this.search.focus();
        break;
      case "escape":
        if (this.marks.size) this.marks.clear();
        else {
          this.filter = "";
          this.search.set_value("");
        }
        break;
      case "enter":
      case "e":
        if (this.sel.kind === "fold") this.fold(cx);
        else this.startEdit(cx, item?.key);
        break;
      case "space":
      case "x":
        if (this.sel.kind === "fold") this.fold(cx);
        else this.toggle(cx);
        break;
      case "shift-j":
        this.shift(cx, 1);
        break;
      case "shift-k":
        this.shift(cx, -1);
        break;
      case "shift-d":
        this.remove(cx);
        break;
      case "v":
        this.mark(cx);
        break;
      case "o":
        this.openLink(cx);
        break;
      case "shift-f10":
      case "m":
        if (item) this.menu(item);
        break;
      default:
        return false;
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  textOf(item, k) {
    const shown = item.done ? item.undated : item.text;
    let at = 0;
    const parts = [];
    for (const p of item.parts) {
      const start = at;
      at += p.text.length;
      if (start >= shown.length) break;
      const text = p.text.slice(0, Math.max(0, shown.length - start));
      if (!p.token) parts.push(div().child(text));
      else
        parts.push(
          h_flex()
            .id(`link-${item.key}-${start}`)
            .gap(3)
            .items_center()
            .text_color(item.done ? k.mutedFg : k.info)
            .cursor_pointer()
            .hover((s) => s.underline())
            .tooltip(p.url ?? "no URL known: set tickets.urlTemplate in the cockpit config")
            .on_click((_e, cx) => {
              cx.stop_propagation();
              cx.spawn(async (cx) => this.act(cx, "open_link", [p.text]));
            })
            .when(p.label === "link", (e) => e.child(Icon("link", { size: 12 })))
            .child(p.label === "link" ? text.replace(/^https?:\/\//, "").slice(0, 48) : text),
        );
    }
    return h_flex()
      .flex_1()
      .min_w(0)
      .gap(0)
      .items_center()
      .overflow_hidden()
      .whitespace_nowrap()
      .when(item.done, (e) => e.line_through().text_color(k.mutedFg))
      .children(parts);
  }

  itemRow(item, k, cx, index) {
    const sel = this.sel.kind === "item" && this.sel.key === item.key;
    const canDrag = !item.done && !this.filter && index !== undefined;
    if (this.editing?.key === item.key) {
      return h_flex().h(36).gap(10).items_center().px(10).child(Checkbox(`edit-check-${item.key}`, { checked: item.done })).child(TextField(this.editor)).child(Kbd("⏎"));
    }
    return ListRow(`item-${item.key}`, {
      selected: sel,
      marked: this.marks.has(item.key),
      flash: this.flashes.of(item.key),
      leading: Checkbox(`check-${item.key}`, { checked: item.done, onClick: (_e, cx) => this.toggle(cx, [item.key]), tooltip: item.done ? "Reopen" : "Done" }),
      title: this.textOf(item, k),
      meta: item.done && item.done_on ? Badge(item.done_on, "outline") : null,
      hover: this.hover,
      grip: canDrag ? this.reorder.grip(item.key, index) : null,
      dimmed: this.reorder.dragging(item.key),
      dropAbove: canDrag && this.reorder.target() === index && !this.reorder.dragging(item.key),
      actions: [
        Button(`edit-${item.key}`, { icon: "pencil", variant: "ghost", size: "icon-sm", tooltip: "Edit  e", onClick: (_e, cx) => this.startEdit(cx, item.key) }),
        Button(`delete-${item.key}`, { icon: "trash", variant: "ghost", size: "icon-sm", tooltip: "Delete  D", onClick: (_e, cx) => this.remove(cx, [item.key]) }),
        Button(`more-${item.key}`, { icon: "more", variant: "ghost", size: "icon-sm", tooltip: "More", onClick: () => this.menu(item) }),
      ],
      onRightClick: () => this.menu(item),
      onClick: (_e, cx) => {
        this.sel = { kind: "item", key: item.key };
        this.handle.focus();
        cx.notify();
      },
      onDoubleClick: (_e, cx) => this.startEdit(cx, item.key),
    });
  }

  rows(k, cx) {
    const { open, done } = this.shown();
    const rows = [];
    const adderOn = this.sel.kind === "adder";
    rows.push({
      height: 36,
      el:
        this.editing && !this.editing.key
          ? h_flex().h(36).gap(10).items_center().px(10).child(Icon("plus", { size: 16, color: k.mutedFg })).child(TextField(this.editor)).child(Kbd("⏎"))
          : ListRow("adder", {
              selected: adderOn,
              muted: true,
              leading: Icon("plus", { size: 16 }),
              title: "Add a to-do",
              meta: Kbd("a"),
              onClick: (_e, cx) => this.startEdit(cx),
            }),
    });
    open.forEach((item, i) => rows.push({ height: 36, el: this.itemRow(item, k, cx, i) }));
    if (done.length) {
      const unfolded = this.unfolded();
      rows.push({
        height: 36,
        el: ListRow("fold", {
          selected: this.sel.kind === "fold",
          muted: true,
          flash: this.flashes.of("\u0000fold"),
          leading: Icon(unfolded ? "chevron-down" : "chevron-right", { size: 16 }),
          title: `${done.length} done`,
          meta: Kbd("z"),
          onClick: (_e, cx) => {
            this.sel = FOLD;
            this.fold(cx);
          },
        }),
      });
      if (unfolded) for (const item of done) rows.push({ height: 36, el: this.itemRow(item, k, cx) });
    }
    return rows;
  }

  body(k, cx) {
    if (!this.model) {
      if (this.status.startsWith("error")) return ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) });
      return v_flex().p(16).gap(10).children([0, 1, 2, 3, 4].map((i) => h_flex().gap(10).items_center().child(Skeleton({ w: 16, h: 16, radius: RADIUS.sm })).child(Skeleton({ w: 180 + ((i * 97) % 260) }))));
    }
    if (!this.items().length && !this.editing) {
      return Empty({
        icon: "list-todo",
        title: "Nothing to do",
        description: "Add a to-do here, or ask your agent to add one.",
        action: Button("empty-add", { label: "Add a to-do", icon: "plus", kbd: "a", onClick: (_e, cx) => this.startEdit(cx) }),
      });
    }
    const all = this.selectable();
    const at = all.findIndex((s) => same(s, this.sel));
    const { open, done } = this.shown();
    if (this.filter && !open.length && !done.length) {
      return Empty({ icon: "search", title: "No to-do matches", description: `Nothing has every word of “${this.filter}”.` });
    }
    return this.reorder.track(this.scroller.view("todo-list", this.rows(k, cx), at), 38, open.length);
  }

  render(cx) {
    const k = tokens();
    const m = this.model;
    const acts = this.acts(cx);
    const description = m ? (m.open ? `${m.open} open · ${m.done} done` : m.done ? `All done · ${m.done} done` : "Empty") : "Loading…";
    const root = v_flex()
      .id("todo")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_mouse_down("left", () => {
        if (!this.editing && !this.filtering && !this.palette) this.handle.focus();
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "list-todo",
          title: "To-do",
          description,
          aside: m && m.open + m.done > 0 ? h_flex().gap(8).items_center().child(Progress(m.done / (m.open + m.done), { w: 120, color: k.success })).child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${Math.round((m.done / (m.open + m.done)) * 100)}%`)) : null,
          actions: [
            Button("undo", { icon: "undo", variant: "ghost", size: "icon", tooltip: "Undo  u", disabled: !m?.can_undo, onClick: (_e, cx) => cx.spawn(async (cx) => this.act(cx, "undo", [])) }),
            Button("commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
            Button("add", { label: "Add", icon: "plus", size: "sm", kbd: "a", onClick: (_e, cx) => this.startEdit(cx) }),
          ],
        }),
      )
      .child(
        h_flex()
          .px(24)
          .py(12)
          .gap(8)
          .flex_none()
          .items_center()
          .child(
            h_flex()
              .id("filter")
              .flex_1()
              .on_mouse_down("left", (_e, cx) => {
                this.filtering = true;
                cx.notify();
              })
              .child(SearchField(this.search, { active: this.filtering, kbd: "/" })),
          )
          .when(this.marks.size > 0, (e) => e.child(Badge(`${this.marks.size} marked`, "info")))
          .child(
            Button("fold-toggle", {
              label: this.unfolded() ? "Hide done" : "Show done",
              icon: this.unfolded() ? "eye-off" : "eye",
              variant: "outline",
              size: "sm",
              kbd: "z",
              onClick: (_e, cx) => this.fold(cx),
            }),
          ),
      )
      .child(v_flex().flex_1().min_h(0).px(16).child(this.body(k, cx)))
      .child(
        h_flex()
          .px(24)
          .py(10)
          .gap(8)
          .flex_none()
          .items_center()
          .border_t_1()
          .border_color(k.border)
          .bg(alpha(k.muted, 0.5))
          .child(
            h_flex()
              .flex_1()
              .gap(6)
              .flex_wrap()
              .children(acts.map((a, i) => Button(`act-${a.id}`, { label: a.label, icon: a.icon, kbd: a.kbd, size: "sm", variant: a.variant ?? (i === 0 ? "default" : "outline"), onClick: () => a.run() }))),
          )
          .when(this.status !== "live", (e) => e.child(Badge(this.status.startsWith("error") ? "offline" : "connecting", this.status.startsWith("error") ? "destructive" : "secondary", { dot: this.status.startsWith("error") ? k.destructive : k.warning }))),
      )
      .child(KeyHints([["j k", "move"], ["x", "done"], ["⏎", "edit"], ["v", "mark"], ["m", "menu"], ["/", "filter"], ["u", "undo"], ["ctrl p", "commands"]]));

    const toasts = this.toasts.render();
    let out = toasts ? root.child(toasts) : root;
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id("palette-backdrop")
          .absolute()
          .inset_0()
          .bg("#00000055")
          .on_mouse_down("left", (_e, cx) => this.closePalette(cx))
          .child(
            v_flex()
              .id("palette")
              .absolute()
              .top(72)
              .left(0)
              .right(0)
              .items_center()
              .child(
                v_flex()
                  .id("palette-card")
                  .w(560)
                  .max_h(420)
                  .rounded(RADIUS.lg)
                  .border_1()
                  .border_color(k.border)
                  .bg(k.popover)
                  .shadow_lg()
                  .overflow_hidden()
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .child(Command("todo-commands", { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id })),
              ),
          ),
      );
    }
    return out;
  }
}
