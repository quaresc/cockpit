// What a person picks beside a prompt (a model, an effort level, fast mode) as the hub's `picks` describe an agent, and
// how a change reaches it: the chat's chips and the Start work forms read it. No natives, so tests load it as it is.

const lower = (s) => String(s ?? "").toLowerCase();

/** "claude-opus-5-5" as people read it: "Opus 5.5"; anything else as it is. */
export function modelName(id) {
  const parts = /^claude-([a-z]+)-(\d+)-(\d+)/.exec(String(id ?? ""));
  if (!parts) return String(id ?? "");
  return `${parts[1][0].toUpperCase()}${parts[1].slice(1)} ${parts[2]}.${parts[3]}`;
}

/** An effort level as the chips say it: "xhigh" is "Extra high". */
export function effortName(level) {
  const named = { minimal: "Minimal", low: "Low", medium: "Medium", high: "High", xhigh: "Extra high", max: "Max", ultra: "Ultra" }[level];
  return named ?? (level ? `${level[0].toUpperCase()}${level.slice(1)}` : "");
}

/** The offer the model in use is, the closest first: its own id, then the model an offer resolves to, then its family. */
export function currentOffer(models, model) {
  const m = lower(model);
  if (!m) return null;
  const list = models ?? [];
  return (
    list.find((o) => lower(o.id) === m) ??
    list.find((o) => o.id !== "default" && lower(o.resolved) === m) ??
    list.find((o) => /^[a-z]+$/.test(lower(o.id)) && m.split(/[-.]/).includes(lower(o.id))) ??
    null
  );
}

/** What it says when the agent turned fast mode down, in plain words. */
function fastWhy(why) {
  return (
    {
      model_not_allowed: "this model has no fast mode",
      preference: "your organisation keeps it off",
      extra_usage_disabled: "it needs extra usage, which is off",
      out_of_credits: "it is out of usage credits",
      network_error: "it could not check whether it may",
      not_first_party: "its provider has no fast mode",
      disabled_by_env: "an environment variable turns it off",
    }[why] ?? why
  );
}

/**
 * How a change of `setting` reaches the agent, as its `picks` say: "live" (a driven session takes it now), "next" (from
 * its next message), "typed" (its own command typed into its terminal), "busy" (the same, once it rests), "hand" (only by
 * hand in its terminal), or null where it cannot change. `live` is the driven session's summary, `status` a terminal's.
 */
export function howPick(setting, picks, { live = null, status = "" } = {}) {
  if (!picks) return null;
  if (live) {
    if (live.paused || live.ended || live.restart) return "next";
    return (picks.live ?? []).includes(setting) ? "live" : "next";
  }
  const typed = picks.typed?.[setting];
  if (typed?.byHand) return "hand";
  if (!typed?.command) return null;
  return ["idle", "done", "failed"].includes(status) ? "typed" : "busy";
}

/** The line a terminal is typed for a pick, from its command template. */
function typedLine(picks, setting, value) {
  return String(picks?.typed?.[setting]?.command ?? "").replace(`{${setting}}`, value);
}

/**
 * The chips the chat's box draws for the agent it shows, one per setting its agent offers: `{id, setting, label, icon,
 * tooltip, items, disabled}`, `items` the menu, each pick a `pick:<setting>:<value>` command, and null when `disabled`.
 * `row` is the agents view's row; `live` the driven session's summary; `ran` what a terminal's transcript says it ran with
 * (`model`, `effort`, `fast`); `asked` what was typed for it since, not in its transcript yet; `about` and `limits` what
 * the model chip's tooltip adds.
 */
