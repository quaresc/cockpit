"""Settings shared with the cockpit Claude Code mod: ~/.config/cockpit/config.json, written by /cockpit:setup."""

import json
import os
import re
import shutil
import subprocess
import sys
from urllib.parse import quote

HOME = os.path.expanduser("~")
PATH = os.path.join(HOME, ".config", "cockpit", "config.json")
INBOX = os.path.join(HOME, ".cache", "cockpit", "inbox.json")

DEFAULTS = {
    "reposDir": "~",
    "github": {"enabled": True, "org": ""},
    "linear": {"enabled": False, "server": "claude.ai Linear", "teams": []},
    "circleci": {"enabled": False, "server": "circleci", "imageJob": ""},
    "platform": {"enabled": False, "server": "platform", "stagePrefix": "", "sandboxArgoUrl": "", "workflowUrl": ""},
    "calendar": {"enabled": False, "server": "claude.ai Google Calendar"},
    "brief": {"enabled": True, "at": "08:30", "weekends": False},
    "notifications": True,
    "chromeSideWindow": False,
    "herdrPluginRoot": "",
    "spaces": ["inbox", "terminal", "to-do", "runbook", "meetings"],
    "runbookTools": [],
}


def open_url(url):
    """Opens a URL in the default browser, on macOS or Linux."""
    subprocess.run(["open" if sys.platform == "darwin" else "xdg-open", url], capture_output=True)


def copy(text):
    """Puts text on the clipboard: pbcopy on macOS, wl-copy or xclip on Linux; False when none exists."""
    for command in (["pbcopy"], ["wl-copy"], ["xclip", "-selection", "clipboard"]):
        if shutil.which(command[0]):
            subprocess.run(command, input=text, text=True)
            return True
    return False


def load():
    try:
        with open(PATH, encoding="utf-8") as f:
            raw = json.load(f)
    except (OSError, ValueError):
        raw = {}
    config = {**DEFAULTS, **raw}
    for key, value in DEFAULTS.items():
        if isinstance(value, dict):
            config[key] = {**value, **(raw.get(key) or {})}
    config["reposDir"] = os.path.expanduser(config["reposDir"]).rstrip("/") or HOME
    return config


def fill(template, **values):
    """A URL template with `{name}` placeholders filled in; None when the template is empty."""
    if not template:
        return None
    return re.sub(r"\{(\w+)\}", lambda m: quote(str(values.get(m.group(1), "")), safe=""), template)


def tool_prefix(server):
    """The tool-name prefix Claude Code gives an MCP server, e.g. "claude.ai Linear" -> mcp__claude_ai_Linear."""
    return "mcp__" + re.sub(r"[^A-Za-z0-9_-]", "_", server)


def _inbox():
    try:
        with open(INBOX, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def ticket_url(ticket_id, inbox=None):
    """A Linear ticket's URL, in the workspace of the tickets assigned to me."""
    for ticket in (inbox if inbox is not None else _inbox()).get("tickets") or []:
        match = re.match(r"^https://linear\.app/[^/]+/issue/", ticket.get("url", ""))
        if match:
            return match.group(0) + ticket_id.upper()
    return None


def team_keys(config, inbox=None):
    """Linear team keys a ticket mention may use: the configured ones and those of my tickets."""
    tickets = (inbox if inbox is not None else _inbox()).get("tickets") or []
    keys = {t.upper() for t in config["linear"]["teams"]} | {t["id"].split("-")[0] for t in tickets if "-" in t.get("id", "")}
    return sorted(k for k in keys if re.fullmatch(r"[A-Z][A-Z0-9]{1,9}", k))


def pr_url(repo, number, config, inbox=None):
    """A PR's URL from the inbox cache, else from the configured GitHub organization."""
    data = inbox if inbox is not None else _inbox()
    for pr in [*(data.get("mine") or []), *(data.get("direct") or []), *(data.get("team") or []), *(data.get("merged") or [])]:
        if pr.get("repo") == repo and str(pr.get("number")) == str(number) and pr.get("url"):
            return pr["url"]
    org = config["github"]["org"]
    return f"https://github.com/{org}/{repo}/pull/{number}" if org else None
