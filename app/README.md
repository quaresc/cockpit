# cockpit for the Herdr app

See every coding agent you are running, and what it is working on: Claude Code, Codex and the other agents Herdr runs,
each with its ticket, pull request, CI run and deploy, and which of them are waiting for your answer.

![cockpit in the Herdr app](preview.png)

## Install

In Claude Code:

```
/plugin install cockpit --marketplace quaresc/cockpit
/cockpit:setup
```

With any other agent, in a shell:

```sh
curl -fsSL https://raw.githubusercontent.com/quaresc/cockpit/main/install.sh | sh
cockpit setup
```

## More

- [The full README](../README.md): what cockpit shows, what it needs, and how to set it up.
- [The website](https://quaresc.github.io/cockpit/), with a video of it running.
- [RECIPE.md](RECIPE.md): this layout, pane by pane, as the Herdr gallery reads it.
- [Releases](https://github.com/quaresc/cockpit/releases) and their changelog.
