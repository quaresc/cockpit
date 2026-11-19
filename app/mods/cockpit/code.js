// A file beside the agent, to read and edit: syntax-highlighted, saved with ⌘S (refused when the file changed on disk
// since it was opened), reloaded when the agent changes it, Markdown with a live preview. "Ask the agent" sends a
// note about the file, or some of its lines, to the agent beside it, as a review comment. Ported from
// yama-lei/herdr-editor-layout's editor pane (Apache-2.0).
// Source: https://github.com/yama-lei/herdr-editor-layout (Apache-2.0), modified for cockpit; see THIRD_PARTY_NOTICES.md.
//
//   pane.split {direction: "right", pane, mod: "cockpit", props: {app: "code", file, line?, terminal?}}
//   the reading layout's side panel: {app: "code", panel: true}, showing the file shared `cockpit.code.panel` names

import { div } from "gpui-kit";
import { v_flex, h_flex, Editor, EditorState, InputState, TextView, h_resizable, resizable_panel } from "gpui-base";
import { exec, readTextFile, writeTextFile, watchFolder, theme, settings, props, run, panes, home, shared, setShared, appHome, focused, layout } from "herdr";
import * as hub from "./hub.js";
import { take } from "./person.js";
import { tokens, TEXT, Icon, Button, Kbd, TextField, Toaster } from "./ui.js";
import { layoutName, CHAT_SUBJECT } from "./screens.js";
import { lent, panelOpen } from "./commands.js";
import { findNode, openPanel, closePanel } from "./status.js";

const MD = /\.(md|markdown|mdx)$/i;
const LANG = { ts: "js", tsx: "js", mts: "js", cts: "js", jsx: "js", mjs: "js", cjs: "js", toml: "yaml", yml: "yaml", jsonc: "json", svelte: "html", vue: "html", htm: "html", md: "markdown", py: "python", rs: "rust", sh: "bash", zsh: "bash", ex: "elixir", exs: "elixir" };
const MAX_EDIT = 400_000;
const MD_NEXT = { split: "preview", preview: "edit", edit: "split" };

const tilde = (p) => {
  const h = home();
  return p?.startsWith(`${h}/`) ? `~${p.slice(h.length)}` : p;
};

/** Where the reading layout's side panel keeps the file it shows: {file, line, terminal, at, back}. */
export const CODE_PANEL = "cockpit.code.panel";

/**
 * Opens a file beside a terminal's pane (the focused one when none is given); in the reading layout, in the side panel
 * beside the chat, which its Close gives back to what it showed before.
 */
export async function openCode(file, { terminal, line } = {}) {
  const all = panes();
  // Beside the given agent, else beside the terminal of the tab in view (the editor itself may have the focus).
  const here = focused()?.tab ?? null;
  const beside = terminal ? all.find((p) => p.terminal === terminal) : (all.find((p) => p.focused && p.terminal) ?? all.find((p) => p.tab === here && p.terminal));
  if (layoutName() === "cockpit-chat") {
    const showing = findNode(layout(), "panel-body")?.active ?? null;
    const back = showing === "code" ? (shared(CODE_PANEL)?.back ?? null) : { open: panelOpen(), child: showing };
    setShared(CODE_PANEL, { file, line: line ?? null, terminal: terminal ?? beside?.terminal ?? null, at: Date.now(), back });
    openPanel("code");
    return;
  }
  const tab = beside?.tab ?? here;
  // One editor per tab: the file replaces the one it showed.
  const open = tab ? (shared(SHOWN) ?? {})[tab] : null;
  if (open && all.some((p) => p.pane === open)) run("pane.close", { pane: open });
  const args = { direction: "right", ...(beside ? { pane: beside.pane } : {}), mod: "cockpit", props: { app: "code", file, line, terminal: terminal ?? beside?.terminal, tab } };
  // Through herdr-app, which answers with the new pane: run() gives nothing back.
  const r = await exec("env", [`HERDR_APP_HOME=${appHome()}`, `${home()}/.local/bin/herdr-app`, "do", "pane.split", JSON.stringify(args)]).catch(() => null);
  const made = r?.code === 0 ? JSON.parse(r.stdout || "{}").pane : null;
  if (!made) run("pane.split", args);
  else if (tab) setShared(SHOWN, { ...(shared(SHOWN) ?? {}), [tab]: made });
}

