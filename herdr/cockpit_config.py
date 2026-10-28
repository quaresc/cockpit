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
SHARED = os.path.join(HOME, ".cache", "cockpit", "config.json")
INBOX = os.path.join(HOME, ".cache", "cockpit", "inbox.json")

DEFAULTS = {
    "reposDir": "~",
    "github": {"enabled": True, "org": ""},
    "tickets": {"enabled": False, "name": "Linear", "server": "claude.ai Linear", "keys": [], "urlTemplate": "", "tools": {}, "fields": {}, "statusTypes": {}},
    "ci": {"enabled": True, "name": "GitHub", "server": "", "imageJob": "", "tools": {}, "fields": {}, "outcomes": {}, "endedPhases": []},
    "platform": {"enabled": False, "server": "", "previewPrefix": "", "previewDashboardUrl": "", "workflowUrl": "", "tools": {}, "fields": {}},
    "calendar": {"enabled": False, "name": "Google Calendar", "server": "claude.ai Google Calendar", "tools": {}, "fields": {}, "skip": {}, "only": {}, "responses": {}},
    "brief": {"enabled": True, "at": "08:30", "weekends": False},
    "meetingBrief": {"enabled": True, "model": "sonnet", "skip": ["lunch", "focus", "break", "wday", "out of office"], "tools": []},
    "notifications": True,
    "e2eBrowser": "terminal-browser",
    "browserIdleMinutes": 60,
    "herdrPluginRoot": "",
    "spaces": ["inbox", "terminal", "actions", "to-do", "runbook", "meetings"],
    "runbookTools": [],
    "worktrees": False,
    "apps": "rust",
    "density": "comfortable",
}


def script(name):
    """One of the plugin's scripts, through the link that follows herdr's updates: an app that outlives its install
    folder (herdr removes old ones) still reaches the current scripts."""
    linked = os.path.join(HOME, ".config", "cockpit", "herdr", name)
    return linked if os.path.exists(linked) else os.path.join(os.path.dirname(os.path.abspath(__file__)), name)


def open_url(url):
    """Opens a URL in the default browser, on macOS or Linux."""
    try:
        subprocess.run(["open" if sys.platform == "darwin" else "xdg-open", url], capture_output=True, timeout=10)
    except (OSError, subprocess.SubprocessError):
        pass


def dashboard_name(url):
    """What a dashboard link is, from its host: ArgoCD, Vercel, Netlify, Render, Railway, Fly, else Dashboard."""
    host = re.match(r"^https?://([^/]+)", url or "")
    host = host.group(1) if host else ""
    if re.search(r"(^|\.)argo(cd)?[.-]", host):
        return "ArgoCD"
    for suffix, name in (("vercel.com", "Vercel"), ("netlify.app", "Netlify"), ("netlify.com", "Netlify"), ("render.com", "Render"),
                         ("railway.app", "Railway"), ("fly.io", "Fly")):
        if host.endswith(suffix):
            return name
    return "Dashboard"


def copy(text):
    """Puts text on the clipboard: pbcopy on macOS, wl-copy or xclip on Linux; False when none exists."""
    for command in (["pbcopy"], ["wl-copy"], ["xclip", "-selection", "clipboard"]):
        if shutil.which(command[0]):
            try:
                subprocess.run(command, input=text, text=True, timeout=5)
            except subprocess.SubprocessError:
                return False
            return True
    return False


def load():
    """The config as the mod resolved it (~/.cache/cockpit/config.json); before any session has run, the file
    /cockpit:setup writes over the defaults."""
    try:
        with open(SHARED, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        pass
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


def is_pinned(label, config):
    """Whether a herdr space label, "[N] <icon>  <name>", is one of the pinned spaces."""
    names = [s["name"] if isinstance(s, dict) else s for s in config["spaces"]]
    return any(label.endswith(f"  {name}") for name in names)


def ci_mark(config):
    """The CI's glyph: CircleCI's, GitHub Actions', GitLab's, else a generic run."""
    name = config["ci"]["name"].lower()
    return {"circleci": "\ue78c", "github": "\ueaff", "gitlab": "\U000f0ba0"}.get(name, "\ueb9e")


def fill(template, **values):
    """A URL template with `{name}` placeholders filled in; None when the template is empty."""
    if not template:
        return None
    return re.sub(r"\{(\w+)\}", lambda m: quote(str(values.get(m.group(1), "")), safe=""), template)


def tool_prefix(server):
    """The tool-name prefix Claude Code gives an MCP server, e.g. "claude.ai Linear" -> mcp__claude_ai_Linear."""
    return "mcp__" + re.sub(r"_+", "_", re.sub(r"[^A-Za-z0-9_-]", "_", server)).rstrip("_")


def _inbox():
    try:
        with open(INBOX, encoding="utf-8") as f:
            return json.load(f)
    except (OSError, ValueError):
        return {}


def ticket_url(ticket_id, config, inbox=None):
    """A ticket's URL: the configured template's, else an assigned ticket's URL cut where its id is a path segment, with
    the id swapped."""
    filled = fill(config["tickets"]["urlTemplate"], id=ticket_id.upper())
    if filled:
        return filled
    for ticket in (inbox if inbox is not None else _inbox()).get("tickets") or []:
        url, known = ticket.get("url", ""), ticket.get("id", "")
        if known and f"/{known}" in url:
            return url[:url.rindex(f"/{known}") + 1] + ticket_id.upper()
    return None


def has_ticket(branch, ticket_id):
    """Whether a branch names the ticket as a whole token: eng-4-login does, eng-42-login does not."""
    key = re.escape((ticket_id or "").lower())
    return bool(key) and re.search(rf"(^|[^a-z0-9]){key}(?![0-9])", (branch or "").lower()) is not None


def team_keys(config, inbox=None):
    """Ticket key prefixes a mention may use: the configured ones and those of my tickets."""
    tickets = (inbox if inbox is not None else _inbox()).get("tickets") or []
    keys = {t.upper() for t in config["tickets"]["keys"]} | {t["id"].split("-")[0] for t in tickets if "-" in t.get("id", "")}
    return sorted(k for k in keys if re.fullmatch(r"[A-Z][A-Z0-9]{1,9}", k))


def pr_url(repo, number, config, inbox=None):
    """A PR's URL from the inbox cache, else from the configured GitHub organization."""
    data = inbox if inbox is not None else _inbox()
    for pr in [*(data.get("mine") or []), *(data.get("direct") or []), *(data.get("team") or []), *(data.get("merged") or [])]:
        if pr.get("repo") == repo and str(pr.get("number")) == str(number) and pr.get("url"):
            return pr["url"]
    org = config["github"]["org"]
    return f"https://github.com/{org}/{repo}/pull/{number}" if org else None
