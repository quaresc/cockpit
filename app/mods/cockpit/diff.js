// A diff to read and comment on, shared by the show view's diff blocks and the review view: files that fold, lines in
// syntax colours with changed words marked, unified or side by side, a cursor moved by keys or clicks, and line
// comments drawn as rows of their own under their line, written in a composer that floats over the list.

import { div } from "gpui-kit";
import { h_flex, v_flex, v_virtual_list, Scrollbar, VirtualListScrollHandle, Input } from "gpui-base";
import { theme, settings } from "herdr";
import { tokens, alpha, TEXT, RADIUS, Icon, Button, Kbd } from "./ui.js";

const H = { file: 34, hunk: 24, line: 20, more: 26, gap: 8 };
const mono = () => settings().font_family ?? "Menlo";

/** A syntax role's colour in the person's theme. */
function syntax(cls) {
  const t = theme();
  return { k: t.ansi5, s: t.ansi2, f: t.ansi4, t: t.ansi3, c: t.dim, p: t.foreground, l: t.accent }[cls] ?? null;
}

const lineNo = (line) => (line[0] === "-" ? line[1] : line[2] ?? line[1]);
const codeOf = (line) => (line?.[3] ?? []).map((p) => p[0]).join("");
const commentHeight = (text) => 30 + Math.max(1, Math.ceil(String(text).length / 70)) * 18;

export class DiffView {
  /** `input`: an InputState made in the owner's init (no state can be made later); `id` keeps lists apart. */
  constructor(id, input) {
    this.id = id;
    this.input = input;
    this.model = null;
    this.folded = new Set();
    this.cursor = null; // { f, l } (l null: the file's header)
    this.comments = [];
    this.composer = null; // { f, l, edit: comment id | null }
    this.split = false;
    this.only = null; // a Set of hunk ids to show (a walkthrough step), else every hunk
    this.scroll = VirtualListScrollHandle.new();
    this.range = { start: 0, end: 0 };
    this.seq = 0;
  }

  /** A new read of the diff: comments stay on the lines that still exist, the cursor on its file. */
  set(model) {
    const path = this.cursor && this.model?.files?.[this.cursor.f]?.path;
    this.model = model;
    const files = model?.files ?? [];
    for (const c of this.comments) {
      const f = files.findIndex((x) => x.path === c.path);
      const l = f < 0 ? -1 : files[f].lines.findIndex((ln) => lineNo(ln) === c.line && (ln[0] === "-") === c.old);
      Object.assign(c, { f, l, stale: l < 0 });
    }
    if (path) {
      const f = files.findIndex((x) => x.path === path);
      this.cursor = f < 0 ? null : { f, l: this.cursor.l !== null && files[f].lines[this.cursor.l] ? this.cursor.l : null };
    }
  }

  files() {
    return this.model?.files ?? [];
  }

  /** The hunk each line belongs to, for showing only a step's hunks. */
  visible(file, i) {
    if (!this.only) return true;
    return (file.hunks ?? []).some((h) => this.only.has(h.id) && i >= h.at && i < h.end);
  }

