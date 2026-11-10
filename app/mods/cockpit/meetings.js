// The meetings: the `meetings` view the hub pushes, asked again each minute so "in 12 min" stays true. The running or
// next meeting on a banner with Join, the week as a list by day, a day's calendar or the whole week's, the selected
// meeting's details and brief beside them, and every key and action of the TUI app (join, open, brief, prepare or
// redo, the week view, a command list on ctrl+p).

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState, TextView } from "gpui-base";
import { settings } from "herdr";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Badge, Button, ListRow, PageHeader, SearchField, KeyHints, Empty, ErrorState, Skeleton, Command,
  Toaster, Flashes, Scroller, Hover, Progress, Section, SidePanel, Tabs, Avatar, Spinner, openMenu, alpha, StageHeading,
} from "./ui.js";

const mono = () => settings().font ?? "JetBrains Mono";
const HOUR = 48;
const ICON = { join: "video", open: "external", brief: "file-text", prepare: "sparkles" };
const RESPONSE = { accepted: ["Accepted", "success"], tentative: ["Maybe", "warning"], needsAction: ["Not answered", "outline"], declined: ["Declined", "destructive"] };
const VIEWS = [
  { value: "list", label: "List", icon: "list" },
  { value: "day", label: "Day", icon: "calendar" },
  { value: "week", label: "Week", icon: "columns" },
];

