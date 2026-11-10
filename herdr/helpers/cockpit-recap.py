#!/usr/bin/env python3
"""
cockpit-recap.py <terminal> [--agent <id>] [--model <name>] [--title] [--session <id> --program <name> [--machine <name>]]
An agent's recap written by any agent and model, not only the hub's own: reads the terminal's session transcript
(Claude Code, Codex, Antigravity or OpenCode), and asks the recap agent headless through `cockpit agent argv run`.
The recap agent is --agent, else the config's agents.recap, else agents.default; the model is --model, else the
config's recap.model, else the agent's default. Agent `litellm` asks an OpenAI-compatible gateway straight (config: litellm.base_url, and the model,
e.g. a local one; the key from $LITELLM_API_KEY or the file litellm.key_file). With no session from the hub, the one the agent reported to Herdr. Writes ~/.cache/cockpit/recaps/<terminal>.json and prints it as JSON.
--title names the session instead, in a few words for its tab: titles.agent/titles.model in the config, else the recap
agent's; the name goes in ~/.cache/cockpit/titles.json under the terminal, with the session and its prompt count.
"""

import glob
import json
import os
import re
import sqlite3
import subprocess
import sys
import time
import urllib.error
import urllib.request

os.environ["PATH"] = os.pathsep.join([os.environ.get("PATH", ""), os.path.expanduser("~/.local/bin"), "/opt/homebrew/bin", "/usr/local/bin"])
COCKPIT = os.path.expanduser("~/.local/bin/cockpit")
CACHE = os.path.expanduser("~/.cache/cockpit/recaps")
CONFIG = os.path.expanduser("~/.config/cockpit/config.json")
# The hub's own recap prompt, so either writer reads the same.
PROMPT = (
    "Below are the messages of a coding agent's session with a person (tool calls and their output left out). Recap it in "
    "two or three sentences for the person coming back to it: what was done, where it stands, and what is left or what the "
    "agent waits for. Plain prose, no preamble, no heading."
)
CFG = {}
MAX_CHARS = 24000
TITLES = os.path.expanduser("~/.cache/cockpit/titles.json")
TITLE_PROMPT = (
    "Below is the start of a coding agent's session with a person, and where it is now. Name the task in at most six "
    "words, the way a person would title a browser tab: plain words, no quotes, no punctuation at the end, no preamble. "
    "Answer with the title only."
)


def text_of(content):
    if isinstance(content, str):
        return content
    if isinstance(content, list):
        return "\n".join(c.get("text", "") for c in content if isinstance(c, dict) and c.get("type") in ("text", "input_text", "output_text"))
    return ""


def claude(session):
    files = glob.glob(os.path.expanduser(f"~/.claude/projects/*/{session}.jsonl"))
    out = []
    for line in open(files[0], encoding="utf-8", errors="ignore") if files else []:
        try:
            e = json.loads(line)
        except Exception:
            continue
        if e.get("type") in ("user", "assistant") and not e.get("isMeta"):
            t = text_of((e.get("message") or {}).get("content"))
            if t.strip() and not t.startswith("<"):
                out.append((e["type"], t))
    return out


def codex(session):
    files = glob.glob(os.path.expanduser(f"~/.codex/sessions/**/*{session}*.jsonl"), recursive=True)
    out = []
    for line in open(files[0], encoding="utf-8", errors="ignore") if files else []:
        try:
            e = json.loads(line)
        except Exception:
            continue
        p = e.get("payload") or {}
        if e.get("type") == "response_item" and p.get("type") == "message" and p.get("role") in ("user", "assistant"):
            t = text_of(p.get("content"))
            if t.strip() and not t.lstrip().startswith("<"):
                out.append((p["role"], t))
    return out


