// Cockpit Chat's empty box is Start work: with no agent to read, what is typed there is a prompt, a ticket's key or a
// PR's URL, the chips under it pick the repo, the worktree and the agent, and ⏎ starts it through `start_work`.

import { div } from "gpui-kit";
import { h_flex, v_flex } from "gpui-base";
import * as hub from "./hub.js";
import { TEXT, RADIUS, Icon, Spinner, Button, HubDown } from "./ui.js";
import { emptyState, requestOf, applyFix, edited, phase, resolved, stepMark, shortPath, choicesOf, agentOf } from "./startform.js";
import { startPicks, keptPicks, effortName } from "./picks.js";

/** How long typing settles before the hub plans what the box holds. */
const PLAN_AFTER_MS = 300;

/** The start a chat with no agent makes from its box, and what it draws in place of the conversation. */
export class ChatStart {
  constructor() {
    this.s = emptyState("");
    this.plan = null;
    this.checking = false;
    this.asked = 0;
    this.askedAt = 0;
    // When what was typed last is planned, once typing settles.
    this.due = 0;
    this.catalogDue = 0;
    // The start once ⏎ sent it, as the `start` view shows it, and why the hub refused it.
    this.run = null;
    this.error = null;
    this.sub = null;
    this.home = null;
    // ⏎ pressed while the plan of what was typed was still coming: the start it asked for, once that plan allows it.
    this.waiting = null;
    // What follows the run once it made something, and whether "Cancel and remove" waits on a yes.
    this.done = null;
    this.removing = false;
  }

  /** The agent the start runs: the one picked, else the plan's. */
  agent() {
    return agentOf(this.plan, this.s);
  }

  /** Whether the agent is one cockpit drives itself, so it runs as a session of this chat; else in a terminal. */
  driven() {
    return this.agent()?.drives === true;
  }

  /** The request: a fix's `driven` (start it in a terminal) wins over the agent's own way. */
  request() {
    return { driven: this.driven(), ...requestOf(this.s) };
  }

  /** Whether this start runs as a session the chat drives. */
  drives() {
    return this.request().driven === true;
  }

  /**
   * What the box holds, read on the chat's own clock while it has no agent: a change is planned once typing settles,
   * and a plan that never came back is asked for again.
   */
  sync(text, cx, now = Date.now()) {
    if (this.home === null) {
      this.home = "";
      cx.spawn(async (cx) => {
        this.home = (await hub.install().catch(() => null))?.home ?? "";
        cx.notify();
      });
    }
    // A start over stays on show until something new is typed, or for a minute; one going on keeps the box out of it.
    const over = this.run && this.run.state !== "running" && now - (this.overAt ?? now) > 60_000;
    if (this.run && !over && (this.run.state === "running" || !text.trim())) return;
    if (this.run) this.reset(cx, false);
    if (text !== this.s.source) {
      this.s = edited(this.s, "source", text);
      // An answer for what was there before is dropped.
      this.asked += 1;
      this.checking = true;
      this.due = now + PLAN_AFTER_MS;
    } else if ((this.due && now >= this.due) || (!this.checking && this.catalogDue && now >= this.catalogDue) || (!this.plan && !this.checking) || (this.checking && !this.due && now - this.askedAt > 15_000)) {
      this.replan(cx);
    }
  }

  /** Asks the hub for a plan now; an answer to an older request is dropped. */
  replan(cx) {
    const n = ++this.asked;
    this.due = 0;
    this.catalogDue = 0;
    this.checking = true;
    this.askedAt = Date.now();
    cx.spawn(async (cx) => {
      let plan;
      try {
        plan = await hub.fetchDetails("start_work", { request: this.request() });
      } catch (e) {
        plan = { ok: false, error: String(e?.message ?? e), down: true, steps: [], blockers: [], notes: [] };
      }
      if (n !== this.asked || this.run) return;
      this.plan = plan;
      this.checking = false;
      this.catalogDue = plan.choices?.agents?.some((a) => a.picks?.loading) ? Date.now() + 500 : 0;
      // Planned before its agent was known: planned again the way that agent runs.
      if (!plan.error && Boolean(plan.driven) !== this.drives()) return this.replan(cx);
      const waiting = this.waiting;
      this.waiting = null;
      if (waiting) this.start(cx, waiting);
      cx.notify();
    });
  }

