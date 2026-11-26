import assert from "node:assert/strict";
import { test } from "node:test";
import { pickChips, howPick, startPicks, keptPicks, currentOffer, effortName, modeOf, modeChoices, unaskedModes, asksLess, nextMode } from "../mods/cockpit/picks.js";

// An agent's `picks` as the hub sends them: Claude Code's table and Codex's, whose fast mode is a toggle in its terminal.
const LEVELS = ["low", "medium", "high", "xhigh", "max"];
const claude = {
  agent: "Claude Code", models: [{ id: "opus", label: "Opus" }, { id: "sonnet", label: "Sonnet" }, { id: "haiku", label: "Haiku" }, { id: "fable", label: "Fable" }],
  efforts: LEVELS, fast: true, start: { model: true, effort: true, fast: true }, live: ["model", "effort", "fast"],
  applies: { model: "call", effort: "turn", fast: "turn" },
  typed: { model: { command: "/model {model}" }, effort: { command: "/effort {effort}" }, fast: { command: "/fast {fast}" } },
};
const codex = {
  agent: "Codex", models: [{ id: "gpt-acme-1", label: "GPT Acme 1", efforts: ["low", "high"], fast: true }, { id: "gpt-acme-mini", label: "GPT Acme mini", efforts: [], fast: false }],
  efforts: ["low", "medium", "high", "xhigh", "max", "ultra"], fast: true, start: { model: true, effort: true, fast: true }, live: [],
  typed: { model: { command: "/model {model}" }, effort: { command: "/model --reasoning={effort}" }, fast: { byHand: "/fast" } },
};
const plain = { agent: "OpenCode", models: [], efforts: [], fast: false, start: { model: true, effort: false, fast: false }, live: [], typed: { model: null, effort: null, fast: null } };

const byId = (chips) => Object.fromEntries(chips.map((c) => [c.setting, c]));
const commands = (c) => (c.items ?? []).filter((i) => i !== "separator" && !i.info).map((i) => i.command);

test("a driven session offers its model, effort and fast mode, each changed live", () => {
  const live = { key: "claude:1", model: "claude-opus-5-5", effort: "high", fast: null, fast_on: true };
  const chips = byId(pickChips({ picks: claude, status: "working" }, { live }));
  assert.deepEqual(Object.keys(chips), ["model", "effort", "fast"]);
  assert.equal(chips.model.label, "Opus 5.5");
  assert.deepEqual(commands(chips.model), ["pick:model:opus", "pick:model:sonnet", "pick:model:haiku", "pick:model:fable"]);
  assert.equal(chips.model.items.find((i) => i.icon === "check").label, "Opus", "the model in use is ticked");
  assert.equal(chips.effort.label, "High");
  assert.deepEqual(commands(chips.effort), LEVELS.map((l) => `pick:effort:${l}`));
  assert.deepEqual([chips.fast.label, commands(chips.fast)], ["Fast", ["pick:fast:on", "pick:fast:off"]]);
  assert.ok(Object.values(chips).every((c) => !c.disabled && !c.items.some((i) => i.info)), "live: no next-message note");
  assert.match(chips.model.tooltip, /the next model call uses this choice/);
  assert.match(chips.effort.tooltip, /this choice applies by the next turn/);
  assert.match(chips.fast.tooltip, /this choice applies by the next turn/);
});

test("a paused session, or a setting its agent takes only at a start, says it applies from the next message", () => {
  for (const live of [{ model: "claude-opus-5-5", paused: true }, { model: "claude-opus-5-5", restart: true }]) {
    const chips = byId(pickChips({ picks: claude }, { live }));
    assert.equal(chips.effort.items[0].label, "From your next message");
    assert.match(chips.effort.tooltip, /from your next message/);
  }
  assert.equal(howPick("effort", { ...claude, live: ["model"] }, { live: { model: "x" } }), "next");
  assert.equal(howPick("model", { ...claude, live: ["model"] }, { live: { model: "x" } }), "live");
});

test("fast mode asked for but turned down says why", () => {
  const live = { model: "claude-haiku-5-5", fast: true, fast_on: false, fast_why: "model_not_allowed" };
  const fast = byId(pickChips({ picks: claude }, { live })).fast;
  assert.equal(fast.label, "Standard");
  assert.equal(fast.tooltip, "Fast mode is asked for, but this model has no fast mode — this choice applies by the next turn");
});

