#!/usr/bin/env python3
"""
cockpit-perf.py '<json>'
What the agents cost the machines, for Home and the sidebar: this computer's CPU, memory and GPU, the GPU of other machines (perf.remotes in the cockpit config), and per agent its processes' CPU and memory and its latest turn's tokens per second.

<json> is [{address, pane, session, program, cwd}] for this machine's agents. Their processes are the ones whose
environment carries the agent's Herdr pane (HERDR_PANE_ID), with their children. Tokens per second is the latest finished
answer's output and reasoning tokens from its first token to the stream's end: OpenCode from opencode.db (the session, or the newest one in
its folder when the hub has none), Claude Code from its transcript. Prints JSON:
  {at, system: {cpu, cores, mem_used, mem_total, gpu}, remote: {<name>: {gpu, vram_used, vram_total, at}},
   agents: {<address>: {cpu, mem, tps, tokens, at, session}}}
cpu is % of one core (an agent) or of the whole machine (system); mem in MB; remote numbers are cached 20 s.
"""

import glob
import json
import os
import platform
import re
import sqlite3
import subprocess
import sys
import time
import urllib.request
from datetime import datetime

CACHE = os.path.expanduser("~/.cache/cockpit")
NOW = time.time()
OPENCODE_DB = os.path.expanduser("~/.local/share/opencode/opencode.db")
os.environ["PATH"] = os.pathsep.join([os.environ.get("PATH", ""), "/opt/homebrew/bin", "/usr/local/bin", "/usr/sbin", "/sbin"])
MAC = platform.system() == "Darwin"


def run(argv, timeout=5):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout).stdout
    except Exception:
        return ""


# ---------------------------------------------------------------------------------------------------------------------
# Processes

def processes():
    """[(pid, ppid, cpu, rss_kb, env)]: env is the HERDR_* variables, from `ps -E` on macOS, /proc on Linux."""
    out = []
    if MAC:
        for line in run(["ps", "-Aww", "-E", "-o", "pid=,ppid=,%cpu=,rss=,command="]).splitlines():
            parts = line.split(None, 4)
            if len(parts) < 4:
                continue
            env = dict(re.findall(r"\b(HERDR_[A-Z_]+)=(\S+)", parts[4] if len(parts) > 4 else ""))
            out.append((int(parts[0]), int(parts[1]), float(parts[2]), int(parts[3]), env))
        return out
    for line in run(["ps", "-A", "-o", "pid=,ppid=,%cpu=,rss="]).splitlines():
        parts = line.split()
        if len(parts) < 4:
            continue
        pid = int(parts[0])
        env = {}
        try:
            with open(f"/proc/{pid}/environ", "rb") as f:
                for kv in f.read().split(b"\0"):
                    if kv.startswith(b"HERDR_"):
                        k, _, v = kv.decode(errors="ignore").partition("=")
                        env[k] = v
        except Exception:
            pass
        out.append((pid, int(parts[1]), float(parts[2]), int(parts[3]), env))
    return out


def terminal_info(address):
    """`hc get`: the terminal's shell pid (app tabs carry no HERDR_PANE_ID, but the core knows the shell) and its title,
    which OpenCode sets to the session's title."""
    try:
        return json.loads(run(["hc", "get", address]) or "{}")
    except Exception:
        return {}


def load_of(procs, pane, address=None):
    """CPU (% of a core) and memory (MB) of the agent's processes and their children: the ones carrying its pane id, else
    the terminal's shell and everything under it."""
    mine = {p[0] for p in procs if pane and p[4].get("HERDR_PANE_ID") == pane}
    if not mine and address:
        pid = terminal_info(address).get("pid")
        mine = {pid} if pid else set()
    if not mine:
        return None
    kids = {}
    for p in procs:
        kids.setdefault(p[1], []).append(p[0])
    todo = list(mine)
    while todo:
        for k in kids.get(todo.pop(), []):
            if k not in mine:
                mine.add(k)
                todo.append(k)
    rows = [p for p in procs if p[0] in mine]
    return {"cpu": round(sum(p[2] for p in rows)), "mem": round(sum(p[3] for p in rows) / 1024), "procs": len(rows)}


# ---------------------------------------------------------------------------------------------------------------------
# The machines

