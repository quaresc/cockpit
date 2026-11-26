//! The permission modes each agent runs in, one table for every surface that names, offers or cycles them: the chat's
//! chip and ⇧⇥, Start work's pills, a routine's mode, `cockpit session modes`. An agent's help (`switches`) says which
//! of them its installed version lists.

use serde_json::{Value, json};

use crate::agent::Agent;

/// One permission mode of one agent: what it is called, how it is drawn, and how it is reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// The agent table's id.
    pub agent: &'static str,
    /// The mode as the agent takes it (`--permission-mode <id>`, `set_permission_mode`).
    pub id: &'static str,
    /// Other names the agent takes or reports for the same mode.
    pub aliases: &'static [&'static str],
    /// What people read: "Asks first", "Edits freely".
    pub label: &'static str,
    /// The chip's icon (a Lucide name).
    pub icon: &'static str,
    /// Whether it lets the agent do more without asking than the default: drawn in the blocked colour, set only from a
    /// menu the person opened or through a confirm.
    pub loud: bool,
    /// Whether the menus offer it; the rest are set on purpose or shown only.
    pub offered: bool,
    /// Whether tools run without asking in it: set only once the person confirms it.
    pub unasked: bool,
    /// Whether the one-key cycle (⇧⇥) goes through it.
    pub cycle: bool,
}

const fn mode(agent: &'static str, id: &'static str, aliases: &'static [&'static str], label: &'static str, icon: &'static str) -> Mode {
    Mode { agent, id, aliases, label, icon, loud: false, offered: false, unasked: false, cycle: false }
}

/// Every agent's modes, in the order the menus show them. An agent not listed has none of its own.
pub const MODES: &[Mode] = &[
    // Claude Code 2.1.311 `--permission-mode`: `manual` is what its help lists for the default; ⇧⇥ cycles the first
    // three; `auto` and `bypassPermissions` run tools unasked; `dontAsk` denies what would ask instead.
    mode("claude", "default", &["manual"], "Asks first", "shield-check").offered().cycle(),
    mode("claude", "acceptEdits", &[], "Edits freely", "triangle-alert").loud().offered().cycle(),
    mode("claude", "plan", &[], "Plan only", "map").offered().cycle(),
    mode("claude", "bypassPermissions", &[], "Full access", "triangle-alert").loud().offered().unasked(),
    mode("claude", "auto", &[], "Auto", "zap").loud().unasked(),
    mode("claude", "dontAsk", &[], "Never asks", "triangle-alert").loud(),
    // Codex 0.167 (developers.openai.com sandbox and approvals): `--sandbox` and `--ask-for-approval` pair up, `--full-auto`
    // and `--yolo` being the second and third pairs; neither is one mode switch, so none cycles.
    mode("codex", "read-only", &["untrusted"], "Asks first", "shield-check").offered(),
    mode("codex", "workspace-write", &["on-request", "full-auto"], "Edits freely", "triangle-alert").loud().offered(),
    mode("codex", "danger-full-access", &["never", "yolo"], "Full access", "triangle-alert").loud().offered().unasked(),
    // Gemini CLI 0.68 `--approval-mode` (geminicli.com configuration): plan reads only; yolo approves everything.
    mode("gemini", "default", &[], "Asks first", "shield-check").offered(),
    mode("gemini", "auto_edit", &[], "Edits freely", "triangle-alert").loud().offered(),
    mode("gemini", "plan", &[], "Plan only", "map").offered(),
    mode("gemini", "yolo", &[], "Full access", "triangle-alert").loud().offered().unasked(),
    // Qwen Code 0.27 `--approval-mode` (qwenlm.github.io configuration): the same shape, its edits mode hyphenated.
    mode("qwen", "default", &[], "Asks first", "shield-check").offered(),
    mode("qwen", "auto-edit", &[], "Edits freely", "triangle-alert").loud().offered(),
    mode("qwen", "plan", &[], "Plan only", "map").offered(),
    mode("qwen", "yolo", &[], "Full access", "triangle-alert").loud().offered().unasked(),
];

