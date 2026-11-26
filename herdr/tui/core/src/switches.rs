//! What an agent's own `--help` says it takes: the permission modes and effort levels its installed version lists. Read
//! in the background like its models, kept beside them in `~/.cache/cockpit/models.json` (`help`) by its version, and
//! laid over the tables: a mode is offered only when the help lists it, and effort levels come from it where the table
//! has none.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use serde_json::{Value, json};

use crate::agent::Agent;
use crate::models::{self, Askers, Version};

/// The most a help text may take to print.
const LIMIT: Duration = Duration::from_secs(20);
/// The most of a help text read.
const CAP: usize = 1 << 20;

/// Which of an agent's options the help is read for: (id, the options whose choices are its permission modes, the
/// option whose choices are its effort levels). An agent not listed has no help read.
const HELP: &[(&str, &[&str], &str)] = &[
    // Claude Code 2.1.311: `--permission-mode <mode> (choices: "acceptEdits", …)`, `--effort <level> (low, …, max)`.
    ("claude", &["--permission-mode"], "--effort"),
    // Gemini CLI 0.68: `--approval-mode` lists its modes; no effort switch.
    ("gemini", &["--approval-mode"], ""),
    // Codex 0.167: its sandbox and approval policies together are its modes.
    ("codex", &["--sandbox", "--ask-for-approval"], ""),
];

/// What a help text says an agent takes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Help {
    /// Its permission modes, as its help lists them. Empty when none was read.
    pub modes: Vec<String>,
    /// Its effort levels, as its help lists them. Empty when none was read.
    pub efforts: Vec<String>,
}

impl Help {
    /// As kept in the file: `{modes, efforts}`.
    fn json(&self) -> Value {
        json!({"modes": self.modes, "efforts": self.efforts})
    }

    fn of_json(v: &Value) -> Help {
        let list = |k: &str| v[k].as_array().into_iter().flatten().filter_map(|s| s.as_str().filter(|w| word(w)).map(str::to_string)).collect();
        Help { modes: list("modes"), efforts: list("efforts") }
    }
}

/// The options read of the agent's help: (the mode options, the effort option); None for an agent not listed.
fn reads(agent: &Agent) -> Option<(&'static [&'static str], &'static str)> {
    HELP.iter().find(|(id, _, _)| *id == agent.id).map(|(_, modes, effort)| (*modes, *effort))
}

/// The modes and effort levels a help text lists under the given options.
pub fn parse_help(text: &str, mode_options: &[&str], effort_option: &str) -> Help {
    let modes = mode_options.iter().flat_map(|o| choices(text, o)).collect();
    let efforts = if effort_option.is_empty() { Vec::new() } else { choices(text, effort_option) };
    Help { modes, efforts }
}

/// A choice as an agent takes it on its command line: one plain word.
fn word(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphanumeric()) && s.len() <= 40 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
}

/// The values a help text lists for `option`, as `(choices: "a", "b")` or `(a, b, c)` in the option's own paragraph:
/// the lines from the one naming it to the next option. Empty when it lists none.
pub fn choices(text: &str, option: &str) -> Vec<String> {
    let names = |line: &str| line.trim_start().starts_with('-');
    // The option's own line names it first, after a short form at most (`-s, --sandbox <mode>`), never a line mentioning
    // it (`--output-format=stream-json)`).
    let own = |line: &str| {
        let mut words = line.split_whitespace().map(|w| w.trim_end_matches(','));
        let first = words.next().unwrap_or_default();
        first == option || (first.len() == 2 && first.starts_with('-') && words.next() == Some(option))
    };
    let mut lines = text.lines().skip_while(|l| !(names(l) && own(l)));
    let Some(first) = lines.next() else { return Vec::new() };
    let paragraph: String = std::iter::once(first).chain(lines.take_while(|l| !names(l))).map(str::trim).collect::<Vec<_>>().join(" ");
    for (at, _) in paragraph.match_indices('(') {
        let Some(inside) = paragraph[at + 1..].split(')').next() else { continue };
        let listed = inside.trim().strip_prefix("choices:").map(str::trim);
        let items: Vec<String> = listed.unwrap_or(inside).split(',').map(|i| i.trim().trim_matches(['"', '\'']).to_string()).collect();
        if items.len() >= 2 && items.iter().all(|i| word(i)) {
            return items;
        }
    }
    Vec::new()
}