  /** Every row the list draws, with its height. */
  rows() {
    const out = [];
    this.files().forEach((file, f) => {
      if (this.only && !(file.hunks ?? []).some((h) => this.only.has(h.id))) return;
      out.push({ key: `f:${f}`, h: H.file, kind: "file", f });
      if (this.folded.has(file.path)) return;
      const lines = file.lines;
      const after = (l) => this.comments.filter((c) => c.f === f && c.l === l && !c.stale);
      const pushComments = (ls) => ls.forEach((l) => after(l).forEach((c) => out.push({ key: `c:${c.id}`, h: commentHeight(c.text) + 8, kind: "comment", f, l, c })));
      for (let i = 0; i < lines.length; i++) {
        if (!this.visible(file, i)) continue;
        const kind = lines[i][0];
        if (kind === "@") {
          out.push({ key: `l:${f}:${i}`, h: H.hunk, kind: "hunk", f, l: i });
          continue;
        }
        if (!this.split || (kind !== "-" && kind !== "+")) {
          out.push({ key: `l:${f}:${i}`, h: H.line, kind: "line", f, l: i });
          pushComments([i]);
          continue;
        }
        // Side by side: a run of removed lines faces the added ones that follow it.
        let d = i;
        while (d < lines.length && lines[d][0] === "-") d++;
        let a = d;
        while (a < lines.length && lines[a][0] === "+") a++;
        const dels = d - i, adds = a - d;
        for (let k = 0; k < Math.max(dels, adds); k++) {
          const left = k < dels ? i + k : null, right = k < adds ? d + k : null;
          out.push({ key: `l:${f}:${right ?? left}`, h: H.line, kind: "pair", f, l: right ?? left, left, right });
          pushComments([left, right].filter((x) => x !== null));
        }
        i = a - 1;
      }
      if (file.more) out.push({ key: `m:${f}`, h: H.more, kind: "more", f });
    });
    return out;
  }

  /** The places the cursor stops: each file's header, then its lines (not hunk headers). */
  stops() {
    const out = [];
    this.files().forEach((file, f) => {
      if (this.only && !(file.hunks ?? []).some((h) => this.only.has(h.id))) return;
      out.push({ f, l: null });
      if (this.folded.has(file.path)) return;
      file.lines.forEach((ln, l) => ln[0] !== "@" && ln[0] !== "!" && this.visible(file, l) && out.push({ f, l }));
    });
    return out;
  }

  step(by) {
    const stops = this.stops();
    if (!stops.length) return;
    const at = this.cursor ? stops.findIndex((s) => s.f === this.cursor.f && s.l === this.cursor.l) : -1;
    const next = Math.max(0, Math.min(stops.length - 1, at < 0 ? (by > 0 ? 0 : stops.length - 1) : at + by));
    this.cursor = stops[next];
    this.reveal();
  }

  jumpFile(forward) {
    const files = this.files();
    if (!files.length) return;
    const f = this.cursor?.f ?? -1;
    this.cursor = forward ? { f: Math.min(files.length - 1, f + 1), l: null } : { f: Math.max(0, this.cursor?.l !== null ? f : f - 1), l: null };
    this.reveal();
  }

  reveal() {
    if (!this.cursor) return;
    const rows = this.rows();
    const i = rows.findIndex((r) => r.f === this.cursor.f && (this.cursor.l === null ? r.kind === "file" : r.l === this.cursor.l || r.left === this.cursor.l));
    if (i >= 0 && (i < this.range.start + 1 || i >= this.range.end - 1)) this.scroll.scroll_to_item(i, "center");
  }

  fold(f) {
    const path = this.files()[f]?.path;
    if (!path) return;
    if (!this.folded.delete(path)) this.folded.add(path);
    this.cursor = { f, l: null };
  }

  foldAll() {
    const files = this.files();
    const all = files.every((f) => this.folded.has(f.path));
    this.folded = all ? new Set() : new Set(files.map((f) => f.path));
  }

  /** Opens the composer on the cursor's line (or the comment `edit`). */
  compose(f, l, edit = null) {
    const line = this.files()[f]?.lines?.[l];
    if (!line || line[0] === "@" || line[0] === "!") return false;
    this.cursor = { f, l };
    this.composer = { f, l, edit };
    this.input.set_value(edit ? this.comments.find((c) => c.id === edit)?.text ?? "" : "");
    this.input.focus();
    return true;
  }

