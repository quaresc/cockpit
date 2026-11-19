import assert from "node:assert/strict";
import { test, beforeEach } from "node:test";

import { normalize, keyOf, join, sharing, together, feedsNow, BEAT_MS, STALL_MS } from "../mods/cockpit/feed.js";

// A copy's async context: its tasks run and its timers fire every few ms. `drop()` stops its timers, as Herdr does for a
// copy a layout switch drops, while a task already waiting on a promise still wakes when another copy resolves it;
// `paused` holds them for a while, as a sleeping machine or a throttled window does.
function context() {
  const timers = new Set();
  const cx = {
    paused: false,
    spawn: (f) => f(cx),
    timer: {
      every: (_ms, fn) => {
        const id = setInterval(() => !cx.paused && fn(cx), 2);
        timers.add(id);
        return { cancel: () => (clearInterval(id), timers.delete(id)) };
      },
    },
    notify() {},
    drop() {
      for (const id of timers) clearInterval(id);
      timers.clear();
    },
  };
  return cx;
}

// hub.subscribe as a test sees it: each open is counted, kept with its views, and can be pushed to or stopped outside.
function hub({ fail = 0 } = {}) {
  const opened = [];
  let failing = fail;
  const open = (cx, opts, onMessage, onState) => {
    if (failing-- > 0) throw new Error("no hub");
    const sub = { cx, opts, onMessage, onState, stopped: false, stop() { this.stopped = true; } };
    opened.push(sub);
    queueMicrotask(() => onState("live", cx));
    return sub;
  };
  return { opened, open, live: () => opened.filter((s) => !s.stopped) };
}

const tick = (ms = 15) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => feedsNow().clear());

test("null and {} ask for every frontend's agents and {frontend: \"app\"} for the app's: two feeds", () => {
  assert.deepEqual([null, {}].map((a) => keyOf("agents", a)), ["agents null", "agents null"]);
  assert.equal(keyOf("agents", { frontend: "app" }), 'agents {"frontend":"app"}');
  assert.equal(normalize("home", {}), null);
  assert.deepEqual(normalize("chat", { terminal: "app:t3" }), { terminal: "app:t3" });
});

test("copies share one subscription, a leaving owner's serves until the next one's first value, the last closes it", async () => {
  const h = hub();
  const seen = [[], [], []];
  const states = [[], [], []];
  const members = [null, {}, null].map((args, i) => join(context(), "agents", args, (m) => seen[i].push(m.value.n), (s) => states[i].push(s), h.open));
  assert.equal(h.opened.length, 1, "one subscription for three copies");
  await tick();
  h.opened[0].onMessage({ view: "agents", value: { n: 1 } });
  await tick();
  assert.deepEqual(seen, [[1], [1], [1]], "each copy gets the value on its own context");
  members[0].stop();
  await tick();
  assert.equal(h.opened.length, 2, "the owner left: another copy opened it at once");
  assert.equal(h.opened[0].stopped, false, "the old one serves until the new one sends");
  h.opened[0].onMessage({ view: "agents", value: { n: 2 } });
  await tick();
  h.opened[1].onMessage({ view: "agents", value: { n: 3 } });
  await tick();
  assert.equal(h.opened[0].stopped, true, "the new one sent: the old one is closed");
  h.opened[0].onMessage({ view: "agents", value: { n: 99 } });
  await tick();
  assert.deepEqual(seen.map((s) => s.at(-1)), [1, 3, 3], "a closed subscription's late value is ignored");
  assert.deepEqual(states.slice(1).map((s) => s.filter((x) => x !== "live")), [["connecting"], ["connecting"]], "no connecting after the first live");
  members[1].stop();
  members[2].stop();
  await tick();
  assert.equal(h.live().length, 0, "nobody follows it: closed");
  assert.equal(feedsNow().size, 0);
});

