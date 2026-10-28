"""A week of history for the demo HOME: the hub takes the inbox and the monitor as they were a while ago, then as
demo_data.py wrote them, so it records what changed between; and the apps are told you last looked before that.
Run it after demo_data.py, before apps_anim.py and palette_anim.py."""
import copy, json, os, subprocess, time

HERE = os.path.dirname(os.path.abspath(__file__))
HOME = os.path.join(HERE, "home")
APPS = os.path.join(HERE, "..", "..", "herdr")
COCKPIT = os.path.join(APPS, "tui", "target", "release", "cockpit")
CACHE = os.path.join(HOME, ".cache", "cockpit")
env = {**os.environ, "HOME": HOME, "COCKPIT_HERDR_DIR": APPS, "HERDR_BIN_PATH": "/usr/bin/true", "COCKPIT_HUB_FEED": "off",
       "COCKPIT_HUB_IDLE_SECS": "120"}

# The search model, shared with this machine's when it has one, else downloaded by the hub's session index.
models = os.path.join(CACHE, "models")
mine = os.path.expanduser("~/.cache/cockpit/models/potion-base-8M")
if os.path.isdir(mine) and not os.path.exists(os.path.join(models, "potion-base-8M")):
    os.makedirs(models, exist_ok=True)
    os.symlink(mine, os.path.join(models, "potion-base-8M"))

subprocess.run(["pkill", "-f", f"{COCKPIT} hub"])
for name in ["store.redb", "hub.sock"]:
    if os.path.exists(os.path.join(CACHE, name)):
        os.remove(os.path.join(CACHE, name))
now = json.load(open(os.path.join(CACHE, "inbox.json")))
monitor_now = json.load(open(os.path.join(CACHE, "monitor.json")))
os.remove(os.path.join(CACHE, "inbox.json")); os.remove(os.path.join(CACHE, "monitor.json"))

# Two hours ago: no review asked on api#2210, web#1838 still green, ENG-413 not in review yet, the api release watched.
before = copy.deepcopy(now)
before["direct"] = [p for p in before["direct"] if p["number"] != 2210]
for p in before["mine"]:
    if p["number"] == 1838: p["ci"] = "SUCCESS"
    if p["number"] == 2198: p["review"] = "REVIEW_REQUIRED"
for t in before["tickets"]:
    if t["id"] == "ENG-413": t["status"] = "In Progress"
monitor_before = copy.deepcopy(monitor_now)
api = monitor_before["apps"][0]
api["releases"][0]["verdict"]["state"] = "watching"
api["monitors"]["alerting"] = []
api["issues"] = []


def put(doc, value):
    path = os.path.join(CACHE, f"history-{doc}.json")
    json.dump(value, open(path, "w"))
    subprocess.run([COCKPIT, "store", "put", doc, path], env=env, stdout=subprocess.DEVNULL, check=True)


for doc, old, new in [("inbox", before, now), ("monitor", monitor_before, monitor_now)]:
    put(doc, old)
    time.sleep(0.05)
    put(doc, new)
events = json.loads(subprocess.run([COCKPIT, "events", "--since", "1d"], env=env, capture_output=True, text=True).stdout)["events"]
print(len(events), "events:", *[e["title"] for e in events], sep="\n  ")

# You last looked two hours ago, so the apps mark what changed since.
seen = os.path.join(CACHE, "seen"); os.makedirs(seen, exist_ok=True)
for app in ["inbox", "monitor"]:
    json.dump({"at": int((time.time() - 2 * 3600) * 1000)}, open(os.path.join(seen, f"{app}.json"), "w"))