test("a terminal gets its agent's own command only at rest; one changed by hand is shown, not offered", () => {
  const idle = byId(pickChips({ picks: claude, status: "idle" }, { ran: { model: "claude-sonnet-5-5", effort: "xhigh", fast: false } }));
  assert.equal(idle.model.label, "Sonnet 5.5");
  assert.equal(idle.effort.label, "Extra high");
  assert.equal(idle.fast.label, "Standard");
  assert.match(idle.effort.tooltip, /types \/effort … into its terminal/);
  const working = byId(pickChips({ picks: claude, status: "working" }, { ran: { model: "claude-sonnet-5-5" } }));
  assert.ok(working.model.disabled && working.model.items === null);
  assert.equal(working.effort.tooltip, "Claude Code is working: change it once it rests");
  const asked = byId(pickChips({ picks: claude, status: "idle" }, { ran: { effort: "high" }, asked: { effort: "low" } }));
  assert.equal(asked.effort.label, "Low", "what was typed shows until its transcript says so");
  const cx = byId(pickChips({ picks: codex, status: "idle" }, { ran: { model: "gpt-acme-1", effort: "high" } }));
  assert.equal(cx.model.label, "GPT Acme 1");
  assert.deepEqual(commands(cx.effort), ["pick:effort:low", "pick:effort:high"], "the model's own levels");
  assert.ok(cx.fast.disabled);
  assert.equal(cx.fast.tooltip, "Change it in Codex's terminal: /fast");
  const mini = byId(pickChips({ picks: codex, status: "idle" }, { ran: { model: "gpt-acme-mini" } }));
  assert.ok(mini.effort.disabled && mini.fast.disabled);
  assert.equal(mini.effort.tooltip, "GPT Acme mini has no effort levels");
});

test("an agent with no effort levels or fast mode shows no such dropdown", () => {
  const chips = pickChips({ picks: plain, status: "idle" }, { ran: { model: "acme/large" } });
  assert.deepEqual(chips.map((c) => c.setting), ["model"]);
  assert.ok(chips[0].disabled, "its terminal cannot change it: shown, not offered");
  assert.deepEqual(pickChips({ picks: plain, status: "idle" }, {}), [], "nothing known and nothing offered");
  assert.deepEqual(pickChips({ status: "idle" }, { ran: { model: "claude-opus-5-5" } }).map((c) => [c.label, c.disabled]), [["Opus 5.5", true]], "no picks: the model as before");
});

test("the model in use is the closest offer", () => {
  const listed = [{ id: "default", resolved: "claude-opus-5-5", label: "Default" }, { id: "opus", resolved: "claude-opus-5-5", label: "Opus" }, { id: "haiku", label: "Haiku" }];
  assert.equal(currentOffer(listed, "claude-opus-5-5").id, "opus");
  assert.equal(currentOffer(listed, "default").id, "default");
  assert.equal(currentOffer(listed, "claude-haiku-5-5").id, "haiku");
  assert.equal(currentOffer(listed, ""), null);
  assert.equal(effortName("xhigh"), "Extra high");
});

test("a Start work form offers what the agent takes at start, its own default first", () => {
  const offered = startPicks({ picks: claude });
  assert.deepEqual(offered.models[0], ["", "Its default"]);
  assert.equal(offered.efforts.length, LEVELS.length + 1);
  assert.deepEqual(offered.fast.map(([v]) => v), ["", "on", "off"]);
  assert.deepEqual(startPicks({ picks: plain }), { models: [], efforts: [], fast: [] });
  assert.deepEqual(startPicks({ picks: codex }, "gpt-acme-mini").efforts, [], "a model without levels offers none");
  assert.deepEqual(keptPicks({ picks: codex }, { model: "gpt-acme-mini", effort: "high", fast: "on" }), { model: "gpt-acme-mini", effort: "", fast: "" });
  assert.deepEqual(keptPicks({ picks: claude }, { model: "gpt-acme-1", effort: "max", fast: "on" }), { model: "", effort: "max", fast: "on" }, "another agent's model goes");
  assert.deepEqual(keptPicks({ picks: plain }, { model: "acme/large", effort: "", fast: "" }).model, "acme/large", "typed for an agent that lists none");
});

