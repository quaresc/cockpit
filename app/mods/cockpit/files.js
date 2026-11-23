// The side panel's Files: the chat agent's folder as a tree, opened down to the repositories it works in, those marked
// (one outside the folder listed above it), and every changed file coloured by its git status; a file opens in cockpit's
// editor.

import { div } from "gpui-kit";
import { h_flex, v_flex, Scrollbar } from "gpui-base";
import { exec } from "herdr";
import { tokens, TEXT, RADIUS, Icon, dragPaths } from "./ui.js";
import { openCode } from "./code.js";
import { where, whereKey, reposOf, statusOf } from "./projects.js";

/** The most entries listed in one folder. */
const PER_FOLDER = 500;

/** Lists a folder, folders first: [{name, dir}], `.git` left out. */
async function listing(path) {
  const script =
    'n=0; for p in "$1"/* "$1"/.[!.]* "$1"/..?*; do [ -e "$p" ] || [ -L "$p" ] || continue; name=${p##*/}; [ "$name" = .git ] && continue; if [ -d "$p" ]; then kind=d; else kind=f; fi; printf "%s\\0%s\\0" "$kind" "$name"; n=$((n+1)); [ "$n" -ge ' +
    PER_FOLDER +
    ' ] && break; done; test -d "$1"';
  const r = await exec("sh", ["-c", script, "files", path]);
  if (r.code !== 0) throw new Error(r.stderr || "Cannot list this folder");
  const parts = r.stdout.split("\0");
  const out = [];
  for (let i = 0; i + 1 < parts.length; i += 2) out.push({ name: parts[i + 1], dir: parts[i] === "d" });
  return out.sort((a, b) => Number(b.dir) - Number(a.dir) || a.name.localeCompare(b.name));
}

/**
 * The marks of a tree from its repositories' changes: each changed file's status by its path, and how many changes sit
 * under each folder up to its repository's top.
 */
export function marksOf(changes) {
  const files = new Map();
  const under = new Map();
  for (const { root, files: list } of changes) {
    for (const f of list) {
      const path = `${root}/${f.path}`;
      files.set(path, f.status);
      for (let dir = path.slice(0, path.lastIndexOf("/")); dir.length >= root.length; dir = dir.slice(0, dir.lastIndexOf("/"))) {
        under.set(dir, (under.get(dir) ?? 0) + 1);
        if (dir === root) break;
      }
    }
  }
  return { files, under };
}

/** The folders to open for the repositories the agent works in: the way down from `root` to each under it, and each. */
export function pathsTo(root, repos) {
  const open = new Set();
  for (const { path } of repos) {
    if (!path.startsWith(`${root}/`)) {
      open.add(path);
      continue;
    }
    let dir = root;
    for (const part of path.slice(root.length + 1).split("/")) {
      dir = `${dir}/${part}`;
      open.add(dir);
    }
  }
  return open;
}

/** The repositories the agent works in outside `root`: listed above its tree, each as a folder of its own. */
export function outside(root, repos) {
  return (repos ?? []).filter((r) => r.path !== root && !r.path.startsWith(`${root}/`));
}

export default class Files {
  init(_props, cx) {
    this.entries = new Map();
    this.open = new Set();
    this.marks = { files: new Map(), under: new Map() };
    this.refresh(cx);
    cx.timer.every(5000, (cx) => this.refresh(cx));
  }

  /** Lists the open folders again and the repositories' changes; a new place starts the tree over. */
  refresh(cx) {
    const w = where();
    const key = whereKey(w);
    if (key !== this.key) {
      const same = w.cwd === this.root;
      this.key = key;
      this.root = w.cwd;
      this.repos = w.repos;
      if (!same) {
        this.entries = new Map();
        this.open = new Set();
      }
      // The way to each repository the agent works in opens, its own folder too.
      for (const dir of pathsTo(w.cwd ?? "", w.repos)) this.open.add(dir);
    }
    if (!this.root || this.loading) return;
    this.loading = true;
    const root = this.root;
    cx.spawn(async (cx) => {
      try {
        for (const dir of [root, ...this.open]) {
          const list = await listing(dir).catch(() => null);
          if (this.root !== root) return;
          if (list) this.entries.set(dir, list);
          else if (dir !== root) this.open.delete(dir);
        }
        const repos = await reposOf({ cwd: root, repos: this.repos });
        const changes = [];
        for (const r of repos) {
          const s = await statusOf(r.path, 2000);
          if (s) changes.push({ root: r.path, files: s.files });
        }
        if (this.root === root) {
          this.marks = marksOf(changes);
          this.error = null;
        }
      } catch (e) {
        this.error = String(e?.message ?? e);
      } finally {
        this.loading = false;
        cx.notify();
      }
    });
  }