const SHOWN = "cockpit.code.panes";

export default class Code {
  init(p) {
    const given = p ?? props() ?? {};
    this.file = given.file ?? null;
    this.terminal = given.terminal ?? null;
    this.line = given.line ?? null;
    this.tab = given.tab ?? null;
    // The side panel's copy shows whichever file was opened last, and loads the next one when that changes.
    this.panel = given.panel === true;
    this.shownAt = null;
    // Markdown opens as its preview where the room is a side panel's, else beside its source.
    this.mdMode = this.panel ? "preview" : "split";
    this.toasts = new Toaster();
    this.ask = InputState.new({ placeholder: "What should change? Start with 12-18: to point at lines" });
    this.loaded = false;
  }

  close() {
    this.code?.release();
    this.watching = null;
  }

  load(cx) {
    this.loaded = true;
    if (!this.file) return;
    cx.spawn(async (cx) => {
      const text = await readTextFile(this.file).catch(() => null);
      if (text === null) this.error = "This file can't be read as text, or is gone.";
      else if (text.includes("\0")) this.error = "A binary file.";
      else this.fill(text);
      this.watch(cx);
      cx.notify();
    });
  }

  fill(text) {
    const t = theme();
    const name = this.file.split("/").pop();
    const ext = name.includes(".") ? name.split(".").pop().toLowerCase() : "txt";
    this.code?.release();
    this.readonly = text.length > MAX_EDIT;
    this.original = text;
    this.dirty = false;
    this.error = null;
    this.code = EditorState.new({
      language: LANG[ext] ?? ext,
      readonly: this.readonly,
      value: this.readonly ? `${text.slice(0, MAX_EDIT)}\n… (file truncated, read-only)` : text,
      font_family: settings().font_family,
      font_size: 13,
      syntax: { keyword: t.ansi5, string: t.ansi2, comment: t.dim, function: t.ansi4, type: t.ansi3, number: t.ansi1, constant: t.ansi6, boolean: t.ansi6 },
    });
    this.code.on("change", (_e, cx) => {
      this.dirty = this.code.value() !== this.original;
      cx.notify();
    });
  }

  /** Reloads when the file changes on disk and has no unsaved edits: the agent's edits show as it makes them. */
  watch(cx) {
    const key = this.file;
    if (this.watching === key) return;
    this.watching = key;
    const dir = this.file.replace(/\/[^/]*$/, "");
    cx.spawn(async (cx) => {
      try {
        while (this.watching === key) {
          const changed = await watchFolder(dir);
          // macOS reports /tmp paths as /private/tmp: compare without it.
          const bare = (p) => p.replace(/^\/private(?=\/)/, "");
          if (this.watching !== key || !changed.some((p) => bare(p) === bare(this.file)) || this.saving) continue;
          const text = await readTextFile(this.file).catch(() => null);
          if (text === null || text === this.original) continue;
          if (this.dirty) {
            this.toasts.push(cx, "Changed on disk", { kind: "warning", description: "Save would be refused: reload to see the new version.", action: { label: "Reload", onClick: (_e, cx) => (this.fill(text), cx.notify()) } });
          } else this.fill(text);
          cx.notify();
        }
      } catch {
        if (this.watching === key) this.watching = null;
      }
    });
  }

  save(cx) {
    if (!this.code || this.readonly || !this.dirty || this.saving) return;
    const text = this.code.value();
    this.saving = true;
    cx.spawn(async (cx) => {
      try {
        await writeTextFile(this.file, text, this.original);
        this.original = text;
        this.dirty = false;
        this.toasts.push(cx, "Saved", { kind: "success" });
      } catch (e) {
        this.toasts.push(cx, "Not saved", { kind: "error", description: `${e?.message ?? e}. The file changed on disk: reload it first.` });
      } finally {
        this.saving = false;
        cx.notify();
      }
    });
  }

