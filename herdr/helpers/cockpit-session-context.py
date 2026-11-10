#!/usr/bin/env python3
"""
cockpit-session-context.py <program> <session> [--limits '{"model": window}']
What one session's context window holds, as Claude's /context shows it: the tokens in use, the window, and the parts, from
the agent's own session data (Claude Code and Codex transcripts, OpenCode's database). The parts are estimated from the
transcript (4 characters a token): your prompts and the model's text, its reasoning, each kind of tool output; what the
transcript cannot show (the system prompt, tool definitions, memory files, an earlier compaction) is the remainder of the
tokens the agent reports in use. Prints JSON.
"""

import glob
import json
import os
import re
import sqlite3
import sys
from collections import defaultdict

LIMITS = {}


def kind_of(name):
    n = str(name).lower()
    if re.search(r"webfetch|websearch|web_search|fetch|search", n):
        return "Web and search"
    if n.startswith("mcp__"):
        return "MCP tools"
    if re.search(r"bash|shell|exec|sleep|terminal|command", n):
        return "Shell"
    if re.search(r"read|glob|grep|list|view|edit|write|patch|file", n):
        return "Files"
    return "Other tools"


def text_len(c):
    if isinstance(c, str):
        return len(c)
    if isinstance(c, list):
        return sum(len(x.get("text", "")) if isinstance(x, dict) else len(str(x)) for x in c)
    return len(str(c or ""))


def new():
    return {"cats": defaultdict(int), "tools": defaultdict(lambda: [0, 0]), "turns": 0, "used": 0, "model": None, "cost": None, "title": None}


def tool(r, name, chars):
    r["cats"][kind_of(name)] += chars
    r["tools"][name][0] += chars
    r["tools"][name][1] += 1


def claude(session):
    r = new()
    files = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}.jsonl"))
    if not files:
        return r
    names, last = {}, {}
    for line in open(files[0], encoding="utf-8", errors="ignore"):
        try:
            e = json.loads(line)
        except Exception:
            continue
        m = e.get("message") or {}
        c = m.get("content")
        if e.get("type") == "assistant":
            u = m.get("usage") or {}
            if u:
                last = {"used": (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0) + (u.get("output_tokens") or 0), "model": m.get("model")}
            for b in c if isinstance(c, list) else []:
                t = b.get("type")
                if t == "text":
                    r["cats"]["Messages"] += len(b.get("text") or "")
                elif t == "thinking":
                    r["cats"]["Reasoning"] += len(b.get("thinking") or "")
                elif t == "tool_use":
                    names[b.get("id")] = b.get("name")
                    r["cats"]["Messages"] += len(json.dumps(b.get("input") or {}))
        elif e.get("type") == "user" and not e.get("isMeta"):
            if isinstance(c, list):
                for b in c:
                    if b.get("type") == "tool_result":
                        tool(r, names.get(b.get("tool_use_id"), "tool"), text_len(b.get("content")))
            elif isinstance(c, str) and not c.startswith("<"):
                r["turns"] += 1
                r["cats"]["Messages"] += len(c)
                r["title"] = r["title"] or re.sub(r"\s+", " ", c).strip()[:70]
    r.update(last)
    return r


