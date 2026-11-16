// The recap popup (cmd-r): every agent as a card, grouped by whose move it is (after herdr-recap's design, MIT): its
// recap, its question answered in place, its facts as chips; stage tabs along the top, and a Time tab with the week's
// agent time per ticket (the ledger). Keys: ↑/↓ move, ⏎ go to it, ⇥ reply, r recap, s snooze, f fork, ←/→ stage.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { props, closeOverlay } from "herdr";
import { tokens, alpha, TEXT, RADIUS, Icon, Kbd, Button, Chip, CountPill, StageHeading, SearchField, KeyHints, Table, Empty, Spinner, dur } from "./ui.js";
import { AgentsModel, QuestionCard, QuotaBars, StageMark, Chips, STAGES, stageColor, stageAge, hours } from "./agentsui.js";

export default class Recap {
  init(_props, cx) {
    this.agents = new AgentsModel();
    this.agents.loadPins();
    this.agents.follow(cx, () => this.keepSelection());
    this.tab = props()?.tab ?? "all";
    this.selected = null;
    this.filtering = false;
    this.filter = InputState.new({ placeholder: "Filter agents: title, ticket, branch, repo…" });
    this.filter.on("change", (_e, cx) => cx.notify());
    this.filter.on("submit", (_e, cx) => {
      this.filtering = false;
      this.handle.focus();
      cx.notify();
    });
    this.handle = cx.focus_handle();
    this.handle.focus();
  }

