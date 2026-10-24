"""Opens a link in terminal-browser, split right of a pane, else in the default browser.

    browse.py [--size <fraction>] [--reload] <url> [<Claude session id>]   beside the session's pane, else the calling one
    browse.py --beside [<Claude session id>]                               the browser beside it, as JSON ({} when none)
    browse.py --external [<Claude session id>]                             its page, opened in the default browser

terminal-browser is used when it is installed, the config's terminalBrowser is not false and the pane is in herdr. A page
already open beside the pane comes to the front instead of opening twice, and the keyboard stays where it was.
"""

import fcntl
import json
import os
import shlex
import shutil
import struct
import subprocess
import sys
import termios

import cockpit_config

PATH = os.pathsep.join(["/opt/homebrew/bin", "/usr/local/bin", os.path.expanduser("~/.local/bin"), os.environ.get("PATH", "")])
HERDR = os.environ.get("HERDR_BIN_PATH") or shutil.which("herdr", path=PATH) or "herdr"
# herdr sends a picture of at most 16 MiB through a file, a larger one inline through the terminal, which is far slower.
MAX_FRAME = 16 * 1024 * 1024
DEFAULT_SIZE = 0.5


def run(argv, env=None, timeout=10):
    try:
        return subprocess.run(argv, capture_output=True, text=True, timeout=timeout, stdin=subprocess.DEVNULL,
                              env={**os.environ, "PATH": PATH, **(env or {})})
    except (OSError, subprocess.TimeoutExpired):
        return None


def herdr_json(*args):
    done = run([HERDR, *args])
    try:
        return json.loads(done.stdout)["result"] if done and done.returncode == 0 else None
    except (ValueError, KeyError):
        return None


def terminal_browser():
    """The terminal-browser command, or None when it is missing or turned off."""
    if cockpit_config.load().get("terminalBrowser") is False:
        return None
    return shutil.which("terminal-browser", path=PATH)


def pane_of(session):
    """The pane of a Claude session, else the calling one, with its tab and space; None outside herdr."""
    panes = (herdr_json("pane", "list") or {}).get("panes") or []
    by_session = [p for p in panes if session and (p.get("agent_session") or {}).get("value") == session]
    calling = [p for p in panes if p.get("pane_id") == os.environ.get("HERDR_PANE_ID")]
    return (by_session or calling or [None])[0]


def browsers_beside(tb, pane):
    done = run([tb, "ls", "--json"], {"HERDR_PANE_ID": pane["pane_id"], "HERDR_TAB_ID": pane["tab_id"]})
    try:
        return json.loads(done.stdout).get("browsers") or [] if done and done.returncode == 0 else []
    except ValueError:
        return []


def same_page(a, b):
    def norm(url):
        if url.startswith("/"):
            url = "file://" + os.path.realpath(url)
        return url.rstrip("/")
    return norm(a) == norm(b)


def split_size(pane, wanted):
    """The fraction of the pane a new split takes: as wanted, but small enough for herdr to send its frames as files."""
    info = herdr_json("pane", "process-info", "--pane", pane["pane_id"]) or {}
    pid = (info.get("process_info") or {}).get("shell_pid")
    tty = run(["ps", "-o", "tty=", "-p", str(pid)]) if pid else None
    name = tty.stdout.strip() if tty else ""
    if not name or name == "??":
        return wanted
    try:
        with open(f"/dev/{name}", "rb", buffering=0) as f:
            _, _, width, height = struct.unpack("HHHH", fcntl.ioctl(f, termios.TIOCGWINSZ, b"\0" * 8))
    except OSError:
        return wanted
    if not width or not height:
        return wanted
    fits = MAX_FRAME / (4 * height) / width
    return round(max(0.2, min(wanted, fits, 0.95)), 2)


def open_beside(tb, pane, url, size, reload):
    browsers = browsers_beside(tb, pane)
    for browser in browsers:
        for tab in browser.get("tabs") or []:
            if same_page(tab.get("url", ""), url):
                done = run([tb, "action", "--browser", browser["key"], "--tab", str(tab["id"]), "--follow", "--",
                            *(["reload"] if reload else ["get", "url"])])
                return bool(done and done.returncode == 0)
    if browsers:
        done = run([tb, "new-tab", "--browser", browsers[0]["key"], url])
        return bool(done and done.returncode == 0)
    # The split is herdr's own, so the keyboard stays where it is; its ratio is what the first pane keeps.
    split = herdr_json("pane", "split", "--pane", pane["pane_id"], "--direction", "right",
                       "--ratio", str(round(1 - split_size(pane, size), 2)), "--no-focus")
    browser = ((split or {}).get("pane") or {}).get("pane_id")
    if not browser:
        return False
    # exec, so the pane closes with the browser.
    done = run([HERDR, "pane", "run", browser, f"exec {shlex.quote(tb)} open {shlex.quote(url)}"])
    return bool(done and done.returncode == 0)


def beside(session):
    tb = terminal_browser()
    pane = pane_of(session) if tb else None
    for browser in browsers_beside(tb, pane) if pane else []:
        tab = next((t for t in browser.get("tabs") or [] if t.get("active")), None)
        if tab:
            return {"key": browser["key"], "tab": tab["id"], "url": tab.get("url", ""), "title": tab.get("title", "")}
    return {}


def main(args):
    if args[:1] == ["--beside"]:
        print(json.dumps(beside(args[1] if len(args) > 1 else None)))
        return
    if args[:1] == ["--external"]:
        page = beside(args[1] if len(args) > 1 else None)
        if not page:
            sys.exit("no terminal-browser beside this pane")
        cockpit_config.open_url(page["url"])
        return
    size, reload = DEFAULT_SIZE, False
    while args and args[0].startswith("--"):
        flag = args.pop(0)
        if flag == "--size" and args:
            size = float(args.pop(0))
        elif flag == "--reload":
            reload = True
    if not args:
        sys.exit(__doc__)
    url, session = args[0], (args[1] if len(args) > 1 else None)
    tb = terminal_browser()
    pane = pane_of(session) if tb else None
    if not (pane and open_beside(tb, pane, url, size, reload)):
        cockpit_config.open_url(url if not url.startswith("/") else "file://" + url)


if __name__ == "__main__":
    main(sys.argv[1:])
