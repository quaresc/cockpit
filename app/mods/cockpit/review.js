// The review view: the followed agent's changes in one of three scopes (uncommitted, its branch against the base, its
// pull request), commented line by line and sent as one message (Submit review · N) to any agent working in that
// repository, a driven session among them, or a new agent; or read as a guided walkthrough: the hunks grouped into steps
// in reading order, each explained, written by an agent run headless in the background through the hub.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { focused, panes, layout, props, shared, setShared } from "herdr";
import * as hub from "./hub.js";
import { take } from "./person.js";
import { sharing } from "./feed.js";
import { openCode } from "./code.js";
import { tokens, alpha, TEXT, RADIUS, Icon, Badge, Button, Kbd, Tabs, Empty, Toaster, Spinner, Overline, openMenu } from "./ui.js";
import { DiffView } from "./diff.js";
import { CHOICES, SUBJECT, agentsIn, isRow, menuItems, nameOf, newAgentRequest, plainError, recipient, recipientLabel } from "./sendto.js";
import { layoutName } from "./screens.js";

/** The shared key of the repos whose review started a new agent once already: the first one asks first. */
const NEW_OK = "review.new-ok";

// The agents, Home and queue views, followed once per window whichever copies ask (feed.js).
const following = sharing(hub.subscribe);

/**
 * Has the review panel follow an agent rather than the focused terminal: `handle` as the agents view names it (`app:t3`,
 * `claude:<id>`), `cwd` its folder. A view calls it for the agent it shows; `followInReview(null)` follows the focus.
 */
export function followInReview(handle, cwd = null) {
  setShared(SUBJECT, handle ? { handle, cwd } : null);
}

const SCOPES = [
  { value: "uncommitted", label: "Uncommitted" },
  { value: "branch", label: "Branch" },
  { value: "pr", label: "PR" },
];
const MODELS = [["haiku", "Haiku", "fast and cheap"], ["sonnet", "Sonnet", "better on large changes"], ["opus", "Opus", "the most careful"]];
const NOTE = { choice: ["Decision", "flag"], upkeep: ["Upkeep", "wrench"], gap: ["Limitation", "triangle-alert"], divergence: ["Divergence", "git-fork"], untested: ["Untested", "circle-help"] };

function shown() {
  const walk = (node, ok) => {
    if (!node || typeof node !== "object") return null;
    if (node.content?.mod === "cockpit" && node.content?.props?.app === "review") return ok && !node.hidden;
    for (const child of node.children ?? []) {
      const visible = ok && !node.hidden && !child.hidden && (node.type !== "stack" || (node.active ?? node.children?.[0]?.id) === child.id);
      const found = walk(child, visible);
      if (found !== null) return found;
    }
    return null;
  };
  return walk(layout(), true) ?? true;
}

export default class Review {
  /** A file of the diff opens beside the agent, in cockpit's editor; a large file's lines are read once asked for. */
  withOpen(diff) {
    diff.onOpen = (path) => this.repo && openCode(`${this.repo}/${path}`, { terminal: this.terminal ?? undefined });
    diff.onLoad = (path) => {
      this.loaded.add(path);
      // Asked for, it shows: a folded file opens.
      diff.folded.delete(path);
      this.fresh = true;
      this.next = 0;
      // A read already on its way does not hold this one back: it goes as soon as that one lands.
      if (this.loading) this.again = true;
    };
    return diff;
  }

  init(p, cx) {
    const pr = props() ?? p ?? {};
    this.fixed = pr.repo ? { repo: pr.repo, terminal: pr.terminal ?? null } : null;
    this.repo = pr.repo ?? null;
    this.terminal = pr.terminal ?? null;
    this.scope = pr.scope ?? "uncommitted";
    this.mode = "diff";
    this.model = null;
    this.error = null;
    this.loading = false;
    this.next = 0;
    this.step = 0;
    this.width = 480;
    this.toasts = new Toaster();
    this.rows = [];
    this.loaded = new Set();
    this.fresh = false;
    this.sending = false;
    this.sub = following(cx, { views: ["agents"] }, (m, cx) => {
      if (m.view !== "agents") return;
      this.rows = m.value?.agents ?? [];
      cx.notify();
    });
    this.handle = cx.focus_handle();
    this.input = InputState.new({ placeholder: "Leave a comment…" });
    this.input.on("submit", (_e, cx) => {
      this.diff.commit();
      this.handle.focus();
      this.badge();
      cx.notify();
    });
    this.diff = this.withOpen(new DiffView("review", this.input));
    this.walkModel = "haiku";
  }