  close() {
    this.agents.stop();
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  /** The rows the tab and the filter leave, in whose-move order. */
  shown() {
    const q = this.filter.value().trim().toLowerCase();
    const match = (r) => !q || [r.title, r.ticket?.key, r.ticket?.title, r.branch, r.repo, r.space?.name, r.tab?.name, r.recap?.text].some((v) => String(v ?? "").toLowerCase().includes(q));
    return this.agents.rows.filter((r) => match(r) && (this.tab === "all" || r.stage === this.tab));
  }

  keepSelection() {
    const rows = this.shown();
    if (!rows.some((r) => r.terminal === this.selected)) this.selected = rows[0]?.terminal ?? null;
  }

  tabs() {
    const counts = new Map();
    for (const r of this.agents.rows) counts.set(r.stage, (counts.get(r.stage) ?? 0) + 1);
    return [
      { value: "all", label: "All", count: this.agents.rows.length },
      ...Object.keys(STAGES)
        .filter((s) => counts.get(s) || s === this.tab)
        .map((s) => ({ value: s, label: STAGES[s].label, count: counts.get(s) ?? 0, color: stageColor(s) })),
      { value: "time", label: "Time", icon: "timer" },
    ];
  }

  step(d, cx) {
    const rows = this.shown();
    const at = rows.findIndex((r) => r.terminal === this.selected);
    const next = rows[Math.max(0, Math.min(rows.length - 1, at + d))];
    if (next) this.selected = next.terminal;
    cx.notify();
  }

  stepTab(d, cx) {
    const tabs = this.tabs();
    const at = tabs.findIndex((t) => t.value === this.tab);
    this.tab = tabs[(at + d + tabs.length) % tabs.length].value;
    this.keepSelection();
    cx.notify();
  }

  key(e, cx) {
    const m = this.agents;
    const row = m.row(this.selected);
    if (m.replying) {
      if (e.key === "escape") m.cancelReply(cx);
      return e.key === "escape";
    }
    if (this.filtering) {
      if (e.key === "escape") {
        this.filtering = false;
        this.handle.focus();
        cx.notify();
        return true;
      }
      return false;
    }
    switch (e.key) {
      case "escape":
        this.dismiss();
        return true;
      case "down":
      case "j":
        this.step(1, cx);
        return true;
      case "up":
      case "k":
        this.step(-1, cx);
        return true;
      case "right":
        this.stepTab(1, cx);
        return true;
      case "left":
        this.stepTab(-1, cx);
        return true;
      case "/":
        this.filtering = true;
        this.filter.focus();
        cx.notify();
        return true;
    }
    if (!row) return false;
    if (e.key === "enter") {
      m.go(cx, row);
      this.dismiss();
    } else if (e.key === "tab" && row.stage === "needs_you") m.startReply(cx, row, row.question?.choices?.find((c) => c.free)?.n ?? null);
    else if (e.key === "r") m.act(cx, "recap", { terminal: row.terminal, force: true }, "recapping");
    else if (e.key === "s") m.menu(row);
    else if (e.key === "f") m.act(cx, "fork", { terminal: row.terminal, place: "tab" });
    else if (e.key === "e") m.act(cx, "seen", { terminal: row.terminal });
    else if (/^[1-9]$/.test(e.key) && row.question?.choices?.length) m.answer(cx, row, Number(e.key));
    else return false;
    return true;
  }

  card(k, r, now) {
    const id = `card-${r.terminal}`;
    const on = r.terminal === this.selected;
    const tone = stageColor(r.stage);
    const busy = this.agents.busy.get(r.terminal);
    const recap = r.recap;
    const recapLine = recap?.text
      ? div().text_size(TEXT.base).text_color(recap.fresh ? k.foreground : k.mutedFg).child(recap.text)
      : recap?.running || busy === "recapping"
        ? h_flex().gap(6).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 12 })).child("Recapping…")
        : recap?.error
          ? div().text_size(TEXT.sm).text_color(k.mutedFg).child(recap.error)
          : h_flex()
              .gap(8)
              .items_center()
              .child(div().text_size(TEXT.sm).italic().text_color(k.mutedFg).child(r.said ? `“${r.said}”` : r.stage === "working" ? "Nothing to recap yet." : "No recap yet."))
              .when(r.stage !== "working", (e) => e.child(Button(`${id}-recap`, { label: "Recap", icon: "sparkles", variant: "ghost", size: "sm", kbd: "r", onClick: (_e, cx) => this.agents.act(cx, "recap", { terminal: r.terminal }, "recapping") })));
    const head = h_flex()
      .gap(10)
      .items_center()
      .child(h_flex().w(16).flex_none().justify_center().child(StageMark(id, r.stage, { size: 9 })))
      .child(div().flex_shrink_1().min_w(0).truncate().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(r.title || r.tab?.name || r.address))
      .child(Chip(`${r.space?.name ?? ""} · ${r.tab?.name ?? ""}`, { mono: true, icon: "terminal" }))
      .when(Boolean(r.repo), (e) => e.child(div().flex_none().text_size(TEXT.sm).text_color(k.dim).child(r.repo)))
      .child(div().flex_1())
      .when(Boolean(r.unread), (e) => e.child(div().size(7).rounded(4).bg(tone)))
      .child(div().flex_none().text_size(TEXT.sm).font_medium().text_color(tone).child(stageAge(r, now)));
    const actions = h_flex()
      .gap(6)
      .items_center()
      .child(Button(`${id}-go`, { label: "Go to it", icon: "corner-down-left", size: "sm", kbd: "⏎", onClick: (_e, cx) => (this.agents.go(cx, r), this.dismiss()) }))
      .when(Boolean(recap?.text), (e) => e.child(Button(`${id}-again`, { label: "Recap again", icon: "sparkles", variant: "ghost", size: "sm", kbd: "r", onClick: (_e, cx) => this.agents.act(cx, "recap", { terminal: r.terminal, force: true }, "recapping") })))
      .child(Button(`${id}-snooze`, { label: r.snoozed ? "Wake" : "Snooze", icon: "alarm-clock", variant: "ghost", size: "sm", kbd: "s", onClick: (_e, cx) => (r.snoozed ? this.agents.act(cx, "snooze", { terminal: r.terminal, until: null }) : this.agents.menu(r)) }))
      .child(Button(`${id}-fork`, { label: "Fork", icon: "git-fork", variant: "ghost", size: "sm", kbd: "f", onClick: (_e, cx) => this.agents.act(cx, "fork", { terminal: r.terminal, place: "tab" }) }))
      .when(Boolean(r.unread) && !r.remote, (e) => e.child(Button(`${id}-seen`, { label: "Seen", icon: "check", variant: "ghost", size: "sm", kbd: "e", onClick: (_e, cx) => this.agents.act(cx, "seen", { terminal: r.terminal }) })));
    return h_flex()
      .id(id)
      .w_full()
      .flex_none()
      .gap(0)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(on ? alpha(tone, 0.55) : k.border)
      .bg(on ? alpha(tone, 0.06) : k.card)
      .overflow_hidden()
      .cursor_pointer()
      .on_click((_e, cx) => {
        this.selected = r.terminal;
        this.handle.focus();
        cx.notify();
      })
      .on_mouse_down("right", () => this.agents.menu(r))
      .child(div().w(3).h_full().flex_none().bg(on ? tone : alpha(tone, 0)))
      .child(
        v_flex()
          .flex_1()
          .min_w(0)
          .px(16)
          .py(12)
          .gap(10)
          .child(head)
          .when(r.stage === "needs_you", (e) => e.child(QuestionCard(`${id}-q`, r, this.agents, { wide: true })))
          .child(recapLine)
          .child(h_flex().flex_wrap().gap(6).children(Chips(r)))
          .when(on && r.stage !== "needs_you", (e) => e.child(actions)),
      );
  }

  /** The week's agent time per ticket, Monday to Sunday, from the ledger. */
  timeTable(k) {
    const week = this.agents.value?.week;
    const tickets = new Map((this.agents.value?.tickets ?? []).map((t) => [t.key, t]));
    if (!week?.rows?.length) return Empty({ icon: "timer", title: "No agent time this week yet", description: "The hub notes how long agents work on each ticket while it runs." });
    const names = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    const today = new Date();
    const todayKey = `${today.getFullYear()}-${String(today.getMonth() + 1).padStart(2, "0")}-${String(today.getDate()).padStart(2, "0")}`;
    const max = Math.max(1, ...week.rows.flatMap((r) => r.secs));
    const cell = (s, isToday) =>
      v_flex()
        .w_full()
        .gap(3)
        .child(div().text_size(TEXT.sm).font_family("JetBrains Mono").text_color(s ? (isToday ? k.foreground : k.mutedFg) : k.dim).child(s ? hours(s) : "·"))
        .child(div().h(3).rounded(2).bg(k.muted).child(div().h_full().rounded(2).bg(isToday ? k.ring : alpha(k.ring, 0.45)).w(`${(s / max) * 100}%`)));
    const columns = [
      { key: "ticket", label: "Ticket" },
      ...week.days.map((d, i) => ({ key: `d${i}`, label: d === todayKey ? `${names[i]} · today` : names[i], width: 92 })),
      { key: "total", label: "Week", width: 84, align: "right" },
    ];
    const rows = [...week.rows]
      .sort((a, b) => b.total - a.total)
      .map((r) => {
        const t = tickets.get(r.key);
        const cells = {
          ticket: h_flex().gap(8).min_w(0).items_center().child(Icon("ticket", { size: 13, color: k.info })).child(div().flex_none().font_semibold().child(r.key)).child(div().min_w(0).truncate().text_color(k.mutedFg).child(t?.title ?? "")),
          total: div().font_semibold().font_family("JetBrains Mono").child(hours(r.total)),
        };
        week.days.forEach((d, i) => (cells[`d${i}`] = cell(r.secs[i], d === todayKey)));
        return { id: r.key, cells };
      });
    const total = week.rows.reduce((a, r) => a + r.total, 0);
    return v_flex()
      .gap(12)
      .child(
        h_flex()
          .gap(8)
          .items_center()
          .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(`Agents worked ${hours(total)} across ${week.rows.length} tickets this week; parallel agents on one ticket count once.`)),
      )
      .child(Table("week", { columns, rows }));
  }

  render(cx) {
    const k = tokens();
    const now = Date.now();
    const rows = this.shown();
    const list = [];
    if (this.tab === "time") list.push(this.timeTable(k));
    else if (!this.agents.value) list.push(h_flex().h(120).justify_center().items_center().gap(8).text_color(k.mutedFg).child(Spinner({ size: 14 })).child("Loading the agents…"));
    else if (!rows.length) list.push(Empty({ icon: "bot", title: this.filter.value() ? "No agent matches" : "No agent here", description: "Agents show up once they report their status." }));
    else {
      let last = null;
      for (const r of rows) {
        if (this.tab === "all" && r.stage !== last) {
          const count = rows.filter((x) => x.stage === r.stage).length;
          list.push(div().pt(last ? 8 : 0).child(StageHeading(`head-${r.stage}`, STAGES[r.stage]?.label ?? r.stage, { color: stageColor(r.stage), count, pulse: r.stage === "needs_you", spin: r.stage === "working" })));
          last = r.stage;
        }
        list.push(this.card(k, r, now));
      }
    }
    const tabs = this.tabs();
    return v_flex()
      .id("recap")
      .track_focus(this.handle)
      .size_full()
      .bg(k.popover)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) {
          cx.stop_propagation();
          cx.notify();
        }
      })
      .child(
        h_flex()
          .px(20)
          .pt(16)
          .pb(12)
          .gap(16)
          .items_center()
          .child(
            h_flex()
              .id("recap-filter")
              .flex_1()
              .on_mouse_down("left", (_e, cx) => {
                this.filtering = true;
                this.filter.focus();
                cx.notify();
              })
              .child(SearchField(this.filter, { active: this.filtering, kbd: "/" })),
          )
          .when(Boolean(this.agents.value?.quota?.length), (e) => e.child(div().w(220).flex_none().child(QuotaBars(this.agents.value.quota, { id: "recap-quota" }))))
          .child(Button("recap-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: () => this.dismiss() })),
      )
      .child(
        h_flex()
          .px(20)
          .pb(12)
          .gap(4)
          .flex_wrap()
          .items_center()
          .children(
            tabs.map((t) => {
              const on = t.value === this.tab;
              const c = t.color ?? k.ring;
              return h_flex()
                .id(`tab-${t.value}`)
                .h(30)
                .px(10)
                .gap(7)
                .items_center()
                .rounded(15)
                .cursor_pointer()
                .text_size(TEXT.base)
                .font_medium()
                .text_color(on ? k.foreground : k.mutedFg)
                .when(on, (e) => e.bg(t.color ? alpha(c, 0.16) : k.selected))
                .when(!on, (e) => e.hover((s) => s.bg(k.accent).text_color(k.foreground)))
                .on_click((_e, cx) => {
                  this.tab = t.value;
                  this.keepSelection();
                  cx.notify();
                })
                .when(Boolean(t.icon), (e) => e.child(Icon(t.icon, { size: 14 })))
                .when(!t.icon, (e) => e.child(div().size(8).rounded(4).bg(t.color ?? k.ring)))
                .child(t.label)
                .when(t.count !== undefined, (e) => e.child(CountPill(t.count)));
            }),
          ),
      )
      .child(div().h(1).bg(k.border))
      .child(v_flex().id("recap-list").flex_1().min_h(0).px(20).py(16).gap(10).overflow_y_scrollbar().children(list))
      .child(
        KeyHints([
          ["↑↓", "move"],
          ["⏎", "go to it"],
          ["⇥", "reply"],
          ["1–9", "answer"],
          ["r", "recap"],
          ["s", "snooze"],
          ["f", "fork"],
          ["←→", "stage"],
          ["/", "filter"],
          ["esc", "close"],
        ]),
      );
  }
}
