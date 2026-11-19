// Needs you: every agent waiting on an answer, its question answerable where it is listed, then the routine runs that
// need a look. What the Home board's NEXT MOVE shows, as a screen of its own for the reading layout.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import { run } from "herdr";
import * as hub from "./hub.js";
import { keyed } from "./person.js";
import { AgentsModel, QuestionCard, DismissButton, dismissable, stageColor } from "./agentsui.js";
import { rowsOf, goToAgent, needing } from "./fleet.js";
import { show } from "./screens.js";
import { tokens, TEXT, RADIUS, Icon, Empty, Spinner, Button, Overline, Toaster, HubDown, dur, AgentLogo } from "./ui.js";

const READ_W = 800;

/** Routines in view in this layout, that run or routine selected: the view takes the target once it has its model. */
export function openRoutine(key) {
  const navigation = (globalThis.cockpitRemoteFocus ??= { targets: new Map(), listeners: new Set() });
  const target = { app: "routines", key };
  navigation.targets.set("routines", target);
  run("cockpit.go:routines");
  for (const listener of navigation.listeners) if (listener(target)) break;
}

export default class Needs {
  init(_props, cx) {
    this.model = new AgentsModel("needs");
    this.model.follow(cx);
    // A dismissal's Undo shows on the screen's own toast.
    this.toasts = this.model.toasts = new Toaster();
    this.followRoutines(cx);
  }

  close() {
    this.model.stop();
    this.routinesSub?.stop();
  }

  command(c, cx, key = null) {
    this.model.command(c, cx, key);
    cx.notify();
  }

  /** The agents asking, then those stopped with an error, the one waiting longest first (the agents view sorts them). */
  waiting() {
    return needing(rowsOf(this.model.value));
  }