def system(procs):
    cores = os.cpu_count() or 1
    cpu = round(min(100.0, sum(p[2] for p in procs) / cores), 1)
    mem_used = mem_total = None
    if MAC:
        mem_total = int(run(["sysctl", "-n", "hw.memsize"]).strip() or 0) // 1048576 or None
        vm = run(["vm_stat"])
        page = int((re.search(r"page size of (\d+)", vm) or [0, 16384])[1])
        pages = {k: int(v) for k, v in re.findall(r"Pages (active|wired down|occupied by compressor):\s+(\d+)", vm)}
        if pages:
            mem_used = sum(pages.values()) * page // 1048576
    else:
        try:
            info = dict(re.findall(r"(\w+):\s+(\d+)", open("/proc/meminfo").read()))
            mem_total = int(info["MemTotal"]) // 1024
            mem_used = mem_total - int(info["MemAvailable"]) // 1024
        except Exception:
            pass
    return {"cpu": cpu, "cores": cores, "mem_used": mem_used, "mem_total": mem_total, "gpu": gpu_here()}


def gpu_here():
    """This machine's GPU busy %: the Apple GPU's Device Utilization, else nvidia-smi."""
    if MAC:
        m = re.search(r'"Device Utilization %"=(\d+)', run(["ioreg", "-r", "-d", "1", "-c", "IOAccelerator"]))
        return int(m[1]) if m else None
    line = run(["nvidia-smi", "--query-gpu=utilization.gpu", "--format=csv,noheader,nounits"]).strip().splitlines()
    return int(line[0]) if line and line[0].strip().isdigit() else None


def remote(name, host):
    """Another machine's GPU (e.g. the box a local model runs on): busy %, VRAM, over ssh at most every 20 s."""
    path = os.path.join(CACHE, f"perf-{re.sub(r'[^A-Za-z0-9_.-]', '_', name)}.json")
    try:
        cached = json.load(open(path))
        if NOW - cached.get("at", 0) / 1000 < 20:
            return cached
    except Exception:
        cached = None
    out = run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=3", host, "nvidia-smi --query-gpu=utilization.gpu,memory.used,memory.total,name --format=csv,noheader,nounits"], timeout=8).strip()
    parts = [p.strip() for p in out.splitlines()[0].split(",")] if out else []
    if len(parts) >= 3 and parts[0].isdigit():
        d = {"gpu": int(parts[0]), "vram_used": int(parts[1]), "vram_total": int(parts[2]), "name": parts[3] if len(parts) > 3 else None, "at": int(NOW * 1000)}
    else:
        # Away: the last answer, marked, until it is a minute old.
        d = {**cached, "stale": True} if cached and NOW - cached.get("at", 0) / 1000 < 60 else {"gpu": None, "at": int(NOW * 1000), "away": True}
    try:
        with open(path + ".tmp", "w") as f:
            json.dump(d, f)
        os.replace(path + ".tmp", path)
    except Exception:
        pass
    return d


def remotes():
    """perf.remotes in ~/.config/cockpit/config.json: [{"name": "gpu-box", "ssh": "host"}], none by default."""
    try:
        cfg = json.load(open(os.path.expanduser("~/.config/cockpit/config.json")))
    except Exception:
        return {}
    return {r["name"]: remote(r["name"], r.get("ssh") or r["name"]) for r in (cfg.get("perf") or {}).get("remotes") or [] if r.get("name")}


# ---------------------------------------------------------------------------------------------------------------------
# Tokens per second

def session_by_name(con, cwd, name):
    """A tab with no session id from the hub: the newest root session in its folder whose title or first prompt carries the
    tab's number (review mono#2007 -> "Pull request 2007 review"). The newest when several do (a reopened tab), none when no one does."""
    nums = re.findall(r"\d{3,}", name or "")
    if not nums or not cwd:
        return None
    hits = []
    for sid, title, first in con.execute(
        "select s.id, s.title, (select data from session_message m where m.session_id = s.id and m.type = 'user' order by m.seq limit 1)"
        " from session_v2 s where s.parent_id is null and s.directory = ? and s.time_archived is null and s.time_updated > ? order by s.time_updated desc",
        (cwd, (NOW - 7 * 86400) * 1000),
    ):
        text = f"{title or ''} {first or ''}"
        if all(n in re.findall(r"\d{3,}", text) for n in nums):
            hits.append(sid)
    return hits[0] if hits else None


def session_by_title(con, cwd, title):
    """OpenCode sets its terminal's title to "<tag> | <session title>": the newest root session in the folder with that title."""
    t = re.sub(r"^\w{1,3} \| ", "", title or "").strip()
    if not t or not cwd:
        return None
    hit = con.execute("select id from session_v2 where parent_id is null and directory = ? and title = ? and time_archived is null order by time_updated desc limit 1", (cwd, t)).fetchone()
    return hit[0] if hit else None