  /**
   * A pick from the chips' menus (`chatstart:<what>:<value>`, keyed): the repo, the worktree, the agent, or the model,
   * effort level and fast mode it starts with (an empty value is its own default).
   */
  command(c, cx) {
    if (!c.startsWith("chatstart:")) return;
    const [what, ...rest] = c.slice("chatstart:".length).split(":");
    const value = rest.join(":");
    if (what === "repo") this.s = edited(this.s, "repo", value);
    else if (what === "here") this.s = edited(this.s, "here", value === "here");
    else if (what === "agent") {
      // Another agent runs its own way: a "start it in a terminal" picked for the last one goes.
      const extra = { ...this.s.extra };
      delete extra.driven;
      this.s = edited({ ...this.s, extra }, "agent", value);
    } else if (what === "model" || what === "effort" || what === "fast") {
      const next = { ...this.s, [what]: value };
      this.s = { ...next, ...keptPicks(this.agent(), next) };
    }
    else return;
    this.replan(cx);
  }

  fix(set, cx) {
    this.s = applyFix(this.s, set);
    this.replan(cx);
    cx.notify();
  }

  at() {
    return phase(this.plan, this.run, { checking: this.checking });
  }

  /** Whether ⏎ would start: a plan of what the box holds now, with nothing blocking it. */
  ready() {
    const at = this.at();
    return at === "ready" || at === "focus";
  }

  /**
   * ⏎ in the empty box: starts what it holds. `done(key, cx)` once the terminal or the session it made is there to
   * follow, or with no key once a start that made none (one brought into view) is over; the run stays on show here
   * until its agent has a conversation to draw.
   */
  start(cx, done, text = null) {
    // ⏎ says the typing is over: what the box holds is planned at once.
    if (text !== null && text !== this.s.source && !this.run) {
      this.s = edited(this.s, "source", text);
      this.replan(cx);
    }
    if (this.checking) return (this.waiting = done), false;
    if (!this.ready()) return false;
    const request = this.request();
    this.run = { id: request.id, label: this.plan.label, state: "running", steps: (this.plan.steps ?? []).map((s) => ({ ...s, state: "todo" })) };
    this.error = null;
    this.followed = false;
    this.overAt = null;
    this.done = done;
    this.follow(cx, request.id);
    cx.spawn(async (cx) => {
      let r;
      try {
        r = await hub.act("start", "work", [request], cx);
      } catch (e) {
        r = { error: String(e?.message ?? e) };
      }
      if (r?.error && this.run?.id === request.id) this.error = String(r.error).replace(/^[a-z_]+: /, "");
      cx.notify();
    });
    cx.notify();
    return true;
  }

  /** A new form, keeping the repo, the worktree and the agent picked; planned at once unless `plan` is false. */
  reset(cx, plan = true) {
    this.waiting = null;
    this.sub?.stop();
    this.sub = null;
    this.run = null;
    this.error = null;
    this.removing = false;
    const { repo, here, agent, model, effort, fast } = this.s;
    this.s = { ...emptyState(""), repo, here, agent, model, effort, fast };
    this.plan = null;
    if (plan) this.replan(cx);
  }

  /** Follows the start's run in the `start` view until it is over; a Retry follows it again. */
  follow(cx, id) {
    this.sub?.stop();
    this.sub = hub.subscribe(cx, { views: [{ name: "start", args: { id } }] }, (m, cx) => {
      const run = m.view === "start" ? m.value?.run : null;
      if (!run) return;
      this.run = run;
      // The terminal or the session it made: the chat follows it from there, and the box is a box again. One named after
      // the run said it was done is followed too.
      const made = run.made?.pane ?? null;
      if (made ? this.followed !== made : !this.followed && run.state === "done") {
        this.followed = made ?? true;
        this.done?.(made, cx);
      }
      if (run.state !== "running") {
        this.overAt ??= Date.now();
        this.sub?.stop();
        this.sub = null;
      }
      cx.notify();
    });
  }