def antigravity(session):
    path = os.path.expanduser(f"~/.gemini/antigravity-cli/brain/{session}/.system_generated/logs/transcript.jsonl")
    out = []
    for line in open(path, encoding="utf-8", errors="ignore") if os.path.exists(path) else []:
        try:
            e = json.loads(line)
        except Exception:
            continue
        if e.get("type") == "USER_INPUT" and e.get("content"):
            out.append(("user", re.sub(r"</?USER_REQUEST>", "", e["content"]).strip()))
        elif e.get("source") == "MODEL" and e.get("type") == "PLANNER_RESPONSE" and e.get("content"):
            out.append(("assistant", e["content"]))
    return out


def opencode(session):
    db = os.path.expanduser("~/.local/share/opencode/opencode.db")
    if not os.path.exists(db):
        return []
    con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    out = []
    for kind, data in con.execute("select type, data from session_message where session_id = ? order by seq", (session,)):
        try:
            d = json.loads(data)
        except Exception:
            continue
        if kind == "user" and d.get("text"):
            out.append(("user", d["text"]))
        elif kind == "assistant":
            t = text_of(d.get("content"))
            if t.strip():
                out.append(("assistant", t))
    return out


READERS = {"claude": claude, "codex": codex, "antigravity": antigravity, "agy": antigravity, "antigravity-cli": antigravity, "opencode": opencode}


def reader_for(program):
    p = (program or "").lower()
    for key, fn in READERS.items():
        if key in p:
            return fn
    return None


def ask_litellm(prompt, model):
    """A model behind an OpenAI-compatible gateway (litellm.base_url in the config), asked straight. The key is
    $LITELLM_API_KEY or the file litellm.key_file; never printed or cached."""
    ll = CFG.get("litellm") or {}
    base = ll.get("base_url")
    if not base or not model:
        raise RuntimeError("litellm needs litellm.base_url in the cockpit config and a model")
    key = os.environ.get("LITELLM_API_KEY", "")
    key_file = os.path.expanduser(ll.get("key_file") or "")
    if not key and key_file and os.path.exists(key_file):
        key = open(key_file, encoding="utf-8").read().strip()
    body = json.dumps({"model": model, "messages": [{"role": "user", "content": prompt}], "max_tokens": 4096}).encode()
    req = urllib.request.Request(base.rstrip("/") + "/v1/chat/completions", data=body, headers={"Content-Type": "application/json", **({"Authorization": f"Bearer {key}"} if key else {})})
    try:
        with urllib.request.urlopen(req, timeout=240) as r:
            d = json.load(r)
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"{model}: HTTP {e.code} {e.read().decode(errors='ignore')[:160]}")
    except Exception as e:
        raise RuntimeError(f"{model}: {e}")
    text = ((d.get("choices") or [{}])[0].get("message") or {}).get("content") or ""
    # A reasoning model may leave its thinking inline.
    text = re.sub(r"(?s)<think>.*?</think>", "", text).strip()
    if not text:
        raise RuntimeError(f"{model} gave an empty answer")
    return text


def ask(prompt, agent, model, use="recap"):
    """The agent's answer to one prompt, headless through `cockpit agent argv run` (`litellm`: the gateway, asked straight)."""
    if agent == "litellm":
        return ask_litellm(prompt, model)
    argv =[COCKPIT, "agent", "argv", "run", "--use", use, "--prompt", prompt]
    if agent:
        argv[4:6] = ["--agent", agent]
    if model:
        argv += ["--model", model]
    line = subprocess.run(argv, capture_output=True, text=True, timeout=30)
    if line.returncode != 0 or not line.stdout.strip():
        raise RuntimeError(f"{agent or 'the recap agent'} has no headless mode")
    run = subprocess.run(["sh", "-c", line.stdout.strip()], capture_output=True, text=True, timeout=240, cwd="/tmp", stdin=subprocess.DEVNULL)
    text = re.sub(r"\x1b\[[0-9;?]*[A-Za-z]", "", run.stdout).strip()
    if not text:
        raise RuntimeError((run.stderr.strip().splitlines() or ["the agent gave an empty answer"])[-1][:200])
    return text


