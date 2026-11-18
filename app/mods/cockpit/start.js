// Start work: a prompt, a ticket or a PR becomes a worktree, a space and an agent, as a dialog over the window (Home's
// New agent, the inbox's Start, the palette), planned again by the hub as you type (`fetch start_work`) with the steps,
// blockers and fixes shown before Start. Then each step shows as it runs (view `start`), with Retry when one fails and a
// Cancel that asks before removing what the start made; opened on a run, it shows that run.

import { div } from "gpui-kit";
import { h_flex, v_flex, InputState } from "gpui-base";
import { closeOverlay, openOverlay, props, shared } from "herdr";
import * as hub from "./hub.js";
import { readInChat } from "./screens.js";
import { tokens, TEXT, RADIUS, Icon, Spinner, Button, Kbd, TextField, Pills, AlertDialog, HubDown, alpha, AgentLogo } from "./ui.js";
import { emptyState, requestOf, applyFix, edited, phase, resolved, stepMark, shortPath, choicesOf, agentOf, takesModel } from "./startform.js";
import { startPicks, keptPicks } from "./picks.js";

const PLAN_AFTER_MS = 250;

/**
 * Esc is the dialog's own while it holds the keyboard, from a field too: a binding deeper than the overlay's, which would
 * close the dialog before its keys hear it, so Esc lets go of a lit choice or closes the confirm first.
 */
export const START_KEYS = [
  { keystroke: "escape", action: "cockpit_start_escape", context: "CockpitStart && start_holds" },
  { keystroke: "escape", action: "cockpit_start_escape", context: "CockpitStart && start_holds > Input" },
];

/** Opens the Start work dialog on a ticket's key, a PR's URL or a prompt, or `{run}` on a start that runs or failed. */
export function openStart(source = "") {
  const given = typeof source === "object" && source ? source : { source };
  openOverlay({ mod: "cockpit", props: { app: "start", source: given.source ?? "", run: given.run ?? "" }, kind: "dialog", modal: true, width: 640, height: 0.86 });
}

export default class Start {
  init(p, cx) {
    const given = p ?? props() ?? {};
    this.s = emptyState(given.source ?? "", given.run || undefined);
    this.closed = false;
    this.plan = null;
    this.checking = !given.run;
    this.asked = 0;
    this.run = null;
    this.reply = null;
    this.started = Boolean(given.run);
    this.confirm = null;
    this.home = "";
    this.sub = null;
    // Cockpit Chat keeps key hints in tooltips; Cockpit shows them on the buttons.
    this.chips = !String(shared("layout") ?? "").startsWith("cockpit-chat");
    this.handle = cx.focus_handle();
    // The keymap is the app's: binding again for the next dialog only repeats the same entries.
    try {
      cx.bind_keys(START_KEYS);
    } catch {}
    // The option or fix the arrows reached (an index into choicesOf), lit only while the dialog itself has the keyboard,
    // and the text field that has it, if one does.
    this.cursor = null;
    this.field = null;
    const field = (key, placeholder) => {
      const state = InputState.new({ placeholder, value: this.s[key] });
      state.on("change", (_e, cx) => this.set(cx, key, state.value()));
      state.on("submit", (_e, cx) => this.start(cx, "key"));
      state.on("focus", (_e, cx) => {
        this.field = state;
        this.cursor = null;
        cx?.notify?.();
      });
      state.on("blur", (_e, cx) => {
        if (this.field === state) this.field = null;
        cx?.notify?.();
      });
      return state;
    };
    this.source = field("source", "A prompt, a ticket's key (ENG-412) or a PR's URL");
    // A window command typed the source (`start:type:`): ⏎ starts nothing until it is emptied; a click on Start does.
    this.typedOutside = false;
    this.base = field("base", "its default branch");
    this.branch = field("branch", "named after the ticket or the prompt");
    this.model = field("model", "its default");
    cx.timer.after(60, () => (this.started ? this.handle : this.source).focus());
    cx.spawn(async (cx) => {
      const install = await hub.install().catch(() => ({}));
      this.home = install?.home ?? "";
      // A remote desk reads another machine's hub, which plans and runs nothing for it.
      this.remote = install?.machine ?? null;
      cx.notify();
    });
    if (this.started) this.follow(cx);
    else this.replan(cx, 0);
  }

