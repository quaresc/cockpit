# Contributing to cockpit

Thanks for helping. Issues and pull requests are welcome; for anything larger than a fix, open an issue first so we can
agree on the shape before you write it.

## What the repo holds

| Path | What it is |
| --- | --- |
| `herdr/tui/` | The Rust workspace behind the one `cockpit` binary: `core` (shared kit, agents table, transcripts), `hub` (the background process every app talks to), the space apps (`agents`, `inbox`, `runbook`, `show`, `todo`, `meetings`, `monitor`, `routines`, `palette`), `update` (self-update) and `cli` (the binary and its subcommands, the MCP server). |
| `app/` | The Herdr app half: `layouts/`, the `mods/cockpit/` mod (plain ES modules on Herdr's `gpui-kit`), `keys.json` and `RECIPE.md`. `docs/app.md` explains how it fits together. |
| `plugin/` | The Claude Code plugin: TypeScript hooks in `hooks/`, commands, the side-browser skill, tests in `tests/`. |
| `codex/` | The Codex plugin: the MCP server entry and the same side-browser skill. |
| `herdr/*.sh` | The herdr TUI integration: the scripts `herdr-plugin.toml` binds to keys and events. |
| `install.sh` | The shell install for agents other than Claude Code. |
| `scripts/` | Developer tools: the GUI sandbox, the skill-copies check. |
| `site/`, `docs/` | The website, the README's pictures, and the tooling that renders them (`docs/screenshots/`). |

## Build and test

### The Rust half

```sh
cd herdr/tui
cargo build
env -u HC_TERMINAL_ID -u HC_TERMINAL_SOCKET -u HC_SCOPE cargo test --workspace
```

Run the tests outside a Herdr terminal, or unset those variables as above: they point cockpit at your live window.
On macOS, hub tests that wait on file watches (`hub/tests/protocol.rs`, `hub/tests/sessions.rs`) can time out when
starting a watch is slow in the checkout's folder (seconds instead of milliseconds); copy the test binary to `/tmp` and run it there with
`CARGO_MANIFEST_DIR=$PWD/hub` to tell a slow watch from a real failure. Snapshot tests use `insta`; review changed
snapshots with `cargo insta review`.

### The Herdr app half

The mod has no build step: Herdr loads `app/mods/cockpit/*.js` as they are. Work on it in the sandbox, never in your
own Herdr window:

```sh
COCKPIT_SANDBOX=/tmp/cc-me sh scripts/sandbox.sh start   # a separate window on the acme demo world, its own hub
sh scripts/sandbox.sh show inbox                         # bring a view to the front
sh scripts/sandbox.sh shot /tmp/inbox.png                # screenshot it
sh scripts/sandbox.sh check                              # herdr-app check on the mod
sh scripts/sandbox.sh stop
```

The sandbox runs on a fake HOME with its own hub and copies `app/` into its window on every save. Keep
`COCKPIT_SANDBOX` short (macOS caps socket paths at 104 bytes); `HC_SESSION=<name>` gives it an empty herdr-core of its
own, so none of your terminals shows in it. The top of `scripts/sandbox.sh` lists every command.

### The plugins

```sh
claude plugin validate plugin
claude plugin test plugin
sh scripts/check-skills.sh
```

`plugin/skills/side-browser/SKILL.md` and `codex/skills/side-browser/SKILL.md` must stay in step: change both, and keep
what only one agent should read between `<!-- agent: … -->` and `<!-- /agent -->` lines.

To type-check the hooks, load the plugin once from this folder (`claude --plugin-dir plugin`), which lays Claude Code's
types in `plugin/.claude-plugin/types/` (ignored by git), then run `npx -p typescript tsc -p plugin`.

## The demo world, and no real data

Every fixture, snapshot, screenshot and example uses the fictional acme world: repos `acme/api` and `acme/web`, tickets
like `ENG-412`, people Maya, Sam, Priya, Leo and Omar, `me@acme.example` and `@example.com` addresses.
`docs/screenshots/demo_data.py` builds it. Never commit anything from a real workspace: no employer or customer names,
real ticket keys, ids, URLs, tokens, e-mail addresses or home-folder paths.

## Screenshots

The SVGs and PNGs in `docs/` are rendered from the demo world; `docs/screenshots/README.md` explains each script.
`sh docs/screenshots/app_shots.sh [view ...]` shoots the sandbox window in both themes into `docs/app/`. Compress new
PNGs losslessly before committing (`oxipng -o 4 --strip safe <file>`).

## Quality gate

Run what applies to your change before opening a pull request; CI runs all of it.

```sh
cd herdr/tui
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
env -u HC_TERMINAL_ID -u HC_TERMINAL_SOCKET -u HC_SCOPE cargo test --workspace
cd ../..
shellcheck -S warning herdr/*.sh install.sh scripts/*.sh docs/screenshots/*.sh
for f in app/mods/cockpit/*.js; do node --input-type=module --check < "$f" || echo "$f"; done
claude plugin validate plugin && claude plugin test plugin
sh scripts/check-skills.sh
```

Rust is formatted by `cargo fmt` with `herdr/tui/rustfmt.toml` (160 columns). The minimum Rust version is the
workspace's `rust-version` in `herdr/tui/Cargo.toml`.

## Third-party code and notices

Code ported from another project keeps a header naming its source and licence, and gets an entry in
`THIRD_PARTY_NOTICES.md`. After a change to `Cargo.lock`, regenerate the crate notices and commit them (CI checks they
are current):

```sh
cargo install cargo-about --locked --features cli
cd herdr/tui && cargo about generate --locked -o THIRD_PARTY_CRATES.md about.md.hbs
```

A new licence among the crates has to be added to `accepted` in `herdr/tui/about.toml` first.

## Adding an agent

cockpit knows each agent through one row of the table in `herdr/tui/core/src/agent.rs`: how to start, resume, fork and
run it headless. Reading its past sessions (search, recaps, costs) is a reader in `herdr/tui/core/src/transcripts.rs`,
and wiring cockpit's MCP server into its config is `herdr/tui/cli/src/connect.rs`. Add tests beside each.

## Commits and pull requests

- One topic per pull request, with a short description of what changes for the person using cockpit.
- Commit subjects are one short line saying what changes, e.g. `Inbox: rerun GitHub Actions CI through gh`.
- Doc comments are one or two plain sentences in the present tense, saying what the code does.
- Don't bump versions or edit `CHANGELOG.md`: the maintainer does both when releasing.

By contributing, you agree that your contributions are licensed under the MIT licence of this repository.
