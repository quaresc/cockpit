// Design mode: a person points at elements of a page and hands them to the agent whose browser it is. "Pick" takes one
// element and a line about it, sent at once; "Annotate" collects notes (a comment, an intent, a priority) on several,
// sent together. Everything goes to the session the browser belongs to: there is nobody else to choose. The page-side
// picker and the Markdown are Orca's, ported in ./grab; this drives them for one tab at a time.

import fs from "node:fs";
import path from "node:path";
import { randomBytes } from "node:crypto";
import { clipboard, dialog, nativeImage } from "electron";
import * as paths from "./paths.js";
import {
  grabScript,
  readGrabSelection,
  readGrabFailure,
  captureSelectionScreenshot,
  formatAnnotationsAsMarkdown,
  formatGrabPayloadAsText,
  grabKeyAction,
  buildAnnotationViewportBridgeScript,
  isAnnotationPageFull,
  ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID,
  ANNOTATION_INTENTS,
  ANNOTATION_PRIORITIES,
  DEFAULT_ANNOTATION_PRIORITY,
  GRAB_TIMEOUT_MS,
  GRAB_BUDGET,
} from "./grab/index.js";
import { noteOf, markersOf, labelOf, summaryOf } from "./notes.js";

export class Design {
  constructor(shell) {
    this.shell = shell;
    this.token = randomBytes(16).toString("hex");
    this.details = false;
    this.draft = null;
    this.serial = 0;
  }

  get armed() {
    return this.shell.grab;
  }

  page(tab) {
    return this.shell.live.get(tab)?.view.webContents || null;
  }

  /** Starts a mode on a tab, or ends it when it is the one running. */
  toggle(owner, tab, mode) {
    if (!tab) return;
    if (this.armed?.tab === tab && this.armed.mode === mode) return this.stop();
    return this.start(owner, tab, mode);
  }

  /** Arms the picker in a tab's page and waits for the person's click. */
  async start(owner, tab, mode = "copy") {
    const found = this.shell.tabOf(owner, tab);
    const record = found && this.shell.ensureView(owner, found.id);
    if (!record) throw new Error("no such tab");
    // An element is picked in a page the person sees: a window of its own turns to that browser, a docked one follows its slot.
    if (this.shell.current !== owner) {
      if (this.shell.slot) throw new Error("this session's browser is not the one in view");
      this.shell.show(owner);
    }
    if (this.shell.browser(owner).active !== record.id) this.shell.activate(owner, record.id);
    await this.stop();
    if (mode === "annotate" && isAnnotationPageFull((this.shell.notes.get(owner) || []).filter((n) => n.browserPageId === record.id).length)) {
      throw new Error(`a page takes ${GRAB_BUDGET.annotationsMaxPerPage ?? 20} notes: send or clear some first`);
    }
    this.shell.grab = { owner, tab: record.id, mode, run: ++this.serial };
    this.shell.push();
    this.wait(record, this.shell.grab);
    return { armed: true, mode };
  }

  /** One pick: arm, wait for the click, then hand the element over; annotate arms again after each note. */
  async wait(record, session) {
    const wc = record.view.webContents;
    const mine = () => this.shell.grab === session && !wc.isDestroyed();
    let result;
    try {
      await wc.executeJavaScript(grabScript("arm"));
      const clicked = wc.executeJavaScript(grabScript("awaitClick"));
      const late = new Promise((_, reject) => setTimeout(() => reject(new Error("cancelled")), GRAB_TIMEOUT_MS).unref?.());
      result = readGrabSelection(await Promise.race([clicked, late]), wc.getURL());
    } catch (error) {
      result = readGrabFailure(error);
    }
    if (!mine()) return;
    if (result.kind === "cancelled") return this.stop();
    if (result.kind === "error") return (this.shell.log(`pick: ${result.reason}`), this.stop());
    const image = await this.capture(wc, result.payload).catch(() => null);
    if (!mine()) return;
    this.openDraft(record, result.payload, image, session.mode);
  }