  close() {
    this.sub?.stop();
  }

  /** The agent to follow: the one a view handed over (`followInReview`), else the focused terminal and its folder. */
  focusFacts() {
    const subject = shared(SUBJECT);
    if (subject?.handle) {
      const row = this.rows.find((r) => isRow(r, subject.handle));
      return { terminal: subject.handle, cwd: subject.cwd ?? row?.cwd ?? null };
    }
    const f = focused();
    const terminal = f?.terminal ?? null;
    return { terminal, cwd: terminal ? f.cwd ?? panes().find((x) => x.terminal === terminal)?.cwd ?? null : null };
  }

  /** Where the focused agent works: its terminal and folder, kept while the focus is on a panel. */
  follow(cx) {
    if (this.fixed) return;
    const { terminal, cwd } = this.focusFacts();
    if (!terminal) return;
    // Only a move of the focus (another agent, or the agent's own cd) changes the repo, not each render.
    if (terminal === this.seen?.terminal && cwd === this.seen?.cwd) return;
    this.seen = { terminal, cwd };
    this.terminal = terminal;
    if (cwd && cwd !== this.repo) {
      this.repo = cwd;
      this.model = null;
      this.error = null;
      this.loaded = new Set();
      this.diff = this.withOpen(new DiffView("review", this.input));
      this.next = 0;
      cx.timer.after(0, (cx) => this.load(cx));
    }
  }

  tick(cx) {
    if (!this.repo || this.loading || Date.now() < this.next || !shown()) return;
    this.load(cx);
  }

  load(cx) {
    if (!this.repo || this.loading) return;
    this.loading = true;
    const scope = this.scope, repo = this.repo;
    cx.spawn(async (cx) => {
      try {
        // A large file asked for is read again in full: the changes did not move, what is sent did.
        const since = this.model?.scope === scope && !this.fresh ? this.model?.hash ?? null : null;
        this.fresh = false;
        const load = this.diff.whole(this.loaded);
        // Which agents work in this repo is git's to say, by each folder's toplevel: the view answers with their handles.
        const agents = this.rows.filter((r) => r.cwd).map((r) => ({ handle: r.terminal, cwd: r.cwd }));
        const m = await hub.view("review", { repo, scope, since, load, agents });
        if (repo === this.repo && scope === this.scope) {
          if (m?.error && !m.files) {
            this.error = m.error;
            this.model = null;
          } else if (m?.same) {
            // The same changes as before an error: the hub answers again, so the error goes.
            this.error = null;
            this.model.walkthrough = m.walkthrough ?? this.model.walkthrough;
            this.model.here = m.here ?? this.model.here;
          } else {
            this.error = null;
            this.model = m;
            this.diff.set(m);
          }
          if (m?.walkthrough && this.model) this.model.walkthrough = m.walkthrough;
          // A walkthrough that finished over the same changes shows its step's hunks alone at once.
          if (this.model) this.applyStep();
          // A choice made before the repo was known is kept now that git named it.
          const pending = this.pendingChoice;
          if (this.model && pending) {
            this.pendingChoice = null;
            if (pending.repo === this.model.repo || pending.repo === repo) this.choose(pending.handle, this.model.repo);
          }
        }
      } catch (e) {
        this.error = String(e?.message ?? e);
      }
      this.loading = false;
      const again = this.again;
      this.again = false;
      this.next = again ? 0 : Date.now() + (this.scope === "pr" ? 20000 : this.model?.walkthrough?.status === "running" ? 2000 : 3000);
      cx.notify();
      if (again) this.load(cx);
    });
  }

