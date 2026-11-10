#!/usr/bin/env python3
"""
cockpit-subagents.py [--hours N] <session> [<session> ...]
The sub-agents each agent session started, which herdr cannot see: OpenCode child sessions (oh-my-opencode-slim's
explorer, fixer, oracle…, from opencode.db) and Claude Code's Task agents (<session>/subagents/*.meta.json).
Prints {"<session>": [{id, name, kind, status, since, until, cost}]} (status: working, done, failed, stopped), newest
first, from the last 24 hours (--hours); also writes it to ~/.cache/cockpit/subagents.json.
"""

import glob
import json
import os
import sqlite3
import sys
import time

CACHE = os.path.expanduser("~/.cache/cockpit/subagents.json")
DB = os.path.expanduser("~/.local/share/opencode/opencode.db")
DAY_MS = 24 * 3600 * 1000
OUTCOMES = {"succeeded": "done", "failed": "failed", "interrupted": "stopped"}


FAILED = set()


def opencode(sessions, now):
    """Children of the given roots, and theirs (a fixer's own explorer), each under its root."""
    out = {}
    if not sessions or not os.path.exists(DB):
        return out
    try:
        con = sqlite3.connect(f"file:{DB}?mode=ro", uri=True, timeout=2)
        marks = ",".join("?" * len(sessions))
        rows = con.execute(
            f"""with recursive kids(id, root) as (
                  select id, parent_id from session_v2 where parent_id in ({marks})
                  union all select s.id, k.root from session_v2 s join kids k on s.parent_id = k.id)
                select k.root, s.id, s.title, s.agent, s.time_created, s.time_busy, s.time_idle, s.idle_outcome, s.cost
                from kids k join session_v2 s on s.id = k.id
                where s.time_updated > ? and s.time_archived is null""",
            (*sessions, now - DAY_MS),
        ).fetchall()
    except sqlite3.Error:
        # A busy database is not "no sub-agents": say which sessions could not be read.
        FAILED.update(sessions)
        return out
    for root, sid, title, agent, created, busy, idle, outcome, cost in rows:
        working = (busy or 0) > (idle or 0) or (idle is None and not outcome)
        out.setdefault(root, []).append({
            "id": sid,
            "name": title or agent or sid,
            "kind": agent,
            "status": "working" if working else OUTCOMES.get(outcome, "done"),
            "since": busy or created,
            "until": None if working else idle,
            "cost": round(cost, 4) if cost else None,
        })
    return out


def last_entry(path):
    """The last JSON line of a transcript, reading only its tail."""
    try:
        with open(path, "rb") as f:
            f.seek(0, 2)
            f.seek(max(0, f.tell() - 65536))
            lines = f.read().decode("utf-8", "ignore").strip().splitlines()
        return json.loads(lines[-1]) if lines else {}
    except Exception:
        return {}


def claude(sessions, now):
    out = {}
    for session in sessions:
        for meta in glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}/subagents/*.meta.json")):
            log = meta[: -len(".meta.json")] + ".jsonl"
            try:
                m = json.load(open(meta))
                mtime = int(os.path.getmtime(log if os.path.exists(log) else meta) * 1000)
                created = int(os.path.getctime(meta) * 1000)
            except Exception:
                continue
            if mtime < now - DAY_MS:
                continue
            last = last_entry(log)
            ended = last.get("type") == "assistant" and (last.get("message") or {}).get("stop_reason") == "end_turn"
            working = not ended and now - mtime < 90_000
            out.setdefault(session, []).append({
                "id": os.path.basename(log)[: -len(".jsonl")],
                "name": m.get("description") or m.get("agentType") or "subagent",
                "kind": m.get("agentType"),
                "status": "working" if working else "done" if ended else "stopped",
                "since": created,
                "until": None if working else mtime,
                "cost": None,
            })
    return out


def main():
    global DAY_MS
    args = sys.argv[1:]
    if "--hours" in args:
        i = args.index("--hours")
        DAY_MS = int(float(args[i + 1]) * 3600 * 1000)
        del args[i : i + 2]
    sessions = [s for s in args if s and not s.startswith("-")]
    now = int(time.time() * 1000)
    found = {}
    for part in (opencode([s for s in sessions if s.startswith("ses_")], now), claude([s for s in sessions if not s.startswith("ses_")], now)):
        found.update(part)
    for kids in found.values():
        kids.sort(key=lambda k: (k["status"] != "working", -(k["since"] or 0)))
    result = {"at": now, "sessions": found, "failed": sorted(FAILED)}
    os.makedirs(os.path.dirname(CACHE), exist_ok=True)
    with open(CACHE + ".tmp", "w", encoding="utf-8") as f:
        json.dump(result, f)
    os.replace(CACHE + ".tmp", CACHE)
    print(json.dumps(result))
    return 0


if __name__ == "__main__":
    sys.exit(main())