/// The agent whose words the table's `mode` column takes (`{mode}`, as `default`, `acceptEdits`, `plan`): an agent with
/// that column and no modes of its own runs in them.
const DEFAULT_WORDS: &str = "claude";

/// The id whose modes `agent` runs in: its own where the table lists them, else `DEFAULT_WORDS` where its `mode`
/// column is filled, else its own, which has none.
pub fn words_of(agent: &Agent) -> &str {
    if agent.mode.is_some() && of_agent(&agent.id).next().is_none() { DEFAULT_WORDS } else { &agent.id }
}

impl Mode {
    const fn loud(mut self) -> Mode {
        self.loud = true;
        self
    }

    const fn offered(mut self) -> Mode {
        self.offered = true;
        self
    }

    const fn unasked(mut self) -> Mode {
        self.unasked = true;
        self
    }

    const fn cycle(mut self) -> Mode {
        self.cycle = true;
        self
    }

    /// Whether `name` is this mode's id or one of its aliases.
    pub fn is(&self, name: &str) -> bool {
        self.id == name || self.aliases.contains(&name)
    }

    /// Its id as the agent's help lists it: an alias when the help lists that and not the id, else the id.
    pub fn listed(&self, known: &[String]) -> &'static str {
        if known.is_empty() || known.iter().any(|k| k == self.id) {
            return self.id;
        }
        self.aliases.iter().copied().find(|a| known.iter().any(|k| k == a)).unwrap_or(self.id)
    }
}

/// The agent's modes, in menu order.
pub fn of_agent(agent: &str) -> impl Iterator<Item = &'static Mode> {
    MODES.iter().filter(move |m| m.agent == agent)
}

/// The ids of the agent's modes.
pub fn ids(agent: &str) -> Vec<&'static str> {
    of_agent(agent).map(|m| m.id).collect()
}

/// The modes the agent's one-key cycle goes through.
pub fn cycle(agent: &str) -> Vec<&'static str> {
    of_agent(agent).filter(|m| m.cycle).map(|m| m.id).collect()
}

/// The mode after `now` in the agent's cycle, which a mode outside it starts again; None for an agent without one.
pub fn next(agent: &str, now: &str) -> Option<&'static str> {
    let cycle: Vec<&Mode> = of_agent(agent).filter(|m| m.cycle).collect();
    let at = cycle.iter().position(|m| m.is(now)).unwrap_or(0);
    cycle.get((at + 1) % cycle.len().max(1)).map(|m| m.id)
}

/// The modes in which the agent's tools run without asking.
pub fn unasked(agent: &str) -> Vec<&'static str> {
    of_agent(agent).filter(|m| m.unasked).map(|m| m.id).collect()
}

/// The agent's mode named `id`, by its id or an alias.
pub fn of(agent: &str, id: &str) -> Option<&'static Mode> {
    of_agent(agent).find(|m| m.is(id))
}

/// `mode` as the agent's help lists it (`known`, the ids its `--permission-mode` takes): the alias the help lists when it
/// lists that and not the id, else `mode` as given.
pub fn listed(agent: &str, mode: &str, known: &[String]) -> String {
    of(agent, mode).map(|m| m.listed(known).to_string()).unwrap_or_else(|| mode.to_string())
}

