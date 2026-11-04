# Security

## Reporting a vulnerability

Please report it privately through GitHub's private vulnerability reporting: on this repository, **Security → Report a
vulnerability** (https://github.com/quaresc/cockpit/security/advisories/new). Don't open a public issue for it. The
maintainer answers there, and the fix ships in a release with credit to you unless you prefer otherwise.

Only the latest release is supported.

## What is sensitive

- **The hub** listens on a unix socket in a folder only you can read, and for the Herdr app on TCP bound to
  `127.0.0.1`. A TCP request must carry the random token written to `~/.cache/cockpit/hub.json`, which only your user
  can read. Anything that lets another local user or a web page reach the hub without that token is a vulnerability.
- **The config**, `~/.config/cockpit/config.json`, can hold API tokens (Linear, CircleCI) and a secret calendar feed
  URL. cockpit writes it readable by you alone.
- **Self-update**: the `cockpit` binary downloads new versions from this repository's GitHub releases and checks each
  archive against the `.sha256` file published beside it before installing it.
- **The shell install** is `curl … | sh`: `install.sh` downloads a release archive from GitHub and checks its
  checksum the same way. Read it first if you prefer; it is short.
- **Agents**: cockpit starts your coding agents and sends them prompts (recaps, routines, reviews). Those run with the
  permissions you gave each agent, never more.