def opencode_tps(con, session, cwd, name=None, title=None):
    session = session or session_by_title(con, cwd, title) or session_by_name(con, cwd, name)
    if not session:
        return None
    # The session and its sub-agents: whichever answered last.
    ids = [session] + [r[0] for r in con.execute(
        "with recursive k(id) as (select id from session_v2 where parent_id = ? union all select s.id from session_v2 s join k on s.parent_id = k.id) select id from k", (session,))]
    marks = ",".join("?" * len(ids))
    for (data,) in con.execute(f"select data from session_message where session_id in ({marks}) and type = 'assistant' order by rowid desc limit 8", ids):
        try:
            d = json.loads(data)
        except Exception:
            continue
        t = d.get("time") or {}
        tok = d.get("tokens") or {}
        n = (tok.get("output") or 0) + (tok.get("reasoning") or 0)
        # From its first part (the first token) to the stream's end (`streamed`): the model's decode speed, not the wait
        # before it nor the tool runs after.
        firsts = [(c.get("time") or {}).get("created") for c in d.get("content") or [] if isinstance(c, dict)]
        start = min([x for x in firsts if x] or [t.get("created") or 0])
        end = t.get("streamed") or t.get("completed")
        if n and start and end and end - start > 200:
            return {"tps": round(n * 1000 / (end - start), 1), "tokens": n, "at": end, "session": session, "model": (d.get("model") or {}).get("id")}
    return {"session": session}


def claude_tps(session):
    files = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}.jsonl"))
    if not files:
        return None
    with open(files[0], "rb") as f:
        f.seek(max(0, os.path.getsize(files[0]) - 262144))
        lines = f.read().decode(errors="ignore").splitlines()[1:]
    prev = None
    best = None
    for line in lines:
        try:
            e = json.loads(line)
            at = e.get("timestamp")
            ms = int(datetime.fromisoformat(at.replace("Z", "+00:00")).timestamp() * 1000) if at else None
        except Exception:
            continue
        u = (e.get("message") or {}).get("usage") if e.get("type") == "assistant" else None
        if u and prev and ms and ms > prev and u.get("output_tokens"):
            best = {"tps": round(u["output_tokens"] * 1000 / (ms - prev), 1), "tokens": u["output_tokens"], "at": ms, "session": session, "model": (e.get("message") or {}).get("model")}
        if ms:
            prev = ms
    return best


# ---------------------------------------------------------------------------------------------------------------------
# What each agent runs on: model, effort, sandbox, cost, and how full its context window is.

def cfg():
    try:
        return json.load(open(os.path.expanduser("~/.config/cockpit/config.json")))
    except Exception:
        return {}


def window_of(model):
    """A model's context window: `context.limits` in the cockpit config, else the gateway's /v1/model/info (hour cache)."""
    c = cfg()
    lim = ((c.get("context") or {}).get("limits") or {}).get(model)
    if lim:
        return int(lim)
    ll = c.get("litellm") or {}
    if not ll.get("base_url"):
        return None
    path = os.path.join(CACHE, "model-limits.json")
    try:
        cache = json.load(open(path))
    except Exception:
        cache = {}
    if NOW - cache.get("at", 0) > 3600:
        key = os.environ.get("LITELLM_API_KEY", "")
        kf = os.path.expanduser(ll.get("key_file") or "")
        if not key and kf and os.path.exists(kf):
            key = open(kf, encoding="utf-8").read().strip()
        try:
            req = urllib.request.Request(ll["base_url"].rstrip("/") + "/v1/model/info", headers={"Authorization": f"Bearer {key}"} if key else {})
            with urllib.request.urlopen(req, timeout=4) as r:
                rows = json.load(r).get("data") or []
            cache = {"at": NOW, "limits": {m["model_name"]: (m.get("model_info") or {}).get("max_input_tokens") for m in rows if m.get("model_name")}}
            with open(path + ".tmp", "w") as f:
                json.dump(cache, f)
            os.replace(path + ".tmp", path)
        except Exception:
            cache = {**cache, "at": NOW - 3000}
    return (cache.get("limits") or {}).get(model)


def ctx(used, window):
    return {"used": used, "window": window, "percent": round(used * 100.0 / window, 1) if window else None} if used else None


def info_opencode(con, session):
    if not (con and session):
        return {}
    for (data,) in con.execute("select data from session_message where session_id = ? and type = 'assistant' order by rowid desc limit 12", (session,)):
        try:
            d = json.loads(data)
        except Exception:
            continue
        tok = d.get("tokens") or {}
        used = (tok.get("input") or 0) + (tok.get("output") or 0) + (tok.get("reasoning") or 0) + ((tok.get("cache") or {}).get("read") or 0) + ((tok.get("cache") or {}).get("write") or 0)
        model = (d.get("model") or {}).get("id")
        if used and model:
            return {"model": model, "context": ctx(used, window_of(model))}
    return {}


