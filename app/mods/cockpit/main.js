// cockpit in the Herdr app: one mod, one view per place it shows (`props.app`), so the views share the hub client
// (hub.js) and the design kit (ui.js). A view is a class with init(props, cx), render(cx), and optionally
// command(name, cx) for keys.json's `cockpit.<command>` and close(cx); add it to VIEWS.

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
import Monitor, { MonitorWindow } from "./monitor.js";
import Routines from "./routines.js";
import Mail from "./mail.js";
import Update from "./update.js";
import Chat from "./chat.js";
import ChatTop from "./chat-top.js";
import SidePeek from "./side-peek.js";
import PanelPeek from "./panel-peek.js";
import Needs from "./needs.js";

const VIEWS = { code: Code, reader: Reader, inbox: Inbox, monitor: Monitor, "monitor-window": MonitorWindow, routines: Routines, mail: Mail, show: Show, review: Review, recap: Recap, home: Home, status: Status, bell: Bell, activity: Activity, "agent-head": AgentHead, queue: Queue, runbook: Runbook, meetings: Meetings, todo: Todo, sidebar: Sidebar, palette: Palette, menu: Menu, kit: Kit, pane: PaneHeader, update: Update, chat: Chat, "chat-top": ChatTop, "side-peek": SidePeek, "panel-peek": PanelPeek, needs: Needs };

export default class Cockpit extends View {
  init(p, cx) {
    const app = props()?.app ?? "";
    const Impl = VIEWS[app] ?? Placeholder;
    this.view = new Impl(app);
    this.view.init(p ?? props() ?? {}, cx);
    cx.spawn(async (cx) => {
      for (;;) {
        const command = await nextCommand();
        // `cockpit.key:<keystroke>` presses a key in the shown views, for tests driving the window from a shell.
        if (command.startsWith("key:") && this.view.key) {
          const keystroke = command.slice(4);
          this.view.key({ keystroke, key: keystroke.split("-").pop(), modifiers: {} }, cx);
          cx.notify();
        } else this.view.command?.(command, cx);
      }
    });
  }

  render(cx) {
    return this.view.render(cx);
  }

  async close(cx) {
    await this.view.close?.(cx);
  }
}
