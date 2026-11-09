<!-- A copy of ../README.md, written by scripts/app-readme.sh: edit that one. -->

<h1 align="center">cockpit</h1>

<p align="center"><b>See every coding agent you are running, and what it is working on.</b></p>

<p align="center"><img src="../app/preview.png" width="100%" alt="cockpit in the Herdr app: the agents list and Home"></p>

> [!TIP]
> See it running on the website: **[quaresc.github.io/cockpit](https://quaresc.github.io/cockpit/)**

Claude Code, Codex and the other agents Herdr runs, each with its ticket, pull request, CI run and deploy, and which
ones are waiting for you. Your reviews, mail, runbooks and meetings are one key away.

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
> In the Herdr app, the gallery installs the layout in [`app/`](../app/). It shows cockpit's data, so install cockpit
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

[Getting started](../docs/getting-started.md) · [Configuration](../docs/configuration.md) · [Show blocks](../docs/show.md) ·
[Troubleshooting](../docs/troubleshooting.md) · [Changelog](https://github.com/quaresc/cockpit/releases) ·
[Contributing](../CONTRIBUTING.md) · MIT ([notices](../THIRD_PARTY_NOTICES.md))