  setScope(scope, cx) {
    if (scope === this.scope) return;
    this.scope = scope;
    this.model = null;
    this.error = null;
    this.loaded = new Set();
    this.diff = this.withOpen(new DiffView("review", this.input));
    this.step = 0;
    this.next = 0;
    this.load(cx);
  }

  badge() {
    const n = this.diff.review().length;
    const value = n ? { count: n, tone: "info" } : null;
    if (JSON.stringify(shared("badge.review") ?? null) !== JSON.stringify(value)) setShared("badge.review", value);
  }

  /** The repository reviewed, as git names it. */
  reviewed() {
    return this.model?.repo ?? this.repo;
  }

  /** The handles of the agents working in the reviewed repo, as the review view lists them. */
  here() {
    return this.model?.here ?? [];
  }

  /** Who the comments go to now: the choice kept for this repo, else the agent followed, else a new agent. */
  recipient() {
    const chosen = (shared(CHOICES) ?? {})[this.reviewed()] ?? null;
    return recipient(this.rows, this.here(), { chosen, followed: this.terminal });
  }

  /** The menu of who may get the comments: the repo's agents, then a new agent; a window command's takes a click only. */
  openRecipients(clickOnly = false) {
    openMenu(menuItems(this.rows, this.here(), this.recipient(), this.reviewed()), { width: 320, clickOnly });
  }

  /** Keeps who `repo`'s comments go to: an agent's handle, or "new". Every copy hears the command; only the one
   * reviewing that repo keeps it, by git's name for it, once its model says what that is. */
  choose(handle, repo = this.reviewed()) {
    if (!this.model) {
      this.pendingChoice = { handle, repo };
      return;
    }
    if (!repo || repo !== this.reviewed()) return;
    const row = this.rows.find((r) => isRow(r, handle));
    const choices = { ...(shared(CHOICES) ?? {}) };
    choices[repo] = handle === "new" ? "new" : { handle, name: row ? nameOf(row) : handle };
    setShared(CHOICES, choices);
  }

  /** Sends the comments: to an agent through `review send` (deliver: a terminal or a driven session), or to a new agent
   * Start work opens in the repo's checkout with the review as its prompt. */
  submit(cx) {
    const comments = this.diff.review();
    if (!comments.length || this.sending) return;
    const to = this.recipient();
    if (to.kind === "gone") {
      this.toasts.push(cx, `${to.name} is no longer running here: pick who gets the review`, { kind: "error" });
      return;
    }
    if (to.kind === "starting") {
      this.toasts.push(cx, `${to.name} is still starting: the review goes once it runs`, { kind: "info" });
      return;
    }
    if (to.kind === "new" && !this.confirmNew(cx)) return;
    // What goes now: a comment written while it is on its way stays.
    const sending = this.diff.comments.filter((c) => !c.stale);
    const review = { scope: this.scope, repo: this.reviewed(), comments };
    this.sending = true;
    cx.spawn(async (cx) => {
      try {
        let r;
        if (to.kind === "agent") {
          r = await hub.act("review", "send", [to.row.terminal, { ...review, to: nameOf(to.row) }]);
        } else {
          const p = await hub.view("review", { prompt: review });
          const request = p?.error ? null : newAgentRequest(review.repo, p.prompt, Date.now());
          r = request ? await hub.act("start", "work", [request]) : { error: p.error };
          // A new agent is picked once: the next review goes to the agent this one started.
          if (!r.error) setShared(CHOICES, { ...(shared(CHOICES) ?? {}), [review.repo]: { space: request.label, name: "the review agent", at: Date.now() } });
        }
        if (r.error) this.toasts.push(cx, plainError(r.error), { kind: "error" });
        else {
          this.diff.comments = this.diff.comments.filter((c) => !sending.includes(c));
          const note = String(r.note || `Review sent to ${recipientLabel(to)}`);
          this.toasts.push(cx, `${note[0].toUpperCase()}${note.slice(1)}`, {
            kind: "success",
            action: { label: "Restore comments", onClick: () => (this.diff.comments = [...this.diff.comments, ...sending]) },
          });
        }
      } catch (e) {
        this.toasts.push(cx, plainError(e?.message ?? e), { kind: "error" });
      }
      this.sending = false;
      this.badge();
      cx.notify();
    });
  }