/// Whether this process reads help texts (`models::ask_agents` turns it on with the models).
static ASKING: Askers = Askers::new();

/// What the agent's help lists: as read when last asked, else nothing. A help older than a day is checked again in the
/// background by the agent's version, and read again only once that changed; `ready` is called when it was.
pub fn of(agent: &Agent, ready: fn()) -> Help {
    of_in(&models::path(), agent, crate::seen::now_ms(), models::asks(), ready)
}

fn of_in(file: &Path, agent: &Agent, now: u64, asks: bool, ready: fn()) -> Help {
    let Some((mode_options, effort_option)) = reads(agent) else { return Help::default() };
    let kept = models::load(file);
    let entry = &kept[&agent.id]["help"];
    let help = Help::of_json(entry);
    let key = format!("help:{}", agent.id);
    let start = asks && !models::fresh(file, entry, &key, now) && agent.installed();
    let (_, begun) = ASKING.join(&agent.id, ready, start);
    if begun {
        let (id, file, agent) = (agent.id.clone(), file.to_path_buf(), agent.clone());
        std::thread::spawn(move || {
            let answer = match models::version(&agent, entry_version(&entry_of(&file, &id))) {
                Version::Same => None,
                Version::Changed(version) => Some((read(&agent).map(|text| parse_help(&text, mode_options, effort_option)), version)),
            };
            keep(&file, &id, answer, now);
            for ready in ASKING.finish(&id) {
                ready();
            }
        });
    }
    help
}

fn entry_of(file: &Path, id: &str) -> Value {
    models::load(file)[id]["help"].clone()
}

fn entry_version(entry: &Value) -> Option<String> {
    entry["version"].as_str().map(str::to_string)
}

/// Writes what the help said, or that reading it failed (`Some((None, _))`), or that the version is the one kept
/// (`None`); a failure keeps the help last read.
fn keep(file: &Path, id: &str, answer: Option<(Option<Help>, Option<String>)>, now: u64) {
    let failed = matches!(answer, Some((None, _)));
    models::update(file, id, |entry| {
        if !entry["help"].is_object() {
            entry["help"] = json!({});
        }
        if let Some((Some(help), version)) = &answer {
            entry["help"] = help.json();
            entry["help"]["at"] = json!(now);
            entry["help"]["version"] = version.as_deref().map_or(Value::Null, Value::from);
        }
        entry["help"]["attempted"] = json!(now);
        entry["help"]["failed"] = json!(failed);
    });
    models::tried(file, &format!("help:{id}"), now, failed);
}