test("the chat's own fields never take the name of one of its methods", async () => {
  const { readFile } = await import("node:fs/promises");
  const text = await readFile(new URL("../mods/cockpit/chat.js", import.meta.url), "utf8");
  const body = text.slice(text.indexOf("export default class Chat"));
  const methods = new Set([...body.matchAll(/^ {2}(?:async |get )?([a-zA-Z]+)\(/gm)].map((m) => m[1]));
  const fields = new Set([...body.matchAll(/this\.([a-zA-Z]+) = /g)].map((m) => m[1]));
  assert.deepEqual([...fields].filter((f) => methods.has(f)), [], "a field named like a method hides it: the chat's clock calls asked(cx)");
});

// Claude Code's modes as the hub's `picks.modes` list them, `default` named `manual` as its help lists it.
const MODES = [
  { id: "manual", aliases: ["default"], label: "Asks first", icon: "shield-check", loud: false, offered: true, unasked: false, cycle: true },
  { id: "acceptEdits", aliases: [], label: "Edits freely", icon: "triangle-alert", loud: true, offered: true, unasked: false, cycle: true },
  { id: "plan", aliases: [], label: "Plan only", icon: "map", loud: false, offered: true, unasked: false, cycle: true },
  { id: "bypassPermissions", aliases: [], label: "Full access", icon: "triangle-alert", loud: true, offered: true, unasked: true, cycle: false },
  { id: "auto", aliases: [], label: "Auto", icon: "zap", loud: true, offered: false, unasked: true, cycle: false },
  { id: "dontAsk", aliases: [], label: "Never asks", icon: "triangle-alert", loud: true, offered: false, unasked: false, cycle: false },
];

test("the modes come from the agent's picks: the menus, the confirms and ⇧⇥'s cycle read one list", () => {
  const picks = { ...claude, modes: MODES };
  assert.deepEqual(modeChoices(picks).map((m) => m.id), ["manual", "acceptEdits", "plan", "bypassPermissions"], "the offered ones");
  assert.deepEqual(modeChoices(picks, { terminal: true }).map((m) => m.id), ["manual", "acceptEdits", "plan"], "a terminal's ⇧⇥ goes through these");
  assert.equal(modeOf(picks, "default")?.label, "Asks first", "found by an alias: what the session reports");
  assert.equal(modeOf(picks, "yolo"), null);
  assert.deepEqual(unaskedModes(picks), ["bypassPermissions", "auto"]);
  assert.deepEqual(asksLess(picks), ["acceptEdits", "bypassPermissions", "auto", "dontAsk"]);
  assert.equal(nextMode(picks, "default"), "acceptEdits");
  assert.equal(nextMode(picks, "acceptEdits"), "plan");
  assert.equal(nextMode(picks, "plan"), "manual", "back to the first, under the name the help lists");
  assert.equal(nextMode(picks, "bypassPermissions"), "acceptEdits", "a mode outside the cycle starts it again");
  for (const picksOf of [null, {}, { modes: [] }, codex]) {
    assert.deepEqual([modeChoices(picksOf), unaskedModes(picksOf), asksLess(picksOf), nextMode(picksOf, "default"), modeOf(picksOf, "plan")], [[], [], [], null, null], "no modes known");
  }
});

test("a terminal agent's model chip ends with its own picker, opened in its terminal", () => {
  const chips = byId(pickChips({ picks: claude, status: "idle", terminal: "app:t3" }, { ran: { model: "claude-opus-5-5" } }));
  assert.deepEqual(chips.model.items.at(-2), "separator");
  assert.deepEqual(chips.model.items.at(-1), { label: "Open /model in its terminal…", icon: "terminal", command: "chat:mirror:/model" });
  assert.deepEqual(commands(chips.model).slice(0, 4), ["pick:model:opus", "pick:model:sonnet", "pick:model:haiku", "pick:model:fable"], "the catalog's picks stay first");
  const live = byId(pickChips({ picks: claude, status: "working", terminal: "claude:1" }, { live: { key: "claude:1", model: "claude-opus-5-5" } }));
  assert.ok(!live.model.items.some((i) => i?.command?.startsWith("chat:mirror:")), "a driven session has no terminal to open it in");
  const bare = byId(pickChips({ picks: plain, status: "idle", terminal: "app:t4" }, { ran: { model: "acme/large" } }));
  assert.ok(!(bare.model?.items ?? []).some((i) => i?.command?.startsWith("chat:mirror:")), "an agent without a model command offers none");
});