test("a copy that joined then was dropped gets nothing more, loses its feed, and is forgotten", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const gone = context();
  const values = [[], []];
  join(gone, "agents", null, (m) => values[0].push(m.value), () => {}, h.open, clock);
  const living = join(context(), "agents", null, (m) => values[1].push(m.value), () => {}, h.open, clock);
  await tick();
  gone.drop();
  now += 2 * BEAT_MS + 1;
  await tick();
  assert.equal(h.opened.length, 2, "the living copy took the feed over from the silent owner");
  h.opened[1].onMessage({ view: "agents", value: "fresh" });
  await tick();
  assert.deepEqual(values, [[], ["fresh"]], "the dropped copy, woken by the new value, is not handed it");
  assert.equal(h.opened[0].stopped, true, "and the dropped copy's subscription is closed");
  now += BEAT_MS;
  await tick();
  const f = feedsNow().get(keyOf("agents", null));
  assert.equal(f.members.size, 1, "silent for three beats: forgotten");
  living.stop();
  await tick();
  assert.equal(feedsNow().size, 0);
});

test("a leaving owner hands its feed to a living copy, never to a dropped one", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const owner = join(context(), "agents", null, () => {}, () => {}, h.open, clock);
  const gone = context();
  join(gone, "agents", null, () => {}, () => {}, h.open, clock);
  await tick();
  gone.drop();
  now += 2 * BEAT_MS + 1;
  owner.stop();
  await tick();
  assert.equal(h.opened.length, 1, "the dropped copy does not take it");
  assert.equal(h.live().length, 0, "and nothing stays open for it");
  const values = [];
  const late = join(context(), "agents", null, (m) => values.push(m.value), () => {}, h.open, clock);
  assert.equal(h.opened.length, 2, "a living copy that joins opens it again");
  h.opened[1].onMessage({ view: "agents", value: "back" });
  await tick();
  assert.deepEqual(values, ["back"]);
  late.stop();
  await tick();
});

test("a throwing callback neither stops the others nor its own next values", async () => {
  const h = hub();
  const seen = [];
  let throws = true;
  const a = join(context(), "agents", null, (m) => {
    if (throws) throw new Error("render failed");
    seen.push(["a", m.value]);
  }, () => {}, h.open);
  const b = join(context(), "agents", null, (m) => seen.push(["b", m.value]), () => {}, h.open);
  await tick();
  h.opened[0].onMessage({ view: "agents", value: 1 });
  await tick();
  throws = false;
  h.opened[0].onMessage({ view: "agents", value: 2 });
  await tick();
  assert.deepEqual(seen, [["b", 1], ["a", 2], ["b", 2]]);
  a.stop();
  b.stop();
  await tick();
});

test("an open that throws leaves no owner, says so, and the next beat opens it again", async () => {
  const h = hub({ fail: 1 });
  const states = [];
  const a = join(context(), "agents", null, () => {}, (s) => states.push(s), h.open);
  await tick();
  assert.equal(states[0], "error: no hub");
  await tick();
  assert.equal(h.opened.length, 1, "opened on the next beat");
  assert.equal(states.at(-1), "live");
  a.stop();
  await tick();
});

test("a subscription the window's watchdog stopped is noticed and opened again", async () => {
  const h = hub();
  const a = join(context(), "agents", null, () => {}, () => {}, h.open);
  await tick();
  h.opened[0].stop();
  await tick();
  assert.equal(h.opened.length, 2, "the owner opened it again on its next beat");
  assert.equal(h.live().length, 1);
  a.stop();
  await tick();
});

// hub.subscribe with the hub stopped: "connecting", then the error, again at each retry until stopped.
const STOPPED = "error: the cockpit hub was stopped on purpose";
function stoppedHub() {
  const opened = [];
  const open = (cx, opts, onMessage, onState) => {
    onState("connecting", cx);
    const retry = setInterval(() => (onState("connecting", cx), onState(STOPPED, cx)), 3);
    const sub = { stopped: false, stop() { this.stopped = true; clearInterval(retry); } };
    opened.push(sub);
    queueMicrotask(() => !sub.stopped && onState(STOPPED, cx));
    return sub;
  };
  return { opened, open };
}

test("with the hub down the owner keeps its feed: no copy takes it over and resets the retries", async () => {
  const { opened, open } = stoppedHub();
  let now = 1_000;
  const clock = () => now;
  const members = [context(), context(), context()].map((cx) => join(cx, "agents", null, () => {}, () => {}, open, clock));
  await tick();
  for (let s = 0; s < 60; s += 1) {
    now += 1000;
    await tick(5);
  }
  assert.equal(opened.length, 1, "one subscription for a minute of the hub down");
  members.forEach((m) => m.stop());
  await tick();
});

