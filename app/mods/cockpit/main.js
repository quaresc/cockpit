// cockpit in the Herdr app: one mod, one view per place it shows (`props.app`), so the views share the hub client
// (hub.js) and the design kit (ui.js). A view is a class with init(props, cx), render(cx), and optionally
// command(name, cx, key) for keys.json's `cockpit.<command>` (`key` names a person's pick, person.js) and close(cx);
// add it to VIEWS.

import { View } from "gpui-kit";
import { props, nextCommand } from "herdr";
import Todo from "./todo.js";
import Inbox from "./inbox.js";
import Sidebar from "./sidebar.js";
import Palette from "./palette.js";
import Menu from "./menu.js";
import Kit from "./kit.js";
import Reader from "./reader.js";
import Code from "./code.js";
import Placeholder from "./placeholder.js";
import PaneHeader from "./pane.js";
import Runbook from "./runbook.js";
import Meetings from "./meetings.js";
import Home from "./home.js";
import Status from "./status.js";
import Bell from "./bell.js";
import Activity from "./activity.js";
import AgentHead from "./agent-head.js";
import Queue from "./queue.js";
import Recap from "./recap.js";
import Show from "./show.js";
import Review from "./review.js";
import Files from "./files.js";
import Changes from "./changes.js";
import Monitor, { MonitorWindow } from "./monitor.js";
import Routines from "./routines.js";
import Mail from "./mail.js";
import Update, { askAfterUpdate } from "./update.js";
import Chat from "./chat.js";
import ChatTop from "./chat-top.js";
import SidePeek from "./side-peek.js";
import PanelPeek from "./panel-peek.js";
import Needs from "./needs.js";
import Start from "./start.js";
import { enter, leave, front } from "./front.js";
import Settings from "./doctor.js";
import Usage from "./usage.js";
import Web from "./web.js";
import { unkey, testKeys } from "./person.js";

const VIEWS = { code: Code, reader: Reader, inbox: Inbox, monitor: Monitor, "monitor-window": MonitorWindow, routines: Routines, mail: Mail, show: Show, review: Review, files: Files, changes: Changes, recap: Recap, home: Home, status: Status, bell: Bell, activity: Activity, "agent-head": AgentHead, queue: Queue, runbook: Runbook, meetings: Meetings, todo: Todo, sidebar: Sidebar, palette: Palette, menu: Menu, kit: Kit, pane: PaneHeader, update: Update, chat: Chat, "chat-top": ChatTop, "side-peek": SidePeek, "panel-peek": PanelPeek, needs: Needs, settings: Settings, diagnostics: Settings, usage: Usage, start: Start, web: Web };

export default class Cockpit extends View {
  init(p, cx) {
    const given = p ?? props() ?? {};
    const app = given.app ?? props()?.app ?? "";
    const Impl = VIEWS[app] ?? Placeholder;
    this.view = new Impl(app);
    this.view.init(given, cx);
    // A popup the mod opens says so with `popup: true`; `overlay` is the id Herdr gives any copy it opens over the window.
    this.copy = { app, overlay: Boolean(given.popup || given.overlay), scoped: Boolean(given.scope), keys: Boolean(this.view.key), at: Date.now(), beat: Date.now() };
    enter(this.copy);
    // After an update, the window's first copy asks about what it brought, once.
    if (!this.copy.overlay) askAfterUpdate(cx);
    // A copy beats while it lives, so one a layout switch dropped without closing it stops counting as in front. The
    // same timer is the copy's only clock: its view's `tick(cx, now)` runs on it each second.
    cx.timer.every(1000, (cx) => {
      this.copy.beat = Date.now();
      try {
        this.view.tick?.(cx, this.copy.beat);
      } catch {}
    });
    cx.spawn(async (cx) => {
      for (;;) {
        // A menu, the palette or a click sends its command with a one-time key (person.js): the view gets it apart, and
        // a command without one only navigates.
        const { command, key } = unkey(await nextCommand());
        // `cockpit.key:<keystroke>` presses a key in the view in front, for tests driving the window from a shell, and
        // only in a sandbox started with COCKPIT_TEST_KEYS=1 in its own environment: elsewhere a key from outside
        // answers and confirms nothing. It is marked as pressed from outside, and the chat's digits ignore it.
        if (command.startsWith("key:")) {
          if (!this.view.key || !front().includes(this.copy) || !(await testKeys())) continue;
          const keystroke = command.slice(4);
          this.view.key({ keystroke, key: keystroke.split("-").pop(), modifiers: {}, injected: true }, cx);
          cx.notify();
        } else if (command.startsWith("chat:") && !front().includes(this.copy)) continue;
        else this.view.command?.(command, cx, key);
      }
    });
  }

  render(cx) {
    return this.view.render(cx);
  }

  async close(cx) {
    leave(this.copy);
    await this.view.close?.(cx);
  }
}