def info_claude(session):
    out = {}
    files = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}.jsonl"))
    if files:
        with open(files[0], "rb") as f:
            f.seek(max(0, os.path.getsize(files[0]) - 262144))
            lines = f.read().decode(errors="ignore").splitlines()
        for line in reversed(lines):
            if '"usage"' not in line:
                continue
            try:
                m = json.loads(line).get("message") or {}
                u = m.get("usage") or {}
                used = (u.get("input_tokens") or 0) + (u.get("cache_read_input_tokens") or 0) + (u.get("cache_creation_input_tokens") or 0) + (u.get("output_tokens") or 0)
                if used and m.get("model"):
                    out = {"model": m["model"], "context": ctx(used, 1_000_000 if used > 200_000 or "1m" in m["model"] else 200_000)}
                    break
            except Exception:
                continue
    # The statusline's payload: effort and cost, when it is this session's.
    try:
        s = json.load(open(os.path.join(CACHE, "usage-claude.json")))
        if s.get("session_id") == session:
            out.update(effort=(s.get("effort") or {}).get("level"), cost=(s.get("cost") or {}).get("total_cost_usd"), model_name=(s.get("model") or {}).get("display_name"))
            cw = s.get("context_window") or {}
            if cw.get("used_percentage") is not None:
                out["context"] = {"percent": round(cw["used_percentage"], 1), "window": cw.get("context_window_size"), "used": None}
    except Exception:
        pass
    return out


def info_codex(session):
    files = glob.glob(os.path.expanduser(f"~/.codex/sessions/**/*{session}*.jsonl"), recursive=True)
    if not files:
        return {}
    out = {}
    with open(files[0], "r", encoding="utf-8", errors="ignore") as f:
        head = [next(f, "") for _ in range(40)]
    for line in head:
        if '"turn_context"' in line:
            try:
                p = json.loads(line).get("payload") or {}
                out = {"model": p.get("model"), "effort": p.get("effort"), "sandbox": (p.get("sandbox_policy") or {}).get("type"), "approval": p.get("approval_policy")}
                break
            except Exception:
                continue
    with open(files[0], "rb") as f:
        f.seek(max(0, os.path.getsize(files[0]) - 262144))
        tail = f.read().decode(errors="ignore").splitlines()
    for line in reversed(tail):
        if '"token_count"' in line:
            try:
                info = (json.loads(line).get("payload") or {}).get("info") or {}
                used = (info.get("last_token_usage") or {}).get("total_tokens")
                if used:
                    out["context"] = ctx(used, info.get("model_context_window") or 200_000)
                    break
            except Exception:
                continue
    return out


def info_antigravity(session):
    path = os.path.expanduser(f"~/.gemini/antigravity-cli/brain/{session}/.system_generated/logs/transcript.jsonl")
    if not os.path.exists(path):
        return {}
    for line in reversed(tail_lines(path)):
        try:
            e = json.loads(line)
        except Exception:
            continue
        if "input_tokens" in e:
            return {"context": ctx((e.get("input_tokens") or 0) + (e.get("output_tokens") or 0) + (e.get("cache_read_tokens") or 0), 1_048_576)}
    return {}


def tail_lines(path, limit=400):
    try:
        with open(path, "r", encoding="utf-8", errors="ignore") as f:
            return f.readlines()[-limit:]
    except Exception:
        return []



def main():
    try:
        wanted = json.loads(sys.argv[1]) if len(sys.argv) > 1 else []
    except Exception:
        wanted = []
    procs = processes()
    con = sqlite3.connect(f"file:{OPENCODE_DB}?mode=ro", uri=True) if os.path.exists(OPENCODE_DB) else None
    agents = {}
    for a in wanted:
        prog = str(a.get("program") or "").lower()
        out = dict(load_of(procs, a.get("pane"), a.get("address")) or {})
        try:
            speed = opencode_tps(con, a.get("session"), a.get("cwd"), a.get("name"), terminal_info(a.get("address")).get("title")) if con and "opencode" in prog else claude_tps(a["session"]) if "claude" in prog and a.get("session") else None
        except Exception:
            speed = None
        out.update(speed or {})
        session = out.get("session") or a.get("session")
        try:
            info = info_opencode(con, session) if "opencode" in prog else info_claude(session) if "claude" in prog and session else info_codex(session) if "codex" in prog and session else info_antigravity(session) if re.search("antigravity|agy|gemini", prog) and session else {}
        except Exception:
            info = {}
        # The answer's own model beats a guess from the transcript.
        out["info"] = {k: v for k, v in {**info, "model": info.get("model") or out.get("model")}.items() if v is not None}
        agents[a.get("address")] = out
    result = {"at": int(NOW * 1000), "system": system(procs), "remote": remotes(), "agents": agents}
    print(json.dumps(result))


if __name__ == "__main__":
    sys.exit(main())