export default class Meetings {
  init(_p, cx) {
    this.model = null;
    this.status = "connecting";
    this.sel = null;
    this.open = new Set();
    this.mode = "list";
    this.day = null;
    this.filter = "";
    this.filtering = false;
    this.palette = null;
    this.showDetails = true;
    this.width = 1000;
    this.closed = false;
    this.hover = new Hover();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();
    this.search = InputState.new({ placeholder: "Filter meetings, people…" });
    this.search.on("change", (_e, cx) => {
      this.filter = this.search.value();
      if (!this.selectable().includes(this.sel)) this.sel = this.selectable()[0] ?? null;
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
      { views: ["meetings"], focus: "meetings" },
      (m, cx) => {
        if (m.view === "meetings") this.take(m.value, cx);
        if (m.focus === "meetings") this.focusOn(m.target?.key, cx);
        cx.notify();
      },
      (state, cx) => {
        this.status = state;
        cx.notify();
      },
    );
    // The times in words move with the clock: the view is asked again at each minute.
    cx.spawn(async (cx) => {
      while (!this.closed) {
        await hub.sleep(cx, 60000 - (Date.now() % 60000) + 300);
        if (this.closed) break;
        try {
          this.take(await hub.view("meetings"), cx);
          cx.notify();
        } catch {}
      }
    });
  }

  close() {
    this.closed = true;
    this.sub?.stop();
  }

  command(c, cx) {
    if (c === "focus" || c === "show:meetings") this.handle.focus();
    const [scope, action, ...rest] = c.split(":");
    if (scope === "meetings") {
      const m = this.meeting(rest.join(":"));
      if (m) {
        this.sel = m.id;
        if (action === "golist") this.focusOn(m.id, cx);
        else this.run(cx, action, m);
      }
    }
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The model

  meetings() {
    return this.model?.meetings ?? [];
  }

  meeting(id) {
    return this.meetings().find((m) => m.id === id);
  }

  matches(m) {
    const words = this.filter.toLowerCase().split(/\s+/).filter(Boolean);
    const text = `${m.title} ${(m.people ?? []).join(" ")} ${m.response}`.toLowerCase();
    return words.every((w) => text.includes(w));
  }

  /** The list's meetings in order: from an hour ago on, those the filter lets through. */
  shownIds() {
    return (this.model?.shown ?? []).filter((id) => {
      const m = this.meeting(id);
      return m && this.matches(m);
    });
  }

  /** What j and k move through: the list's meetings, or the calendar's blocks in time order. */
  selectable() {
    if (this.mode === "list") return this.shownIds();
    const grid = this.model?.grid;
    const days = this.mode === "day" ? [this.dayIndex()] : (grid?.days ?? []).map((_, i) => i);
    return (grid?.blocks ?? []).filter((b) => days.includes(b.day) && this.meeting(b.id) && this.matches(this.meeting(b.id))).map((b) => b.id);
  }

  selected() {
    return this.meeting(this.sel);
  }

  /** The running or next meeting, else the first shown. */
  defaultSel() {
    const ids = this.shownIds();
    return ids.find((id) => ["now", "soon", "upcoming"].includes(this.meeting(id)?.state)) ?? ids[0] ?? null;
  }

  take(model, cx) {
    const before = this.model;
    this.model = model;
    if (!before) {
      this.sel = this.defaultSel();
      return;
    }
    const k = tokens();
    const was = new Map((before.meetings ?? []).map((m) => [m.id, m]));
    for (const m of this.meetings()) {
      const old = was.get(m.id);
      if (!old && before.meetings?.length) this.flashes.start(cx, m.id, k.ring);
      if (old && !old.brief && m.brief) {
        this.flashes.start(cx, m.id, k.success);
        this.toasts.push(cx, "Brief ready", { kind: "success", description: m.title, action: { label: "Show", onClick: (_e, cx) => this.focusOn(m.id, cx) } });
      }
      if (old && old.state !== "now" && m.state === "now") this.flashes.start(cx, m.id, k.ring);
    }
    if (!this.meeting(this.sel) || !this.selectable().includes(this.sel)) this.sel = this.mode === "list" ? this.defaultSel() : this.selectable()[0] ?? this.sel;
  }

  focusOn(id, cx) {
    const m = this.meeting(id);
    if (!m) {
      if (id) this.toasts.push(cx, "That meeting is not in this week's calendar any more");
      return;
    }
    if (!this.matches(m)) {
      this.filter = "";
      this.search.set_value("");
    }
    this.mode = "list";
    this.sel = m.id;
    this.open.add(m.id);
    this.showDetails = true;
    this.flashes.start(cx, m.id, tokens().ring);
    this.handle.focus();
    cx.notify();
  }

  dayIndex() {
    const days = this.model?.grid?.days ?? [];
    const at = days.findIndex((d) => d.date === (this.day ?? this.model?.today));
    return at >= 0 ? at : 0;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async hubAct(cx, action, args) {
    try {
      const r = await hub.act("meetings", action, args);
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note && !r.note.startsWith("opened ")) this.toasts.push(cx, r.note, { kind: "success" });
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    }
    cx.notify();
  }

  /** A meeting's action from the model: join, open, brief (fold it), prepare (write it, or again). */
  run(cx, id, m = this.selected()) {
    if (!m) return;
    switch (id) {
      case "join":
        if (m.join) cx.spawn(async (cx) => this.hubAct(cx, "open_url", [m.join]));
        break;
      case "open":
        if (m.url) cx.spawn(async (cx) => this.hubAct(cx, "open_url", [m.url]));
        else this.toasts.push(cx, "No calendar link for this one");
        break;
      case "brief":
        if (!this.open.delete(m.id)) this.open.add(m.id);
        this.showDetails = true;
        break;
      case "prepare":
        this.open.add(m.id);
        this.showDetails = true;
        cx.spawn(async (cx) => this.hubAct(cx, "prepare", [m.raw]));
        break;
    }
    cx.notify();
  }

  acts(m) {
    return (m?.acts ?? []).map((a) => ({ ...a, label: a.id === "brief" && this.open.has(m.id) ? "Hide brief" : a.label }));
  }

  menu(m) {
    openMenu(
      [
        ...this.acts(m).map((a) => ({ label: a.label, icon: ICON[a.id], kbd: a.key, command: `meetings:${a.id}:${m.id}` })),
        "separator",
        { label: "Show in the list", icon: "list", command: `meetings:golist:${m.id}` },
      ],
      { width: 220 },
    );
  }

  setMode(cx, mode) {
    this.mode = mode;
    if (mode === "day") this.day = this.day ?? this.selected()?.day ?? this.model?.today ?? null;
    const ids = this.selectable();
    if (!ids.includes(this.sel)) this.sel = ids.find((id) => this.meeting(id)?.state !== "past") ?? ids[0] ?? this.sel;
    cx.notify();
  }

  shiftDay(cx, by) {
    const days = this.model?.grid?.days ?? [];
    if (!days.length) return;
    const at = Math.max(0, Math.min(days.length - 1, this.dayIndex() + by));
    this.day = days[at].date;
    this.sel = this.selectable()[0] ?? this.sel;
    cx.notify();
  }

  move(cx, by) {
    const ids = this.selectable();
    if (!ids.length) return;
    const at = Math.max(0, ids.indexOf(this.sel));
    this.sel = ids[Math.max(0, Math.min(ids.length - 1, at + by))];
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The command list (ctrl+p): the views, then every meeting's actions and a jump to each, today's calls first

  paletteGroups(cx) {
    const q = this.query.value().toLowerCase().split(/\s+/).filter(Boolean);
    const hit = (text) => q.every((w) => text.toLowerCase().includes(w));
    const views = VIEWS.filter((v) => v.value !== this.mode).map((v) => ({ id: `p-view-${v.value}`, label: `${v.label} view`, icon: v.icon, shortcut: v.value === "day" ? "d" : "w", run: (cx) => this.setMode(cx, v.value) }));
    const calls = [];
    const rest = [];
    for (const id of this.model?.shown ?? []) {
      const m = this.meeting(id);
      if (!m) continue;
      const hint = `${m.when} · ${m.duration}`;
      for (const a of this.acts(m)) {
        const item = { id: `p-${m.id}-${a.id}`, label: `${a.label}  ${m.title}`, hint, icon: ICON[a.id], run: (cx) => ((this.sel = m.id), this.run(cx, a.id, m)) };
        (a.id === "join" && m.day === this.model.today && m.state !== "past" ? calls : rest).push(item);
      }
      rest.push({ id: `p-${m.id}-go`, label: `Go to  ${m.title}`, hint, icon: "chevron-right", run: (cx) => this.focusOn(m.id, cx) });
    }
    return [
      { heading: "Views", items: views.filter((i) => hit(i.label)) },
      { heading: "Today's calls", items: calls.filter((i) => hit(`${i.label} ${i.hint}`)) },
      { heading: "Meetings", items: q.length ? rest.filter((i) => hit(`${i.label} ${i.hint}`)) : [] },
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
    const item = picked ?? this.paletteGroups(cx).flatMap((g) => g.items)[this.palette?.at ?? 0];
    this.closePalette(cx);
    if (item) item.run(cx);
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
    const m = this.selected();
    switch (k) {
      case "ctrl-p":
        this.openPalette(cx);
        break;
      case "j":
      case "down":
        this.move(cx, 1);
        break;
      case "k":
      case "up":
        this.move(cx, -1);
        break;
      case "pagedown":
      case "ctrl-d":
        this.move(cx, 10);
        break;
      case "pageup":
      case "ctrl-u":
        this.move(cx, -10);
        break;
      case "g":
      case "home":
        this.move(cx, -1e6);
        break;
      case "shift-g":
      case "end":
        this.move(cx, 1e6);
        break;
      case "w":
        this.setMode(cx, this.mode === "week" ? "list" : "week");
        break;
      case "d":
        this.setMode(cx, this.mode === "day" ? "list" : "day");
        break;
      case "]":
      case "l":
      case "right":
        if (this.mode !== "day") return false;
        this.shiftDay(cx, 1);
        break;
      case "[":
      case "h":
      case "left":
        if (this.mode !== "day") return false;
        this.shiftDay(cx, -1);
        break;
      case "/":
        this.filtering = true;
        this.search.focus();
        break;
      case "escape":
        if (this.mode !== "list") this.setMode(cx, "list");
        else if (this.filter) {
          this.filter = "";
          this.search.set_value("");
        } else return false;
        break;
      case "enter": {
        const acts = this.acts(m);
        const act = acts.find((a) => a.main) ?? acts[0];
        if (act) this.run(cx, act.id, m);
        break;
      }
      case "shift-f10":
        if (m) this.menu(m);
        break;
      default: {
        const act = this.acts(m).find((a) => a.key === k);
        if (!act) return false;
        this.run(cx, act.id, m);
      }
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  stateColor(m, k) {
    if (m.state === "now") return k.ring;
    if (m.state === "soon") return k.warning;
    if (m.state === "past") return k.mutedFg;
    return m.response === "accepted" ? k.success : k.mutedFg;
  }

  banner(k) {
    const b = this.model?.banner;
    if (!b || b.state === "none") {
      return h_flex().mx(24).mt(12).px(16).h(44).flex_none().items_center().rounded(RADIUS.lg).border_1().border_color(k.border).text_color(k.mutedFg).child("Nothing left this week.");
    }
    const now = b.state === "now";
    const lit = now || Boolean(b.soon);
    const color = now ? k.ring : b.soon ? k.warning : k.mutedFg;
    const m = this.meeting(b.id);
    return h_flex()
      .id("meetings-banner")
      .mx(24)
      .mt(12)
      .px(16)
      .py(12)
      .gap(14)
      .flex_none()
      .items_center()
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(lit ? alpha(color, 0.5) : k.border)
      .bg(lit ? alpha(color, 0.08) : k.card)
      .cursor_pointer()
      .on_click((_e, cx) => this.focusOn(b.id, cx))
      .child(
        h_flex()
          .id("banner-icon")
          .size(36)
          .flex_none()
          .items_center()
          .justify_center()
          .rounded(RADIUS.lg)
          .bg(alpha(now ? k.ring : b.soon ? k.warning : k.mutedFg, 0.15))
          .child(Icon(now ? "video" : "clock", { size: 18, color }))
          .when(Boolean(b.soon), (e) => e.loop("opacity", { from: 0.5, to: 1, period: 1400 })),
      )
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .gap(4)
          .child(
            h_flex()
              .gap(8)
              .items_center()
              .child(div().text_size(TEXT.xs).font_semibold().text_color(now ? k.ring : b.soon ? k.warning : k.mutedFg).child(now ? "NOW" : "NEXT"))
              .child(div().text_size(TEXT.sm).font_medium().text_color(k.mutedFg).child(now ? `until ${b.until}` : b.when)),
          )
          .child(div().truncate().text_size(TEXT.lg).font_semibold().child(b.title))
          .when(now, (e) => e.child(Progress(b.progress ?? 0, { h: 4, color: k.ring }))),
      )
      .when(Boolean(m?.url) && this.width >= 520, (e) => e.child(Button("banner-open", { label: "Open", icon: "external", variant: "outline", size: "sm", onClick: (_e, cx) => (cx.stop_propagation(), this.run(cx, "open", m)) })))
      .when(Boolean(b.join), (e) => e.child(Button("banner-join", { label: "Join", icon: "video", size: "sm", onClick: (_e, cx) => (cx.stop_propagation(), this.run(cx, "join", m)) })));
  }

  meetingRow(m, k) {
    const id = `meeting-${m.id}`;
    const past = m.state === "past";
    const now = m.state === "now";
    const upcomingToday = m.day === this.model.today && !past && !now;
    const [label, variant] = RESPONSE[m.response] ?? [m.response, "outline"];
    return ListRow(id, {
      height: 48,
      selected: this.sel === m.id,
      flash: this.flashes.of(m.id),
      leading: h_flex()
        .gap(10)
        .items_center()
        .child(div().w(3).h(30).rounded(2).flex_none().bg(now ? k.ring : past ? k.border : m.response === "accepted" ? alpha(k.success, 0.7) : k.border))
        .child(
          v_flex()
            .w(52)
            .flex_none()
            .child(div().font_family(mono()).text_size(TEXT.sm).text_color(past ? k.mutedFg : k.foreground).child(m.time))
            .child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(m.duration)),
        ),
      title: v_flex()
        .flex_1()
        .min_w(0)
        .gap(2)
        .child(div().truncate().when(now, (e) => e.font_semibold()).text_color(past ? k.mutedFg : k.foreground).child(m.title))
        .child(
          h_flex()
            .gap(8)
            .items_center()
            .text_size(TEXT.xs)
            .text_color(k.mutedFg)
            .when(now, (e) => e.child(div().text_color(k.ring).font_medium().child("now")))
            .when(upcomingToday, (e) => e.child(div().text_color(m.state === "soon" ? k.warning : k.mutedFg).font_medium().child(m.until)))
            .when(m.response !== "accepted", (e) => e.child(div().text_color(variant === "warning" ? k.warning : k.mutedFg).child(label)))
            .when(m.attendees > 1, (e) => e.child(h_flex().gap(3).items_center().child(Icon("users", { size: 11 })).child(String(m.attendees))))
            .when(Boolean(m.brief), (e) => e.child(h_flex().gap(3).items_center().child(Icon("file-text", { size: 11 })).child("brief")))
            .when(Boolean(m.preparing), (e) => e.child(h_flex().gap(3).items_center().text_color(k.warning).child(Spinner({ size: 11, color: k.warning })).child("preparing the brief"))),
        ),
      hover: this.hover,
      actions: this.acts(m)
        .filter((a) => a.id !== "brief")
        .slice(0, 3)
        .map((a) => Button(`${id}-${a.id}`, { icon: ICON[a.id], variant: a.main ? "secondary" : "ghost", size: "icon-sm", tooltip: `${a.label}  ${a.key}`, onClick: (_e, cx) => (cx.stop_propagation(), (this.sel = m.id), this.run(cx, a.id, m)) })),
      onClick: (_e, cx) => {
        this.sel = m.id;
        this.handle.focus();
        cx.notify();
      },
      onDoubleClick: (_e, cx) => this.run(cx, m.join ? "join" : "open", m),
      onRightClick: () => this.menu(m),
    });
  }

  listRows(k) {
    const ids = new Set(this.shownIds());
    const rows = [];
    for (const day of this.model?.days ?? []) {
      const list = day.ids.filter((id) => ids.has(id)).map((id) => this.meeting(id));
      if (!list.length) continue;
      rows.push({
        height: 32,
        el: div().h(32).px(10).pt(6).child(StageHeading(`day-${day.label}`, day.label, { color: day.today ? k.ring : k.dim, count: list.length })),
      });
      for (const m of list) rows.push({ height: 48, el: this.meetingRow(m, k), id: m.id });
    }
    return rows;
  }

  list(k) {
    if (!this.shownIds().length) {
      return this.filter
        ? Empty({ icon: "search", title: "No meeting matches", description: `Nothing has every word of “${this.filter}”.` })
        : Empty({ icon: "calendar", title: "No meeting this week", description: "The calendar is clear from now on." });
    }
    const rows = this.listRows(k);
    return this.scroller.view("meetings-list", rows, rows.findIndex((r) => r.id === this.sel));
  }

  /** A calendar of these days (indexes into the grid's), the hours down the side, a block per meeting. */
  calendar(days, k) {
    const grid = this.model.grid;
    const height = Math.max(1, grid.to - grid.from) * HOUR;
    const hours = [];
    for (let h = Math.ceil(grid.from); h <= grid.to; h++) hours.push(h);
    const top = (h) => (h - grid.from) * HOUR;
    const lines = div()
      .absolute()
      .inset_0()
      .children(hours.map((h) => div().absolute().left(0).right(0).top(top(h)).h(1).bg(alpha(k.border, 0.7))));
    const column = (i) => {
      const day = grid.days[i];
      let col = div()
        .id(`cal-col-${day.date}`)
        .relative()
        .flex_1()
        .min_w(0)
        .h(height)
        .border_l_1()
        .border_color(k.border)
        .when(Boolean(day.today), (e) => e.bg(alpha(k.ring, 0.05)))
        .children(grid.blocks.filter((b) => b.day === i).map((b) => this.block(b, k)));
      if (grid.now && grid.now.day === i) {
        col = col.child(div().absolute().left(0).right(0).top(`${grid.now.at * 100}%`).h(2).bg(k.destructive).child(div().absolute().left(-1).top(-3).size(8).rounded(4).bg(k.destructive)));
      }
      return col;
    };
    const head = h_flex()
      .h(44)
      .pl(52)
      .flex_none()
      .border_b_1()
      .border_color(k.border)
      .children(
        days.map((i) => {
          const d = grid.days[i];
          const n = grid.blocks.filter((b) => b.day === i).length;
          return v_flex()
            .id(`cal-head-${d.date}`)
            .flex_1()
            .min_w(0)
            .justify_center()
            .px(8)
            .border_l_1()
            .border_color(k.border)
            .cursor_pointer()
            .hover((s) => s.bg(k.accent))
            .on_click((_e, cx) => ((this.day = d.date), this.setMode(cx, "day")))
            .child(div().truncate().text_size(TEXT.sm).font_semibold().text_color(d.today ? k.ring : k.foreground).child(this.mode === "day" ? d.label : d.short))
            .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(n ? `${n} meeting${n > 1 ? "s" : ""}` : "free"));
        }),
      );
    const body = div()
      .id("cal-body")
      .flex_1()
      .min_h(0)
      .overflow_y_scrollbar()
      .child(
        h_flex()
          .relative()
          .h(height + 12)
          .pt(6)
          .child(
            div()
              .w(52)
              .flex_none()
              .relative()
              .h(height)
              .children(hours.map((h) => div().absolute().right(8).top(top(h) - 7).font_family(mono()).text_size(TEXT.xs).text_color(k.mutedFg).child(`${String(h).padStart(2, "0")}:00`))),
          )
          .child(div().relative().flex_1().h(height).child(lines).child(h_flex().size_full().children(days.map(column)))),
      );
    return v_flex().flex_1().min_h(0).mx(24).mb(8).rounded(RADIUS.lg).border_1().border_color(k.border).overflow_hidden().child(head).child(body);
  }

  block(b, k) {
    const m = this.meeting(b.id);
    if (!m) return div();
    const sel = this.sel === m.id;
    const color = this.stateColor(m, k);
    const h = Math.max(18, (b.bottom - b.top) * (this.model.grid.to - this.model.grid.from) * HOUR - 2);
    const firm = m.response === "accepted";
    const flash = this.flashes.of(m.id);
    const fill = m.state === "now" ? alpha(k.ring, 0.28) : m.state === "past" ? k.muted : firm ? alpha(color, 0.14) : k.background;
    return div()
      .id(`cal-${m.id}`)
      .absolute()
      .top(`${b.top * 100}%`)
      .h(h)
      .left(`${(b.lane / b.lanes) * 100}%`)
      .w(`${100 / b.lanes}%`)
      .p(1)
      .child(
        v_flex()
          .id(`cal-${m.id}-card`)
          .size_full()
          .relative()
          .px(6)
          .py(h < 26 ? 0 : 2)
          .when(h < 26, (e) => e.justify_center())
          .overflow_hidden()
          .rounded(RADIUS.sm)
          .border_1()
          .border_color(sel ? k.foreground : firm ? alpha(color, 0.55) : k.border)
          .bg(fill)
          .cursor_pointer()
          .hover((s) => s.bg(alpha(color, 0.26)))
          .tooltip(`${m.title}\n${m.span} · ${m.duration}${m.attendees > 1 ? ` · ${m.attendees} people` : ""}`)
          .on_click((e, cx) => {
            if (e.click_count > 1) return this.run(cx, m.join ? "join" : "open", m);
            this.sel = m.id;
            this.handle.focus();
            cx.notify();
          })
          .on_mouse_down("right", (_e, cx) => ((this.sel = m.id), this.menu(m), cx.notify()))
          .when(Boolean(flash), (e) => e.child(div().id(`cal-${m.id}-flash`).absolute().inset_0().bg(alpha(flash?.color ?? k.ring, 0.35)).opacity(flash?.on ? 1 : 0).transition("opacity", { duration: 1100, easing: "ease-out" })))
          .child(div().truncate().text_size(h < 22 ? 10 : TEXT.xs).line_height(1.2).font_semibold().text_color(m.state === "past" ? k.mutedFg : k.foreground).child(m.title))
          .when(h >= 34, (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(m.span))),
      );
  }

  details(m, k) {
    const id = "meetings-side";
    const [label, variant] = RESPONSE[m.response] ?? [m.response, "outline"];
    const people = (m.people ?? []).slice(0, 12);
    let brief;
    if (m.brief) {
      brief = Section(`${id}-brief`, {
        title: "Brief",
        open: this.open.has(m.id) || this.width >= 860,
        onToggle: (_e, cx) => this.run(cx, "brief", m),
        action: Button(`${id}-redo`, { label: "Redo", icon: "sparkles", variant: "ghost", size: "sm", kbd: "r", onClick: (_e, cx) => (cx.stop_propagation(), this.run(cx, "prepare", m)) }),
        children: [
          div()
            .px(8)
            .child(
              TextView.markdown(`${id}-md-${m.id}`, m.brief.slice(0, 8000))
                .selectable(true)
                .text_size(TEXT.base)
                .text_color(k.foreground)
                .on_link_click((url, cx) => cx.spawn(async (cx) => this.hubAct(cx, "open_url", [url]))),
            ),
        ],
      });
    } else if (m.preparing) {
      brief = v_flex()
        .gap(8)
        .child(h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.warning).child(Spinner({ size: 12, color: k.warning })).child("Preparing the brief, about a minute…"))
        .children([0, 1, 2].map((i) => Skeleton({ w: 220 - i * 40, h: 12 })));
    } else if (m.acts.some((a) => a.id === "prepare")) {
      brief = v_flex()
        .gap(8)
        .items_start()
        .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("A brief gathers what the meeting is about from its invite, tickets and PRs."))
        .child(Button(`${id}-prepare`, { label: "Prepare a brief", icon: "sparkles", variant: "outline", size: "sm", kbd: "p", onClick: (_e, cx) => this.run(cx, "prepare", m) }));
    } else {
      brief = div().text_size(TEXT.sm).text_color(k.mutedFg).child(m.state === "past" ? "Over: no brief to prepare." : "No brief for this one.");
    }
    return SidePanel(id, {
      title: m.title,
      description: m.when,
      width: Math.min(400, Math.max(300, this.width * 0.36)),
      onClose: (_e, cx) => ((this.showDetails = false), this.open.delete(m.id), cx.notify()),
      children: [
        h_flex()
          .gap(6)
          .flex_wrap()
          .child(Badge(m.duration, "secondary", { icon: "clock" }))
          .child(Badge(label, variant))
          .when(m.attendees > 1, (e) => e.child(Badge(`${m.attendees} people`, "secondary", { icon: "users" }))),
        ...(people.length
          ? [
              v_flex()
                .gap(6)
                .child(div().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child("PEOPLE"))
                .child(
                  h_flex()
                    .gap(6)
                    .flex_wrap()
                    .children(people.map((p) => h_flex().h(26).pl(2).pr(8).gap(6).items_center().rounded(13).bg(k.muted).text_size(TEXT.sm).child(Avatar(p, { size: 22 })).child(p)))
                    .when(m.attendees > people.length, (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(`+${m.attendees - people.length} more`))),
                ),
            ]
          : []),
        m.brief ? brief : v_flex().gap(6).child(div().text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child("BRIEF")).child(brief),
      ],
      footer: h_flex()
        .flex_1()
        .gap(6)
        .flex_wrap()
        .children(this.acts(m).filter((a) => a.id !== "brief" && a.id !== "prepare").map((a) => Button(`${id}-${a.id}`, { label: a.label, icon: ICON[a.id], kbd: a.key, size: "sm", variant: a.main ? "default" : "outline", onClick: (_e, cx) => this.run(cx, a.id, m) }))),
    });
  }

  actionBar(k) {
    const m = this.selected();
    return h_flex()
      .px(24)
      .py(10)
      .gap(6)
      .flex_none()
      .flex_wrap()
      .items_center()
      .border_t_1()
      .border_color(k.border)
      .bg(alpha(k.muted, 0.5))
      .children(this.acts(m).map((a) => Button(`meetings-bar-${a.id}`, { label: a.label, icon: ICON[a.id], kbd: a.key, size: "sm", variant: a.main ? "default" : "outline", onClick: (_e, cx) => this.run(cx, a.id, m) })))
      .when(this.status !== "live", (e) => e.child(div().flex_1()).child(Badge(this.status.startsWith("error") ? "offline" : "connecting", this.status.startsWith("error") ? "destructive" : "secondary")));
  }

  body(k) {
    if (this.mode === "list") return v_flex().flex_1().min_h(0).px(14).child(this.list(k));
    const grid = this.model.grid;
    if (!grid?.days?.length) return Empty({ icon: "calendar", title: "No week to draw yet" });
    if (this.mode === "week") return this.calendar(grid.days.map((_, i) => i), k);
    const at = this.dayIndex();
    return v_flex()
      .flex_1()
      .min_h(0)
      .child(
        h_flex()
          .px(24)
          .pb(8)
          .gap(6)
          .flex_none()
          .items_center()
          .child(Button("day-prev", { icon: "chevron-left", variant: "outline", size: "icon-sm", tooltip: "Day before  [", disabled: at === 0, onClick: (_e, cx) => this.shiftDay(cx, -1) }))
          .child(Button("day-next", { icon: "chevron-right", variant: "outline", size: "icon-sm", tooltip: "Day after  ]", disabled: at >= grid.days.length - 1, onClick: (_e, cx) => this.shiftDay(cx, 1) }))
          .child(div().text_size(TEXT.base).font_semibold().child(grid.days[at].label)),
      )
      .child(this.calendar([at], k));
  }

  overlays(root, k, cx) {
    const toasts = this.toasts.render();
    let out = toasts ? root.child(toasts) : root;
    if (this.palette) {
      const groups = this.paletteGroups(cx);
      const all = groups.flatMap((g) => g.items);
      this.palette.at = Math.min(this.palette.at, Math.max(0, all.length - 1));
      const wrapped = groups.map((g) => ({ ...g, items: g.items.map((i) => ({ ...i, onSelect: (_e, cx) => this.runPalette(cx, i) })) }));
      out = out.child(
        div()
          .id("meetings-palette-backdrop")
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
              .child(
                v_flex()
                  .id("meetings-palette-card")
                  .w(Math.min(600, this.width - 32))
                  .max_h(440)
                  .rounded(RADIUS.lg)
                  .border_1()
                  .border_color(k.border)
                  .bg(k.popover)
                  .shadow_lg()
                  .overflow_hidden()
                  .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
                  .child(Command("meetings-commands", { state: this.query, groups: wrapped, selected: all[this.palette.at]?.id })),
              ),
          ),
      );
    }
    return out;
  }