  /** The picked element's picture, cropped from the page and written where an agent can open it. */
  async capture(wc, payload) {
    const shot = await captureSelectionScreenshot(payload.target.rectViewport, wc);
    if (!shot?.dataUrl) return null;
    const file = path.join(paths.grabsDir(), `${Date.now().toString(36)}-${++this.serial}.png`);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, nativeImage.createFromDataURL(shot.dataUrl).toPNG());
    return file;
  }

  /** Shows the card a note is written in, under the picked element: sent at once after a pick, kept when annotating. */
  openDraft(record, payload, image, mode) {
    const { shell } = this;
    this.draft = { owner: record.owner, tab: record.id, payload, image, mode };
    const rect = payload.target.rectViewport;
    const scale = shell.window.pageScale || 1;
    const box = shell.window.page?.getBounds() || { x: 0, y: 0 };
    shell.window.setToolbarOpen(true);
    shell.window.send({
      ...shell.uiState(),
      draft: { label: labelOf(payload), ask: mode !== "annotate", x: Math.round(box.x + rect.x * scale), y: Math.round(box.y + (rect.y + rect.height) * scale), intents: ANNOTATION_INTENTS, priorities: ANNOTATION_PRIORITIES, priority: DEFAULT_ANNOTATION_PRIORITY },
    });
    shell.window.focusToolbar();
  }

  /** What the card held: a pick's line goes to the agent at once; a note is kept, badged in the page, and the next one awaited. */
  async saveDraft({ comment, intent, priority } = {}) {
    const { shell } = this;
    const draft = this.draft;
    this.draft = null;
    shell.window.setToolbarOpen(false);
    if (!draft) return;
    const record = shell.live.get(draft.tab);
    const said = String(comment || "").trim();
    const note = said ? noteOf({ id: `n${++this.serial}`, tab: draft.tab, payload: draft.payload, comment: said, intent, priority, image: draft.image }) : null;
    if (!note && draft.image) fs.rmSync(draft.image, { force: true });
    if (draft.mode !== "annotate") {
      if (note) this.send(draft.owner, [note], false);
      await this.stop();
      return shell.window.focusPage();
    }
    if (note) {
      shell.notes.set(draft.owner, [...(shell.notes.get(draft.owner) || []), note]);
      if (record) await this.mark(record);
    }
    shell.push();
    if (record && shell.grab?.tab === draft.tab && shell.grab.mode === "annotate") this.wait(record, shell.grab);
    shell.window.focusPage();
  }

  /** Draws, or clears, the numbered badges of a tab's notes in its page. */
  async mark(record) {
    const markers = markersOf(this.shell.notes.get(record.owner) || [], record.id);
    const code = buildAnnotationViewportBridgeScript({ enabled: markers.length > 0, emitViewport: false, markers, token: this.token });
    await record.view.webContents.executeJavaScriptInIsolatedWorld(ANNOTATION_VIEWPORT_BRIDGE_WORLD_ID, [{ code }]).catch(() => {});
  }

  /** Ends the mode: the picker leaves the page, and an open card is dropped. */
  async stop() {
    const session = this.shell.grab;
    this.shell.grab = null;
    if (this.draft) ((this.draft = null), this.shell.window.setToolbarOpen(false));
    const wc = session && this.page(session.tab);
    if (wc && !wc.isDestroyed()) await wc.executeJavaScript(grabScript("teardown")).catch(() => {});
    this.shell.push();
  }

  /** A key while the picker is armed: C copies the element under the pointer, S its picture, Escape ends the mode. */
  key(event, input) {
    const session = this.shell.grab;
    const action = session && !this.draft ? grabKeyAction(input) : null;
    if (!action) return false;
    event.preventDefault();
    const wc = this.page(session.tab);
    if (action === "cancel" || !wc) return this.stop(), true;
    wc.executeJavaScript(grabScript("extractHover"))
      .then(async (raw) => {
        const { payload } = readGrabSelection(raw, wc.getURL());
        if (!payload) return;
        if (action === "copy") return clipboard.writeText(formatGrabPayloadAsText(payload));
        const image = await this.capture(wc, payload);
        if (image) clipboard.writeImage(nativeImage.createFromPath(image));
      })
      .catch(() => {});
    return true;
  }

  /** From a page's own menu: the picker, armed on that tab. */
  pickAt(record) {
    return this.start(record.owner, record.id, "copy");
  }

  /** The notes of the shown browser as the toolbar lists them. */
  summaries(owner) {
    return (this.shell.notes.get(owner) || []).map(summaryOf);
  }

  /** What the list of notes asks: send them all, copy them, drop one or all, or say whether page details go along. */
  recap(owner, message) {
    const { shell } = this;
    const notes = shell.notes.get(owner) || [];
    if (message.do === "details") this.details = message.on === true;
    else if (message.do === "copy") clipboard.writeText(formatAnnotationsAsMarkdown(notes, { includeDiagnostics: this.details }));
    else if (message.do === "clear") this.clear(owner);
    else if (message.do === "send" && notes.length) this.send(owner, notes, true);
    else if (message.do === "delete") {
      const gone = notes.find((n) => n.id === message.id);
      for (const image of gone?.imagePaths || []) fs.rmSync(image, { force: true });
      shell.notes.set(owner, notes.filter((n) => n !== gone));
      for (const record of shell.live.values()) if (record.owner === owner) this.mark(record);
    }
    shell.push();
  }

  /** Hands notes to the session the browser belongs to: the hub, which follows this event, types them as its next prompt. */
  send(owner, notes, kept) {
    const markdown = formatAnnotationsAsMarkdown(notes, { includeDiagnostics: this.details });
    this.sending = { owner, kept, images: notes.flatMap((n) => n.imagePaths || []) };
    this.shell.emit("grab.send", { owner, markdown, images: this.sending.images });
  }

  /** What the hub answers once the agent has the notes, or why it does not. */
  sent(owner, ok, error) {
    const sending = this.sending;
    this.sending = null;
    if (ok) {
      if (sending?.kept) this.clear(owner, { files: false });
      return { ok: true };
    }
    dialog.showMessageBox(this.shell.window.win, { type: "warning", message: "The agent did not get it", detail: String(error || "cockpit could not send it."), buttons: ["OK"] });
    return { ok: false };
  }

  /** Forgets a browser's notes and takes their badges off its pages. */
  clear(owner, { files = true } = {}) {
    const { shell } = this;
    const notes = shell.notes.get(owner) || [];
    shell.notes.delete(owner);
    if (files) for (const note of notes) for (const image of note.imagePaths || []) fs.rmSync(image, { force: true });
    for (const record of shell.live.values()) if (record.owner === owner) this.mark(record);
    if (shell.grab?.owner === owner) this.stop();
    shell.push();
  }
}