  /** The composer's text becomes a comment (or replaces the one edited); an empty text removes an edited one. */
  commit() {
    const c = this.composer;
    if (!c) return;
    const text = this.input.value().trim();
    this.composer = null;
    if (c.edit) {
      const at = this.comments.findIndex((x) => x.id === c.edit);
      if (at >= 0 && text) this.comments[at].text = text;
      else if (at >= 0) this.comments.splice(at, 1);
      return;
    }
    if (!text) return;
    const file = this.files()[c.f], line = file.lines[c.l];
    this.comments.push({ id: `${this.id}-${++this.seq}`, path: file.path, line: lineNo(line), old: line[0] === "-", code: codeOf(line), text, f: c.f, l: c.l });
  }

  remove(id) {
    this.comments = this.comments.filter((c) => c.id !== id);
    if (this.composer?.edit === id) this.composer = null;
  }

  /** The comments as the hub sends them. */
  review() {
    return this.comments.filter((c) => !c.stale).map(({ path, line, old, code, text }) => ({ path, line, old, code, text }));
  }

  /** A key on the diff: true when it used it. */
  key(e) {
    const key = e.keystroke;
    if (this.composer) {
      if (key === "escape") {
        this.composer = null;
        return true;
      }
      return false;
    }
    switch (key) {
      case "j": case "down": this.step(1); return true;
      case "k": case "up": this.step(-1); return true;
      case "g": case "home": this.cursor = null; this.step(1); return true;
      case "shift-g": case "end": this.step(1e9); return true;
      case "]": this.jumpFile(true); return true;
      case "[": this.jumpFile(false); return true;
      case "s": this.split = !this.split; return true;
      case "z": this.foldAll(); return true;
      case "enter":
        if (this.cursor?.l === null) this.fold(this.cursor.f);
        else if (this.cursor) this.compose(this.cursor.f, this.cursor.l);
        return true;
      case "c": return this.cursor?.l !== null && this.cursor ? this.compose(this.cursor.f, this.cursor.l) : false;
      case "e": return this.openCursor();
    }
    return false;
  }