  close() {
    this.closed = true;
    this.sub?.stop();
  }

  dismiss() {
    closeOverlay(props().overlay);
  }

  /** `cockpit.start:type:<text>` types the source for checks driven from a shell; only a click on Start starts it. */
  command(c, cx) {
    if (!c.startsWith("start:type:")) return;
    const text = c.slice("start:type:".length);
    this.source.set_value(text);
    this.set(cx, "source", text);
    this.typedOutside = Boolean(text.trim());
    cx.notify();
  }

  /** A key's hint: on the button in Cockpit, in its tooltip in Cockpit Chat. */
  hint(key, words = "") {
    return this.chips ? { kbd: key, ...(words ? { tooltip: words } : {}) } : { tooltip: words ? `${words}  ${key}` : key };
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The plan and the run

  set(cx, key, value) {
    if (key === "source" && !value.trim()) this.typedOutside = false;
    if (this.s[key] === value) return;
    this.cursor = null;
    this.s = edited(this.s, key, value);
    if (key === "agent") this.model.set_value(this.s.model);
    this.replan(cx);
    cx.notify();
  }

  /**
   * Asks the hub for a plan once typing settles; an answer to an older request is dropped. The timer's own context asks:
   * a keystroke's is gone by the time it fires.
   */
  replan(cx, after = PLAN_AFTER_MS) {
    const n = ++this.asked;
    this.checking = true;
    cx.timer.after(after, (cx) => {
      if (n !== this.asked || this.started || this.closed) return;
      cx.spawn(async (cx) => {
        let plan;
        try {
          plan = await hub.fetchDetails("start_work", { request: requestOf(this.s) });
        } catch (e) {
          plan = { ok: false, error: String(e?.message ?? e), down: true, steps: [], blockers: [], notes: [] };
        }
        if (n !== this.asked || this.closed) return;
        // A lit option or fix stays lit on the same label; one the new plan dropped gives the source the keyboard back.
        const lit = this.cursor === null ? null : (choicesOf(this.plan)[this.cursor]?.label ?? null);
        this.plan = plan;
        const still = lit === null ? -1 : choicesOf(plan).findIndex((c) => c.label === lit);
        this.cursor = still >= 0 ? still : null;
        if (lit !== null && still < 0) this.source.focus();
        this.checking = false;
        if (plan.choices?.agents?.some((a) => a.picks?.loading)) {
          cx.timer.after(500, (cx) => {
            if (!this.closed && !this.started && n === this.asked) this.replan(cx, 0);
          });
        }
        cx.notify();
      });
    });
  }

  fix(cx, set) {
    this.cursor = null;
    this.s = applyFix(this.s, set);
    if (set.branch !== undefined) this.branch.set_value(String(set.branch));
    if (set.base !== undefined) this.base.set_value(String(set.base));
    this.replan(cx, 0);
    cx.notify();
  }

  pick(cx, key, value) {
    if (key === "mode" && value === "bypassPermissions" && this.s.mode !== value) {
      this.confirm = {
        title: "Give the agent full access?",
        description: "It runs any command and edits any file without asking first.",
        label: "Full access",
        run: (cx) => {
          this.s = applyFix(edited(this.s, "mode", value), { confirm: true });
          this.replan(cx, 0);
        },
      };
      cx.notify();
      return;
    }
    // A model, an effort level or fast mode the agent or the model picked now does not take goes back to its default.
    if (key === "model" || key === "effort" || key === "fast") {
      const next = { ...this.s, [key]: value };
      const kept = keptPicks(agentOf(this.plan, this.s), next);
      if (kept.model !== this.model.value()) this.model.set_value(kept.model);
      this.s = { ...this.s, ...kept };
      this.replan(cx);
      return cx.notify();
    }
    this.set(cx, key, value);
  }

  at() {
    return phase(this.plan, this.run, { checking: this.checking, started: this.started });
  }

  ready() {
    const at = this.at();
    return !this.remote && (at === "ready" || at === "focus");
  }

  /** Follows the run in the `start` view, and closes the dialog a moment after it is done. */
  follow(cx) {
    this.sub?.stop();
    this.sub = hub.subscribe(cx, { views: [{ name: "start", args: { id: this.s.id } }] }, (m, cx) => {
      if (m.view !== "start") return;
      if (m.value?.run) this.run = m.value.run;
      else if (this.reply?.error) this.run = { state: "failed", steps: this.run?.steps ?? [], error: { step: "start", text: this.reply.error }, retry: true };
      // A session cockpit drives that was open already is brought up in Cockpit Chat, where it is read.
      if (this.run?.state === "done" && this.plan?.session === true && this.run.made?.pane && !this.opened) {
        this.opened = true;
        readInChat(this.run.made.pane);
      }
      if (this.run?.state === "done") cx.timer.after(900, () => this.dismiss());
      cx.notify();
    });
  }

  /** Starts what the form holds; ⏎ (`how` "key") leaves a source a window command typed to a click on Start. */
  start(cx, how = "click") {
    if (!this.ready()) return;
    if (how === "key" && this.typedOutside) return cx.notify();
    this.started = true;
    // The fields go: the dialog itself takes the keyboard, for r, c and Esc.
    this.handle.focus();
    this.run = null;
    this.reply = null;
    this.follow(cx);
    cx.spawn(async (cx) => this.call(cx, "work", [requestOf(this.s)]));
    cx.notify();
  }

  async call(cx, action, args) {
    try {
      const r = await hub.act("start", action, args);
      this.reply = r?.error ? { error: r.error } : { note: r?.note ?? "" };
    } catch (e) {
      this.reply = { error: String(e?.message ?? e) };
    }
    // A start the hub holds no record of: the reply is all there is.
    if (this.reply.error && action !== "cancel" && (!this.run || this.run.state === "running")) {
      this.run = { ...(this.run ?? { steps: [] }), state: "failed", error: { step: "start", text: this.reply.error }, retry: true, cancel: false };
    }
    cx.notify();
    return this.reply;
  }

  retry(cx) {
    this.reply = null;
    if (this.run) this.run = { ...this.run, state: "running", error: null };
    cx.spawn(async (cx) => {
      const r = await this.call(cx, "retry", [this.s.id]);
      // Never recorded, as when the hub could not be reached: the start runs again from the form's request.
      if (r.error && /no such start/.test(r.error)) await this.call(cx, "work", [requestOf(this.s)]);
    });
    cx.notify();
  }

  /** Cancel: before the start, the dialog closes; once it runs or failed, removing what it made is asked first. */
  cancel(cx) {
    const at = this.at();
    if ((at === "running" || at === "failed") && this.run?.cancel) {
      this.confirm = {
        title: "Remove what this start made?",
        description: "Its space closes, and the worktree and branch it made go. A worktree that holds changes is kept.",
        label: "Remove",
        run: (cx) =>
          cx.spawn(async (cx) => {
            await this.call(cx, "cancel", [this.s.id]);
            this.dismiss();
          }),
      };
    } else this.dismiss();
    cx.notify();
  }

  key(e, cx) {
    const k = e.keystroke;
    if (this.confirm) {
      if (k === "escape") this.confirm = null;
      else if (k === "enter") {
        const c = this.confirm;
        this.confirm = null;
        c.run(cx);
      } else return false;
      cx.notify();
      return true;
    }
    const turn = /^ctrl-(shift-)?([rtgomls])$/.exec(k ?? "");
    const field = { r: "repo", t: "here", g: "agent", o: "mode", m: "model", l: "effort", s: "fast" }[turn?.[2]];
    if (turn && !this.started) return (this.cursor = null), this.turn(cx, field, turn[1] ? -1 : 1), true;
    // Options and fixes: ↓ from a field reaches them, ↑/↓ move, ⏎ applies, Esc goes back; 1–9 apply one once the
    // dialog itself holds the keyboard, never while a field types, and none of it while the plan is being checked.
    const root = this.handle.is_focused();
    if (!root) this.cursor = null;
    const choices = this.started || this.checking ? [] : choicesOf(this.plan);
    if (k === "down" && choices.length) {
      this.cursor = this.cursor === null ? 0 : Math.min(this.cursor + 1, choices.length - 1);
      this.handle.focus();
      cx.notify();
      return true;
    }
    if (k === "up" && this.cursor !== null) {
      this.cursor = this.cursor > 0 ? this.cursor - 1 : null;
      if (this.cursor === null) this.source.focus();
      cx.notify();
      return true;
    }
    if (k === "escape" && this.cursor !== null) {
      this.cursor = null;
      this.source.focus();
      cx.notify();
      return true;
    }
    const digit = /^[1-9]$/.test(k ?? "") && root ? choices[Number(k) - 1] : null;
    const reached = k === "enter" && this.cursor !== null ? choices[this.cursor] : null;
    // ⏎ on a lit choice whose plan is being checked again waits for it.
    if (k === "enter" && this.cursor !== null && !reached) return true;
    if (digit || reached) {
      this.cursor = null;
      this.fix(cx, (digit ?? reached).set ?? {});
      this.source.focus();
      return true;
    }
    const at = this.at();
    if (k === "escape") return (at === "running" || at === "failed" || at === "done" ? this.dismiss() : this.cancel(cx)), true;
    // ⏎ starts from a text field only: on the dialog with nothing lit it does nothing, on a button it is the button's.
    if (k === "enter" && this.field && !root && this.ready()) return this.start(cx, e.injected ? "click" : "key"), true;
    if (k === "r" && at === "failed") return this.retry(cx), true;
    if (k === "c" && (at === "running" || at === "failed")) return this.cancel(cx), true;
    return false;
  }

  /** The next (or, by -1, the previous) choice of a row from the keyboard: ⌃R repo, ⌃T worktree, ⌃G agent, ⌃O mode. */
  turn(cx, key, by) {
    const p = this.plan ?? {};
    const c = p.choices ?? {};
    if (key === "here") return this.pick(cx, "here", !this.s.here);
    const agent = agentOf(p, this.s);
    const offered = startPicks(agent, this.s.model);
    const values = (l) => l.map(([v]) => v);
    const list = {
      repo: c.repos ?? [], agent: (c.agents ?? []).map((a) => a.id), mode: (agent?.modes ?? []).map((m) => m.id),
      model: values(offered.models), effort: values(offered.efforts), fast: values(offered.fast),
    }[key];
    if (!list?.length) return;
    const now = ["model", "effort", "fast"].includes(key) ? this.s[key] : (this.s[key] ?? p[key] ?? c[key] ?? list[0]);
    const i = list.indexOf(now);
    this.pick(cx, key, list[i < 0 ? 0 : (i + by + list.length) % list.length]);
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  label(text, k) {
    return div().w(72).flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(text);
  }

  row(text, child, k, keys = null) {
    return h_flex()
      .gap(10)
      .items_center()
      .child(this.label(text, k))
      .child(div().flex_1().min_w(0).child(child))
      .when(Boolean(keys) && this.chips, (e) => e.child(div().flex_none().child(Kbd(keys))));
  }

  /** Fix and option buttons, numbered from `first`: the one the arrows reached is lit, 1–9 pick one. */
  buttons(id, list, first) {
    return h_flex()
      .gap(6)
      .flex_wrap()
      .children(
        list.map((f, j) => {
          const n = first + j + 1;
          const key = n <= 9 ? this.hint(String(n), "↓ then ⏎, or the digit when no field types") : {};
          const lit = this.cursor === first + j;
          return Button(`${id}-${j}`, { label: f.label, variant: lit ? "default" : "outline", size: "sm", ...key, onClick: (_e, cx) => this.fix(cx, f.set ?? {}) });
        }),
      );
  }

  fields(k) {
    const p = this.plan ?? {};
    const c = p.choices ?? {};
    const repo = this.s.repo ?? p.repo ?? "";
    const agent = agentOf(p, this.s);
    const mode = this.s.mode ?? (p.mode || c.mode || "default");
    const repos = (c.repos ?? []).map((r) => [r, r]);
    const agents = (c.agents ?? []).map((a) => [a.id, h_flex().gap(6).items_center().child(AgentLogo(a.icon ?? "agent", { size: 13 })).child(a.installed ? a.name : `${a.name} (not installed)`)]);
    const modes = (agent?.modes ?? []).map((m) => [m.id, m.label]);
    const pr = p.kind === "pr";
    const what = resolved(p);
    const out = [
      v_flex()
        .gap(6)
        .child(this.row("Source", TextField(this.source), k))
        .when(Boolean(what && this.s.source.trim()), (e) => e.child(h_flex().pl(82).gap(6).items_center().child(Icon(pr ? "git-pull-request" : "ticket", { size: 12, color: k.success })).child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(what ?? ""))))
        .when((p.options ?? []).length > 0, (e) => e.child(div().pl(82).child(this.buttons("start-option", p.options ?? [], 0)))),
      this.row("Repo", repos.length ? Pills("start-repo", repos, repo, (v, cx) => this.pick(cx, "repo", v)) : div().text_size(TEXT.sm).text_color(k.dim).child("no checkout in the repos folder"), k, "⌃R"),
      this.row("Worktree", Pills("start-where", [["new", "New worktree"], ["here", "In the checkout"]], this.s.here ? "here" : "new", (v, cx) => this.pick(cx, "here", v === "here")), k, "⌃T"),
    ];
    if (!this.s.here && !pr) {
      const branch = h_flex().gap(8).items_center().child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Branch")).child(TextField(this.branch));
      out.push(h_flex().gap(12).child(div().w(220).flex_none().child(this.row("Base", TextField(this.base), k))).child(div().flex_1().min_w(0).child(branch)));
      const shown = [!this.s.base && p.base ? `Base: ${p.base}` : null, !this.s.branch && p.branch ? `Branch: ${p.branch}` : null].filter(Boolean).join(" · ");
      if (shown) out.push(div().pl(82).text_size(TEXT.xs).text_color(k.dim).child(shown));
    } else if (pr && p.branch) out.push(this.row("Branch", div().text_size(TEXT.sm).text_color(k.mutedFg).child(`${p.branch} (the PR's)`), k));
    out.push(this.row("Agent", Pills("start-agent", agents, agent?.id ?? "", (v, cx) => this.pick(cx, "agent", v)), k, "⌃G"));
    // The model, effort level and fast mode it starts with, as its agent takes them: a model it lists none of is typed.
    const offered = startPicks(agent, this.s.model);
    const pills = (id, list, key) => Pills(id, list, this.s[key] ?? "", (v, cx) => this.pick(cx, key, v));
    if (offered.models.length) out.push(this.row("Model", pills("start-model", offered.models, "model"), k, "⌃M"));
    else if (takesModel(agent)) out.push(this.row("Model", TextField(this.model), k));
    if (offered.efforts.length) out.push(this.row("Effort", pills("start-effort", offered.efforts, "effort"), k, "⌃L"));
    if (offered.fast.length) out.push(this.row("Speed", pills("start-fast", offered.fast, "fast"), k, "⌃S"));
    if (modes.length) out.push(this.row("Mode", Pills("start-mode", modes, mode, (v, cx) => this.pick(cx, "mode", v)), k, "⌃O"));
    return v_flex().gap(10).children(out);
  }

  mark(state, k) {
    const m = stepMark(state);
    if (!m.icon) return div().w(16).flex_none().child(Spinner({ size: 12, color: k.info }));
    const color = { success: k.success, destructive: k.destructive, dim: k.dim }[m.tone] ?? k.mutedFg;
    return div().w(16).flex_none().child(Icon(m.icon, { size: 14, color }));
  }

  /** The plan before Start: each blocker with its fixes first, then its steps and notes; the run's steps after. */
  planEl(k) {
    const p = this.plan;
    const at = this.at();
    const short = (t) => shortPath(t, this.home);
    const line = (key, mark, text, color) => h_flex().id(key).gap(8).items_start().child(mark).child(div().flex_1().min_w(0).text_size(TEXT.sm).text_color(color).child(short(text)));
    if (this.run || this.started) {
      const steps = this.run?.steps ?? (p?.steps ?? []).map((s) => ({ ...s, state: "todo" }));
      const error = this.run?.error?.text ?? this.reply?.error;
      return v_flex()
        .gap(6)
        .when(!this.run, (e) => e.child(h_flex().gap(8).items_center().child(Spinner({ size: 12 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Asking the hub how it goes…"))))
        .children(steps.map((s, i) => line(`run-${i}`, this.mark(s.state, k), s.note ? `${s.text}: ${s.note}` : s.text, s.state === "failed" ? k.destructive : s.state === "skipped" ? k.dim : k.foreground)))
        .when(Boolean(error) && at === "failed", (e) => e.child(h_flex().mt(4).p(10).gap(8).rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).items_start().child(Icon("triangle-alert", { size: 14, color: k.destructive })).child(div().flex_1().text_size(TEXT.sm).text_color(k.destructive).child(short(error ?? "")))))
        .when(at === "done", (e) => e.child(div().text_size(TEXT.sm).text_color(k.success).child(this.reply?.note || "Started")));
    }
    if (!p) return h_flex().gap(8).items_center().child(Spinner({ size: 12 })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Checking the source, the repo and the branch…"));
    if (p.down) return HubDown("start-hub-down", this, p.error.replace(/^error: /, ""), k, { maxW: 100_000, onStarted: (cx) => this.replan(cx, 0) });
    if (p.error) return line("plan-error", Icon("circle-x", { size: 14, color: k.destructive }), p.error, k.destructive);
    // The fixes are numbered after the options the fields drew.
    let first = p.place === "focus" ? 0 : (p.options ?? []).length;
    return v_flex()
      .gap(6)
      .children(
        (p.blockers ?? []).map((b, i) => {
          const from = first;
          first += (b.fixes ?? []).length;
          // No source yet is a hint, not a fault: the one way on without one is starting in the checkout.
          const quiet = b.step === "empty";
          return v_flex()
            .id(`blocker-${i}`)
            .gap(6)
            .p(10)
            .rounded(RADIUS.md)
            .bg(quiet ? k.muted : alpha(k.destructive, 0.08))
            .child(h_flex().gap(8).items_start().child(Icon(quiet ? "info" : "circle-x", { size: 14, color: quiet ? k.mutedFg : k.destructive })).child(div().flex_1().min_w(0).text_size(TEXT.sm).text_color(quiet ? k.mutedFg : k.destructive).child(quiet ? b.text : `${b.step}: ${short(b.text)}`)))
            .when((b.fixes ?? []).length > 0, (e) => e.child(div().pl(22).child(this.buttons(`fix-${i}`, b.fixes, from))));
        }),
      )
      .children((p.steps ?? []).map((s, i) => line(`step-${i}`, div().w(16).flex_none().child(h_flex().size(16).rounded(8).bg(k.muted).text_size(10).text_color(k.mutedFg).items_center().justify_center().child(String(i + 1))), s.text, k.foreground)))
      .children((p.notes ?? []).map((n, i) => line(`note-${i}`, Icon("info", { size: 14, color: k.dim }), n, k.mutedFg)));
  }

  footer(k) {
    const at = this.at();
    const buttons = [];
    const removable = Boolean(this.run?.cancel);
    if (at === "failed") {
      if (removable) buttons.push(Button("start-remove", { label: "Cancel and remove", variant: "outline", ...this.hint("c"), onClick: (_e, cx) => this.cancel(cx) }));
      buttons.push(Button("start-keep", { label: "Keep it", variant: "outline", ...this.hint("esc", "Leave it as it is: the inbox shows it"), onClick: () => this.dismiss() }));
      buttons.push(Button("start-retry", { label: "Retry", icon: "rotate-ccw", ...this.hint("r"), onClick: (_e, cx) => this.retry(cx) }));
    } else if (at === "running") {
      if (removable) buttons.push(Button("start-remove", { label: "Cancel and remove", variant: "outline", ...this.hint("c"), onClick: (_e, cx) => this.cancel(cx) }));
      buttons.push(Button("start-hide", { label: "Hide", ...this.hint("esc", "The start goes on: the inbox shows it until it ends"), onClick: () => this.dismiss() }));
    } else if (at === "done" || at === "cancelled") {
      buttons.push(Button("start-close", { label: "Close", ...this.hint("esc"), onClick: () => this.dismiss() }));
    } else {
      buttons.push(Button("start-cancel", { label: "Cancel", variant: "outline", ...this.hint("esc"), onClick: () => this.dismiss() }));
      const label = at === "focus" ? "Bring it into view" : at === "blocked" ? "Can't start yet" : at === "checking" ? "Checking…" : "Start";
      buttons.push(Button("start-go", { label, icon: "play", ...(this.typedOutside ? { tooltip: "Typed from outside the window: click to start it" } : this.hint("⏎")), disabled: !this.ready(), onClick: (_e, cx) => this.start(cx) }));
    }
    const hint = this.typedOutside && (at === "ready" || at === "focus") ? "Typed from outside the window: ⏎ does not start it, Start does." : this.run?.setup ? "The setup runs in its space; the agent starts once it succeeds." : at === "running" ? "Each step shows here as it runs." : at === "blocked" ? "Fix what is marked, or change the choices above." : "";
    return h_flex().gap(8).items_center().child(div().flex_1().min_w(0).truncate().text_size(TEXT.xs).text_color(k.dim).child(hint)).children(buttons);
  }

  /** The AlertDialog as Cockpit Chat draws it: its keys in the buttons' tooltips, not on them. */
  quietConfirm(id, { title, description, confirmLabel, onConfirm, onCancel }) {
    const k = tokens();
    return div()
      .id(id)
      .absolute()
      .inset_0()
      .bg("#00000066")
      .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
      .child(
        v_flex()
          .size_full()
          .items_center()
          .justify_center()
          .child(
            v_flex()
              .w(420)
              .p(24)
              .gap(16)
              .rounded(RADIUS.lg)
              .border_1()
              .border_color(k.border)
              .bg(k.popover)
              .shadow_lg()
              .child(v_flex().gap(6).child(div().text_size(TEXT.lg).font_semibold().child(title)).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(description)))
              .child(
                h_flex()
                  .justify_end()
                  .gap(8)
                  .child(Button(`${id}-cancel`, { label: "Cancel", variant: "outline", tooltip: "Cancel  esc", onClick: onCancel }))
                  .child(Button(`${id}-ok`, { label: confirmLabel, variant: "destructive", tooltip: `${confirmLabel}  ⏎`, onClick: onConfirm })),
              ),
          ),
      );
  }

  render() {
    const k = tokens();
    const p = this.plan;
    const started = this.started || Boolean(this.run);
    const name = p?.title || this.run?.title;
    const title = started && name ? `Start work · ${name}` : "Start work";
    return v_flex()
      .id("start-dialog")
      .size_full()
      .bg(k.popover)
      .track_focus(this.handle)
      // A menu opened from the dialog has the keyboard while it is up: its Esc is its own.
      .key_context(this.handle.is_focused() || this.field ? "CockpitStart start_holds" : "CockpitStart")
      .on_action("cockpit_start_escape", (_e, cx) => (this.key({ keystroke: "escape", key: "escape" }, cx), cx.notify()))
      .on_key_down((e, cx) => {
        if (!this.key(e, cx)) return;
        cx.stop_propagation();
        cx.notify();
      })
      .child(
        v_flex()
          .flex_1()
          .min_h(0)
          .id("start-body")
          .overflow_y_scrollbar()
          .p(20)
          .gap(16)
          .child(v_flex().gap(4).child(div().truncate().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(title)).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("A prompt, a ticket or a PR becomes a worktree, a space and an agent.")))
          .when(Boolean(this.remote), (e) => e.child(div().p(12).rounded(RADIUS.md).bg(k.muted).text_size(TEXT.sm).text_color(k.mutedFg).child(`Start work runs on ${this.remote}'s own cockpit: this window only reads it. Start it from a window on ${this.remote}.`)))
          .when(!this.remote && !started && p?.place !== "focus", (e) => e.child(this.fields(k)))
          .when(!this.remote && !started && p?.place === "focus", (e) => e.child(this.row("Source", TextField(this.source), k)))
          .when(!this.remote, (e) => e.child(div().h(1).bg(k.border)).child(this.planEl(k))),
      )
      .child(div().px(20).py(14).border_t_1().border_color(k.border).child(this.footer(k)))
      .when(Boolean(this.confirm), (e) =>
        e.child(
          (this.chips ? AlertDialog : this.quietConfirm.bind(this))("start-confirm", {
            title: this.confirm.title,
            description: this.confirm.description,
            confirmLabel: this.confirm.label,
            destructive: true,
            onConfirm: (_e, cx) => {
              const c = this.confirm;
              this.confirm = null;
              c.run(cx);
              cx.notify();
            },
            onCancel: (_e, cx) => ((this.confirm = null), cx.notify()),
          }),
        ),
      );
  }
}
