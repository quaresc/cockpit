"""What a notification's button does, with the inbox app's own actions and the inbox cache.

    act.py review <PR url>       a Claude review of the PR in the actions space
    act.py ci <PR url>           a Claude investigation of the PR's failing CI in a new space
    act.py rerun <PR url>        the PR's failed CI jobs rerun, by the next Claude session
    act.py plan <ticket id>      a Claude plan for the ticket in the actions space
"""

import subprocess
import sys

import cockpit_config
import inbox
import view


def pr_of(url):
    data = view.load_json(inbox.INBOX, {})
    for pr in [*(data.get("mine") or []), *(data.get("direct") or []), *(data.get("team") or [])]:
        if pr.get("url") == url:
            return pr
    sys.exit(f"{url} is not in the inbox cache")


def ci_run_of(pr):
    runs = view.load_json(inbox.CI, {}).get("runs") or []
    return next((r for r in runs if r.get("repo") == pr["repo"] and r.get("branch") == pr.get("branch")), None)


def main(args):
    if len(args) != 2:
        sys.exit(__doc__)
    kind, ref = args
    state = {"pending": {}, "note": "", "note_at": 0, "choice": None}
    if kind == "review":
        inbox.review(state, pr_of(ref))
    elif kind == "ci":
        pr = pr_of(ref)
        inbox.fix_ci(state, pr, ci_run_of(pr), "here")
    elif kind == "rerun":
        pr = pr_of(ref)
        run = ci_run_of(pr)
        if not run or not run.get("workflowId"):
            sys.exit(f"no CI run to rerun for {pr['repo']}#{pr['number']}")
        inbox.request(state, {"type": "rerun", "workflowId": run["workflowId"], "label": f"rerun {pr['repo']}#{pr['number']}"},
                      "notification", "rerunning…")
    elif kind == "plan":
        url = cockpit_config.ticket_url(ref, cockpit_config.load()) or ""
        inbox.plan(state, {"id": ref.upper(), "url": url})
    else:
        sys.exit(__doc__)
    if state["note"]:
        print(state["note"])


if __name__ == "__main__":
    try:
        main(sys.argv[1:])
    except SystemExit as stop:
        # Pressed from a notification, nobody reads stderr: a failure comes back as one.
        if stop.code not in (None, 0):
            subprocess.run(["sh", cockpit_config.script("notify.sh"), "⚠️ Cockpit", str(stop.code)[:200], "space:inbox"], capture_output=True)
        raise
