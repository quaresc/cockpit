# Working on cockpit as a coding agent

Read `CONTRIBUTING.md` first: it maps the repo and gives the build, test and quality-gate commands. This file adds the
rules that matter most when an agent does the work.

## Never touch the person's live setup

cockpit runs on the machine you are working on, probably beside you. Leave all of this alone:

- their Herdr window and herdr-core terminals: never create, close or type into their spaces, tabs or panes;
- their running hub (`~/.cache/cockpit/hub.json`) and cockpit data under `~/.cache/cockpit` and `~/.config/cockpit`;
- their agents' settings, such as `~/.claude/settings.json` or `~/.codex/config.toml`.

Check GUI work in the sandbox only: `COCKPIT_SANDBOX=/tmp/cc-<letter> HC_SESSION=cc-<letter> sh scripts/sandbox.sh start`,
then `show`, `shot`, `check`, and `stop` when done. It has its own fake HOME, hub and herdr-core.

Run Rust tests with `env -u HC_TERMINAL_ID -u HC_TERMINAL_SOCKET -u HC_SCOPE cargo test …`, so they never reach the
live window.

## No real data

Fixtures, snapshots, screenshots and examples use the acme demo world only (`acme/api`, `ENG-412`, Maya, Sam, Priya,
Leo, Omar, `me@acme.example`, `@example.com`). Never copy anything from the person's own work into the repo: names,
ticket keys, ids, URLs, tokens, paths.

## House rules

- Doc comments: one or two plain sentences in the present tense, saying what the code does. No history ("no longer",
  "used to"), no issue or PR numbers.
- Rust: about 160 columns, single-line let-else; `cargo fmt` with `herdr/tui/rustfmt.toml` decides.
- Agent-specific behavior goes through the table in `herdr/tui/core/src/agent.rs`, never a hard-coded `claude`.
- Ported code keeps a header naming its source and licence, and an entry in `THIRD_PARTY_NOTICES.md`.
- Don't commit, push, tag, bump versions or edit `CHANGELOG.md` unless the person asks.

## Before saying it is done

Run the quality gate in `CONTRIBUTING.md` for what you touched: `cargo fmt --check`,
`cargo clippy --workspace --all-targets --locked -- -D warnings`, the relevant `cargo test`, `shellcheck -S warning` on
changed scripts, `node --input-type=module --check` on changed mod files, `claude plugin validate plugin` and
`claude plugin test plugin` for plugin changes, `sh scripts/check-skills.sh` for skill changes. For a visible change,
screenshot it in the sandbox in both themes. Report what failed honestly.
