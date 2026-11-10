#!/usr/bin/env python3
"""
cockpit-usage-sync.py
Two things per agent (Claude Code, Codex, Antigravity), kept apart:
  context  how full the latest session's context window is (Home's agent cards)
  windows  the account's real rate limits: the 5-hour session and the 7-day week, % used, reset time, pace
           (Home's Usage & Limits card, the Usage tab, and the hub's quota through limits.json)

Where the limits come from, each asked straight, never through LiteLLM:
  Claude Code  Anthropic's OAuth usage endpoint with Claude Code's own login (Keychain), else the statusline's rate_limits
  Codex        `codex app-server`, JSON-RPC account/rateLimits/read (Codex does its own auth)
  Antigravity  `agy -p /quota --output-format json`: one 5h + weekly pair per model group, no tokens spent
Each answer is cached (usage-src-<id>.json) and asked again once stale, so the 15 s refresh stays cheap; a failed ask
keeps the last answer. With no answer at all, the window is an estimate: transcript tokens in the last 5h / 7d against
`usage.budgets` in ~/.config/cockpit/config.json. LiteLLM's /key/info, when a key env is configured, adds its spend.
Writes ~/.cache/cockpit/agent-usage.json and ~/.cache/cockpit/limits.json.
"""

import glob
import json
import os
import select
import shutil
import subprocess
import sys
import threading
import time
import urllib.request
from datetime import datetime

CACHE = os.path.expanduser("~/.cache/cockpit")
CONFIG = os.path.expanduser("~/.config/cockpit/config.json")
NOW = time.time()
TTL = {"claude": 300, "codex": 120, "antigravity": 120}
if "--force" in sys.argv:
    # Refresh in the Usage tab: ask again, but Anthropic's endpoint at most once a minute.
    TTL = {"claude": 60, "codex": 0, "antigravity": 0}
# --only <id>: ask that agent alone (the Claude statusline uses it); the others answer from their caches, however old.
ONLY = sys.argv[sys.argv.index("--only") + 1] if "--only" in sys.argv[:-1] else None
if ONLY:
    TTL = {k: (v if k == ONLY else float("inf")) for k, v in TTL.items()}
WINDOW_MINS = {"5h": 300, "7d": 10080}
# The Herdr app may start this with a short PATH: the agents' CLIs live in these too.
os.environ["PATH"] = os.pathsep.join([os.environ.get("PATH", ""), os.path.expanduser("~/.local/bin"), "/opt/homebrew/bin", "/usr/local/bin"])


