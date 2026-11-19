// The browser's one window: a toolbar on top, the active tab's page under it, DevTools beside or under the page when
// open. Docked, it is a frameless panel that takes the keyboard without making its app the active one.

import path from "node:path";
import { fileURLToPath } from "node:url";
import { BaseWindow, WebContentsView, nativeTheme } from "electron";
import { TOOLBAR, areas, pageBox } from "./layout.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const BACKDROP = { light: "#f3f2ef", dark: "#1c1c1e" };

export class Window {
  /** `docked` makes the frameless panel; `onUi` receives what the toolbar sends. */
  constructor({ docked, onUi, onKey, onClosed, onFocus }) {
    this.docked = docked;
    this.win = new BaseWindow({
      type: docked ? "panel" : undefined,
      frame: !docked,
      show: false,
      hasShadow: !docked,
      acceptFirstMouse: true,
      width: 1100,
      height: 800,
      minWidth: 320,
      minHeight: 240,
      title: "Cockpit Browser",
      backgroundColor: this.backdrop(),
      roundedCorners: !docked,
    });
    // A panel is made floating: at the normal level it stays one step above the host and under every other app.
    if (docked) this.win.setAlwaysOnTop(false);
    this.toolbar = new WebContentsView({
      webPreferences: { preload: path.join(here, "..", "ui", "preload.cjs"), sandbox: true, contextIsolation: true, nodeIntegration: false, partition: "cockpit-internal" },
    });
    this.toolbar.setBackgroundColor("#00000000");
    this.toolbar.webContents.loadFile(path.join(here, "..", "ui", "toolbar.html"));
    this.toolbar.webContents.ipc.on("ui", (_event, message) => onUi(message));
    this.toolbar.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
    this.toolbar.webContents.on("will-navigate", (event) => event.preventDefault());
    this.toolbar.webContents.on("before-input-event", (event, input) => onKey?.(event, input));
    this.win.contentView.addChildView(this.toolbar);
    this.page = null;
    this.devtools = null;
    this.devtoolsSide = null;
    this.viewport = null;
    this.toolbarOpen = false;
    this.win.on("resize", () => this.layout());
    this.win.on("close", (event) => {
      if (this.quitting) return;
      event.preventDefault();
      this.win.hide();
      onClosed?.();
    });
    this.win.on("focus", () => onFocus?.());
    nativeTheme.on("updated", () => this.win.setBackgroundColor(this.backdrop()));
  }

  backdrop() {
    return nativeTheme.shouldUseDarkColors ? BACKDROP.dark : BACKDROP.light;
  }

  /** Lays the views out for the window's size; the toolbar stays on top of the page when its menus are open. */
  layout() {
    const [width, height] = this.win.getContentSize();
    const a = areas({ width, height }, { devtools: this.devtools ? this.devtoolsSide : null, toolbarOpen: this.toolbarOpen });
    this.toolbar.setBounds(a.toolbar);
    if (this.page) {
      const box = pageBox(a.page, this.viewport);
      this.page.setBounds({ x: box.x, y: box.y, width: box.width, height: box.height });
      this.pageScale = box.scale;
    }
    if (this.devtools && a.devtools) this.devtools.setBounds(a.devtools);
    return a;
  }

  /** Shows a tab's page under the toolbar, in place of the one before; null shows none. */
  setPage(view, viewport = null) {
    if (this.page && this.page !== view) this.win.contentView.removeChildView(this.page);
    this.page = view;
    this.viewport = viewport;
    if (view) {
      this.win.contentView.addChildView(view);
      // The toolbar is added last so its open menus draw over the page.
      this.win.contentView.addChildView(this.toolbar);
    }
    this.layout();
  }

  setDevtools(view, side = "bottom") {
    if (this.devtools && this.devtools !== view) this.win.contentView.removeChildView(this.devtools);
    this.devtools = view;
    this.devtoolsSide = view ? side : null;
    if (view) {
      this.win.contentView.addChildView(view);
      this.win.contentView.addChildView(this.toolbar);
    }
    this.layout();
  }

  setToolbarOpen(open) {
    if (this.toolbarOpen === open) return;
    this.toolbarOpen = open;
    this.layout();
  }

  send(state) {
    if (!this.toolbar.webContents.isDestroyed()) this.toolbar.webContents.send("state", state);
  }

  /** Puts the window on the slot and one step above the host window. */
  place(rect, host) {
    this.win.setBounds(rect);
    this.layout();
    if (!this.win.isVisible()) this.win.showInactive();
    this.raise(host);
  }

  raise(host) {
    if (host?.id) this.win.moveAbove(`window:${host.id}:0`);
  }

  hide() {
    if (this.win.isVisible()) this.win.hide();
  }

  show() {
    this.win.show();
  }

  focusPage() {
    this.win.focus();
    this.page?.webContents.focus();
  }

  focusToolbar() {
    this.win.focus();
    this.toolbar.webContents.focus();
  }

  destroy() {
    this.quitting = true;
    this.win.destroy();
  }
}