  click(key, cx) {
    const [kind, a, b] = key.split(":");
    if (kind === "f") this.fold(Number(a));
    else if (kind === "l") {
      const f = Number(a), l = Number(b);
      if (this.cursor?.f === f && this.cursor?.l === l) this.compose(f, l);
      else this.cursor = { f, l };
    } else if (kind === "c") {
      const c = this.comments.find((x) => x.id === key.slice(2));
      if (c) this.compose(c.f, c.l, c.id);
    }
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  fileRow(r, k) {
    const file = this.files()[r.f];
    const folded = this.folded.has(file.path);
    const on = this.cursor?.f === r.f && this.cursor.l === null;
    const count = this.comments.filter((c) => c.path === file.path && !c.stale).length;
    const slash = file.path.lastIndexOf("/");
    return h_flex().h(H.file).px(10).gap(8).items_center().border_b_1().border_t_1().border_color(k.border).bg(on ? k.selected : k.muted).cursor_pointer()
      .child(Icon(folded ? "chevron-right" : "chevron-down", { size: 14, color: k.mutedFg }))
      .child(h_flex().flex_1().min_w(0).text_size(TEXT.sm).font_family(mono())
        .child(div().flex_none().text_color(k.mutedFg).child(slash >= 0 ? file.path.slice(0, slash + 1) : ""))
        .child(div().truncate().font_semibold().text_color(k.foreground).child(file.path.slice(slash + 1))))
      .when(count > 0, (e) => e.child(h_flex().gap(4).items_center().text_size(TEXT.xs).text_color(k.ring).child(Icon("message-square", { size: 12 })).child(String(count))))
      .child(div().text_size(TEXT.xs).font_family(mono()).text_color(k.success).child(`+${file.added}`))
      .child(div().text_size(TEXT.xs).font_family(mono()).text_color(k.destructive).child(`−${file.removed}`));
  }

  pieces(line, tint) {
    return (line?.[3] ?? []).map(([text, cls, changed]) => {
      let el = div().child(text);
      const color = syntax(cls);
      if (color) el = el.text_color(color);
      if (changed && tint) el = el.bg(alpha(tint, 0.32)).rounded(2);
      return el;
    });
  }

  half(line, l, f, k, gutterW, cursorOn) {
    const kind = line?.[0] ?? " ";
    const tint = kind === "+" ? k.success : kind === "-" ? k.destructive : null;
    const commented = l !== null && this.comments.some((c) => c.f === f && c.l === l && !c.stale);
    return h_flex().flex_1().min_w(0).h_full().items_center()
      .when(Boolean(tint), (e) => e.bg(alpha(tint, 0.1)))
      .when(!line, (e) => e.bg(alpha(k.muted, 0.5)))
      .child(h_flex().w(gutterW).flex_none().h_full().items_center().justify_end().pr(6).text_size(TEXT.xs).font_family(mono())
        .text_color(cursorOn ? k.foreground : k.dim).when(Boolean(tint), (e) => e.bg(alpha(tint, 0.14)))
        .when(commented, (e) => e.child(div().size(6).mr(4).rounded_full().bg(k.ring)))
        .child(line ? String(lineNo(line) ?? "") : ""))
      .child(div().w(16).flex_none().text_center().text_size(TEXT.sm).font_family(mono()).text_color(tint ?? k.dim).child(kind === "-" ? "−" : kind === "+" ? "+" : ""))
      .child(h_flex().flex_1().min_w(0).overflow_hidden().whitespace_nowrap().text_size(TEXT.sm).font_family(mono()).text_color(k.foreground)
        .children(line ? this.pieces(line, tint) : []));
  }

  lineRow(r, k) {
    const file = this.files()[r.f];
    const on = this.cursor?.f === r.f && (this.cursor.l === r.l || (r.kind === "pair" && this.cursor.l === r.left));
    let row = h_flex().h(H.line).w_full().relative();
    if (r.kind === "hunk") {
      const text = file.lines[r.l][3]?.[0]?.[0] ?? "";
      return row.px(10).items_center().bg(alpha(k.info, 0.07)).text_size(TEXT.xs).font_family(mono()).text_color(k.mutedFg).h(H.hunk).child(div().truncate().child(text));
    }
    if (r.kind === "pair") {
      row = row.child(this.half(r.left !== null ? file.lines[r.left] : null, r.left, r.f, k, 44, on))
        .child(div().w(1).h_full().bg(k.border))
        .child(this.half(r.right !== null ? file.lines[r.right] : null, r.right, r.f, k, 44, on));
    } else {
      const line = file.lines[r.l];
      if (line[0] === "!") return row.px(10).items_center().text_size(TEXT.xs).text_color(k.dim).child(codeOf(line));
      if (this.split && line[0] === " ") {
        row = row.child(this.half(line, r.l, r.f, k, 44, on)).child(div().w(1).h_full().bg(k.border)).child(this.half(line, null, r.f, k, 44, on));
      } else row = row.child(this.half(line, r.l, r.f, k, 52, on));
    }
    if (on) row = row.child(div().absolute().left(0).top(0).bottom(0).w(2).bg(k.ring)).child(div().absolute().inset_0().bg(alpha(k.ring, 0.08)));
    return row;
  }

  commentRow(r, k) {
    const c = r.c;
    const editing = this.composer?.edit === c.id;
    return div().px(10).py(4).child(
      v_flex().ml(this.split ? 0 : 52).p(8).gap(4).rounded(RADIUS.md).border_1().border_color(editing ? k.ring : alpha(k.ring, 0.45)).bg(alpha(k.ring, 0.06))
        .child(h_flex().gap(6).items_center().text_size(TEXT.xs).text_color(k.mutedFg)
          .child(Icon("message-square", { size: 12, color: k.ring }))
          .child(`Comment · line ${c.line}${c.old ? " (removed)" : ""}`)
          .child(div().flex_1())
          .child(editing ? "editing" : "click to edit"))
        .child(div().text_size(TEXT.sm).text_color(k.foreground).child(c.text)),
    );
  }

  moreRow(r, k) {
    const file = this.files()[r.f];
    return h_flex().h(H.more).px(10).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(`${file.more} more lines not drawn`);
  }

  /** The composer floating over the list's top right, or docked under it when the list is narrow. */
  composerCard(k, cx, narrow) {
    const c = this.composer;
    const file = this.files()[c.f], line = file?.lines?.[c.l];
    if (!line) return null;
    const stop = (el) => el.on_mouse_down("left", (_e, cx) => cx.stop_propagation()).on_mouse_up("left", (_e, cx) => cx.stop_propagation());
    const card = v_flex().id(`${this.id}-composer`).gap(8).p(12).rounded(RADIUS.lg).border_1().border_color(k.ring).bg(k.popover).shadow_lg()
      .child(h_flex().gap(6).items_center().text_size(TEXT.sm).font_semibold().text_color(k.foreground)
        .child(Icon("message-square-plus", { size: 14, color: k.ring }))
        .child(div().truncate().child(`${file.path.split("/").pop()}:${lineNo(line)}`)))
      .child(div().px(8).py(4).rounded(RADIUS.sm).bg(k.muted).text_size(TEXT.xs).font_family(mono()).text_color(k.mutedFg).truncate().child(codeOf(line).trim() || " "))
      .child(Input.new(this.input).h(32).px(8).rounded(RADIUS.md).border_1().border_color(k.input).bg(k.background).text_size(TEXT.sm))
      .child(h_flex().gap(6).items_center()
        .child(h_flex().gap(4).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(Kbd("⏎")).child("add").child(Kbd("esc")).child("cancel"))
        .child(div().flex_1())
        .when(Boolean(c.edit), (e) => e.child(Button(`${this.id}-del`, { label: "Delete", variant: "ghost", size: "sm", onClick: (_e, cx) => { this.remove(c.edit); cx.notify(); } })))
        .child(Button(`${this.id}-cancel`, { label: "Cancel", variant: "outline", size: "sm", onClick: (_e, cx) => { this.composer = null; cx.notify(); } }))
        .child(Button(`${this.id}-add`, { label: c.edit ? "Save" : "Add comment", size: "sm", onClick: (_e, cx) => { this.commit(); cx.notify(); } })));
    return stop(narrow ? card.mx(8).mb(8) : card.absolute().top(8).right(12).w(340));
  }

  /** The file under the cursor, opened beside the agent to read or edit (when the view gives `onOpen`). */
  openCursor() {
    const file = this.cursor ? this.files()[this.cursor.f] : null;
    if (!file || !this.onOpen) return false;
    this.onOpen(file.path);
    return true;
  }

  /** The list, `height` px tall (flex when null), its composer over it. */
  render(cx, { height = null, narrow = false } = {}) {
    const k = tokens();
    const rows = this.rows();
    const list = v_virtual_list(`${this.id}-rows`, rows.length, rows.map((r) => r.h), (i) => rows[i].key, (range) => {
      this.range = range;
      return rows.slice(range.start, range.end).map((r) =>
        r.kind === "file" ? this.fileRow(r, k) : r.kind === "comment" ? this.commentRow(r, k) : r.kind === "more" ? this.moreRow(r, k) : this.lineRow(r, k));
    }).track_scroll(this.scroll).size_full().on_item_click((key, cx) => this.click(key, cx));
    let box = div().relative().w_full().overflow_hidden().child(list).child(Scrollbar.vertical(`${this.id}-rows`).viewport_from_layout().absolute().inset_0());
    box = height === null ? box.flex_1().min_h(0) : box.h(Math.min(height, rows.reduce((s, r) => s + r.h, 0) + 2));
    const composer = this.composer ? this.composerCard(k, cx, narrow) : null;
    if (!composer) return box;
    return narrow ? v_flex().w_full().when(height === null, (e) => e.flex_1().min_h(0)).child(box).child(composer) : box.child(composer);
  }
}