test("after a sleep every copy still gets the values: a late beat forgets nobody", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const seen = [[], [], []];
  const cxs = [context(), context(), context()];
  const members = cxs.map((cx, i) => join(cx, "agents", null, (m) => seen[i].push(m.value), () => {}, h.open, clock));
  await tick();
  // No timer runs while the machine sleeps, and the wall clock has moved on when it wakes.
  cxs.forEach((cx) => (cx.paused = true));
  now += 60_000;
  await tick(5);
  cxs.forEach((cx) => (cx.paused = false));
  await tick(30);
  const f = feedsNow().get(keyOf("agents", null));
  assert.equal(f.members.size, 3, "every copy is still a member");
  assert.equal(h.live().length, 1);
  h.live()[0].onMessage({ view: "agents", value: "after-wake" });
  await tick(30);
  assert.deepEqual(seen.map((s) => s.at(-1)), ["after-wake", "after-wake", "after-wake"]);
  members.forEach((m) => m.stop());
  await tick();
});

test("a copy held up long enough to drop out comes back with its next beat", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const seen = [[], []];
  const a = context();
  const b = context();
  const ma = join(a, "agents", null, (m) => seen[0].push(m.value), () => {}, h.open, clock);
  const mb = join(b, "agents", null, (m) => seen[1].push(m.value), () => {}, h.open, clock);
  await tick();
  b.paused = true;
  now += 3 * BEAT_MS + 1;
  await tick();
  const f = feedsNow().get(keyOf("agents", null));
  assert.equal(f.members.size, 1, "silent for three beats: out");
  b.paused = false;
  await tick();
  assert.equal(f.members.size, 2, "its next beat brings it back");
  h.live().at(-1).onMessage({ view: "agents", value: "later" });
  await tick();
  assert.deepEqual(seen.map((s) => s.at(-1)), ["later", "later"]);
  ma.stop();
  mb.stop();
  await tick();
});

test("a feed closed while a dropped-out copy still lived is made again by its next beat", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const values = [];
  const a = context();
  const b = context();
  const ma = join(a, "agents", null, () => {}, () => {}, h.open, clock);
  const mb = join(b, "agents", null, (m) => values.push(m.value), () => {}, h.open, clock);
  await tick();
  b.paused = true;
  now += 3 * BEAT_MS + 1;
  await tick();
  ma.stop();
  await tick();
  assert.equal(feedsNow().size, 0, "its last member left: closed");
  assert.equal(h.live().length, 0);
  b.paused = false;
  await tick();
  assert.equal(h.live().length, 1, "the copy that came back opened it again");
  h.live()[0].onMessage({ view: "agents", value: "again" });
  await tick();
  assert.deepEqual(values.at(-1), "again");
  mb.stop();
  await tick();
});

test("a leaving owner hands its feed to the copy that beat last, not to one silent for most of a beat", async () => {
  const h = hub();
  let now = 1_000;
  const clock = () => now;
  const owner = join(context(), "agents", null, () => {}, () => {}, h.open, clock);
  const slow = context();
  const quick = context();
  const ms = join(slow, "agents", null, () => {}, () => {}, h.open, clock);
  const mq = join(quick, "agents", null, () => {}, () => {}, h.open, clock);
  await tick();
  slow.paused = true;
  for (let s = 0; s < 4; s += 1) {
    now += 1000;
    await tick(5);
  }
  owner.stop();
  await tick();
  assert.equal(h.opened.length, 2);
  assert.equal(h.opened[1].cx, quick, "the copy that beat last opened it");
  ms.stop();
  mq.stop();
  await tick();
});

