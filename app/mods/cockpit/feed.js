// One hub subscription per view and its arguments in a window, shared by every copy of the mod there (copies share
// only globalThis): the first copy that asks opens it, the others join and get each new value on their own async
// context, a copy that leaves hands it on, and the last one closes it. `sharing(hub.subscribe)` is hub.subscribe's
// drop-in; this module imports nothing, so tests load it as it is.

/**
 * How often each member beats on its own timer: a holder silent for two beats loses its feed, and a member silent for
 * three drops out of it until its next beat brings it back.
 */
export const BEAT_MS = 3000;

/**
 * How long a subscription short of live may go without a word (not even "connecting" again) before a member opens it
 * again: one opened in the middle of a layout switch can be left waiting for good, while one retrying says so often.
 */
export const STALL_MS = 12_000;

const feeds = () => (globalThis.cockpitFeeds ??= new Map());

/** The arguments a view is followed with, in one shape per meaning: null and {} both mean none. */
export function normalize(_name, args) {
  if (args === undefined || args === null) return null;
  if (typeof args === "object" && !Array.isArray(args) && Object.keys(args).length === 0) return null;
  return args;
}

/** A view as a subscription lists it, read as [name, normalized args]. */
function viewOf(v) {
  return typeof v === "string" ? [v, null] : [v.name, normalize(v.name, v.args)];
}

export function keyOf(name, args) {
  return `${name} ${JSON.stringify(normalize(name, args))}`;
}

/** The window's feeds, for tests and measures: key → {members, owner, sub, state, version}. */
export function feedsNow() {
  return feeds();
}

function wake(f) {
  const resolve = f.resolve;
  f.next = null;
  f.resolve = null;
  resolve?.();
}

function changed(f) {
  f.next ??= new Promise((resolve) => (f.resolve = resolve));
  return f.next;
}

function shut(sub) {
  try {
    sub?.handle?.stop();
  } catch {}
}

function setState(f, state) {
  if (f.state === state) return;
  f.state = state;
  f.stateVersion += 1;
}

/** The previous subscription, kept while the new one has sent nothing, ends: the new one's state is the feed's. */
function settle(f) {
  shut(f.old);
  f.old = null;
  setState(f, f.sub?.state ?? "connecting");
}

/**
 * This member opens the feed's subscription on its own context. A live one it replaces keeps serving until the new one
 * sends its first value; each carries a generation, so callbacks of one replaced since are ignored.
 */
function take(f, m, cx, open, now) {
  const gen = ++f.gen;
  const prev = f.sub;
  f.owner = m;
  f.handoff = null;
  f.takenAt = now();
  f.sub = { gen, handle: null, state: "connecting", heard: now() };
  if (prev && !f.old && !prev.handle?.stopped && f.state === "live") f.old = prev;
  else shut(prev);
  try {
    const handle = open(
      cx,
      { views: [f.args === null ? f.name : { name: f.name, args: f.args }] },
      (message) => {
        if (message?.view !== f.name) return;
        if (f.sub?.gen === gen) {
          f.sub.state = "live";
          f.sub.heard = now();
          if (f.old) settle(f);
          else setState(f, "live");
        } else if (f.old?.gen !== gen) return;
        f.message = message;
        f.version += 1;
        wake(f);
      },
      (state) => {
        if (f.sub?.gen === gen) {
          f.sub.state = state;
          f.sub.heard = now();
          if (f.old) return;
          setState(f, state);
          wake(f);
        } else if (f.old?.gen === gen && state !== "live") {
          settle(f);
          wake(f);
        }
      },
    );
    if (f.sub?.gen === gen) f.sub.handle = handle;
    else handle?.stop();
    f.opened += 1;
  } catch (e) {
    if (f.sub?.gen !== gen) return;
    // Nobody owns it now: the next member to beat opens it again.
    f.sub = null;
    f.owner = null;
    if (!f.old) setState(f, `error: ${e?.message ?? e}`);
    wake(f);
  }
}

/** Whether this member has beaten lately: only such a member takes the feed over or is handed values. */
const fresh = (m, t) => !m.left && t - m.beat <= 2 * BEAT_MS;

/**
 * The feed wants a new owner: its subscription was stopped outside or stalled short of live, its owner went silent for
 * two beats, or it has none and no member a leaving owner named is still within its beat to take it.
 */
function orphaned(f, m, t) {
  if (f.sub?.handle?.stopped || (f.sub && f.sub.state !== "live" && t - f.sub.heard > STALL_MS)) return true;
  if (f.owner) return f.owner !== m && t - f.owner.beat > 2 * BEAT_MS;
  return !(f.handoff && f.handoff.to !== m && t - f.handoff.at <= BEAT_MS);
}

/** Out of the feed until its own next beat brings it back: dropping out is never final for a copy that still beats. */
function forget(f, m) {
  f.members.delete(m);
  if (f.owner === m) f.owner = null;
}

function newFeed(key, name, args) {
  return { key, name, args: normalize(name, args), members: new Set(), owner: null, handoff: null, sub: null, old: null, gen: 0, takenAt: 0, message: null, version: 0, state: "connecting", stateVersion: 0, next: null, resolve: null, opened: 0 };
}

