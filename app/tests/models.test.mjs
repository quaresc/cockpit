import assert from "node:assert/strict";
import { test } from "node:test";

import { modelChoices, modelLabel, asPills, listHint, PILLS } from "../mods/cockpit/models.js";

test("a routine's form offers the models its agent takes, its default first", () => {
  // What the hub lists for Claude Code: a routine's own lightest, then the models Start work and the chat offer.
  const claude = ["", "lightest", "opus", "sonnet", "haiku", "fable"];
  assert.deepEqual(
    modelChoices(claude, ""),
    [["", "Default"], ["lightest", "Lightest"], ["opus", "Opus"], ["sonnet", "Sonnet"], ["haiku", "Haiku"], ["fable", "Fable"]],
    "each named as the pickers name it, none from a list of the view's own",
  );
  assert.equal(asPills(modelChoices(claude, "")), true, "few enough for pills");
  assert.deepEqual([modelLabel("opus"), modelLabel("gpt-acme-1"), modelLabel("acme/large"), modelLabel("")], ["Opus", "gpt-acme-1", "acme/large", "Default"]);
  assert.deepEqual(modelChoices(["acme/large", "acme/small"], "acme/small"), [["", "Default"], ["acme/large", "acme/large"], ["acme/small", "acme/small"]], "another agent's as it lists them");
  assert.deepEqual(modelChoices([], "acme/old"), [["", "Default"], ["acme/old", "acme/old"]], "a model set before stays offered");
  assert.deepEqual(modelChoices([""], ""), [], "nothing to pick but its default: no model row");
  assert.deepEqual(modelChoices(undefined, ""), [], "an agent the hub lists no models for offers none, as the TUI's form");
});

test("a long list is typed, with how many there are and a few names", () => {
  const many = Array.from({ length: 40 }, (_, i) => `acme/model-${i}`);
  assert.equal(asPills(modelChoices(many)), false);
  assert.equal(asPills(modelChoices(many.slice(0, PILLS - 1))), true);
  assert.equal(listHint(["", ...many]), "40 models, such as acme/model-0, acme/model-1, acme/model-2");
  assert.equal(listHint([""]), "");
});