def title(terminal, row, messages, agent, model):
    """A few words for the session's tab, from its first prompts and its latest answer; kept in titles.json."""
    users = [t for role, t in messages if role == "user"]
    last = next((t for role, t in reversed(messages) if role == "assistant"), "")
    body = "\n\n".join(f"Person: {t.strip()[:1500]}" for t in users[:3])
    if last:
        body += f"\n\nAgent, latest: {last.strip()[:1500]}"
    text = ask(TITLE_PROMPT + "\n\n" + body, agent, model)
    name = re.sub(r"\s+", " ", text.strip().splitlines()[-1]).strip(" \"'`*#.:")[:60]
    if not name:
        raise RuntimeError("the agent gave no title")
    try:
        titles = json.load(open(TITLES))
    except Exception:
        titles = {}
    entry = {"name": name, "session": row["session"], "prompts": len(users), "at": int(time.time() * 1000)}
    titles[terminal] = entry
    os.makedirs(os.path.dirname(TITLES), exist_ok=True)
    with open(TITLES + ".tmp", "w", encoding="utf-8") as f:
        json.dump(titles, f)
    os.replace(TITLES + ".tmp", TITLES)
    return entry


def session_of(row):
    """The agent's session when the hub has none yet: what the agent reported to Herdr (`hc ls --json`, its report's
    agent session id), else for OpenCode the newest root session in the terminal's folder."""
    address = str(row.get("address") or row.get("terminal") or "").replace("app:", "")
    try:
        ls = json.loads(subprocess.run(["hc", "ls", "--json"], capture_output=True, text=True, timeout=15).stdout or "[]")
    except Exception:
        ls = []
    ls = ls.get("terminals", []) if isinstance(ls, dict) else ls
    t = next((t for t in ls if isinstance(t, dict) and address and address in (t.get("address"), t.get("id"))), {}) or {}
    report = t.get("report") or {}
    for key in ("agent_session_id", "session_id", "session"):
        if isinstance(report, dict) and report.get(key):
            return report[key]
    cwd = row.get("cwd") or t.get("cwd") or t.get("folder")
    db = os.path.expanduser("~/.local/share/opencode/opencode.db")
    # OpenCode sets its terminal's title to "<tag> | <session title>": the newest session in the folder with that title.
    try:
        title = re.sub(r"^\w{1,3} \| ", "", json.loads(subprocess.run(["hc", "get", address], capture_output=True, text=True, timeout=15).stdout or "{}").get("title") or "").strip()
        if title and cwd and os.path.exists(db):
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            hit = con.execute("select id from session_v2 where directory = ? and title = ? and time_archived is null order by time_updated desc limit 1", (cwd, title)).fetchone()
            if hit:
                sid = hit[0]
                for _ in range(8):  # a sub-agent's title (the TUI shows it while you watch): its root
                    up = con.execute("select parent_id from session_v2 where id = ?", (sid,)).fetchone()
                    if not up or not up[0]:
                        break
                    sid = up[0]
                return sid
    except Exception:
        pass
    nums = re.findall(r"\d{3,}", str((row.get("tab") or {}).get("name") or row.get("title") or ""))
    if cwd and nums and "opencode" in str(row.get("program") or row.get("agent") or "").lower() and os.path.exists(db):
        # The newest session in its folder carrying the tab's number (review mono#2007 -> "Pull request 2007 review").
        try:
            con = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
            hits = []
            for sid, title, first in con.execute("select s.id, s.title, (select data from session_message m where m.session_id = s.id and m.type = 'user' order by m.seq limit 1) from session_v2 s where s.parent_id is null and s.directory = ? and s.time_archived is null order by s.time_updated desc", (cwd,)):
                if all(n in re.findall(r"\d{3,}", f"{title or ''} {first or ''}") for n in nums):
                    hits.append(sid)
            return hits[0] if hits else None
        except Exception:
            return None
    return None