def load_json(path, default=None):
    try:
        with open(path, "r", encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return default


def save_json(path, value):
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        json.dump(value, f, indent=2)
    os.replace(path + ".tmp", path)


def iso_ms(s):
    """An ISO time (Z or offset) as epoch milliseconds."""
    if not s:
        return None
    try:
        return int(datetime.fromisoformat(str(s).replace("Z", "+00:00")).timestamp() * 1000)
    except Exception:
        return None


def window(name, percent, resets_at_ms, source, group=None, **extra):
    """A limit window with how much of it has gone by and whether usage runs ahead of that."""
    mins = WINDOW_MINS[name]
    w = {"name": name, "percent": round(max(0.0, float(percent)), 1), "resets_at": resets_at_ms, "window_mins": mins, "source": source}
    if group:
        w["group"] = group
    if resets_at_ms:
        left = max(0.0, resets_at_ms / 1000 - NOW)
        w["elapsed"] = round(max(0.0, min(1.0, 1 - left / (mins * 60))), 3)
        w["ahead"] = w["percent"] / 100 > w["elapsed"] + 0.1
    w.update({k: v for k, v in extra.items() if v is not None})
    return w


def recent(pattern, days=7):
    cutoff = NOW - days * 86400
    files = [f for f in glob.glob(pattern, recursive=True) if os.path.getmtime(f) >= cutoff]
    return sorted(files, key=os.path.getmtime)


def tail_lines(path, limit=4000):
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as f:
            return f.readlines()[-limit:]
    except Exception:
        return []


# ---------------------------------------------------------------------------------------------------------------------
# Official limits, one asker per agent: each returns a list of windows or raises.

def claude_login():
    """Claude Code's own login: the Keychain on macOS, its credentials file elsewhere (Linux)."""
    if shutil.which("security"):
        raw = subprocess.run(["security", "find-generic-password", "-s", "Claude Code-credentials", "-w"], capture_output=True, text=True, timeout=10).stdout.strip()
        if raw:
            return raw
    path = os.path.join(os.environ.get("CLAUDE_CONFIG_DIR") or os.path.expanduser("~/.claude"), ".credentials.json")
    return open(path, encoding="utf-8").read() if os.path.exists(path) else ""


def ask_claude():
    raw = claude_login()
    token = json.loads(raw).get("claudeAiOauth", {}).get("accessToken") if raw else None
    if not token:
        raise RuntimeError("no Claude Code login (Keychain or ~/.claude/.credentials.json)")
    req = urllib.request.Request(
        "https://api.anthropic.com/api/oauth/usage",
        headers={"Authorization": f"Bearer {token}", "anthropic-beta": "oauth-2025-04-20", "User-Agent": "cockpit-usage"},
    )
    with urllib.request.urlopen(req, timeout=10) as r:
        d = json.load(r)
    out = []
    for key, name in (("five_hour", "5h"), ("seven_day", "7d")):
        v = d.get(key) or {}
        if v.get("utilization") is not None:
            out.append(window(name, v["utilization"], iso_ms(v.get("resets_at")), "official"))
    for key, group in (("seven_day_opus", "Opus"), ("seven_day_sonnet", "Sonnet")):
        v = d.get(key) or {}
        if v.get("utilization") is not None:
            out.append(window("7d", v["utilization"], iso_ms(v.get("resets_at")), "official", group=group))
    if not out:
        raise RuntimeError("no limits in the answer")
    return out


def claude_statusline():
    """rate_limits from the statusline payload, when Claude Code sends them."""
    d = load_json(os.path.join(CACHE, "usage-claude.json"), {}) or {}
    rate = d.get("rate_limits") or {}
    out = []
    for key, name in (("five_hour", "5h"), ("seven_day", "7d")):
        v = rate.get(key) or {}
        if v.get("used_percentage") is not None:
            r = v.get("resets_at")
            ms = int(r * 1000) if isinstance(r, (int, float)) and r < 1e12 else (r if isinstance(r, (int, float)) else iso_ms(r))
            out.append(window(name, v["used_percentage"], ms, "official"))
    return out


def ask_codex():
    p = subprocess.Popen(["codex", "app-server"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
    try:
        def send(o):
            p.stdin.write(json.dumps(o) + "\n")
            p.stdin.flush()

        send({"id": 1, "method": "initialize", "params": {"clientInfo": {"name": "cockpit", "version": "1"}}})
        send({"method": "initialized"})
        send({"id": 2, "method": "account/rateLimits/read"})
        end = time.time() + 20
        while time.time() < end:
            ready, _, _ = select.select([p.stdout], [], [], 1)
            if not ready:
                continue
            line = p.stdout.readline()
            if not line:
                break
            m = json.loads(line)
            if m.get("id") == 2:
                if "error" in m:
                    raise RuntimeError(str(m["error"])[:200])
                rl = (m.get("result") or {}).get("rateLimits") or {}
                out = []
                for key in ("primary", "secondary"):
                    v = rl.get(key) or {}
                    if v.get("usedPercent") is None:
                        continue
                    name = "5h" if (v.get("windowDurationMins") or 300) <= 300 else "7d"
                    out.append(window(name, v["usedPercent"], int(v["resetsAt"] * 1000) if v.get("resetsAt") else None, "official", plan=rl.get("planType")))
                if not out:
                    raise RuntimeError("no limits in the answer")
                return out
        raise RuntimeError("codex app-server did not answer")
    finally:
        p.kill()


def ask_antigravity():
    r = subprocess.run(["agy", "-p", "/quota", "--output-format", "json", "--print-timeout", "30s"], capture_output=True, text=True, timeout=45, cwd="/tmp")
    d = json.loads(r.stdout)
    groups = ((d.get("command") or {}).get("data") or {}).get("groups") or []
    short = {"Gemini Models": "Gemini", "Claude and GPT models": "Claude & GPT", "Gemini 4 Argon": "Argon"}
    out = []
    for g in groups:
        for b in g.get("buckets", []):
            name = "5h" if b.get("window") == "5h" else "7d"
            if b.get("remaining_fraction") is None:
                continue
            out.append(window(name, (1 - b["remaining_fraction"]) * 100, iso_ms(b.get("reset_time")), "official", group=short.get(g.get("name"), g.get("name"))))
    if not out:
        raise RuntimeError("no quota groups in the answer")
    return out


ASKERS = {"claude": ask_claude, "codex": ask_codex, "antigravity": ask_antigravity}


def official(agent_id):
    """The cached answer while fresh, else a new one; a failed ask keeps the last (marked stale)."""
    path = os.path.join(CACHE, f"usage-src-{agent_id}.json")
    cached = load_json(path, {}) or {}
    if cached.get("windows") and NOW - cached.get("fetched_at", 0) < TTL[agent_id]:
        return cached
    if TTL[agent_id] == float("inf"):
        return cached
    try:
        wins = ASKERS[agent_id]()
        cached = {"fetched_at": NOW, "windows": wins, "error": None}
    except Exception as e:
        cached = {**cached, "error": str(e)[:200], "tried_at": NOW}
        if cached.get("windows"):
            cached["stale"] = True
    save_json(path, cached)
    return cached


def refresh(w):
    """Recompute elapsed/ahead for a cached window; a window past its reset is dropped (it starts over at 0)."""
    if w.get("resets_at") and w["resets_at"] / 1000 < NOW:
        return None
    extra = {k: v for k, v in w.items() if k not in ("name", "percent", "resets_at", "window_mins", "source", "group", "elapsed", "ahead")}
    return window(w["name"], w["percent"], w.get("resets_at"), w["source"], group=w.get("group"), **extra)


# ---------------------------------------------------------------------------------------------------------------------
# Estimates: transcript tokens in the last 5h / 7d, for an agent with no official answer.

def token_events(agent_id):
    """(epoch seconds, tokens) for each model turn in the last 7 days."""
    out = []
    if agent_id == "claude":
        for f in recent(os.path.expanduser("~/.claude/projects/**/*.jsonl")):
            for line in tail_lines(f):
                if '"usage"' not in line:
                    continue
                try:
                    e = json.loads(line)
                    u = e["message"]["usage"]
                    t = iso_ms(e.get("timestamp"))
                    if t:
                        out.append((t / 1000, u.get("input_tokens", 0) + u.get("cache_creation_input_tokens", 0) + u.get("output_tokens", 0)))
                except Exception:
                    continue
    elif agent_id == "codex":
        for f in recent(os.path.expanduser("~/.codex/sessions/**/*.jsonl")):
            for line in tail_lines(f):
                if '"token_count"' not in line:
                    continue
                try:
                    e = json.loads(line)
                    u = (e["payload"].get("info") or {}).get("last_token_usage") or {}
                    t = iso_ms(e.get("timestamp"))
                    if t and u:
                        out.append((t / 1000, u.get("input_tokens", 0) - u.get("cached_input_tokens", 0) + u.get("output_tokens", 0)))
                except Exception:
                    continue
    elif agent_id == "antigravity":
        for f in recent(os.path.expanduser("~/.gemini/antigravity-cli/brain/*/.system_generated/logs/transcript.jsonl")):
            for line in tail_lines(f):
                if '"input_tokens"' not in line:
                    continue
                try:
                    e = json.loads(line)
                    t = iso_ms(e.get("created_at"))
                    if t:
                        out.append((t / 1000, e.get("input_tokens", 0) + e.get("output_tokens", 0)))
                except Exception:
                    continue
    return out


def estimate(agent_id, budgets):
    events = token_events(agent_id)
    out = []
    for name, mins in WINDOW_MINS.items():
        inside = [(t, n) for t, n in events if t >= NOW - mins * 60]
        used = sum(n for _, n in inside)
        budget = (budgets.get(agent_id) or {}).get(name) or 0
        resets = int((min(t for t, _ in inside) + mins * 60) * 1000) if inside else None
        pct = used * 100.0 / budget if budget else 0.0
        out.append(window(name, pct, resets, "estimate", used_tokens=used, budget_tokens=budget or None))
    return out


def litellm(agent_id, cfg):
    """Spend and budget of the agent's LiteLLM key, when its env var is configured and set."""
    ll = cfg.get("litellm") or {}
    env = (ll.get("keys") or {}).get(agent_id)
    key = os.environ.get(env) if env else None
    if not key or not ll.get("base_url"):
        return None
    try:
        req = urllib.request.Request(ll["base_url"].rstrip("/") + "/key/info", headers={"Authorization": f"Bearer {key}"})
        with urllib.request.urlopen(req, timeout=5) as r:
            i = json.load(r).get("info") or {}
        return {k: i.get(k) for k in ("key_alias", "spend", "max_budget", "budget_duration", "budget_reset_at")}
    except Exception:
        return None


# ---------------------------------------------------------------------------------------------------------------------
# Context windows: the latest session of each agent.

def context_antigravity():
    files = recent(os.path.expanduser("~/.gemini/antigravity-cli/brain/*/.system_generated/logs/transcript.jsonl"), days=30)
    if not files:
        return None
    for line in reversed(tail_lines(files[-1])):
        try:
            e = json.loads(line)
            if "input_tokens" in e:
                used = e.get("input_tokens", 0) + e.get("output_tokens", 0) + e.get("cache_read_tokens", 0)
                win = 1_048_576
                return {"percent": min(100.0, round(used * 100.0 / win, 1)), "used": used, "window": win, "at": int(os.path.getmtime(files[-1]) * 1000)}
        except Exception:
            continue
    return None


def context_codex():
    files = recent(os.path.expanduser("~/.codex/sessions/**/*.jsonl"), days=30)
    if not files:
        return None
    for line in reversed(tail_lines(files[-1])):
        try:
            e = json.loads(line)
            if e.get("type") == "event_msg" and e.get("payload", {}).get("type") == "token_count":
                info = e["payload"].get("info") or {}
                used = (info.get("last_token_usage") or info.get("total_token_usage") or {}).get("total_tokens")
                win = info.get("model_context_window") or 200_000
                if used is not None:
                    return {"percent": min(100.0, round(used * 100.0 / win, 1)), "used": used, "window": win, "at": int(os.path.getmtime(files[-1]) * 1000)}
        except Exception:
            continue
    return None


def context_claude():
    d = load_json(os.path.join(CACHE, "usage-claude.json"), {}) or {}
    cw = d.get("context_window") or {}
    if cw.get("used_percentage") is not None:
        win = cw.get("context_window_size") or 200_000
        return {"percent": round(cw["used_percentage"], 1), "used": int(win * cw["used_percentage"] / 100), "window": win, "at": int(os.path.getmtime(os.path.join(CACHE, "usage-claude.json")) * 1000)}
    files = recent(os.path.expanduser("~/.claude/projects/**/*.jsonl"), days=30)
    if not files:
        return None
    for line in reversed(tail_lines(files[-1])):
        try:
            u = json.loads(line).get("message", {}).get("usage")
            if u:
                used = u.get("input_tokens", 0) + u.get("cache_read_input_tokens", 0) + u.get("cache_creation_input_tokens", 0) + u.get("output_tokens", 0)
                win = 1_000_000 if used > 200_000 else 200_000
                return {"percent": min(100.0, round(used * 100.0 / win, 1)), "used": used, "window": win, "at": int(os.path.getmtime(files[-1]) * 1000)}
        except Exception:
            continue
    return None


AGENTS = [
    ("claude", "Claude Code", "terminal", context_claude),
    ("codex", "Codex", "sparkles", context_codex),
    ("antigravity", "Antigravity", "bot", context_antigravity),
]


def main():
    os.makedirs(CACHE, exist_ok=True)
    cfg = (load_json(CONFIG, {}) or {}).get("usage") or {}
    budgets = cfg.get("budgets") or {}

    # The three asks are slow-ish (a second or a few each), so they run side by side.
    answers = {}
    threads = [threading.Thread(target=lambda i=i: answers.__setitem__(i, official(i))) for i, *_ in AGENTS]
    for t in threads:
        t.start()
    for t in threads:
        t.join(60)

    agents = []
    for agent_id, name, icon, context_of in AGENTS:
        ans = answers.get(agent_id) or {}
        wins = [w for w in (refresh(w) for w in ans.get("windows") or []) if w]
        if agent_id == "claude" and not wins:
            wins = claude_statusline()
        source = "official" if wins else "estimate"
        if not wins:
            wins = estimate(agent_id, budgets)
        ctx = context_of()
        a = {
            "id": agent_id,
            "name": name,
            "icon": icon,
            "context": ctx,
            "windows": wins,
            "source": source,
            "stale": bool(ans.get("stale")),
            "error": ans.get("error"),
            "fetched_at": int(ans["fetched_at"] * 1000) if ans.get("fetched_at") else None,
            "updated_at": int(NOW * 1000),
        }
        ll = litellm(agent_id, cfg) if agent_id != "claude" else None
        if ll:
            a["litellm"] = ll
        agents.append(a)

    save_json(os.path.join(CACHE, "agent-usage.json"), {"updated_at": int(NOW * 1000), "agents": agents})

    # The hub reads limits.json into the agents view's quota (the recap popup's bars).
    limits = []
    for a in agents:
        if a["source"] != "official":
            continue
        for w in a["windows"]:
            label = f"{a['name']}{' ' + w['group'] if w.get('group') else ''} {w['name']}"
            resets_in = int((w["resets_at"] / 1000 - NOW) / 60) if w.get("resets_at") else None
            limits.append({"name": label, "percent": w["percent"], "resetsAt": w.get("resets_at"), "resets_in_min": resets_in})
    if limits:
        save_json(os.path.join(CACHE, "limits.json"), {"at": int(NOW * 1000), "limits": limits})

    print(f"Synced usage for {len(agents)} agents: " + ", ".join(f"{a['name']} ({a['source']})" for a in agents))


if __name__ == "__main__":
    sys.exit(main())
