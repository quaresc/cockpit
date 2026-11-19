# Agent capability inventory

This inventory covers the 24 Herdr agent kinds named by `every_kind_herdr_detects_is_an_agent_of_the_table`.
The agent table owns executable names, aliases and command templates; an absent template disables that operation.
A configured command can override the table. Runtime availability also requires the executable and its existing
provider authentication; this work does not sign in or create credentials.

`Yes` records a command template, verified against the upstream reference named for the row or the prior source
check recorded beside it in `core/src/agent.rs`. Local provider calls, account authentication and quota values are not
validated here. Only seven formats have transcript readers; agents without readers do not invent sessions or costs.
The source checks and their limits are recorded in the lane evidence journal.

| Herdr kind / table id | Row name | Start | Resume | Fork | Headless / routines | Transcripts, search and tokens | Limits | Model argument / discovery | Reference |
|---|---|---|---|---|---|---|---|---|---|
| `claude` | Claude Code | Yes | Yes | Yes | Yes | Claude (local files) | Statusline + driven events | Yes / manual name | [Upstream](https://code.claude.com/docs/en/cli-reference) |
| `codex` | Codex | Yes | Yes | Yes | Yes | Codex (local files) | Local rollout | Yes / manual name | [Upstream](https://developers.openai.com/codex/cli/reference/) |
| `pi` | Pi | Yes | Yes | Yes | Yes | Pi (local files) | Not reported | Yes / CLI list | [Upstream](https://github.com/earendil-works/pi) |
| `omp` | Oh My Pi | Yes | Yes | Yes | Yes | Omp (local files) | Not reported | Yes / CLI list | [Upstream](https://github.com/can1357/oh-my-pi/blob/main/docs/cli-reference.md) |
| `copilot` | GitHub Copilot CLI | Yes | Yes | Off | Yes | Copilot (local files) | Not reported | Yes / manual name | [Upstream](https://docs.github.com/en/copilot/reference/copilot-cli-reference/cli-command-reference) |
| `devin` | Devin | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://docs.devin.ai/cli) |
| `droid` | Droid | Yes | Yes | Yes | Yes | Off | Not reported | Yes / manual name | [Upstream](https://docs.factory.com/droid-cli/cli-reference) |
| `kimi` | Kimi Code | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://github.com/MoonshotAI/kimi-cli) |
| `opencode` | OpenCode | Yes | Yes | Yes | Yes | Off | Not reported | Yes / CLI list | [Upstream](https://opencode.ai/docs/cli/commands/) |
| `kilo` | Kilo Code | Yes | Yes | Yes | Yes | Off | Not reported | Yes / CLI list | [Upstream](https://kilo.ai/docs/code-with-ai/platforms/cli) |
| `hermes` | Hermes | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://github.com/NousResearch/hermes-agent) |
| `qodercli` | Qoder CLI | Yes | Yes | Yes | Yes | Off | Not reported | Yes / manual name | [Upstream](https://docs.qoder.com/cli/cli-reference) |
| `qwen` | Qwen Code | Yes | Yes | Yes | Yes | Qwen (local files) | Not reported | Yes / manual name | [Upstream](https://qwenlm.github.io/qwen-code-docs/en/users/configuration/settings/) |
| `letta` | Letta Code | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://github.com/letta-ai/letta-code) |
| `cursor` | Cursor Agent | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://docs.cursor.com/en/cli/reference/parameters) |
| `mastracode` | Mastra Code | Yes | Yes | Yes | Yes | Off | Not reported | Yes / manual name | [Upstream](https://code.mastra.ai/) |
| `agy / antigravity-cli` | Antigravity | Yes | Yes | Off | Yes | Off | Not reported | Yes / CLI list | [Upstream](https://antigravity.google/docs/getting-started?tab=cli) |
| `grok` | Grok | Yes | Yes | Yes | Yes | Off | Not reported | Yes / manual name | [Upstream](https://docs.x.ai/) |
| `gemini` | Gemini CLI | Yes | Yes | Off | Yes | Gemini (local files) | Not reported | Yes / manual name | [Upstream](https://github.com/google-gemini/gemini-cli) |
| `cline` | Cline | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://github.com/cline/cline/blob/main/apps/cli/src/commands/program.ts) |
| `kiro` | Kiro CLI | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://kiro.dev/docs/cli/headless/) |
| `amp` | Amp | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://ampcode.com/docs/cli/execute-mode) |
| `maki` | Maki | Yes | Yes | Yes | Yes | Off | Not reported | Yes / CLI list | [Upstream](https://maki.sh/docs/cli/) |
| `muse` | Muse Code | Yes | Yes | Off | Yes | Off | Not reported | Yes / manual name | [Upstream](https://dev.meta.ai/docs/muse-code/configuration) |

## Isolation and unsupported capabilities

Recaps accept every supported transcript format but execute only through a verified text-only runner. At present,
that is an unmodified builtin Claude Code command with `--bare`, disabled tools, MCP, hooks, plugins and browser
integration, no persistent session, and an already-supplied `ANTHROPIC_API_KEY`. OAuth, keychain access and API-key
helpers are unavailable in bare mode. Without that runner and credential, recap generation reports unavailable.
Other agents' ordinary headless modes remain available for explicitly configured routines; they are not safe recap
sandboxes. Nothing here enables a new streaming driver: driven sessions remain limited to the measured Claude
protocol. Unverified fork, permission, interrupt and streaming controls remain absent.

The generic model cache asks only the six registered listing commands, at most once a day, and keeps the last good
list when a refresh fails. Codex's richer composer/Start work catalog belongs to the controls lane and is reconciled
at integration. The routine form preserves a selected model even if a refreshed catalog does not list it.

Quota sources remain local Claude statusline/driven events and Codex rollouts. Token counting additionally reads Pi,
Oh My Pi, Qwen, Copilot and Gemini local transcripts; this is not a new quota endpoint. No unofficial endpoint,
keychain read or additional credential is added. A missing price stays unpriced.

Direct-key isolation refuses a present Bedrock, Vertex or Foundry selector even when an API key is supplied; those
provider paths can resolve other credentials. The no-spawn `recap_auth` regression covers all selectors and OAuth-only
unavailability with acme dummy values in its own test process.