  /** The first new agent a repo's review starts asks first: a second Submit within a few seconds starts it. */
  confirmNew(cx) {
    const repo = this.reviewed();
    const ok = shared(NEW_OK) ?? {};
    if (ok[repo]) return true;
    const armed = this.armedNew;
    if (armed?.repo === repo && Date.now() - armed.at < 8000) {
      this.armedNew = null;
      setShared(NEW_OK, { ...ok, [repo]: true });
      return true;
    }
    this.armedNew = { repo, at: Date.now() };
    const name = this.model?.name ?? repo;
    this.toasts.push(cx, `This starts a new agent in ${name} on your comments: Submit again to start it`, { kind: "info" });
    return false;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The walkthrough

  walk() {
    return this.model?.walkthrough ?? { status: "none" };
  }

  steps() {
    const w = this.walk();
    if (w.status !== "done") return [];
    const out = [];
    (w.parts ?? []).forEach((part, p) => part.steps.forEach((s) => out.push({ ...s, part: p, partTitle: part.title })));
    if (w.uncovered?.length) out.push({ id: "uncovered", title: "Not covered", uncovered: true, hunks: w.uncovered.flatMap((u) => u.hunks), text: "", notes: [], weight: "skim" });
    return out;
  }

  applyStep() {
    const steps = this.steps();
    if (this.mode !== "walk" || !steps.length) {
      this.diff.only = null;
      return;
    }
    this.step = Math.max(0, Math.min(steps.length - 1, this.step));
    this.diff.only = new Set(steps[this.step].hunks ?? []);
  }

  generate(cx) {
    cx.spawn(async (cx) => {
      const r = await hub.act("review", "walkthrough", [this.repo, this.scope, this.walkModel]).catch((e) => ({ error: String(e?.message ?? e) }));
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else this.toasts.push(cx, r.note || "Writing the walkthrough", { kind: "info" });
      this.next = 0;
      this.load(cx);
    });
  }

  stopWalk(cx) {
    cx.spawn(async (cx) => {
      await hub.act("review", "stop_walkthrough", [this.repo, this.scope]).catch(() => {});
      this.next = 0;
      this.load(cx);
    });
  }

  setMode(mode, cx) {
    this.mode = mode;
    this.applyStep();
    cx.notify();
  }

  moveStep(by) {
    const steps = this.steps();
    if (!steps.length) return;
    this.step = Math.max(0, Math.min(steps.length - 1, this.step + by));
    this.applyStep();
  }

  copyMarkdown(cx) {
    const w = this.walk();
    if (w.status !== "done") return;
    const lines = [`# ${w.title}`, "", w.summary, ""];
    for (const part of w.parts) {
      lines.push(`## ${part.title}`, part.blurb ? `${part.blurb}\n` : "");
      part.steps.forEach((s) => {
        lines.push(`**${s.title}**${s.weight === "key" ? " (key change)" : s.weight === "skim" ? " (context)" : ""}`, "", s.text, "");
        s.notes.forEach((n) => lines.push(`> ${NOTE[n.kind]?.[0] ?? n.kind}: ${n.text}`, ""));
      });
    }
    if (w.uncovered?.length) lines.push("## Not covered", ...w.uncovered.map((u) => `- \`${u.path}\`${u.note ? `: ${u.note}` : ""}`));
    cx.write_to_clipboard(lines.join("\n"));
    this.toasts.push(cx, "Copied the walkthrough as Markdown", { kind: "success" });
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys and commands

  command(c, cx, key = null) {
    if (c === "focus" || c === "show:review") this.handle.focus();
    const [scope, action, ...rest] = c.split(":");
    const arg = rest.join(":");
    if (scope === "review") {
      // Who gets the comments, and sending them, come from the person: a window command only opens the menu of who.
      if (action === "to" && arg && take(c, key)) {
        const [handle, repo] = arg.split("|");
        this.choose(handle, repo || this.reviewed());
      } else if (action === "to") this.openRecipients(!take(c, key));
      else if (action === "follow") followInReview(arg || null);
      else if (action === "open" && arg.startsWith("/")) {
        // Another view (the palette, show's diff) asks for a repo's review: it stays on it until the focus moves.
        this.repo = arg;
        this.seen = this.focusFacts();
        this.model = null;
        this.error = null;
        this.diff = this.withOpen(new DiffView("review", this.input));
        this.next = 0;
        this.load(cx);
      } else if (action === "scope") this.setScope(arg, cx);
      else if (action === "mode") this.setMode(arg, cx);
      else if (action === "submit" && take(c, key)) this.submit(cx);
    }
    cx.notify();
  }

  key(e, cx) {
    const key = e.keystroke;
    // A key pressed from outside the window (a sandbox's test) types into the open composer, which takes no other.
    if (e.injected && this.diff.composer && key !== "escape") {
      if (key === "enter") {
        this.diff.commit();
        this.badge();
      } else {
        // A character, never a key's name: "shift-a" types "A", "tab" types nothing.
        const typed = key === "space" ? " " : key.length === 1 ? key : /^shift-.$/.test(key) ? key.slice(-1).toUpperCase() : "";
        this.input.set_value(this.input.value() + typed);
      }
      return true;
    }
    if (this.diff.key(e)) return true;
    // While a comment is being written, letters are its text: no shortcut takes them.
    if (this.diff.composer) return false;
    switch (key) {
      case "1": case "2": case "3": this.setScope(SCOPES[Number(key) - 1].value, cx); return true;
      case "w": this.setMode(this.mode === "walk" ? "diff" : "walk", cx); return true;
      case "shift-s": case "ctrl-enter": this.submit(cx); return true;
      case "shift-t": this.openRecipients(); return true;
      case "left": case "h": this.moveStep(-1); return true;
      case "right": case "l": this.moveStep(1); return true;
      case "r": this.next = 0; this.load(cx); return true;
    }
    return false;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  render(cx) {
    this.follow(cx);
    const k = tokens();
    const narrow = this.width < 560;
    let root = v_flex().id("review-root").size_full().relative().bg(k.background).text_color(k.foreground).track_focus(this.handle)
      .on_size_change((size, cx) => {
        if (Math.abs(size.width - this.width) > 1) {
          this.width = size.width;
          cx.notify();
        }
      })
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) {
          cx.stop_propagation();
          this.badge();
          cx.notify();
        }
      })
      .child(this.header(k, narrow));
    if (!this.repo) root = root.child(Empty({ icon: "git-compare", title: "No changes to review yet", description: "Focus an agent's terminal: its repository's changes show here, to comment line by line and send back." }));
    else if (this.error) root = root.child(Empty({ icon: "alert", title: this.scope === "pr" ? "No pull request to show" : "Could not read the changes", description: this.error, action: Button("review-retry", { label: "Try again", variant: "outline", size: "sm", onClick: (_e, cx) => { this.next = 0; this.load(cx); } }) }));
    else if (!this.model) root = root.child(v_flex().flex_1().items_center().justify_center().gap(8).child(Spinner({ size: 18 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Reading the changes…")));
    else if (!this.diff.files().length) root = root.child(Empty({ icon: "circle-check", title: "Nothing changed", description: this.scope === "uncommitted" ? "The working tree is clean. Try Branch to see what the branch adds." : "No changes in this scope." }));
    else root = root.child(this.mode === "walk" ? this.walkthrough(k, cx, narrow) : this.diff.render(cx, { narrow })).child(this.footer(k, narrow));
    const toasts = this.toasts.render();
    return toasts ? root.child(toasts) : root;
  }

  header(k, narrow) {
    const m = this.model;
    const about = m?.about ?? {};
    const name = m?.name ?? (this.repo ? this.repo.split("/").pop() : "Review");
    const facts = [about.branch, this.scope === "branch" && about.base ? `→ ${about.base}` : null, this.scope === "branch" && about.commits !== undefined ? `${about.commits} commit${about.commits === 1 ? "" : "s"}` : null].filter(Boolean).join(" ");
    const top = h_flex().h(44).flex_none().px(12).gap(8).items_center()
      .child(h_flex().size(24).flex_none().rounded(RADIUS.md).items_center().justify_center().bg(alpha(k.ring, 0.14)).child(Icon("git-compare", { size: 14, color: k.ring })))
      .child(v_flex().flex_1().min_w(0)
        .child(div().truncate().text_size(TEXT.base).font_semibold().child(name))
        .child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(facts || (this.repo ?? ""))))
      .when(Boolean(m?.files?.length), (e) => e.child(h_flex().gap(6).items_center().text_size(TEXT.xs).font_family("JetBrains Mono")
        .child(div().text_color(k.mutedFg).child(`${m.files.length} file${m.files.length === 1 ? "" : "s"}`))
        .child(div().text_color(k.success).child(`+${m.added}`)).child(div().text_color(k.destructive).child(`−${m.removed}`))))
      .when(Boolean(this.loading), (e) => e.child(Spinner({ size: 12, color: k.mutedFg })))
      .child(Button("review-refresh", { icon: "refresh-cw", size: "icon-sm", variant: "ghost", tooltip: "Read again (r)", onClick: (_e, cx) => { this.next = 0; this.load(cx); } }));
    const pr = about.pr;
    const bar = h_flex().h(40).flex_none().px(12).gap(8).items_center().border_b_1().border_color(k.border)
      .child(Tabs("review-scope", { items: SCOPES.map((s, i) => ({ ...s, label: narrow && s.value === "uncommitted" ? "Local" : s.label, kbd: String(i + 1) })), value: this.scope, onChange: (v, cx) => { this.setScope(v, cx); cx.notify(); } }))
      .child(div().flex_1())
      .when(Boolean(pr?.number) && !narrow, (e) => e.child(Badge(`#${pr.number}${pr.isDraft ? " draft" : ""}`, "outline", { id: "review-pr", icon: "git-pull-request", tooltip: pr.title, onClick: () => hub.openUrl(pr.url) })))
      .child(Tabs("review-mode", { items: [{ value: "diff", label: narrow ? "" : "Diff", icon: "file-diff" }, { value: "walk", label: narrow ? "" : "Walkthrough", icon: "route" }], value: this.mode, onChange: (v, cx) => this.setMode(v, cx) }));
    return v_flex().flex_none().border_b_1().border_color(k.border).child(top).child(bar);
  }

  footer(k, narrow) {
    const n = this.diff.review().length;
    const to = this.recipient();
    const who = recipientLabel(to);
    // A narrow panel keeps the To button to a few words, so Clear and Submit stay in view; the tooltip names it whole.
    const max = narrow ? (n > 0 ? 14 : 22) : 26;
    const cut = (text, m) => (text.length > m ? `${text.slice(0, m - 1)}…` : text);
    const short = to.kind === "gone" ? `${cut(to.name, max - 6)} (gone)` : cut(who, max);
    const none = to.kind === "new" && !agentsIn(this.rows, this.here()).length;
    const hints = this.diff.composer ? [["⏎", "add"], ["esc", "cancel"]] : [["j/k", "line"], ["c", "comment"], ["e", "edit file"], ["s", "split"], ["w", "walkthrough"]];
    // Cockpit puts its keys on the buttons; Cockpit Chat keeps them in the tooltips alone.
    const chat = layoutName() === "cockpit-chat";
    const withKey = (text, key) => (chat ? `${text}  ${key}` : text);
    return h_flex().h(48).flex_none().px(12).gap(10).items_center().border_t_1().border_color(k.border)
      .when(!narrow && !chat, (e) => e.children(hints.map(([key, what]) => h_flex().flex_none().gap(4).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(Kbd(key)).child(what))))
      .child(div().flex_1())
      .when(n > 0, (e) => e.child(Button("review-clear", { label: "Clear", icon: narrow ? "trash" : undefined, size: narrow ? "icon-sm" : "sm", variant: "ghost", tooltip: narrow ? "Clear the comments" : undefined, onClick: (_e, cx) => {
        const kept = this.diff.comments;
        this.diff.comments = [];
        this.toasts.push(cx, `${kept.length} comment${kept.length === 1 ? "" : "s"} cleared`, { action: { label: "Undo", onClick: () => (this.diff.comments = kept) } });
        this.badge();
      } })))
      .child(Button("review-to", {
        label: `To ${short}`,
        iconRight: "chevron-down",
        variant: to.kind === "gone" ? "destructive" : "outline",
        size: "sm",
        kbd: chat || narrow ? undefined : "⇧T",
        tooltip: none
          ? "No agent works in this repository: the review starts a new one here"
          : to.kind === "gone" ? `${who}: pick who gets the review` : withKey(`${who} gets the comments: pick an agent working here, or a new one`, "⇧T"),
        onClick: () => this.openRecipients(),
      }))
      .child(Button("review-submit", {
        label: to.kind === "new" ? (n ? `Start an agent · ${n}` : "Start an agent") : n ? `Submit review · ${n}` : "Submit review",
        icon: to.kind === "new" ? "plus" : "send",
        size: "sm",
        kbd: chat || narrow ? undefined : "⇧S",
        disabled: !n || this.sending || to.kind === "gone" || to.kind === "starting",
        tooltip: to.kind === "new" ? withKey("Start a new agent here on the comments", "⇧S") : withKey(`Send the comments to ${who} as one message`, "⇧S"),
        onClick: (_e, cx) => this.submit(cx),
      }));
  }

  walkthrough(k, cx, narrow) {
    const w = this.walk();
    if (w.status === "none" || w.status === "failed") return this.generateCard(k, w, narrow);
    if (w.status === "running") {
      return v_flex().flex_1().items_center().justify_center().gap(10)
        .child(Spinner({ size: 20 }))
        .child(div().text_size(TEXT.base).font_medium().child(`Writing the walkthrough with ${w.model}…`))
        .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("It runs in the background: close this, it is waiting when you come back."))
        .child(Button("review-stop", { label: "Stop", variant: "outline", size: "sm", onClick: (_e, cx) => this.stopWalk(cx) }));
    }
    const steps = this.steps();
    const s = steps[this.step] ?? steps[0];
    const list = v_flex().id("review-steps").gap(2).p(8).overflow_y_scrollbar()
      .child(v_flex().gap(4).p(8).child(div().text_size(TEXT.base).font_semibold().child(w.title ?? "Walkthrough")).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(w.summary ?? "")))
      .children(steps.flatMap((st, i) => {
        const head = i === 0 || steps[i - 1].part !== st.part ? [Overline(st.uncovered ? "" : st.partTitle).px(8).pt(10).pb(2)] : [];
        const on = i === this.step;
        return head.concat([h_flex().id(`review-step-${i}`).min_h(32).px(8).py(4).gap(8).items_center().rounded(RADIUS.md).cursor_pointer()
          .when(on, (e) => e.bg(k.selected)).when(!on, (e) => e.hover((x) => x.bg(k.accent)))
          .on_click((_e, cx) => { this.step = i; this.applyStep(); cx.notify(); })
          .child(div().w(18).flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(st.uncovered ? "·" : String(i + 1)))
          .child(div().flex_1().min_w(0).truncate().text_size(TEXT.sm).when(on, (e) => e.font_medium()).child(st.title))
          .when(st.weight === "key", (e) => e.child(Badge("key", "info")))
          .when(Boolean(st.outdated), (e) => e.child(Badge("outdated", "warning")))]);
      }));
    const detail = v_flex().flex_1().min_w(0).min_h(0).h_full()
      .child(v_flex().flex_none().gap(8).p(12).border_b_1().border_color(k.border)
        .child(h_flex().gap(8).items_center()
          .child(Button("review-prev", { icon: "chevron-left", size: "icon-sm", variant: "outline", tooltip: "Previous step (←)", disabled: this.step === 0, onClick: (_e, cx) => { this.moveStep(-1); cx.notify(); } }))
          .child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(`${this.step + 1} / ${steps.length}`))
          .child(Button("review-next", { icon: "chevron-right", size: "icon-sm", variant: "outline", tooltip: "Next step (→)", disabled: this.step >= steps.length - 1, onClick: (_e, cx) => { this.moveStep(1); cx.notify(); } }))
          .child(div().flex_1().min_w(0).truncate().text_size(TEXT.base).font_semibold().child(s?.title ?? ""))
          .when(s?.weight === "skim", (e) => e.child(Badge("context", "secondary")))
          .when(!w.fresh, (e) => e.child(Button("review-regen", { label: narrow ? undefined : "Regenerate", icon: "refresh", size: narrow ? "icon-sm" : "sm", variant: "outline", tooltip: "The changes moved on since this was written", onClick: (_e, cx) => this.generate(cx) })))
          .child(Button("review-copy", { icon: "copy", size: "icon-sm", variant: "ghost", tooltip: "Copy as Markdown", onClick: (_e, cx) => this.copyMarkdown(cx) })))
        .when(Boolean(s?.text), (e) => e.child(div().text_size(TEXT.base).text_color(k.foreground).child(s.text)))
        .when(Boolean(s?.uncovered), (e) => e.children((w.uncovered ?? []).filter((u) => u.note).map((u) => h_flex().gap(6).text_size(TEXT.sm).child(div().font_family("JetBrains Mono").text_color(k.mutedFg).child(u.path)).child(div().child(u.note)))))
        .children((s?.notes ?? []).map((n) => {
          const [label, icon] = NOTE[n.kind] ?? [n.kind, "info"];
          return h_flex().gap(8).p(8).items_start().rounded(RADIUS.md).bg(alpha(k.warning, 0.1)).text_size(TEXT.sm)
            .child(Icon(icon, { size: 14, color: k.warning })).child(div().font_medium().child(label)).child(div().flex_1().min_w(0).child(n.text));
        }))
        .when(Boolean(s?.outdated), (e) => e.child(div().text_size(TEXT.xs).text_color(k.warning).child(`${s.missing} of this step's hunks changed since it was written.`))))
      .child(this.diff.render(cx, { narrow }));
    const cost = w.cost ? ` · $${Number(w.cost).toFixed(3)}` : "";
    if (narrow) return v_flex().flex_1().min_h(0).child(detail).child(div().flex_none().px(12).py(4).text_size(TEXT.xs).text_color(k.dim).child(`${w.model}${cost}`));
    return h_flex().flex_1().min_h(0)
      .child(v_flex().w(250).flex_none().h_full().border_r_1().border_color(k.border).child(list.flex_1()).child(div().flex_none().px(16).py(6).text_size(TEXT.xs).text_color(k.dim).child(`${w.model}${cost}`)))
      .child(detail);
  }

