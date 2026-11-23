// The side panel's Changes: the uncommitted changes of the repositories the chat agent works in, the latest first, one
// at a time, each file's diff under the list, and a commit of everything in the one shown.

import { div } from "gpui-kit";
import { h_flex, v_flex, Scrollbar, Input, InputState, v_virtual_list } from "gpui-base";
import { exec, readTextFile } from "herdr";
import { tokens, TEXT, RADIUS, dragPaths } from "./ui.js";
import { READ, where, whereKey, reposOf, statusOf } from "./projects.js";

/** The most diff lines drawn, and the characters kept of one. */
const LINES = 1500;
const WIDE = 4000;

/** A diff's lines numbered once, so a frame only draws the rows on screen. */
export function diffRows(lines) {
  let old = 0;
  let next = 0;
  const untab = (text) => text.replace(/[^\t]*\t/g, (s) => s.slice(0, -1).padEnd(s.length - 1 - ((s.length - 1) % 4) + 4));
  return lines.map((line) => {
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)/.exec(line);
    if (hunk) {
      old = Number(hunk[1]);
      next = Number(hunk[2]);
      return { kind: "@", text: line };
    }
    const kind = line[0];
    const text = untab(line.slice(1)) || " ";
    if (kind === "-") return { kind, number: old++, text };
    if (kind === "+") return { kind, number: next++, text };
    if (kind === " ") {
      old++;
      return { kind, number: next++, text };
    }
    return { kind: "", text: line };
  });
}

/** Lines added and removed per path, from `git diff --numstat -z` outputs. */
function numstats(outputs) {
  const stats = {};
  for (const out of outputs) {
    for (const entry of out.split("\0")) {
      const [a, d, ...parts] = entry.split("\t");
      if (!parts.length) continue;
      const path = parts.join("\t");
      const was = stats[path] ?? { add: 0, del: 0 };
      stats[path] = { add: was.add + (Number(a) || 0), del: was.del + (Number(d) || 0), binary: a === "-" };
    }
  }
  return stats;
}

export default class Changes {
  init(_props, cx) {
    this.repos = [];
    this.files = [];
    this.rows = [];
    this.input = InputState.new({ placeholder: "Commit message" });
    this.input.on("submit", (_e, cx) => this.commit(cx));
    this.input.on("change", (_e, cx) => cx.notify());
    this.refresh(cx);
    cx.timer.every(5000, (cx) => this.refresh(cx));
  }

  /** The repository shown: the one picked, else the one the agent touched last. */
  get repo() {
    return this.repos.find((r) => r.path === this.picked)?.path ?? this.repos[0]?.path ?? null;
  }

  refresh(cx) {
    if (this.loading || this.busy) return;
    const w = where();
    const key = whereKey(w);
    if (key !== this.key) {
      this.key = key;
      this.picked = null;
      this.open = null;
      this.rows = [];
      this.note = null;
    }
    this.loading = true;
    cx.spawn(async (cx) => {
      try {
        const repos = await reposOf(w);
        if (whereKey(where()) !== key) return;
        // How many files changed in each, for the switcher.
        const counts = await Promise.all(repos.map((r) => statusOf(r.path).then((s) => s?.files.length ?? 0)));
        this.repos = repos.map((r, i) => ({ ...r, count: counts[i] }));
        await this.read(cx);
      } catch (e) {
        this.note = String(e?.message ?? e);
      } finally {
        this.loading = false;
        cx.notify();
      }
    });
  }

  /** The shown repository's changed files with their line counts, and the open file's diff. */
  async read() {
    const root = this.repo;
    if (!root) {
      this.files = [];
      return;
    }
    const status = await statusOf(root);
    if (!status) throw new Error("Not a git repository");
    const git = (args) => exec("git", [...READ, ...args], { cwd: root });
    const head = await git(["rev-parse", "--verify", "HEAD"]);
    this.hasHead = head.code === 0;
    const outs = this.hasHead ? [await git(["diff", "HEAD", "--numstat", "-z", "--no-renames"])] : [await git(["diff", "--cached", "--numstat", "-z", "--no-renames"]), await git(["diff", "--numstat", "-z", "--no-renames"])];
    const stats = numstats(outs.filter((o) => o.code === 0).map((o) => o.stdout));
    if (root !== this.repo) return;
    this.files = status.files.map((f) => ({ ...f, ...(stats[f.path] ?? { add: 0, del: 0 }) }));
    this.limited = status.limited;
    if (this.open && !this.files.some((f) => f.path === this.open)) this.open = null;
    if (!this.open && !this.closed && this.files.length) this.open = this.files[0].path;
    if (this.open) await this.load(this.open);
  }