/// The agent's modes as the views read them, `[{id, aliases, label, icon, loud, offered, unasked, cycle}]`: `id` as the
/// help lists it, and `offered` only for a mode the help lists when `known` names any.
pub fn json(agent: &str, known: &[String]) -> Value {
    let listed = |m: &Mode| known.is_empty() || known.iter().any(|k| m.is(k));
    Value::Array(
        of_agent(agent)
            .map(|m| {
                let id = m.listed(known);
                let aliases: Vec<&str> = std::iter::once(m.id).chain(m.aliases.iter().copied()).filter(|a| *a != id).collect();
                json!({"id": id, "aliases": aliases, "label": m.label, "icon": m.icon, "loud": m.loud, "offered": m.offered && listed(m), "unasked": m.unasked, "cycle": m.cycle})
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_one_key_cycle_goes_through_the_modes_that_ask_and_never_reaches_full_access() {
        assert_eq!(cycle("claude"), ["default", "acceptEdits", "plan"]);
        assert_eq!(next("claude", "default"), Some("acceptEdits"));
        assert_eq!(next("claude", "acceptEdits"), Some("plan"));
        assert_eq!(next("claude", "plan"), Some("default"));
        assert_eq!(next("claude", "manual"), Some("acceptEdits"), "an alias is the same mode");
        assert_eq!(next("claude", "bypassPermissions"), Some("acceptEdits"), "a mode outside the cycle starts it again");
        for from in ["default", "acceptEdits", "plan", "bypassPermissions", "dontAsk", "auto", ""] {
            assert!(unasked("claude").iter().all(|u| next("claude", from) != Some(u)), "never reached from {from:?}");
        }
        assert_eq!(next("codex", "read-only"), None, "no measured cycle");
        assert_eq!(next("acme", "x"), None);
        assert_eq!(unasked("claude"), ["bypassPermissions", "auto"]);
        assert_eq!(unasked("gemini"), ["yolo"]);
        assert!(ids("acme").is_empty());
    }

    #[test]
    fn a_mode_is_found_by_its_id_or_an_alias_and_named_as_the_help_lists_it() {
        assert_eq!(of("claude", "manual").map(|m| m.label), Some("Asks first"));
        assert_eq!(of("codex", "yolo").map(|m| m.id), Some("danger-full-access"));
        assert_eq!(of("claude", "yolo"), None, "another agent's");
        let help = strs(&["acceptEdits", "auto", "bypassPermissions", "manual", "dontAsk", "plan"]);
        assert_eq!(listed("claude", "default", &help), "manual", "the help lists manual, not default");
        assert_eq!(listed("claude", "plan", &help), "plan");
        assert_eq!(listed("claude", "default", &[]), "default", "no help read yet");
        assert_eq!(listed("claude", "default", &strs(&["default", "plan"])), "default");
        assert_eq!(listed("claude", "whatever", &help), "whatever", "a mode the table lacks goes as given");
    }

    #[test]
    fn the_views_get_every_mode_offered_only_where_the_help_lists_it() {
        let all = json("claude", &[]);
        let ids: Vec<&str> = all.as_array().unwrap().iter().map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["default", "acceptEdits", "plan", "bypassPermissions", "auto", "dontAsk"]);
        assert_eq!(all[0]["aliases"], json!(["manual"]));
        let flag = |i: usize, k: &str| all[i][k].as_bool();
        assert_eq!((flag(0, "offered"), flag(4, "offered"), flag(1, "loud"), flag(3, "unasked")), (Some(true), Some(false), Some(true), Some(true)));
        let help = strs(&["acceptEdits", "manual", "plan"]);
        let some = json("claude", &help);
        let offered: Vec<&str> = some.as_array().unwrap().iter().filter(|m| m["offered"] == true).map(|m| m["id"].as_str().unwrap()).collect();
        assert_eq!(offered, ["manual", "acceptEdits", "plan"], "named as the help lists them, Full access not offered by this version");
        assert_eq!(some[0]["aliases"], json!(["default"]), "the table's id becomes the alias");
        assert_eq!(json("acme", &[]), json!([]));
        let table = crate::agent::builtin();
        let row = |id: &str| table.iter().find(|a| a.id == id).unwrap();
        assert_eq!((words_of(row("claude")), words_of(row("codex")), words_of(row("pi"))), ("claude", "codex", "pi"));
        let mut moded = row("pi").clone();
        moded.mode = Some(vec!["--permission-mode".into(), "{mode}".into()]);
        assert_eq!(words_of(&moded), "claude", "a mode column takes Claude Code's words");
    }
}