test("the drop-in shares the common views and keeps the others, focus included, in one subscription of the copy", async () => {
  const h = hub();
  const subscribe = sharing(h.open);
  const states = [];
  const home = subscribe(context(), { views: ["agents", "home", "queue", "inbox", { name: "mail", args: {} }] }, () => {}, (s) => states.push(s));
  const bell = subscribe(context(), { views: ["agents", "home"] }, () => {}, () => {});
  const peek = subscribe(context(), { views: [{ name: "agents", args: { frontend: "app" } }] }, () => {}, () => {});
  assert.equal(h.opened.length, 5, "agents for all and for the app, home and queue once each, plus Home's inbox and mail");
  assert.deepEqual(h.opened.at(-2).opts.views, ["inbox", { name: "mail", args: {} }]);
  await tick();
  assert.equal(states.at(-1), "live", "live once every part is");
  const sidebar = subscribe(context(), { views: ["todo"], notify: true }, () => {}, () => {});
  assert.equal(h.opened.length, 6, "notifications are a copy's own");
  const inbox = subscribe(context(), { views: [{ name: "inbox", args: { group: "feature" } }, { name: "agents", args: {} }], focus: "inbox" }, () => {}, () => {});
  assert.equal(h.opened.length, 7, "the inbox keeps its own view and focus, and joins the agents feed");
  assert.deepEqual([h.opened.at(-1).opts.views, h.opened.at(-1).opts.focus], [[{ name: "inbox", args: { group: "feature" } }], "inbox"]);
  const focusOnly = subscribe(context(), { views: ["agents"], focus: "agents" }, () => {}, () => {});
  assert.equal(h.opened.at(-1).opts.focus, "agents", "a focus alone still opens the copy's own subscription");
  [home, bell, peek, sidebar, inbox, focusOnly].forEach((s) => s.stop());
  await tick();
  assert.equal(h.live().length, 0);
});

test("feeds together are live when all are", () => {
  assert.equal(together(["live", "live"]), "live");
  assert.equal(together(["live", "connecting"]), "connecting");
  assert.equal(together(["live", "error: gone"]), "error: gone");
});

test("the hub stopped across a hand-off: every member, a late one too, ends on the stopped state", async () => {
  const { opened, open } = stoppedHub();
  let now = 1_000;
  const clock = () => now;
  const states = [[], [], []];
  const owner = join(context(), "agents", null, () => {}, (s) => states[0].push(s), open, clock);
  const a = join(context(), "agents", null, () => {}, (s) => states[1].push(s), open, clock);
  await tick();
  owner.stop();
  await tick();
  assert.equal(opened.length, 2, "handed over");
  const late = join(context(), "agents", null, () => {}, (s) => states[2].push(s), open, clock);
  await tick();
  assert.deepEqual([states[1].at(-1), states[2].at(-1)], [STOPPED, STOPPED]);
  a.stop();
  late.stop();
  await tick();
});

test("a copy that joins while the first owner is still opening gets connecting, then live and the value", async () => {
  const subs = [];
  const open = (cx, opts, onMessage, onState) => {
    onState("connecting", cx);
    const sub = { stopped: false, stop() { this.stopped = true; }, live: () => (onState("live", cx), onMessage({ view: "agents", value: "first" }, cx)) };
    subs.push(sub);
    return sub;
  };
  const seen = [[], []];
  const first = join(context(), "agents", null, (m) => seen[0].push(m.value), (s) => seen[0].push(s), open);
  const second = join(context(), "agents", null, (m) => seen[1].push(m.value), (s) => seen[1].push(s), open);
  await tick();
  subs[0].live();
  await tick();
  assert.equal(subs.length, 1);
  assert.deepEqual(seen, [["connecting", "live", "first"], ["connecting", "live", "first"]]);
  first.stop();
  second.stop();
  await tick();
});

test("a subscription left waiting since it was opened, short of live, is opened again by a beat", async () => {
  const subs = [];
  const open = (cx, opts, onMessage, onState) => {
    onState("connecting", cx);
    // The first one was opened in the middle of a layout switch: it never says anything again.
    const sub = { stopped: false, stop() { this.stopped = true; } };
    if (subs.length > 0) queueMicrotask(() => (onState("live", cx), onMessage({ view: "agents", value: "here" }, cx)));
    subs.push(sub);
    return sub;
  };
  let now = 1_000;
  const clock = () => now;
  const values = [];
  const a = join(context(), "agents", null, (m) => values.push(m.value), () => {}, open, clock);
  await tick();
  for (let s = 0; s * 1000 < STALL_MS; s += 1) {
    now += 1000;
    await tick(5);
  }
  assert.equal(subs.length, 1, "not before STALL_MS");
  now += 1001;
  await tick();
  assert.equal(subs.length, 2);
  assert.equal(subs[0].stopped, true);
  assert.deepEqual(values, ["here"]);
  a.stop();
  await tick();
});