def remote_messages(machine, program, session):
    """The session's messages read on another machine (herdr-core runs this same script there: nothing to install)."""
    src = open(__file__, encoding="utf-8").read()
    out = subprocess.run(["hc", "exec", "--on", machine, "python3", "-c", src, "--dump", program, session], capture_output=True, text=True, timeout=60)
    try:
        return [tuple(m) for m in json.loads(out.stdout)]
    except Exception:
        raise RuntimeError((out.stderr.strip().splitlines() or [f"could not read the session on {machine}"])[-1][:200])


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__.strip(), file=sys.stderr)
        return 2
    if args[0] == "--dump" and len(args) >= 3:
        # Run on the machine the agent lives on: its messages as JSON.
        read = reader_for(args[1])
        print(json.dumps(read(args[2]) if read else []))
        return 0
    terminal = args[0]
    opt = lambda name: args[args.index(name) + 1] if name in args and args.index(name) + 1 < len(args) else None
    global CFG
    cfg = {}
    try:
        cfg = json.load(open(CONFIG))
    except Exception:
        pass
    CFG = cfg
    naming = "--title" in args
    titles = cfg.get("titles") or {}
    own = naming and titles.get("agent")
    agent = opt("--agent") or own or (cfg.get("agents") or {}).get("recap")
    # A model belongs to its agent: the titles' agent takes the titles' model, else its own default.
    model = opt("--model") or (titles.get("model") if own else (cfg.get("recap") or {}).get("model"))

    rows = json.loads(subprocess.run([COCKPIT, "agents", "--json"], capture_output=True, text=True, timeout=30).stdout or "{}").get("agents", [])
    row = next((r for r in rows if terminal in (r.get("terminal"), r.get("address"), f"app:{r.get('address')}")), None)
    result = {"terminal": terminal, "at": int(time.time() * 1000), "agent": agent, "model": model}
    try:
        # The mod knows a remote agent's session and machine (the hub has neither on this machine).
        if opt("--session"):
            row = {**(row or {}), "session": opt("--session"), "program": (row or {}).get("program") or opt("--program")}
        if not row:
            raise RuntimeError("no such agent")
        read = reader_for(row.get("program") or row.get("agent"))
        if not read:
            raise RuntimeError(f"no transcript reader for {row.get('program') or 'this agent'}")
        if not row.get("session"):
            row["session"] = session_of(row)
        if not row.get("session"):
            raise RuntimeError("no session known for this agent yet")
        machine = opt("--machine")
        messages = remote_messages(machine, row.get("program") or row.get("agent"), row["session"]) if machine else read(row["session"])
        if not messages:
            raise RuntimeError("nothing to recap yet")
        if naming:
            result.update(title(terminal, row, messages, agent, model))
            print(json.dumps(result))
            return 0
        lines, size = [], 0
        for role, t in reversed(messages):
            chunk = f"{'Person' if role == 'user' else 'Agent'}: {t.strip()[:4000]}"
            if size + len(chunk) > MAX_CHARS:
                break
            lines.append(chunk)
            size += len(chunk)
        prompt = PROMPT + "\n\n" + "\n\n".join(reversed(lines))

        text = ask(prompt, agent, model)
        result.update(text=text, agent=agent or subprocess.run([COCKPIT, "agent", "id", "--use", "recap"], capture_output=True, text=True).stdout.strip())
    except Exception as e:
        result["error"] = str(e)[:300]
        if naming:
            print(json.dumps(result))
            return 1

    os.makedirs(CACHE, exist_ok=True)
    path = os.path.join(CACHE, re.sub(r"[^A-Za-z0-9_.-]", "_", terminal) + ".json")
    with open(path + ".tmp", "w", encoding="utf-8") as f:
        json.dump(result, f)
    os.replace(path + ".tmp", path)
    print(json.dumps(result))
    return 0 if "text" in result else 1


if __name__ == "__main__":
    sys.exit(main())