  header(row, k, failed = false) {
    const since = row.waiting_since ?? row.since;
    return h_flex()
      .w_full()
      .gap(8)
      .items_center()
      .child(div().size(8).flex_none().rounded(4).bg(stageColor(failed ? "failed" : "needs_you")))
      .child(AgentLogo(row, { size: 14, tooltip: row.agent_name, id: `needs-${row.terminal}-logo` }))
      .child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(row.title ?? row.terminal))
      .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(failed ? "stopped with an error" : row.question ? "asks you" : row.waits?.kind === "limit" ? "is at its limit" : "waits for you"))
      .child(div().flex_1())
      .when(Boolean(since), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(dur(Date.now() - since))))
      .child(
        div()
          .id(`needs-open-${row.terminal}`)
          .size(24)
          .flex()
          .items_center()
          .justify_center()
          .rounded(RADIUS.md)
          .cursor_pointer()
          .text_color(k.dim)
          .hover((s) => s.bg(k.accent).text_color(k.foreground))
          .tooltip("Read the conversation")
          .on_click((_e, cx) => {
            // A session cockpit drives has no terminal to focus: the chat opens on it.
            if (row.driven) run(keyed(`cockpit.via:chat|chat:open:${row.terminal}`));
            else {
              goToAgent({ address: row.address });
              show("chat");
            }
            cx.notify();
          })
          .child(Icon("message-square", { size: 13 })),
      );
  }

  /** An agent stopped with an error: what it said, and the way to its conversation, where it is told what to do next. */
  failedCard(row, k) {
    return v_flex()
      .id(`needs-failed-${row.terminal}`)
      .w_full()
      .p(14)
      .gap(6)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .on_mouse_down("right", () => this.model.menu(row))
      .child(this.header(row, k, true))
      .child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.foreground).child(row.error || "It stopped on an error: read its conversation to see why."))
      .when(dismissable(row), (e) => e.child(h_flex().w_full().child(div().flex_1()).child(DismissButton(`needs-failed-${row.terminal}`, row, this.model))));
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Routines: the runs that failed or were missed and the broken routines, from the routines view's `look`

  followRoutines(cx) {
    this.routines = { state: "loading", look: [] };
    this.routinesSub = hub.subscribe(
      cx,
      { views: ["routines"] },
      (m, cx) => {
        if (m.view !== "routines") return;
        this.routines = { state: "ready", look: m.value?.look ?? [] };
        cx.notify();
      },
      (state, cx) => {
        if (!String(state).startsWith("error")) return;
        this.routines = { ...this.routines, state: "error", error: String(state).slice(7) };
        cx.notify();
      },
    );
  }

  routineCard(x, k) {
    // A remote desk reads only: the act is refused there, and says so.
    const read = (cx) =>
      cx.spawn(async (cx) => {
        try {
          const answer = await hub.act("routines", "read", [x.slug, x.id]);
          if (answer?.error) this.toasts.push(cx, answer.error, { kind: "error" });
        } catch (e) {
          this.toasts.push(cx, "Could not mark it read", { kind: "error", description: String(e?.message ?? e) });
        }
        cx.notify();
      });
    const icon = { missed: "skip-forward", broken: "triangle-alert" }[x.word] ?? "circle-x";
    return v_flex()
      .id(`needs-routine-${x.key}`)
      .w_full()
      .max_w(READ_W)
      .p(14)
      .gap(6)
      .rounded(RADIUS.lg)
      .border_1()
      .border_color(k.border)
      .cursor_pointer()
      .hover((s) => s.bg(k.accent))
      .tooltip(x.id ? "Open the run in Routines" : "Open the routine in Routines")
      .on_click(() => openRoutine(x.key))
      .child(
        h_flex()
          .w_full()
          .gap(8)
          .items_center()
          .child(Icon(icon, { size: 14, color: k.mutedFg }))
          .child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(x.name))
          .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(x.word === "broken" ? "cannot run" : x.word))
          .child(div().flex_1())
          .when(Boolean(x.ago), (e) => e.child(div().text_size(TEXT.xs).text_color(k.dim).child(x.ago)))
          .when(Boolean(x.id), (e) => e.child(Button(`needs-read-${x.key}`, { icon: "mail-check", variant: "ghost", size: "icon-sm", tooltip: "Mark read", onClick: (_e, cx) => (cx.stop_propagation(), read(cx)) }))),
      )
      .child(div().whitespace_normal().text_size(TEXT.sm).text_color(k.foreground).child(x.reason))
      .when(Boolean(x.summary) && x.summary !== x.reason, (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(x.summary)));
  }

  /** The Routines section below the agents: nothing while it loads or when no run needs a look. */
  routinesSection(k) {
    const r = this.routines;
    if (r.state === "error") return div().w_full().max_w(READ_W).text_size(TEXT.sm).text_color(k.mutedFg).child(`Routines did not load: ${r.error || "the hub did not answer"}`);
    if (!r.look.length) return null;
    return v_flex()
      .w_full()
      .max_w(READ_W)
      .gap(8)
      .pt(8)
      .child(Overline(`Routines · ${r.look.length}`))
      .children(r.look.map((x) => this.routineCard(x, k)));
  }

  render(_cx) {
    const k = tokens();
    if (String(this.model.state ?? "").startsWith("error")) this.lastHubError = this.model.state;
    const down = this.model.down ? HubDown("needs-hub-down", this, String(this.lastHubError ?? this.model.lastError ?? "").replace(/^error: /, ""), k) : null;
    if (!this.model.value) return v_flex().size_full().px(24).bg(k.canvas).items_center().justify_center().child(down ?? Spinner({ size: 18, color: k.mutedFg }));
    const rows = this.waiting();
    const routines = this.routinesSection(k);
    const body =
      rows.length === 0 && !routines
        ? v_flex().flex_1().items_center().justify_center().child(Empty({ icon: "check", title: "Nobody is waiting", description: "No agent asks you or stopped with an error, and no routine run needs a look." }))
        : v_flex()
            .id("needs-list")
            .flex_1()
            .min_h(0)
            .px(24)
            .py(16)
            .gap(12)
            .items_center()
            .overflow_y_scrollbar()
            .children(rows.map((r) => div().w_full().max_w(READ_W).child(r.stage === "needs_you" || r.question ? QuestionCard(`needs-${r.terminal}`, r, this.model, { wide: true, header: this.header(r, k) }) : this.failedCard(r, k))))
            .when(Boolean(routines), (e) => e.child(routines));
    const toasts = this.toasts.render();
    return v_flex()
      .id("needs")
      .size_full()
      .relative()
      .bg(k.canvas)
      .text_color(k.foreground)
      .when(Boolean(down), (e) => e.child(h_flex().w_full().px(24).pt(12).justify_center().child(div().w_full().max_w(READ_W).child(down))))
      .child(body)
      .when(Boolean(toasts), (e) => e.child(toasts));
  }
}