  generateCard(k, w, narrow) {
    const pills = h_flex().gap(6).flex_wrap().children(MODELS.map(([value, label, about]) => {
      const on = value === this.walkModel;
      return h_flex().id(`review-model-${value}`).h(28).px(10).gap(6).items_center().rounded(RADIUS.md).cursor_pointer().text_size(TEXT.sm)
        .border_1().border_color(on ? k.ring : k.border).bg(on ? alpha(k.ring, 0.08) : k.card).hover((s) => s.bg(k.accent))
        .tooltip(about).on_click((_e, cx) => { this.walkModel = value; cx.notify(); })
        .child(div().font_medium().child(label)).when(!narrow, (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(about)));
    }));
    return v_flex().flex_1().items_center().justify_center().p(24).child(
      v_flex().max_w(460).gap(14).p(20).rounded(RADIUS.lg).border_1().border_color(k.border).bg(k.card)
        .child(h_flex().gap(8).items_center().child(Icon("route", { size: 18, color: k.ring })).child(div().text_size(TEXT.lg).font_semibold().child("Walk through these changes")))
        .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("An agent groups the hunks into steps in the order they make sense, explains what each changes, and lists what no step covers. It explains, it does not judge. One background call, no tools, cached by the exact changes."))
        .when(w.status === "failed", (e) => e.child(h_flex().gap(8).p(10).rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).text_size(TEXT.sm).child(Icon("alert", { size: 14, color: k.destructive })).child(div().flex_1().child(w.error ?? "it failed"))))
        .child(pills)
        .child(h_flex().justify_end().child(Button("review-generate", { label: w.status === "failed" ? "Try again" : "Generate", icon: "sparkles", onClick: (_e, cx) => this.generate(cx) }))),
    );
  }
}