  /** Opens or closes a folder, by its full path; an opened one is listed now. */
  toggle(dir, cx) {
    if (this.open.has(dir)) {
      for (const f of [...this.open]) if (f === dir || f.startsWith(`${dir}/`)) this.open.delete(f);
      return cx.notify();
    }
    this.open.add(dir);
    const root = this.root;
    cx.spawn(async (cx) => {
      const list = await listing(dir).catch(() => null);
      if (this.root !== root) return;
      if (list) this.entries.set(dir, list);
      else this.open.delete(dir);
      cx.notify();
    });
    cx.notify();
  }

  /**
   * The tree's rows as drawn, depth first through the open folders: the repositories outside the folder first, as
   * folders named by their place, then the folder's own entries.
   */
  rows() {
    const out = [];
    const walk = (dir, depth) => {
      for (const e of this.entries.get(dir) ?? []) {
        const path = `${dir}/${e.name}`;
        out.push({ ...e, path, depth });
        if (e.dir && this.open.has(path)) walk(path, depth + 1);
      }
    };
    const home = this.root?.match(/^\/(Users|home)\/[^/]+/)?.[0];
    for (const r of outside(this.root, this.repos)) {
      out.push({ name: home && r.path.startsWith(`${home}/`) ? `~${r.path.slice(home.length)}` : r.path, dir: true, path: r.path, depth: 0 });
      if (this.open.has(r.path)) walk(r.path, 1);
    }
    walk(this.root, 0);
    return out;
  }

  render(_cx) {
    const k = tokens();
    const repos = new Set((this.repos ?? []).map((r) => r.path));
    const colour = { A: k.success, M: k.blocked, D: k.destructive, U: k.destructive };
    const row = (e) => {
      const abs = e.path;
      const status = this.marks.files.get(abs);
      const changed = this.marks.under.get(abs) ?? 0;
      const worked = e.dir && repos.has(abs);
      return dragPaths(h_flex().id(`file-${abs}`), [abs])
        .h(26)
        .flex_none()
        .pl(8 + e.depth * 14)
        .pr(8)
        .gap(6)
        .items_center()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_size(TEXT.sm)
        .text_color(status ? colour[status] : worked || changed ? k.foreground : k.mutedFg)
        .when(worked, (el) => el.bg(k.selected))
        .hover((s) => s.bg(k.accent))
        .tooltip(worked ? `${abs} — the agent works here` : abs)
        .on_click((_e, cx) => (e.dir ? this.toggle(abs, cx) : openCode(abs)))
        .child(div().w(12).flex_none().when(e.dir, (el) => el.child(Icon(this.open.has(abs) ? "chevron-down" : "chevron-right", { size: 12, color: k.dim }))))
        .child(Icon(e.dir ? (worked ? "folder-git" : "folder") : "file-text", { size: 13, color: worked ? k.foreground : k.dim }))
        .child(div().flex_1().min_w(0).truncate().when(worked, (el) => el.font_medium()).child(e.name))
        .when(e.dir && changed > 0, (el) => el.child(div().flex_none().text_size(TEXT.xs).text_color(k.blocked).child(String(changed))))
        .when(Boolean(status), (el) => el.child(div().flex_none().w(12).text_size(TEXT.xs).child(status)));
    };
    const rows = this.rows();
    return v_flex()
      .id("cockpit-files")
      .size_full()
      .min_h(0)
      .bg(k.canvas)
      .text_color(k.foreground)
      .child(
        h_flex()
          .h(42)
          .flex_none()
          .px(14)
          .gap(8)
          .items_center()
          .border_b_1()
          .border_color(k.border)
          .child(div().font_semibold().child("Files"))
          .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(this.root ?? "")),
      )
      .child(
        div()
          .flex_1()
          .min_h(0)
          .relative()
          .child(v_flex().id("cockpit-files-list").absolute().inset_0().overflow_y_scroll().p(6).children(rows.map(row)))
          .child(Scrollbar.vertical("cockpit-files-list").absolute().inset_0())
          .when(!rows.length, (el) => el.child(div().p(12).text_size(TEXT.sm).text_color(k.dim).child(this.error || (this.loading ? "Loading files…" : this.root ? "No files" : "No folder")))),
      );
  }
}