  /** The diff of one file, a new file as all its lines added. */
  async load(path) {
    const root = this.repo;
    const file = this.files.find((f) => f.path === path);
    if (!root || !file) return;
    let lines;
    if (file.fresh || (!this.hasHead && file.status === "A")) {
      const text = await readTextFile(`${root}/${path}`).catch(() => null);
      if (text === null) lines = ["Cannot preview this file (binary, too large, or unreadable)"];
      else if (text.includes("\0")) lines = ["Binary file · no text diff"];
      else {
        const all = text.slice(0, 400000).replace(/\n$/, "").split("\n");
        file.add = all.length;
        lines = [`@@ -0,0 +1,${all.length} @@`, ...all.map((l) => `+${l}`)];
      }
    } else {
      const r = await exec("git", [...READ, "diff", ...(this.hasHead ? ["HEAD"] : []), "--no-ext-diff", "--no-textconv", "--no-color", "--", path], { cwd: root });
      const all = r.code === 0 ? r.stdout.replace(/\n$/, "").split("\n") : [];
      const first = all.findIndex((l) => l.startsWith("@@"));
      lines = first < 0 ? [file.binary ? "Binary file changed" : "No text diff available"] : all.slice(first);
    }
    if (root !== this.repo || this.open !== path) return;
    this.rows = diffRows(lines.slice(0, LINES).map((l) => (l.length > WIDE ? `${l.slice(0, WIDE)}… (line cut)` : l)));
    this.width = Math.max(300, ...this.rows.map((r) => (r.text.length + 1) * 7 + 50));
  }

  /** Shows another repository's changes. */
  show(path, cx) {
    this.picked = path;
    this.open = null;
    this.closed = false;
    this.rows = [];
    this.note = null;
    cx.spawn(async (cx) => {
      await this.read().catch((e) => (this.note = String(e?.message ?? e)));
      cx.notify();
    });
    cx.notify();
  }

  /** Opens a file's diff, or closes the one open. */
  pick(path, cx) {
    this.open = this.open === path ? null : path;
    this.closed = !this.open;
    this.rows = [];
    if (this.open) cx.spawn(async (cx) => (await this.load(path).catch(() => {}), cx.notify()));
    cx.notify();
  }

  /** Commits everything changed in the repository shown, with the message typed. */
  commit(cx) {
    const message = this.input.value().trim();
    const root = this.repo;
    if (!message || !root || this.busy) return;
    this.busy = true;
    this.note = null;
    cx.spawn(async (cx) => {
      try {
        for (const args of [["add", "-A"], ["commit", "-m", message]]) {
          const r = await exec("git", args, { cwd: root });
          if (r.code !== 0) throw new Error((r.stderr || r.stdout).trim());
        }
        this.input.set_value("");
      } catch (e) {
        this.note = `Commit failed: ${e?.message ?? e}`;
      } finally {
        this.busy = false;
        this.refresh(cx);
        cx.notify();
      }
    });
    cx.notify();
  }