  /** "12-18: text" points at lines; the note reaches the agent as a one-comment review of this file. */
  sendAsk(cx) {
    const raw = this.ask.value().trim();
    if (!raw) return;
    const m = /^(\d+)(?:\s*-\s*(\d+))?\s*:\s*([\s\S]+)$/.exec(raw);
    const [from, to, text] = m ? [Number(m[1]), Number(m[2] ?? m[1]), m[3]] : [this.line ?? 1, null, raw];
    const lines = (this.code?.value() ?? "").split("\n");
    const code = m ? lines.slice(from - 1, to).join("\n") : "";
    cx.spawn(async (cx) => {
      const root = await exec("git", ["-C", this.file.replace(/\/[^/]*$/, ""), "rev-parse", "--show-toplevel"]).catch(() => null);
      const top = root?.code === 0 ? root.stdout.trim() : "";
      const path = top && this.file.startsWith(`${top}/`) ? this.file.slice(top.length + 1) : this.file;
      const to = await this.askTarget();
      const review = { scope: "file", repo: top, comments: [{ path, line: from, code, text }], ...(to.name ? { to: to.name } : {}) };
      const r = await hub.act("review", "send", [to.handle, review]).catch((e) => ({ error: String(e?.message ?? e) }));
      if (r?.error) this.toasts.push(cx, `Not sent${to.name ? ` to ${to.name}` : ""}`, { kind: "error", description: String(r.error).replace(/^[a-z_]+: /, "") });
      else {
        this.toasts.push(cx, `Sent to ${to.name ?? "the agent"}`, { kind: "success" });
        this.ask.set_value("");
        this.asking = false;
      }
      cx.notify();
    });
  }

  /**
   * Where "Ask the agent" goes, as {handle, name}: the chat's subject in Cockpit Chat, a driven session included, else
   * the agent whose terminal the file opened beside.
   */
  async askTarget() {
    const handle = (layoutName() === "cockpit-chat" ? shared(CHAT_SUBJECT) : null) ?? this.terminal ?? "";
    const bare = String(handle).replace(/^app:/, "");
    const rows = lent()?.agents ?? (await hub.view("agents", { frontend: "app" }).catch(() => null))?.agents ?? [];
    const row = rows.find((r) => r.terminal === handle || r.address === bare || r.terminal === `app:${bare}`);
    return { handle, name: row?.title || row?.tab?.name || null };
  }

  key(e, cx) {
    const k = e.keystroke;
    if (k === "cmd-s" || k === "ctrl-s") return this.save(cx), true;
    if (k === "cmd-shift-v") return (this.mdMode = MD_NEXT[this.mdMode]), cx.notify(), true;
    if (this.asking && k === "enter") return this.sendAsk(cx), true;
    if (this.asking && k === "escape") return (this.asking = false), cx.notify(), true;
    return false;
  }

  /** `code:save` from a menu or the palette, by its key: a window command saves nothing. */
  command(c, cx, key = null) {
    if (c === "code:save" && take(c, key)) this.save(cx);
    cx.notify();
  }

  closePane() {
    if (this.panel) {
      // The panel goes back to what it showed before the file, or closes when it was closed then.
      const back = shared(CODE_PANEL)?.back;
      if (back?.open && back.child && back.child !== "code") openPanel(back.child);
      else closePanel();
      return;
    }
    const me = this.tab ? (shared(SHOWN) ?? {})[this.tab] : null;
    if (me) run("pane.close", { pane: me });
  }

  /** The side panel's copy takes up the file opened last, once. */
  follow() {
    const spec = shared(CODE_PANEL);
    if (!spec?.file || spec.at === this.shownAt) return;
    this.shownAt = spec.at;
    this.file = spec.file;
    this.line = spec.line ?? null;
    this.terminal = spec.terminal ?? null;
    this.code?.release();
    this.code = null;
    this.error = null;
    this.dirty = false;
    this.asking = false;
    this.watching = null;
    this.loaded = false;
  }