/// The agent's help text; None when it failed or took too long.
fn read(agent: &Agent) -> Option<String> {
    let mut cmd = Command::new(agent.path());
    cmd.arg("--help");
    let out = crate::run::output(&mut cmd, LIMIT, CAP, &Default::default()).ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    const CLAUDE_HELP: &str = include_str!("../tests/fixtures/claude-help.txt");

    fn nobody() {}

    #[test]
    fn claude_s_help_lists_its_permission_modes_and_effort_levels() {
        let help = parse_help(CLAUDE_HELP, &["--permission-mode"], "--effort");
        assert_eq!(help.modes, ["acceptEdits", "auto", "bypassPermissions", "manual", "dontAsk", "plan"]);
        assert_eq!(help.efforts, ["low", "medium", "high", "xhigh", "max"]);
        assert_eq!(choices(CLAUDE_HELP, "--output-format"), ["text", "json", "stream-json"], "a list broken over two lines");
        assert!(choices(CLAUDE_HELP, "--model").is_empty(), "an example in prose is no list");
        assert!(choices(CLAUDE_HELP, "--max-budget-usd").is_empty(), "a parenthesis of words is no list");
        assert!(choices(CLAUDE_HELP, "--nope").is_empty());
        assert_eq!(parse_help(CLAUDE_HELP, &["--approval-mode"], ""), Help::default(), "another agent's options");
        let short =
            "  -s, --sandbox <mode>   The policy (choices: \"read-only\", \"workspace-write\")\n  -a, --ask-for-approval <p>  (untrusted, on-request, never)\n";
        assert_eq!(parse_help(short, &["--sandbox", "--ask-for-approval"], "").modes, ["read-only", "workspace-write", "untrusted", "on-request", "never"]);
        let hostile = "  --effort <l>   (low, $(touch x), high)\n  --mode <m>  (choices: \"-p\", \"plan\")\n";
        assert!(
            parse_help(hostile, &["--mode"], "--effort").modes.is_empty() && parse_help(hostile, &[], "--effort").efforts.is_empty(),
            "nothing a shell would act on"
        );
    }

    #[test]
    fn a_help_is_read_once_and_again_only_when_the_agent_s_version_changed() {
        let dir = tempfile::tempdir().unwrap();
        let (version, count) = (dir.path().join("version"), dir.path().join("read"));
        std::fs::write(&version, "1.0.0 (Acme)\n").unwrap();
        let script = format!(
            "#!/bin/sh\nif [ \"$1\" = --version ]; then cat '{}'; exit 0; fi\necho x >> '{}'\nprintf '  --permission-mode <mode>  (choices: \"manual\", \"plan\")\\n  --effort <level>  (low, high)\\n'\n",
            version.display(),
            count.display()
        );
        let bin = dir.path().join("claude");
        std::fs::write(&bin, script).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut agent = crate::agent::builtin().into_iter().find(|a| a.id == "claude").unwrap();
        agent.bin = bin.to_string_lossy().into_owned();
        let file = dir.path().join("models.json");
        assert_eq!(of_in(&file, &agent, 1_000, true, nobody), Help::default(), "nothing until it answered");
        let wait = |want: usize| {
            for _ in 0..200 {
                if std::fs::read_to_string(&count).map(|c| c.lines().count()).unwrap_or(0) >= want && !ASKING.join("claude", nobody, false).0 {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("the help was not read {want} times");
        };
        wait(1);
        let read = of_in(&file, &agent, 2_000, true, nobody);
        assert_eq!((read.modes.clone(), read.efforts.clone()), (vec!["manual".to_string(), "plan".into()], vec!["low".to_string(), "high".into()]));
        assert_eq!(crate::files::load(&file).unwrap()["claude"]["help"]["version"], "1.0.0 (Acme)");
        // A day later the version is checked: unchanged, the help is not read again.
        of_in(&file, &agent, 1_000 + models::KEPT, true, nobody);
        for _ in 0..200 {
            if crate::files::load(&file).unwrap()["claude"]["help"]["attempted"] == 1_000 + models::KEPT {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        assert_eq!(std::fs::read_to_string(&count).unwrap().lines().count(), 1, "the same version: not read again");
        std::fs::write(&version, "2.0.0 (Acme)\n").unwrap();
        of_in(&file, &agent, 1_000 + 2 * models::KEPT, true, nobody);
        wait(2);
        assert_eq!(crate::files::load(&file).unwrap()["claude"]["help"]["version"], "2.0.0 (Acme)", "a new version: read again");
        assert_eq!(of_in(&file, &agent, 1_000 + 2 * models::KEPT + 1, true, nobody).modes, ["manual", "plan"]);
        assert_eq!(of_in(&file, &crate::agent::builtin().into_iter().find(|a| a.id == "pi").unwrap(), 1, true, nobody), Help::default(), "no help read for it");
    }
}
