<p align="center"><img src="docs/logo.svg" width="96" alt="cockpit logo"></p>

<h1 align="center">cockpit</h1>

<p align="center"><b>See every coding agent you are running, and what it is working on.</b><br>
Claude Code, Codex and the other agents Herdr runs, each shown with its ticket, pull request, CI run and deploy,<br>
and which of them are waiting for an answer. Your reviews, mail, runbooks, meetings and releases are one key away.</p>

<p align="center"><a href="https://quaresc.github.io/cockpit/"><b>Website</b></a> ·
<a href="https://quaresc.github.io/cockpit/desktop.html">Herdr APP</a> ·
<a href="https://quaresc.github.io/cockpit/terminal.html">Herdr TUI</a> ·
<a href="https://github.com/quaresc/cockpit/releases">Changelog</a></p>

<p align="center"><img src="app/preview.png" width="100%" alt="cockpit in the Herdr app: the agents list and Home"></p>

## Install

You need macOS or Linux, [herdr](https://herdr.dev) 0.9+ or the Herdr app, `gh` signed in, and `jq`.

**In Claude Code**

```
/plugin install cockpit --marketplace quaresc/cockpit
/cockpit:setup
```

**With any other agent**, in a shell

```sh
curl -fsSL https://raw.githubusercontent.com/quaresc/cockpit/main/install.sh | sh
cockpit setup
```

> [!NOTE]
> In the Herdr app, the gallery installs the layout in [`app/`](app/). It shows cockpit's data, so install cockpit
> as above too.

`cockpit update` updates every part later; cockpit also tells you when a new version is out.

<details>
<summary><b>Questions</b></summary>

- **Do I need API tokens?** No. cockpit reads what your agents already reach through MCP, and GitHub through `gh`.
- **What does it cost?** Nothing. Work it asks an agent to do (recaps, routines, briefs) is billed to that agent.
- **Herdr app or herdr?** Either, or both. Setup asks.
- **Is my data sent anywhere?** No. It stays on your machine.

</details>

<details>
<summary><b>Agents and tools</b></summary>

- Works with every agent Herdr runs.
- Tickets, CI, meetings, mail and deploys come from MCP servers you already use. Setup knows Linear, Jira, GitHub,
  GitLab, CircleCI, Datadog, Sentry, Gmail and Google Calendar, and your agent maps any other.
- Agents draw charts, tables and diagrams beside their session with the `show` tool.

</details>

## More

[Getting started](docs/getting-started.md) · [Configuration](docs/configuration.md) · [Show blocks](docs/show.md) ·
[Troubleshooting](docs/troubleshooting.md) · [Changelog](https://github.com/quaresc/cockpit/releases) ·
[Contributing](CONTRIBUTING.md) · MIT ([notices](THIRD_PARTY_NOTICES.md))
