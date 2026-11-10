#!/usr/bin/env python3
"""
cockpit-usage-stats.py [--days N]
Where the tokens went in the last N days (7 by default), for the Usage tab, read from each agent's own session data:
  sessions  the biggest sessions across Claude Code, Codex and OpenCode: tokens (input + output + reasoning, cached
            input counted apart), turns, cost where the agent records one, and the folder they ran in
  eaters    what fills a context window, per agent: the output of each tool, the model's reasoning, its text, your
            prompts, as tokens estimated at 4 characters each
Writes ~/.cache/cockpit/usage-stats.json (kept for ten minutes) and prints it.
"""

import glob
import json
import os
import re
import sqlite3
import sys
import time
from collections import defaultdict
from datetime import datetime

CACHE = os.path.expanduser("~/.cache/cockpit")
OUT = os.path.join(CACHE, "usage-stats.json")
NOW = time.time()
DAYS = int(sys.argv[sys.argv.index("--days") + 1]) if "--days" in sys.argv[:-1] else 7
SINCE = NOW - DAYS * 86400


def est(chars):
    return int(chars / 4)


def clean_title(text):
    t = re.sub(r"<[^>]+>", " ", text or "")
    t = re.sub(r"\s+", " ", t).strip()
    return t[:70] or None


def top(d, n=8):
    return [{"name": k, "tokens": v["chars"] // 4, "calls": v["calls"]} for k, v in sorted(d.items(), key=lambda kv: -kv[1]["chars"])[:n]]


def bump(d, key, chars):
    d[key]["chars"] += chars
    d[key]["calls"] += 1


def text_len(content):
    if isinstance(content, str):
        return len(content)
    if isinstance(content, list):
        return sum(len(c.get("text", "")) if isinstance(c, dict) else len(str(c)) for c in content)
    return len(str(content or ""))


# ---------------------------------------------------------------------------------------------------------------------
def claude():
    sessions, eaters = [], defaultdict(lambda: {"chars": 0, "calls": 0})
    for f in glob.glob(os.path.expanduser("~/.claude/projects/*/*.jsonl")):
        if os.path.getmtime(f) < SINCE:
            continue
        names, seen, title, turns, tokens, cached, last = {}, {}, None, 0, 0, 0, os.path.getmtime(f)
        try:
            for line in open(f, encoding="utf-8", errors="ignore"):
                if '"usage"' not in line and '"tool_use"' not in line and '"tool_result"' not in line and '"type":"user"' not in line:
                    continue
                try:
                    e = json.loads(line)
                except Exception:
                    continue
                m = e.get("message") or {}
                content = m.get("content")
                if e.get("type") == "assistant":
                    u = m.get("usage") or {}
                    if m.get("id"):
                        seen[m["id"]] = ((u.get("input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0) + (u.get("output_tokens") or 0), u.get("cache_read_input_tokens") or 0)
                    for c in content if isinstance(content, list) else []:
                        if c.get("type") == "tool_use":
                            names[c.get("id")] = c.get("name")
                        elif c.get("type") == "thinking":
                            bump(eaters, "reasoning", len(c.get("thinking") or ""))
                        elif c.get("type") == "text":
                            bump(eaters, "model text", len(c.get("text") or ""))
                elif e.get("type") == "user" and not e.get("isMeta"):
                    if isinstance(content, list):
                        for c in content:
                            if c.get("type") == "tool_result":
                                bump(eaters, names.get(c.get("tool_use_id"), "tool"), text_len(c.get("content")))
                    else:
                        t = clean_title(content if isinstance(content, str) else "")
                        if t and not t.startswith("[Request"):
                            turns += 1
                            title = title or t
                            bump(eaters, "your prompts", len(content))
        except Exception:
            continue
        tokens = sum(v[0] for v in seen.values())
        cached = sum(v[1] for v in seen.values())
        if tokens:
            sessions.append({"agent": "claude", "id": os.path.basename(f)[:-6], "title": title, "project": os.path.basename(os.path.dirname(f)).replace("-Users-", "~/").replace("-", "/")[:60], "tokens": tokens, "cached": cached, "turns": turns, "at": int(last * 1000)})
    return sessions, top(eaters)


def codex():
    sessions, eaters = [], defaultdict(lambda: {"chars": 0, "calls": 0})
    for f in glob.glob(os.path.expanduser("~/.codex/sessions/**/*.jsonl"), recursive=True):
        if os.path.getmtime(f) < SINCE:
            continue
        calls, title, total, cached, turns = {}, None, 0, 0, 0
        try:
            for line in open(f, encoding="utf-8", errors="ignore"):
                try:
                    e = json.loads(line)
                except Exception:
                    continue
                p = e.get("payload") or {}
                if e.get("type") == "event_msg" and p.get("type") == "token_count":
                    u = (p.get("info") or {}).get("total_token_usage") or {}
                    if u:
                        total = (u.get("input_tokens") or 0) - (u.get("cached_input_tokens") or 0) + (u.get("output_tokens") or 0)
                        cached = u.get("cached_input_tokens") or 0
                elif e.get("type") == "response_item":
                    t = p.get("type")
                    if t == "function_call":
                        calls[p.get("call_id")] = p.get("name")
                    elif t == "function_call_output":
                        o = p.get("output")
                        bump(eaters, calls.get(p.get("call_id"), "tool"), text_len(o.get("content") if isinstance(o, dict) else o))
                    elif t == "message" and p.get("role") == "user":
                        txt = "".join(c.get("text", "") for c in p.get("content") or [] if isinstance(c, dict))
                        if txt and not txt.lstrip().startswith("<"):
                            turns += 1
                            title = title or clean_title(txt)
                            bump(eaters, "your prompts", len(txt))
                    elif t == "reasoning":
                        bump(eaters, "reasoning", len(json.dumps(p.get("summary") or "")))
        except Exception:
            continue
        if total:
            sessions.append({"agent": "codex", "id": os.path.basename(f)[:-6][-36:], "title": title, "project": None, "tokens": total, "cached": cached, "turns": turns, "at": int(os.path.getmtime(f) * 1000)})
    return sessions, top(eaters)


def opencode():
    db = os.path.expanduser("~/.local/share/opencode/opencode.db")
    if not os.path.exists(db):
        return [], []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    rows = con.execute(
        "with recursive k(id, root) as (select id, id from session_v2 where parent_id is null union all select s.id, k.root from session_v2 s join k on s.parent_id = k.id) "
        "select r.id, r.title, r.directory, sum(s.tokens_input + s.tokens_output + s.tokens_reasoning), sum(s.tokens_cache_read), sum(s.cost), max(s.time_updated), count(s.id) "
        "from k join session_v2 s on s.id = k.id join session_v2 r on r.id = k.root where s.time_updated > ? group by r.id order by 4 desc limit 40",
        (int(SINCE * 1000),),
    ).fetchall()
    sessions = [{"agent": "opencode", "id": r[0], "title": clean_title(r[1]), "project": (r[2] or "").replace(os.path.expanduser("~"), "~")[:60], "tokens": int(r[3] or 0), "cached": int(r[4] or 0), "cost": r[5] or None, "turns": None, "at": int(r[6] or 0), "agents": r[7]} for r in rows if r[3]]
    eaters = defaultdict(lambda: {"chars": 0, "calls": 0})
    for r in rows[:12]:
        for (data,) in con.execute("with recursive k(id) as (select ? union all select s.id from session_v2 s join k on s.parent_id = k.id) select data from session_message where session_id in (select id from k) and type in ('assistant','user')", (r[0],)):
            try:
                d = json.loads(data)
            except Exception:
                continue
            if d.get("text"):
                bump(eaters, "your prompts", len(d["text"]))
            for c in d.get("content") or []:
                t = c.get("type")
                if t == "tool":
                    bump(eaters, c.get("name") or "tool", len(str((c.get("state") or {}).get("content") or "")))
                elif t == "reasoning":
                    bump(eaters, "reasoning", len(c.get("text") or ""))
                elif t == "text":
                    bump(eaters, "model text", len(c.get("text") or ""))
    return sessions, top(eaters)


def main():
    try:
        old = json.load(open(OUT))
        if old.get("days") == DAYS and "per_agent" in old and NOW - old.get("at", 0) / 1000 < 600 and "--force" not in sys.argv:
            print(json.dumps(old))
            return 0
    except Exception:
        pass
    sessions, eaters = [], {}
    for name, fn in (("claude", claude), ("codex", codex), ("opencode", opencode)):
        try:
            s, e = fn()
        except Exception:
            s, e = [], []
        sessions += s
        eaters[name] = e
    sessions.sort(key=lambda s: -s["tokens"])
    by = defaultdict(lambda: {"tokens": 0, "cached": 0, "sessions": 0})
    for s in sessions:
        by[s["agent"]]["tokens"] += s["tokens"]
        by[s["agent"]]["cached"] += s["cached"]
        by[s["agent"]]["sessions"] += 1
    # The five biggest of each agent, for its own column.
    per = {a: [s for s in sessions if s["agent"] == a][:5] for a in ("claude", "codex", "opencode")}
    result = {"at": int(NOW * 1000), "days": DAYS, "sessions": sessions[:10], "per_agent": per, "by_agent": by, "eaters": eaters}
    os.makedirs(CACHE, exist_ok=True)
    with open(OUT + ".tmp", "w") as f:
        json.dump(result, f)
    os.replace(OUT + ".tmp", OUT)
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
