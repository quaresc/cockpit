// An app whose Herdr view is not built yet: says so, in the kit's empty state.

import { v_flex } from "gpui-base";
import { tokens, Empty, PageHeader } from "./ui.js";

const NAMES = { inbox: ["Inbox", "inbox"], runbook: ["Runbook", "book"], meetings: ["Meetings", "calendar"], monitor: ["Monitor", "activity"], routines: ["Routines", "clock"], mail: ["Mail", "mail"] };

export default class Placeholder {
  constructor(app) {
    this.app = app;
  }

  init() {}

  render() {
    const k = tokens();
    const [title, icon] = NAMES[this.app] ?? [this.app || "cockpit", "layout"];
    return v_flex()
      .size_full()
      .bg(k.background)
      .child(PageHeader({ icon, title }))
      .child(Empty({ icon, title: `${title} is on its way`, description: "This app is not drawn in the Herdr app yet." }));
  }
}