export function pickChips(row, { live = null, ran = null, asked = {}, about = "", limits = "" } = {}) {
  const picks = row?.picks;
  const name = picks?.agent || "The agent";
  const status = row?.status ?? "";
  const now = {
    model: live?.model || asked.model || ran?.model || "",
    effort: live?.effort || asked.effort || ran?.effort || "",
    fast: live ? (live.fast_on ?? live.fast ?? null) : (asked.fast ?? ran?.fast ?? null),
  };
  const models = picks?.models ?? [];
  const current = currentOffer(models, now.model);
  const out = [];
  const next = { label: "From your next message", icon: "clock", info: true };
  const liveTip = (setting, what) => {
    const when = picks?.applies?.[setting];
    if (when === "call") return `${what} — the next model call uses this choice`;
    if (when === "turn") return `${what} — this choice applies by the next turn`;
    return what;
  };
  const tip = (setting, what, how) =>
    ({
      live: liveTip(setting, what),
      next: `${what} — a change applies from your next message`,
      busy: `${name} is working: change it once it rests`,
      hand: `Change it in ${name}'s terminal: ${picks?.typed?.[setting]?.byHand ?? ""}`,
    })[how] ?? what;
  const chip = (setting, label, icon, what, items, offered) => {
    const how = howPick(setting, picks, { live, status });
    const open = Boolean(offered) && ["live", "next", "typed"].includes(how);
    const typedTip = how === "typed" && open ? `${what} — picking one types ${typedLine(picks, setting, "…")} into its terminal` : null;
    out.push({
      id: `chat-pick-${setting}`,
      setting,
      label,
      icon,
      tooltip: !offered ? what : (typedTip ?? tip(setting, what, how)),
      items: open ? [...(how === "next" ? [next, "separator"] : []), ...items] : null,
      disabled: !open,
    });
  };
  // The model: shown once something is known of it, or its agent offers some.
  if (now.model || (models.length && picks)) {
    const label = /^claude-/.test(now.model) ? modelName(now.model) : (current?.label ?? (now.model || "Model"));
    const what = [about ? `The model it answers with — ${about}` : "The model it answers with", limits].filter(Boolean).join("\n");
    const items = models.map((o) => ({ label: o.label, icon: o === current ? "check" : null, command: `pick:model:${o.id}` }));
    chip("model", label, null, what, items, models.length > 0);
  }
  // Effort: only for an agent that has levels; a model without them says so.
  const levels = current?.efforts ?? picks?.efforts ?? [];
  if ((picks?.efforts ?? []).length) {
    const items = levels.map((l) => ({ label: effortName(l), icon: l === now.effort ? "check" : null, command: `pick:effort:${l}` }));
    const what = levels.length ? "How hard it thinks before it answers" : `${current?.label ?? "This model"} has no effort levels`;
    chip("effort", effortName(now.effort) || "Effort", "gauge", what, items, levels.length > 0);
  }
  // Fast mode: only for an agent that has it; a model without it says so.
  if (picks?.fast) {
    const has = current?.fast !== false;
    const label = now.fast === true ? "Fast" : now.fast === false ? "Standard" : "Speed";
    const turnedDown = live && live.fast === true && live.fast_on === false && live.fast_why ? `Fast mode is asked for, but ${fastWhy(live.fast_why)}` : null;
    const what = !has ? `${current?.label ?? "This model"} has no fast mode` : (turnedDown ?? "Fast mode answers sooner and spends your plan's usage faster");
    const items = [
      { label: "Fast", icon: now.fast === true ? "check" : null, command: "pick:fast:on" },
      { label: "Standard speed", icon: now.fast === false ? "check" : null, command: "pick:fast:off" },
    ];
    chip("fast", label, "zap", what, items, has);
  }
  return out;
}

/**
 * What a Start work form offers for an agent beside its prompt, as its `picks` say: `{models, efforts, fast}`, each a list
 * of [value, label] whose first is the agent's own default (""); a list is empty where the agent takes no such setting at
 * start. `model` is the model picked, whose own effort levels and fast mode count.
 */
export function startPicks(agent, model = "") {
  const p = agent?.picks;
  if (!p) return { models: [], efforts: [], fast: [] };
  const offers = p.start?.model ? (p.models ?? []) : [];
  const offer = offers.find((m) => m.id === model);
  const levels = p.start?.effort ? (offer?.efforts ?? p.efforts ?? []) : [];
  const fast = Boolean(p.start?.fast) && offer?.fast !== false;
  return {
    models: offers.length ? [["", "Its default"], ...offers.map((m) => [m.id, m.label])] : [],
    efforts: levels.length ? [["", "Its default"], ...levels.map((l) => [l, effortName(l)])] : [],
    fast: fast ? [["", "Its default"], ["on", "Fast"], ["off", "Standard"]] : [],
  };
}

/** A form's picks once the agent or the model changed: what the new one does not take goes back to its default. */
export function keptPicks(agent, s) {
  const offered = startPicks(agent, s.model);
  const has = (list, v) => !v || list.some(([x]) => x === v);
  // A model typed by hand stays for an agent that lists none.
  const model = offered.models.length && !has(offered.models, s.model) ? "" : s.model;
  const now = startPicks(agent, model);
  return { model, effort: has(now.efforts, s.effort) ? s.effort : "", fast: has(now.fast, s.fast) ? s.fast : "" };
}
