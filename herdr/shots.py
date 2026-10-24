"""Screenshots of a page at phone, tablet and desktop sizes, taken in the terminal-browser beside a pane.

    shots.py <url> <out dir> [--sizes mobile,tablet,desktop] [--full] [--session <Claude session id>]

Each size gets a tab of its own, closed afterwards, so the person's tabs keep their size. Prints the screenshots as JSON,
{size: path}. Without terminal-browser beside a pane in herdr, it exits 2 and says so.
"""

import json
import os
import sys
import time

import browse

SIZES = {
    "mobile": ["set", "device", "iPhone 16"],
    "tablet": ["set", "device", "iPad"],
    "desktop": ["set", "viewport", "1440", "900"],
}


def action(tb, key, tab, *command, timeout=30):
    done = browse.run([tb, "action", "--browser", key, "--tab", str(tab), "--", *command], timeout=timeout)
    if not done or done.returncode != 0:
        raise RuntimeError(f"{' '.join(command[:2])}: {(done.stderr or done.stdout).strip()[:200] if done else 'timed out'}")
    return done.stdout


def main(args):
    sizes, full, session = list(SIZES), False, None
    rest = []
    while args:
        arg = args.pop(0)
        if arg == "--sizes" and args:
            sizes = [s for s in args.pop(0).split(",") if s in SIZES]
        elif arg == "--full":
            full = True
        elif arg == "--session" and args:
            session = args.pop(0)
        else:
            rest.append(arg)
    if len(rest) != 2:
        sys.exit(__doc__)
    url, out = rest[0], os.path.abspath(rest[1])
    tb = browse.terminal_browser()
    pane = browse.pane_of(session) if tb else None
    if not pane:
        print("terminal-browser is not beside a pane in herdr: take these screenshots in Chrome instead", file=sys.stderr)
        sys.exit(2)
    if not browse.browsers_beside(tb, pane) and not browse.open_beside(tb, pane, url, browse.DEFAULT_SIZE, False):
        sys.exit("could not open terminal-browser beside the pane")
    # A browser just split in takes a moment to start.
    for _ in range(40):
        browsers = browse.browsers_beside(tb, pane)
        if browsers:
            break
        time.sleep(0.5)
    else:
        sys.exit("terminal-browser did not start beside the pane")
    key = browsers[0]["key"]
    os.makedirs(out, exist_ok=True)
    shots = {}
    for size in sizes:
        opened = browse.run([tb, "new-tab", "--browser", key, url])
        tab = json.loads(opened.stdout).get("openedTab") if opened and opened.returncode == 0 else None
        if not tab:
            sys.exit(f"could not open a tab for {size}")
        try:
            # A new tab only answers once its page has a target.
            for _ in range(40):
                done = browse.run([tb, "action", "--browser", key, "--tab", str(tab), "--", "get", "url"])
                if done and done.returncode == 0:
                    break
                time.sleep(0.5)
            action(tb, key, tab, *SIZES[size])
            action(tb, key, tab, "reload")
            action(tb, key, tab, "wait", "--load", "networkidle", timeout=60)
            path = os.path.join(out, f"{size}.png")
            action(tb, key, tab, "screenshot", path, *(["--full"] if full else []), timeout=60)
            shots[size] = path
        finally:
            browse.run([tb, "action", "--browser", key, "--tab", str(tab), "--", "tab", "close"])
    browse.run([tb, "action", "--browser", key, "done"])
    print(json.dumps(shots))


if __name__ == "__main__":
    main(sys.argv[1:])