def codex(session):
    r = new()
    files = glob.glob(os.path.expanduser(f"~/.codex/sessions/**/*{session}*.jsonl"), recursive=True)
    if not files:
        return r
    calls, win = {}, None
    for line in open(files[0], encoding="utf-8", errors="ignore"):
        try:
            e = json.loads(line)
        except Exception:
            continue
        p = e.get("payload") or {}
        if e.get("type") == "turn_context":
            r["model"] = p.get("model") or r["model"]
        if e.get("type") == "event_msg" and p.get("type") == "token_count":
            info = p.get("info") or {}
            r["used"] = (info.get("last_token_usage") or {}).get("total_tokens") or r["used"]
            win = info.get("model_context_window") or win
        elif e.get("type") == "response_item":
            t = p.get("type")
            if t == "function_call":
                calls[p.get("call_id")] = p.get("name")
            elif t == "function_call_output":
                o = p.get("output")
                tool(r, calls.get(p.get("call_id"), "tool"), text_len(o.get("content") if isinstance(o, dict) else o))
            elif t == "message":
                txt = "".join(c.get("text", "") for c in p.get("content") or [] if isinstance(c, dict))
                if p.get("role") == "user" and txt and not txt.lstrip().startswith("<"):
                    r["turns"] += 1
                    r["title"] = r["title"] or re.sub(r"\s+", " ", txt).strip()[:70]
                r["cats"]["Messages"] += len(txt)
            elif t == "reasoning":
                r["cats"]["Reasoning"] += len(json.dumps(p.get("summary") or ""))
    r["window"] = win
    return r


def opencode(session):
    r = new()
    db = os.path.expanduser("~/.local/share/opencode/opencode.db")
    if not os.path.exists(db):
        return r
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    row = con.execute("select title, cost from session_v2 where id = ?", (session,)).fetchone()
    if row:
        r["title"], r["cost"] = row[0], row[1] or None
    for kind, data in con.execute("select type, data from session_message where session_id = ? order by seq", (session,)):
        try:
            d = json.loads(data)
        except Exception:
            continue
        if kind == "user" and d.get("text"):
            r["turns"] += 1
            r["cats"]["Messages"] += len(d["text"])
        elif kind == "assistant":
            tok = d.get("tokens") or {}
            used = (tok.get("input") or 0) + (tok.get("output") or 0) + (tok.get("reasoning") or 0) + ((tok.get("cache") or {}).get("read") or 0) + ((tok.get("cache") or {}).get("write") or 0)
            if used:
                r["used"], r["model"] = used, (d.get("model") or {}).get("id") or r["model"]
            for c in d.get("content") or []:
                t = c.get("type")
                if t == "tool":
                    tool(r, c.get("name") or "tool", len(str((c.get("state") or {}).get("content") or "")))
                elif t == "reasoning":
                    r["cats"]["Reasoning"] += len(c.get("text") or "")
                elif t == "text":
                    r["cats"]["Messages"] += len(c.get("text") or "")
    return r


def main():
    if len(sys.argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    program, session = sys.argv[1].lower(), sys.argv[2]
    if "--limits" in sys.argv:
        try:
            LIMITS.update(json.loads(sys.argv[sys.argv.index("--limits") + 1]))
        except Exception:
            pass
    r = claude(session) if "claude" in program else codex(session) if "codex" in program else opencode(session) if "opencode" in program else new()
    used = int(r["used"] or 0)
    window = r.get("window") or LIMITS.get(r["model"]) or (1_000_000 if used > 200_000 or "1m" in str(r["model"]) else 200_000 if "claude" in program else None)
    est = {k: v // 4 for k, v in r["cats"].items() if v}
    total = sum(est.values())
    parts, note = [], None
    if used and total > used:
        # An earlier compaction dropped part of what the transcript still holds: scale to what is in use.
        est = {k: int(v * used / total) for k, v in est.items()}
        total, note = sum(est.values()), "scaled: part of the transcript was compacted away"
    for k, v in sorted(est.items(), key=lambda kv: -kv[1]):
        parts.append({"name": k, "tokens": v})
    if used > total:
        parts.append({"name": "System, tools and memory", "tokens": used - total})
    tools = [{"name": n, "tokens": c // 4, "calls": k} for n, (c, k) in sorted(r["tools"].items(), key=lambda kv: -kv[1][0])[:6]]
    print(json.dumps({"program": program, "session": session, "title": r["title"], "model": r["model"], "used": used, "window": window, "percent": round(used * 100.0 / window, 1) if window and used else None, "cost": r["cost"], "turns": r["turns"], "parts": parts, "tools": tools, "note": note}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