  render(cx) {
    const k = tokens();
    const m = this.model;
    const description = !m ? "Loading…" : m.waiting ? "Waiting for the first fetch from your calendar" : m.updated ? `Updated ${m.updated}` : "This week";
    let root = v_flex()
      .id("meetings")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        const w = size.width / (settings().zoom ?? 1);
        if (Math.abs(w - this.width) > 1) {
          this.width = w;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => {
        if (!this.filtering && !this.palette) this.handle.focus();
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "calendar",
          title: "Meetings",
          description,
          actions: [
            Tabs("meetings-mode", { items: VIEWS.map((v) => ({ ...v, label: this.width < 640 ? "" : v.label })), value: this.mode, onChange: (v, cx) => this.setMode(cx, v) }),
            Button("meetings-commands", { icon: "command", variant: "ghost", size: "icon", tooltip: "Commands  ctrl+p", onClick: (_e, cx) => this.openPalette(cx) }),
            Button("meetings-details", { icon: "panel-right", variant: "ghost", size: "icon", tooltip: "Details", onClick: (_e, cx) => ((this.showDetails = !this.showDetails), cx.notify()) }),
          ],
        }),
      );
    if (!m) {
      const inner = this.status.startsWith("error")
        ? ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) })
        : v_flex().p(24).gap(12).children([0, 1, 2, 3].map((i) => h_flex().gap(10).child(Skeleton({ w: 52, h: 32 })).child(Skeleton({ w: 180 + ((i * 97) % 220), h: 32 }))));
      return this.overlays(root.child(inner), k, cx);
    }
    if (m.waiting) {
      const why = m?.connected
        ? "The hub fetches this week from your calendar, within a few minutes."
        : "No calendar is connected. cockpit setup adds one, or give the config the calendar's secret iCal address.";
      return this.overlays(root.child(Empty({ icon: "calendar", title: "No calendar yet", description: why })), k, cx);
    }
    const sel = this.selected();
    const wide = this.width >= 860;
    const main = v_flex()
      .flex_1()
      .min_w(0)
      .h_full()
      .when(Boolean(m.error), (e) =>
        e.child(h_flex().mx(24).mt(12).px(12).py(8).gap(8).items_center().rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).text_size(TEXT.sm).text_color(k.destructive).child(Icon("alert", { size: 14 })).child(div().flex_1().truncate().child(m.error ?? ""))),
      )
      .child(this.banner(k))
      .child(
        h_flex()
          .px(24)
          .py(12)
          .gap(8)
          .flex_none()
          .items_center()
          .child(h_flex().id("meetings-filter").flex_1().on_mouse_down("left", (_e, cx) => ((this.filtering = true), cx.notify())).child(SearchField(this.search, { active: this.filtering, kbd: "/" }))),
      )
      .child(this.body(k));
    root = root
      .child(h_flex().flex_1().min_h(0).child(main).when(wide && this.showDetails && Boolean(sel), (e) => e.child(this.details(sel, k))))
      .when(!wide || !this.showDetails || !sel, (e) => e.child(this.actionBar(k)))
      .child(KeyHints([["j k", "move"], ["⏎", "join"], ["o", "open"], ["b", "brief"], ["p", "prepare"], ["w", "week"], ["d", "day"], ["/", "filter"], ["ctrl p", "commands"]]));
    // Narrow, the details come over the list from the right while a brief is open.
    if (!wide && sel && this.open.has(sel.id) && this.showDetails) {
      root = root.child(
        div()
          .id("meetings-sheet")
          .absolute()
          .inset_0()
          .bg("#00000044")
          .on_mouse_down("left", (_e, cx) => (this.open.delete(sel.id), cx.notify()))
          .child(h_flex().absolute().top(0).bottom(0).right(0).on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(this.details(sel, k).shadow_lg())),
      );
    }
    return this.overlays(root, k, cx);
  }
}
