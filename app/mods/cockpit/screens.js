// Which screen the window shows, in whichever cockpit layout is on: `cockpit` stacks its screens under `center`,
// `cockpit-chat` under `screen`. `work` is where you deal with an agent: its panes in one, its conversation in the other.

import { run, shared, setShared } from "herdr";

const LAYOUTS = {
  cockpit: { stack: "center", work: "work", first: "home" },
  // The reading layout puts the conversation and the panes in a stack of their own, beside the side panel.
  "cockpit-chat": { stack: "screen", work: "chat", first: "chat", inner: { stack: "view", screen: "work", children: ["chat", "tab"] } },
};

/** The shown layout's screens; the cockpit layout's when it is some other layout. */
export function screens() {
  return LAYOUTS[shared("layout")] ?? LAYOUTS.cockpit;
}

/** Shows a screen of the shown layout and gives that app the keyboard. */
export function show(child) {
  const s = screens();
  if (s.inner?.children.includes(child)) {
    run("layout.select", { stack: s.stack, child: s.inner.screen });
    run("layout.select", { stack: s.inner.stack, child });
  } else {
    run("layout.select", { stack: s.stack, child });
  }
  setShared("cockpit.app", child);
  if (child !== s.work) run(`cockpit.show:${child}`);
}

/** Whether the reading layout's sidebar is open; it remembers across restarts. */
export const DOCKED = "cockpit.chat.sidebar";

/** Opens or closes the sidebar. */
export function toggleSidebar() {
  showSidebar(shared(DOCKED) === false);
}

/** Puts the sidebar in a known state, so the flag and the layout never disagree. */
export function showSidebar(on) {
  setShared(DOCKED, on === true);
  run(on ? "layout.show" : "layout.hide", { id: "sidebar" });
}

/** Shows where you deal with the agent: its panes in the cockpit layout, its conversation in the reading one. */
export function showWork() {
  show(screens().work);
}