/**
 * Joins the window's feed of one view: `onMessage(message, cx)` with each new {view, args, value} and `onState(state,
 * cx)` with "connecting", "live" or "error: …", both on this copy's `cx`. Returns {stop()}.
 */
export function join(cx, name, args, onMessage, onState, open, now = () => Date.now()) {
  const key = keyOf(name, args);
  let f = feeds().get(key);
  if (!f) {
    f = newFeed(key, name, args);
    feeds().set(key, f);
  }
  const m = { beat: now(), left: false, timer: null, seen: 0, seenState: -1, looping: false };
  f.members.add(m);
  const deliver = (cx) => {
    if (!fresh(m, now())) return;
    if (f.stateVersion !== m.seenState) {
      m.seenState = f.stateVersion;
      try {
        onState(f.state, cx);
      } catch {}
    }
    if (f.version !== m.seen && f.message) {
      m.seen = f.version;
      try {
        onMessage(f.message, cx);
      } catch {}
    }
  };
  // Values come on this copy's own task, which ends while the member is out of the feed and starts again with it.
  const loop = (cx) => {
    m.looping = true;
    cx.spawn(async (cx) => {
      try {
        while (!m.left && f.members.has(m)) {
          // The owner that left named this member: it takes the feed at once, the old subscription serving meanwhile.
          if (!f.owner && f.handoff?.to === m) take(f, m, cx, open, now);
          deliver(cx);
          await changed(f);
        }
      } finally {
        m.looping = false;
      }
    });
  };
  if (orphaned(f, m, now())) take(f, m, cx, open, now);
  // The beat comes from this copy's own timer, which stops with the copy.
  m.timer = cx.timer.every(BEAT_MS, (tcx = cx) => {
    if (m.left) return;
    const t = now();
    const held = t - m.beat > 2 * BEAT_MS;
    m.beat = t;
    // Back in the window's feed, the feed itself made again if the last member left it meanwhile.
    const current = feeds().get(key);
    if (!current) feeds().set(key, f);
    else if (current !== f) {
      const was = f;
      f = current;
      m.seen = m.seenState = -1;
      wake(was);
    }
    f.members.add(m);
    let moved = false;
    // A beat that comes late says this copy was held up (the machine slept, the window was throttled), not the others.
    if (!held) {
      for (const other of f.members) {
        if (other !== m && t - other.beat > 3 * BEAT_MS) {
          forget(f, other);
          moved = true;
        }
      }
      if (f.old && t - f.takenAt > 2 * BEAT_MS) {
        settle(f);
        moved = true;
      }
      if (orphaned(f, m, t)) {
        take(f, m, tcx, open, now);
        moved = true;
      }
    }
    if (moved) wake(f);
    if (!m.looping) loop(tcx);
    deliver(tcx);
  });
  loop(cx);
  return {
    stop() {
      if (m.left) return;
      m.left = true;
      m.timer?.cancel?.();
      const owned = f.owner === m;
      forget(f, m);
      const t = now();
      if (f.members.size === 0) {
        shut(f.sub);
        shut(f.old);
        f.sub = f.old = f.handoff = null;
        if (feeds().get(key) === f) feeds().delete(key);
      } else if (owned) {
        // The member that beat last, within a beat, takes it over; with none that beat lately it closes now.
        const next = [...f.members].filter((x) => t - x.beat <= BEAT_MS).sort((a, b) => b.beat - a.beat)[0];
        if (next) f.handoff = { to: next, at: t };
        else if (![...f.members].some((x) => fresh(x, t))) {
          shut(f.sub);
          shut(f.old);
          f.sub = f.old = null;
        }
      }
      wake(f);
    },
  };
}

/** Where several feeds stand together: live when all are, the first error, else connecting. */
export function together(states) {
  if (states.length && states.every((s) => s === "live")) return "live";
  return states.find((s) => String(s).startsWith("error")) ?? "connecting";
}

/** The views many copies of a window follow at once, which a window follows once. */
const SHARED = new Set(["agents", "home", "queue"]);

/**
 * hub.subscribe's drop-in: each view of `names` is a shared feed of the window; the other views, with the documents,
 * notifications and focus asked, go to `open` together as one subscription of this copy.
 */
export function sharing(open, names = SHARED) {
  return (cx, opts, onMessage, onState = () => {}) => {
    const views = opts.views ?? [];
    const common = views.map(viewOf).filter(([name]) => names.has(name));
    const own = views.filter((v) => !names.has(viewOf(v)[0]));
    const rest = own.length > 0 || opts.docs?.length > 0 || Boolean(opts.notify) || Boolean(opts.focus);
    const states = [...common.map(() => "connecting"), ...(rest ? ["connecting"] : [])];
    const tell = (i) => (state, cx) => {
      states[i] = state;
      onState(together(states), cx);
    };
    const joined = common.map(([name, args], i) => join(cx, name, args, onMessage, tell(i), open));
    if (rest) joined.push(open(cx, { ...opts, views: own }, onMessage, tell(common.length)));
    return { stop: () => joined.forEach((j) => j.stop()) };
  };
}
