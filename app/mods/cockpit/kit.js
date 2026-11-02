// The design kit's gallery: every component of ui.js on sample data, to check a change in both themes.
// `cockpit.kit` opens it as a window (sh app/sandbox.sh kit).

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { props, closeOverlay } from "herdr";
import {
  tokens, TEXT, Badge, Button, Kbd, Checkbox, Card, Tabs, Table, Skeleton, Empty, SectionHeader, ListRow, Icon, Avatar,
  Sparkline, LineChart, BarChart, Ring, Progress, StatTile, Section, HoverCard, Separator, CornerCount,
} from "./ui.js";

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

export default class Kit {
  init(_props, cx) {
    this.tab = props()?.tab ?? "components";
    this.open = true;
    this.sort = { key: "p95", desc: true };
    this.handle = cx.focus_handle();
  }

  key(e) {
    if (e.keystroke === "escape" && props()?.overlay) closeOverlay(props().overlay);
  }

  render() {
    const k = tokens();
    const rows = [
      { id: "api", app: "api", errors: 12, p95: 340, state: "Healthy" },
      { id: "web", app: "web", errors: 48, p95: 910, state: "Moved" },
      { id: "billing", app: "billing", errors: 3, p95: 120, state: "Healthy" },
    ].sort((a, b) => (this.sort.desc ? -1 : 1) * (a[this.sort.key] > b[this.sort.key] ? 1 : -1));
    const components = v_flex()
      .gap(20)
      .child(h_flex().gap(8).flex_wrap().child(Button("b1", { label: "Default", kbd: "⏎" })).child(Button("b2", { label: "Secondary", variant: "secondary" })).child(Button("b3", { label: "Outline", variant: "outline", icon: "plus" })).child(Button("b4", { label: "Ghost", variant: "ghost" })).child(Button("b5", { label: "Delete", variant: "destructive", icon: "trash" })).child(Button("b6", { icon: "more", variant: "outline", size: "icon", tooltip: "More" })))
      .child(h_flex().gap(8).flex_wrap().items_center().child(Badge("default", "default")).child(Badge("secondary")).child(Badge("outline", "outline")).child(Badge("failing", "destructive", { dot: k.destructive })).child(Badge("green", "success", { icon: "check" })).child(Badge("running", "warning")).child(Badge("info", "info")).child(Kbd(["⌘", "K"])).child(Checkbox("c1", { checked: true })).child(Checkbox("c2")).child(Avatar("Maya Ortiz")).child(Avatar("jonas")).child(h_flex().relative().size(30).items_center().justify_center().child(Icon("bell", { size: 18 })).child(CornerCount(3, { top: -2, right: -2 }))))
      .child(
        h_flex()
          .gap(12)
          .child(
            Card({
              title: "Card title",
              description: "A description under it, muted.",
              action: Button("card-act", { label: "Action", size: "sm", variant: "outline" }),
              children: [Progress(0.62), h_flex().gap(8).child(Skeleton({ w: 120 })).child(Skeleton({ w: 60 }))],
            }).flex_1(),
          )
          .child(
            Card({
              title: "Hover card",
              children: [HoverCard("hc", Badge("ENG-405  hover me", "outline"), v_flex().gap(4).child(div().font_semibold().child("ENG-405 · Search rate limit")).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("In review · api#2198 · CI green")))],
            }).flex_1(),
          ),
      )
      .child(
        Section("sec", {
          title: "Collapsible section",
          count: 3,
          open: this.open,
          onToggle: (_e, cx) => {
            this.open = !this.open;
            cx.notify();
          },
          children: [
            ListRow("r1", { selected: true, leading: Checkbox("rc1"), title: "A selected row", meta: Badge("today", "outline") }),
            ListRow("r2", { marked: true, leading: Checkbox("rc2"), title: "A marked row", meta: "meta" }),
            ListRow("r3", { muted: true, leading: Icon("plus"), title: "A muted row", meta: Kbd("a") }),
          ],
        }),
      )
      .child(
        Table("tbl", {
          columns: [{ key: "app", label: "App", sortable: true }, { key: "errors", label: "Errors", width: 100, align: "right", sortable: true }, { key: "p95", label: "p95 ms", width: 100, align: "right", sortable: true }, { key: "state", label: "Verdict", width: 120 }],
          rows: rows.map((r) => ({ id: r.id, cells: { app: r.app, errors: String(r.errors), p95: String(r.p95), state: Badge(r.state, r.state === "Moved" ? "destructive" : "success") } })),
          sort: this.sort,
          onSort: (key, cx) => {
            this.sort = { key, desc: this.sort.key === key ? !this.sort.desc : true };
            cx.notify();
          },
        }),
      );
    const charts = v_flex()
      .gap(16)
      .child(
        h_flex()
          .gap(12)
          .child(StatTile("s1", { label: "Open PRs", icon: "link", value: 7, delta: "+2", deltaGood: false, spark: [3, 4, 4, 6, 5, 7, 7] }))
          .child(StatTile("s2", { label: "CI green", icon: "check", value: "92%", delta: "+4%", deltaGood: true, spark: [80, 84, 83, 88, 90, 91, 92], sparkColor: k.success }))
          .child(StatTile("s3", { label: "p95 api", icon: "activity", value: "340 ms", delta: "−12%", deltaGood: true, hint: "since v2026.11.02", spark: [420, 400, 390, 380, 360, 350, 340], sparkColor: k.warning })),
      )
      .child(
        h_flex()
          .gap(12)
          .child(Card({ title: "Errors per hour", children: [LineChart("lc", { series: [{ label: "api", values: [4, 6, 5, 9, 12, 8, 7, 5] }, { label: "web", values: [2, 3, 8, 14, 10, 6, 4, 3] }], xLabels: ["08:00", "15:00"] })] }).flex_1())
          .child(Card({ title: "Deploys this week", children: [BarChart("bc", { bars: DAYS.map((d, i) => ({ label: d, value: [3, 5, 2, 6, 4, 0, 1][i] })) })] }).w(320)),
      )
      .child(
        h_flex()
          .gap(16)
          .items_center()
          .child(Ring(0.72, { size: 56 }))
          .child(Ring(0.35, { size: 56, color: k.warning }))
          .child(Ring(1, { size: 56, color: k.success, label: "done" }))
          .child(Separator({ vertical: true, mx: 8 }).h(40))
          .child(v_flex().gap(6).w(240).child(Progress(0.3)).child(Progress(0.7, { color: k.success })).child(Progress(0.95, { color: k.destructive })))
          .child(Sparkline([5, 3, 6, 2, 7, 4, 8], { w: 120, h: 32, area: true })),
      );
    return v_flex()
      .id("kit")
      .track_focus(this.handle)
      .size_full()
      .bg(k.background)
      .text_size(TEXT.base)
      .text_color(k.foreground)
      .on_key_down((e) => {
        if (e.key === "escape" && props()?.overlay) closeOverlay(props().overlay);
      })
      .child(
        h_flex()
          .h(56)
          .px(24)
          .gap(12)
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .child(div().flex_1().text_size(TEXT.lg).font_semibold().child("cockpit design kit"))
          .child(
            Tabs("kit-tabs", {
              items: [{ value: "components", label: "Components" }, { value: "charts", label: "Charts" }, { value: "empty", label: "States" }],
              value: this.tab,
              onChange: (v, cx) => {
                this.tab = v;
                cx.notify();
              },
            }),
          )
          .when(Boolean(props()?.overlay), (e) => e.child(Button("kit-close", { icon: "x", variant: "ghost", size: "icon-sm", onClick: () => closeOverlay(props().overlay) }))),
      )
      .child(
        v_flex()
          .id("kit-body")
          .flex_1()
          .min_h(0)
          .p(24)
          .overflow_y_scrollbar()
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .child(this.tab === "components" ? components : this.tab === "charts" ? charts : h_flex().gap(12).h(240).child(Empty({ icon: "inbox", title: "Inbox zero", description: "Nothing waits for you.", action: Button("e1", { label: "Refresh", variant: "outline", size: "sm" }) })).child(SectionHeader("A section header", { count: 4 }))),
      );
  }
}
