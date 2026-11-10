#!/usr/bin/env python3
"""
cockpit-subagent-output.py <program> <subagent-id> [--last N]
What a sub-agent did, newest last: its prompt, its text, the tools it ran with their output, from the agent's own session
data (an OpenCode child session in opencode.db, a Claude Code Task agent's transcript under the parent session's
subagents folder). Each item is {kind: prompt|text|tool|reasoning, name, text}, long text cut short. Prints JSON.
"""

import glob
import json
import os
import re
import sqlite3
import sys

CUT = 1800


def cut(s):
    s = str(s or "")
    return s if len(s) <= CUT else s[:CUT] + f"… ({len(s) - CUT} more characters)"


def brief(v):
    if isinstance(v, dict):
        for k in ("command", "description", "filePath", "file_path", "path", "pattern", "url", "query", "prompt"):
            if v.get(k):
                return str(v[k])
        return json.dumps(v)[:160]
    return str(v or "")[:160]


def text_of(c):
    if isinstance(c, str):
        return c
    if isinstance(c, list):
        return "\n".join(x.get("text", "") if isinstance(x, dict) else str(x) for x in c)
    return str(c or "")


def opencode(sid):
    db = os.path.expanduser("~/.local/share/opencode/opencode.db")
    if not os.path.exists(db):
        return None, []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True, timeout=2)
    row = con.execute("select title from session_v2 where id = ?", (sid,)).fetchone()
    items = []
    for kind, data in con.execute("select type, data from session_message where session_id = ? order by seq", (sid,)):
        try:
            d = json.loads(data)
        except Exception:
            continue
        if kind == "user" and d.get("text"):
            items.append({"kind": "prompt", "name": None, "text": cut(d["text"])})
        elif kind == "assistant":
            for c in d.get("content") or []:
                t = c.get("type")
                if t == "text" and c.get("text"):
                    items.append({"kind": "text", "name": None, "text": cut(c["text"])})
                elif t == "reasoning" and c.get("text"):
                    items.append({"kind": "reasoning", "name": None, "text": cut(c["text"])})
                elif t == "tool":
                    st = c.get("state") or {}
                    items.append({"kind": "tool", "name": c.get("name"), "text": cut(f"{brief(st.get('input'))}\n{text_of(st.get('content'))}".strip())})
    return (row[0] if row else None), items


def claude(sid):
    files = glob.glob(os.path.expanduser(f"~/.claude/projects/*/*/subagents/{sid}.jsonl"))
    if not files:
        return None, []
    names, items, title = {}, [], None
    meta = files[0][:-6] + ".meta.json"
    try:
        title = json.load(open(meta)).get("description")
    except Exception:
        pass
    for line in open(files[0], encoding="utf-8", errors="ignore"):
        try:
            e = json.loads(line)
        except Exception:
            continue
        c = (e.get("message") or {}).get("content")
        if e.get("type") == "assistant":
            for b in c if isinstance(c, list) else []:
                t = b.get("type")
                if t == "text" and b.get("text"):
                    items.append({"kind": "text", "name": None, "text": cut(b["text"])})
                elif t == "thinking" and b.get("thinking"):
                    items.append({"kind": "reasoning", "name": None, "text": cut(b["thinking"])})
                elif t == "tool_use":
                    names[b.get("id")] = (b.get("name"), brief(b.get("input")))
        elif e.get("type") == "user":
            if isinstance(c, list):
                for b in c:
                    if b.get("type") == "tool_result":
                        name, arg = names.get(b.get("tool_use_id"), ("tool", ""))
                        items.append({"kind": "tool", "name": name, "text": cut(f"{arg}\n{text_of(b.get('content'))}".strip())})
            elif isinstance(c, str) and c.strip():
                items.append({"kind": "prompt", "name": None, "text": cut(c)})
    return title, items


def main():
    if len(sys.argv) < 3:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    program, sid = sys.argv[1].lower(), sys.argv[2]
    last = int(sys.argv[sys.argv.index("--last") + 1]) if "--last" in sys.argv[:-1] else 40
    try:
        title, items = claude(sid) if "claude" in program else opencode(sid)
    except sqlite3.Error as e:
        print(json.dumps({"error": str(e)[:120], "items": []}))
        return 0
    print(json.dumps({"id": sid, "title": title, "total": len(items), "items": items[-last:]}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