  render(_cx) {
    const k = tokens();
    const add = this.files.reduce((n, f) => n + (f.add ?? 0), 0);
    const del = this.files.reduce((n, f) => n + (f.del ?? 0), 0);
    const ready = Boolean(this.input.value().trim() && this.repo && !this.busy);
    const colour = { A: k.success, M: k.blocked, D: k.destructive, U: k.destructive };
    const switcher = this.repos.length > 1
      ? h_flex()
          .flex_none()
          .px(10)
          .py(6)
          .gap(4)
          .flex_wrap()
          .border_b_1()
          .border_color(k.border)
          .children(
            this.repos.map((r) =>
              h_flex()
                .id(`changes-repo-${r.path}`)
                .h(24)
                .px(9)
                .gap(5)
                .items_center()
                .rounded(12)
                .cursor_pointer()
                .text_size(TEXT.xs)
                .text_color(r.path === this.repo ? k.foreground : k.mutedFg)
                .when(r.path === this.repo, (e) => e.bg(k.selected))
                .hover((s) => s.bg(k.accent))
                .tooltip(r.path)
                .on_click((_e, cx) => this.show(r.path, cx))
                .child(div().child(r.name))
                .when(r.count > 0, (e) => e.child(div().text_color(k.blocked).child(String(r.count)))),
            ),
          )
      : null;
    const fileRow = (f) => {
      const parts = f.path.split("/");
      const name = parts.pop();
      return dragPaths(h_flex().id(`change-${f.path}`), f.status === "D" || !this.repo ? [] : [`${this.repo}/${f.path}`])
        .h(28)
        .flex_none()
        .px(8)
        .gap(8)
        .items_center()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_size(TEXT.sm)
        .when(this.open === f.path, (e) => e.bg(k.selected))
        .hover((s) => s.bg(k.accent))
        .on_click((_e, cx) => this.pick(f.path, cx))
        .child(div().w(14).text_size(TEXT.xs).text_color(colour[f.status]).child(f.status))
        .child(h_flex().flex_1().min_w(0).child(div().text_color(k.dim).truncate().child(parts.length ? `${parts.join("/")}/` : "")).child(div().min_w(0).truncate().child(name)))
        .when(f.add > 0, (e) => e.child(div().text_size(TEXT.xs).text_color(k.success).child(`+${f.add}`)))
        .when(f.del > 0, (e) => e.child(div().text_size(TEXT.xs).text_color(k.destructive).child(`−${f.del}`)));
    };
    const diffRow = ({ kind, number, text }) =>
      kind === "@"
        ? div().w_full().px(12).h(20).flex_none().truncate().bg(k.muted).text_color(k.dim).child(text)
        : h_flex()
            .w_full()
            .h(20)
            .flex_none()
            .gap(10)
            .when(kind === "+", (e) => e.bg(`${k.success}22`))
            .when(kind === "-", (e) => e.bg(`${k.destructive}22`))
            .child(div().w(30).flex_none().text_right().text_color(k.dim).child(number === undefined ? "" : String(number)))
            .child(div().flex_1().min_w(0).truncate().child(text));
    const empty = this.note || (this.loading && !this.files.length ? "Loading changes…" : this.repo ? "No uncommitted changes" : "Not in a git repository");
    return v_flex()
      .id("cockpit-changes")
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
          .child(div().font_semibold().child("Changes"))
          .child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(this.repo ? this.repo.split("/").pop() : ""))
          .child(div().text_size(TEXT.xs).text_color(k.success).child(`+${add}`))
          .child(div().text_size(TEXT.xs).text_color(k.destructive).child(`−${del}`)),
      )
      .when(Boolean(switcher), (e) => e.child(switcher))
      .child(
        div()
          .flex_none()
          .h(Math.min(Math.max(this.files.length, 1) * 28 + 12, 240))
          .relative()
          .border_b_1()
          .border_color(k.border)
          .child(v_flex().id("cockpit-changes-list").absolute().inset_0().overflow_y_scroll().p(6).children(this.files.map(fileRow)))
          .child(Scrollbar.vertical("cockpit-changes-list").absolute().inset_0())
          .when(!this.files.length, (e) => e.child(div().p(8).text_size(TEXT.sm).text_color(k.dim).child(empty))),
      )
      .when(this.limited, (e) => e.child(div().px(12).py(4).text_size(TEXT.xs).text_color(k.dim).child("Showing the first 1,000 changed files")))
      .child(
        div()
          .flex_1()
          .min_h(0)
          .relative()
          .font_family("JetBrains Mono")
          .text_size(TEXT.xs)
          .child(
            div()
              .id("cockpit-changes-x")
              .size_full()
              .overflow_x_scrollbar()
              .child(div().w_full().min_w(this.width ?? 300).h_full().child(v_virtual_list("cockpit-changes-diff", this.rows.length, 20, (i) => String(i), (range) => this.rows.slice(range.start, range.end).map(diffRow)).size_full())),
          )
          .child(Scrollbar.vertical("cockpit-changes-diff").absolute().inset_0())
          .when(!this.open && this.files.length > 0, (e) => e.child(div().absolute().top(6).px(12).text_color(k.dim).child("Pick a file to see its changes"))),
      )
      .when(Boolean(this.note) && this.files.length > 0, (e) => e.child(div().px(12).pb(6).text_size(TEXT.xs).text_color(k.destructive).child(this.note)))
      .child(
        h_flex()
          .flex_none()
          .p(10)
          .gap(8)
          .border_t_1()
          .border_color(k.border)
          .child(div().flex_1().min_w(0).h(32).px(10).flex().items_center().rounded(RADIUS.lg).border_1().border_color(k.border).child(Input.new(this.input).h_full().text_size(TEXT.sm)))
          .child(
            div()
              .id("cockpit-commit")
              .h(32)
              .px(14)
              .flex()
              .items_center()
              .rounded(RADIUS.lg)
              .text_size(TEXT.sm)
              .bg(ready ? k.primary : k.muted)
              .text_color(ready ? k.primaryFg : k.dim)
              .when(ready, (e) => e.cursor_pointer())
              .tooltip(this.repo ? `Commit everything changed in ${this.repo.split("/").pop()}` : "")
              .on_click((_e, cx) => this.commit(cx))
              .child(this.busy ? "Committing…" : "Commit"),
          ),
      );
  }
}