  /** Runs the failed step again; a start the hub never recorded, as when it could not be reached, runs again from the box. */
  retry(cx) {
    const id = this.run?.id;
    if (!id) return;
    this.error = null;
    this.removing = false;
    this.overAt = null;
    this.run = { ...this.run, state: "running", error: null };
    this.sub?.stop();
    this.sub = null;
    cx.spawn(async (cx) => {
      const act = (action, args) => hub.act("start", action, args, cx).catch((e) => ({ error: String(e?.message ?? e) }));
      let r = await act("retry", [id]);
      if (r?.error && /no such start/.test(r.error)) r = await act("work", [this.request()]);
      if (this.run?.id !== id) return;
      // Followed once the hub has it running again: its view then shows the retried run, never the failure before it.
      this.follow(cx, id);
      if (r?.error) this.error = String(r.error).replace(/^[a-z_]+: /, "");
      cx.notify();
    });
    cx.notify();
  }

  /** Cancel and remove: the start's space, worktree and branch go once the person says so; a worktree with changes stays. */
  cancel(cx, sure = false) {
    const id = this.run?.id;
    if (!id || !this.run?.cancel) return;
    if (!sure) return (this.removing = true), cx.notify();
    this.removing = false;
    cx.spawn(async (cx) => {
      const r = await hub.act("start", "cancel", [id], cx).catch((e) => ({ error: String(e?.message ?? e) }));
      if (r?.error) this.error = String(r.error).replace(/^[a-z_]+: /, "");
      else this.reset(cx);
      cx.notify();
    });
    cx.notify();
  }

  close() {
    this.sub?.stop();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing: monochrome, as the chat is, its keys in tooltips

  /** The where and agent chips, built with the chat's own `picker(id, {label, icon, logo, tooltip, items})`. */
  chips(picker) {
    const p = this.plan;
    const c = p?.choices;
    if (!c || this.run) return [];
    const repo = this.s.repo ?? p.repo ?? c.repo ?? "";
    const here = this.s.here;
    const agent = this.agent();
    const out = [];
    if ((c.repos ?? []).length) {
      out.push(
        picker("chat-start-where", {
          label: `${repo || "Pick a repo"} · ${here ? "its checkout" : "new worktree"}`,
          icon: "folder",
          tooltip: "Where it works: a repo of your repos folder, in a worktree of its own or in its checkout",
          items: [
            ...c.repos.map((r) => ({ label: r, icon: r === repo ? "check" : null, command: `chatstart:repo:${r}` })),
            "separator",
            { label: "In a new worktree", icon: here ? null : "check", command: "chatstart:here:new" },
            { label: "In the repo's checkout", icon: here ? "check" : null, command: "chatstart:here:here" },
          ],
        }),
      );
    }
    const agents = (c.agents ?? []).filter((a) => a.installed);
    if (agent && agents.length) {
      const how = (a) => (a.drives ? "in this chat" : "in a terminal");
      out.push(
        picker("chat-start-agent", {
          label: agent.name,
          logo: agent.icon ?? "agent",
          icon: this.drives() ? "message-square" : "terminal",
          tooltip: this.drives() ? `${agent.name} runs as a session this chat drives` : `${agent.name} runs in a terminal of its own, and the chat follows it`,
          items: agents.map((a) => ({ label: `${a.name}, ${how(a)}`, logo: a.icon ?? "agent", check: a.id === agent.id, command: `chatstart:agent:${a.id}` })),
        }),
      );
    }
    // What it starts with beside the prompt, where its agent takes it: each its own default until picked.
    const offered = startPicks(agent, this.s.model);
    const menu = (what, list, now) => list.map(([v, label]) => ({ label, icon: v === now ? "check" : null, command: `chatstart:${what}:${v}` }));
    const named = (list, now, empty) => list.find(([v]) => v === now && v)?.[1] ?? empty;
    if (offered.models.length) {
      out.push(picker("chat-start-model", { label: named(offered.models, this.s.model, "Model"), tooltip: "The model it starts with", items: menu("model", offered.models, this.s.model) }));
    }
    if (offered.efforts.length) {
      const label = this.s.effort ? effortName(this.s.effort) : "Effort";
      out.push(picker("chat-start-effort", { label, icon: "gauge", tooltip: "How hard it thinks before it answers", items: menu("effort", offered.efforts, this.s.effort) }));
    }
    if (offered.fast.length) {
      const label = named(offered.fast, this.s.fast, "Speed");
      out.push(picker("chat-start-fast", { label, icon: "zap", tooltip: "Fast mode answers sooner and spends your plan's usage faster", items: menu("fast", offered.fast, this.s.fast) }));
    }
    return out;
  }

  /** In place of the conversation: what ⏎ will do with what the box holds, or how the start goes, then `below`. */
  body(k, { width, gutter, below = null }) {
    return v_flex()
      .flex_1()
      .min_h(0)
      .items_center()
      .justify_center()
      .gap(16)
      .child(
        v_flex()
          .w_full()
          .max_w(width)
          .px(gutter)
          .gap(14)
          .child(
            v_flex()
              .gap(4)
              .child(h_flex().gap(8).items_center().child(Icon("play", { size: 16, color: k.mutedFg })).child(div().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(this.title())))
              .child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(this.about())),
          )
          .child(this.planEl(k)),
      )
      .when(Boolean(below), (e) => e.child(div().w_full().max_w(width).px(gutter).child(below)));
  }

