//! `cockpit agent`: the command line that starts, runs, resumes or forks a coding agent, for the plugin's scripts, so
//! none of them names an agent's flags itself.

use cockpit_core::agent::{self, Agent, Headless};

pub const USAGE: &str = "cockpit agent id [--agent <id>] [--use <key>]
    the agent's id: the one given (an id or a program Herdr reports), else the config's agents.<key>, else the default
cockpit agent argv <start|run|resume|fork> [--agent <id>] [--use <key>] [--prompt <text>] [--session <id>] [--model <name>]
                   [--edits] [--dir <folder>]... [--system <text>]
    its command as one shell line: start or resume interactive, run headless (on a fork of --session when given),
    fork interactive. A model it does not take is left out; exits 1 when the agent cannot do it";

#[derive(Default)]
struct Asked {
    agent: Option<String>,
    key: Option<String>,
    prompt: Option<String>,
    session: Option<String>,
    model: Option<String>,
    edits: bool,
    dirs: Vec<String>,
    system: Option<String>,
}

fn parse(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked::default();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        if flag == "--edits" {
            asked.edits = true;
            continue;
        }
        let value = it.next().cloned().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--agent" => asked.agent = Some(value),
            "--use" => asked.key = Some(value),
            "--prompt" => asked.prompt = Some(value),
            "--session" => asked.session = Some(value),
            "--model" => asked.model = Some(value),
            "--dir" => asked.dirs.push(value),
            "--system" => asked.system = Some(value),
            other => return Err(format!("unknown option {other}")),
        }
    }
    Ok(asked)
}

fn agent_of(asked: &Asked) -> Result<Agent, String> {
    match asked.agent.as_deref().filter(|a| !a.is_empty()) {
        Some(id) => agent::get(id).or_else(|| agent::of_program(id)).ok_or_else(|| format!("no agent {id:?}")),
        None => Ok(asked.key.as_deref().map_or_else(agent::default, agent::for_use)),
    }
}

/// The argv for the kind of launch asked.
fn argv(kind: &str, asked: &Asked, a: &Agent) -> Result<Vec<String>, String> {
    let model = asked.model.as_deref().filter(|m| !m.is_empty() && a.takes_model(m));
    let prompt = asked.prompt.as_deref().filter(|p| !p.is_empty());
    let session = || asked.session.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| format!("{kind} needs --session"));
    match kind {
        "start" => Ok(a.start_argv(prompt, model)),
        "run" => {
            let how = Headless {
                model,
                fork: asked.session.as_deref().filter(|s| !s.is_empty()),
                edits: asked.edits,
                dirs: asked.dirs.clone(),
                system: asked.system.as_deref(),
            };
            a.headless(prompt.unwrap_or_default(), &how).ok_or_else(|| {
                if how.fork.is_some() { format!("{} cannot run headless on a fork of a session", a.name) } else { format!("{} has no headless mode", a.name) }
            })
        }
        "resume" => a.resume_with(session()?, prompt).ok_or_else(|| format!("{} cannot resume a session", a.name)),
        "fork" => a.fork_with(session()?, prompt).ok_or_else(|| format!("{} cannot fork a session", a.name)),
        other => Err(format!("{other}? one of start, run, resume, fork")),
    }
}

/// `cockpit agent ...`: what it prints, or why not.
pub fn run(args: &[String]) -> Result<String, String> {
    match args.split_first() {
        Some((what, rest)) if what == "id" => Ok(agent_of(&parse(rest)?)?.id),
        Some((what, rest)) if what == "argv" => {
            let (kind, rest) = rest.split_first().ok_or(USAGE)?;
            let asked = parse(rest)?;
            let a = agent_of(&asked)?;
            Ok(agent::shell_line(&a.typed(&argv(kind, &asked, &a)?)))
        }
        _ => Err(USAGE.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(args: &[&str]) -> Result<String, String> {
        let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        let (kind, rest) = args.split_first().unwrap();
        let asked = parse(rest)?;
        let mut a = agent_of(&asked)?;
        a.bin = format!("/bin/{}", a.bin);
        Ok(agent::shell_line(&argv(kind, &asked, &a)?))
    }

    #[test]
    fn each_launch_reads_as_the_agents_own_command() {
        let brief = ["run", "--agent", "codex", "--prompt", "Write it.", "--model", "sonnet", "--edits", "--dir", "/b"];
        assert_eq!(
            line(&brief),
            Ok("/bin/codex exec --skip-git-repo-check --sandbox workspace-write --add-dir /b 'Write it.'".into()),
            "sonnet is Claude Code's"
        );
        let claude = ["run", "--agent", "claude", "--prompt", "Go", "--model", "sonnet", "--edits"];
        assert_eq!(line(&claude), Ok("/bin/claude -p Go --model sonnet --permission-mode acceptEdits".into()));
        assert_eq!(line(&["resume", "--agent", "codex", "--session", "s-1"]), Ok("/bin/codex resume s-1".into()));
        assert_eq!(line(&["resume", "--agent", "claude", "--session", "s-1", "--prompt", "hi"]), Ok("/bin/claude --resume s-1 hi".into()));
        assert_eq!(line(&["fork", "--agent", "copilot", "--session", "s-1"]), Err("GitHub Copilot CLI cannot fork a session".into()));
        assert_eq!(line(&["start", "--agent", "agy", "--prompt", "hi"]), Ok("/bin/agy -i hi".into()), "a program Herdr reports names its agent");
        assert_eq!(line(&["resume", "--agent", "codex"]), Err("resume needs --session".into()));
        assert!(line(&["run", "--agent", "nobody"]).unwrap_err().starts_with("no agent"));
    }
}