  render(cx) {
    if (this.panel) this.follow();
    if (!this.loaded) this.load(cx);
    const k = tokens();
    const t = theme();
    const name = this.file ? this.file.split("/").pop() : "No file";
    const md = Boolean(this.file && MD.test(this.file) && this.code);
    const narrow = (this.width ?? Infinity) < 520;
    const header = h_flex()
      .h(36)
      .flex_none()
      .pl(12)
      .pr(6)
      .gap(8)
      .items_center()
      .border_b_1()
      .border_color(k.border)
      .child(Icon("file-text", { size: 14, color: k.mutedFg }))
      .child(div().min_w(0).max_w(260).truncate().font_semibold().child(name))
      .when(Boolean(this.dirty), (e) => e.child(div().size(8).rounded_full().bg(k.foreground).tooltip("Unsaved changes")))
      .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(`${tilde(this.file ?? "")}${this.readonly ? " · read-only" : ""}`))
      .when(Boolean(this.dirty), (e) => e.child(Button("code-save", { label: "Save", icon: "check", size: "sm", kbd: narrow ? undefined : "⌘S", tooltip: narrow ? "Save  ⌘S" : undefined, onClick: (_e, cx) => this.save(cx) })))
      .when(md, (e) => e.child(Button("code-md", { icon: this.mdMode === "preview" ? "file-text" : "eye", variant: "ghost", size: "icon-sm", tooltip: `Preview: ${this.mdMode}  ⌘⇧V`, onClick: (_e, cx) => ((this.mdMode = MD_NEXT[this.mdMode]), cx.notify()) })))
      // Narrow (a side panel), the buttons keep their icons and leave their words to the tooltips.
      .child(Button("code-ask", { label: "Ask the agent", icon: "message-square", variant: this.asking ? "secondary" : "outline", size: narrow ? "icon-sm" : "sm", tooltip: narrow ? "Ask the agent: a note about this file, or lines of it" : "A note about this file, or lines of it, sent to the agent", onClick: (_e, cx) => ((this.asking = !this.asking), this.asking && this.ask.focus(), cx.notify()) }))
      .child(Button("code-vscode", { icon: "external", variant: "ghost", size: "icon-sm", tooltip: "Open in VS Code", onClick: () => exec("code", ["-g", `${this.file}${this.line ? `:${this.line}` : ""}`]).catch(() => {}) }))
      .child(Button("code-close", { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close", onClick: () => this.closePane() }));
    let body;
    if (!this.file) body = div().p(16).text_color(k.dim).child(this.panel ? "No file open yet." : "No file given.");
    else if (this.error) body = div().p(16).text_color(k.dim).child(this.error);
    else if (!this.code) body = div().p(16).text_color(k.dim).child("Loading…");
    else if (md && this.mdMode !== "edit") {
      const preview = div().id("code-md").size_full().min_h(0).overflow_y_scrollbar().px(24).py(16).child(TextView.markdown("code-md-text", this.code.value()).selectable(true));
      body =
        this.mdMode === "preview"
          ? div().flex_1().min_h(0).child(preview)
          : h_resizable("code-md-split")
              .flex_1()
              .min_h(0)
              .child(resizable_panel().size_range(160).child(Editor.new(this.code).size_full()))
              .child(resizable_panel().size_range(160).child(div().size_full().border_l_1().border_color(k.border).child(preview)));
    } else body = Editor.new(this.code).flex_1().min_h(0);
    const asking = this.asking
      ? h_flex()
          .flex_none()
          .p(8)
          .gap(8)
          .items_center()
          .border_t_1()
          .border_color(k.border)
          .child(div().flex_1().child(TextField(this.ask, { leading: Icon("message-square", { size: 14, color: k.mutedFg }) })))
          .child(Button("code-send", { label: "Send", icon: "send", size: "sm", kbd: "⏎", onClick: (_e, cx) => this.sendAsk(cx) }))
      : null;
    let root = v_flex()
      .id("cockpit-code")
      .on_size_change((size, cx) => {
        const was = (this.width ?? Infinity) < 520;
        this.width = size.width;
        if (was !== size.width < 520) cx.notify();
      })
      .size_full()
      .min_h(0)
      .relative()
      .bg(t.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(header)
      .child(body);
    if (asking) root = root.child(asking);
    const toasts = this.toasts.render();
    return toasts ? root.child(toasts) : root;
  }
}