  title() {
    const label = this.run?.label || this.run?.title;
    if (!this.run || !label) return "Start work";
    return { running: `Starting \u201c${label}\u201d`, done: `Started \u201c${label}\u201d` }[this.run.state] ?? `\u201c${label}\u201d did not start`;
  }

  about() {
    if (!this.run) return "Type a prompt, a ticket's key or a PR's link below: it gets a worktree and an agent, and this chat follows it.";
    if (this.followed) return "This chat follows it: its conversation shows here once its agent reports.";
    return "Each step shows here as it runs.";
  }

  line(key, mark, text, color, k) {
    return h_flex().id(key).gap(8).items_start().child(mark).child(div().flex_1().min_w(0).text_size(TEXT.sm).text_color(color ?? k.foreground).child(shortPath(text, this.home)));
  }

  mark(state, k) {
    const m = stepMark(state);
    if (!m.icon) return div().w(16).flex_none().child(Spinner({ size: 12, color: k.mutedFg }));
    return div().w(16).flex_none().child(Icon(m.icon, { size: 14, color: m.tone === "destructive" ? k.foreground : k.mutedFg }));
  }

  /** The run's steps as they go, or the plan: the source as the hub read it, its blockers and fixes, its steps. */
  planEl(k) {
    const p = this.plan;
    if (this.run) {
      const failed = this.run.state === "failed" || Boolean(this.error);
      const why = this.error ?? this.run.error?.text;
      return v_flex()
        .gap(6)
        .children((this.run.steps ?? []).map((s, i) => this.line(`chat-start-run-${i}`, this.mark(s.state, k), s.note ? `${s.text}: ${s.note}` : s.text, s.state === "skipped" ? k.dim : k.foreground, k)))
        .when(failed && Boolean(why), (e) => e.child(this.line("chat-start-error", Icon("circle-x", { size: 14, color: k.foreground }), why ?? "", k.foreground, k)))
        .when(this.removing, (e) =>
          e.child(
            v_flex()
              .id("chat-start-removing")
              .gap(8)
              .p(10)
              .rounded(RADIUS.md)
              .bg(k.muted)
              .child(div().text_size(TEXT.sm).text_color(k.foreground).child("Remove what this start made? Its space closes, and the worktree and branch it made go. A worktree that holds changes is kept."))
              .child(
                h_flex()
                  .gap(8)
                  .child(Button("chat-start-remove-keep", { label: "Keep it", size: "sm", variant: "ghost", onClick: (_e, cx) => ((this.removing = false), cx.notify()) }))
                  .child(Button("chat-start-remove-go", { label: "Remove", size: "sm", variant: "outline", onClick: (_e, cx) => this.cancel(cx, true) })),
              ),
          ),
        )
        .when(!this.removing && (failed || Boolean(this.run.cancel)), (e) =>
          e.child(
            h_flex()
              .gap(8)
              .when(failed, (e) => e.child(Button("chat-start-retry", { label: "Retry", size: "sm", variant: "outline", icon: "rotate-ccw", tooltip: "Run the failed step again", onClick: (_e, cx) => this.retry(cx) })))
              .when(Boolean(this.run.cancel), (e) => e.child(Button("chat-start-remove", { label: "Cancel and remove", size: "sm", variant: "ghost", tooltip: "Close its space, and remove the worktree and branch it made", onClick: (_e, cx) => this.cancel(cx) })))
              .when(failed, (e) => e.child(Button("chat-start-keep", { label: "Keep it", size: "sm", variant: "ghost", tooltip: "The inbox lists it until you retry or cancel it", onClick: (_e, cx) => (this.reset(cx), cx.notify()) }))),
          ),
        );
    }
    if (!p) return this.checking ? h_flex().gap(8).items_center().child(Spinner({ size: 12, color: k.mutedFg })).child(div().text_size(TEXT.sm).text_color(k.mutedFg).child("Checking the repo and the agent…")) : div();
    if (p.down) return HubDown("chat-start-hub-down", this, p.error.replace(/^error: /, ""), k, { onStarted: (cx) => this.replan(cx) });
    if (p.error) return this.line("chat-start-plan-error", Icon("circle-x", { size: 14, color: k.foreground }), p.error, k.foreground, k);
    const what = resolved(p);
    const choice = (f, id) => Button(id, { label: f.label, size: "sm", variant: "outline", onClick: (_e, cx) => this.fix(f.set ?? {}, cx) });
    const options = choicesOf(p).filter((f) => (p.options ?? []).includes(f));
    return v_flex()
      .gap(8)
      .when(Boolean(what && this.s.source.trim()), (e) => e.child(this.line("chat-start-source", Icon(p.kind === "pr" ? "git-pull-request" : "ticket", { size: 14, color: k.mutedFg }), what ?? "", k.foreground, k)))
      .when(options.length > 0, (e) => e.child(h_flex().gap(6).flex_wrap().children(options.map((f, i) => choice(f, `chat-start-option-${i}`)))))
      .children(
        (p.blockers ?? []).map((b, i) => {
          const quiet = b.step === "empty";
          return v_flex()
            .id(`chat-start-blocker-${i}`)
            .gap(6)
            .p(10)
            .rounded(RADIUS.md)
            .bg(k.muted)
            .child(this.line(`chat-start-blocker-text-${i}`, Icon(quiet ? "info" : "circle-x", { size: 14, color: quiet ? k.mutedFg : k.foreground }), quiet ? b.text : `${b.step}: ${b.text}`, quiet ? k.mutedFg : k.foreground, k))
            .when((b.fixes ?? []).length > 0, (e) => e.child(h_flex().pl(24).gap(6).flex_wrap().children(b.fixes.map((f, j) => choice(f, `chat-start-fix-${i}-${j}`)))));
        }),
      )
      .when(this.s.source.trim().length > 0 || p.ok, (e) =>
        e.children((p.steps ?? []).map((s, i) => this.line(`chat-start-step-${i}`, div().w(16).flex_none().text_size(TEXT.xs).text_color(k.dim).child(String(i + 1)), s.text, k.mutedFg, k))),
      )
      .children((p.notes ?? []).map((n, i) => this.line(`chat-start-note-${i}`, Icon("info", { size: 14, color: k.dim }), n, k.mutedFg, k)));
  }
}
