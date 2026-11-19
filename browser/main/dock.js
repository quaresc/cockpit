// Docking: the browser's window is laid exactly over a slot of the Herdr window and kept one step above it. The slot
// comes from cockpit's mod (a rectangle inside the window), the window's place on screen from the tracker.

/** The smallest slot a browser is shown in. */
const MIN = { width: 160, height: 120 };

/** Whether two rectangles are the same on screen, to the pixel. */
export const sameRect = (a, b) => !!a && !!b && a.x === b.x && a.y === b.y && a.width === b.width && a.height === b.height;

/** The slot on screen: the host window's place, its title bar's height, and the slot inside its drawable area. */
export function slotOnScreen(host, slot) {
  if (!host || !slot?.rect) return null;
  const bar = slot.viewport?.height > 0 ? Math.max(0, host.h - slot.viewport.height) : 0;
  const x = Math.round(host.x + slot.rect.x);
  const y = Math.round(host.y + bar + slot.rect.y);
  const width = Math.round(Math.min(slot.rect.width, host.x + host.w - x));
  const height = Math.round(Math.min(slot.rect.height, host.y + host.h - y));
  return width >= MIN.width && height >= MIN.height ? { x, y, width, height } : null;
}

/** Whether the browser's window sits right above the host: among the windows in front of it, only the browser's own are closer. */
export function glued(above, self) {
  const mine = above.findIndex((w) => w.pid === self);
  return mine >= 0 && above.slice(mine + 1).every((w) => w.pid === self);
}

/** What the window should do for one reading of the tracker and the slot: hide, wait, place itself, come back above, or stay. */
export function next(state, { host, above }, slot, self, now) {
  const want = slot && slot.visible && !slot.cover ? slotOnScreen(host, slot) : null;
  if (!want) return { ...state, step: state.shown ? "hide" : "none", shown: false, host: host || null, target: null, settleAt: 0 };
  const hostMoved = !state.host || state.host.x !== host.x || state.host.y !== host.y || state.host.w !== host.w || state.host.h !== host.h || state.host.id !== host.id;
  const slotMoved = !sameRect(state.target, want);
  if (hostMoved || slotMoved) {
    // A window that moves is followed once it rests: the browser hides for the trip rather than trail behind.
    const hide = state.shown && hostMoved && !!state.host;
    return { ...state, step: hide ? "hide" : "wait", shown: hide ? false : state.shown, host, target: want, settleAt: now + (hostMoved && state.host ? 120 : 40) };
  }
  if (state.settleAt && now < state.settleAt) return { ...state, step: "wait" };
  if (!state.shown || state.settleAt) return { ...state, step: "place", shown: true, settleAt: 0 };
  if (!glued(above, self)) return { ...state, step: "raise" };
  return { ...state, step: "none" };
}

export const initial = () => ({ step: "none", shown: false, host: null, target: null, settleAt: 0 });

/** Follows the tracker and the slot, and tells the window what to do through `ops` ({place, raise, hide, snapshot}). */
export class Dock {
  constructor(ops, self = process.pid) {
    this.ops = ops;
    this.self = self;
    this.state = initial();
    this.reading = { host: null, above: [] };
    this.slot = null;
    this.timer = null;
  }

  /** One line of the tracker: the host window and the windows in front of it. */
  track(reading) {
    this.reading = { host: reading?.host || null, above: Array.isArray(reading?.above) ? reading.above : [] };
    this.step();
  }

  /** The slot the mod reports, or null when the browser has none to sit in. */
  setSlot(slot) {
    this.slot = slot;
    this.step();
  }

  get hostPid() {
    return this.slot?.hostPid || 0;
  }

  step(now = Date.now()) {
    const before = this.state;
    this.state = next(before, this.reading, this.slot, this.self, now);
    clearTimeout(this.timer);
    const { step, target, host } = this.state;
    if (step === "hide") this.ops.hide();
    else if (step === "place") this.ops.place(target, host);
    else if (step === "raise") this.ops.raise(host);
    if (this.state.settleAt) this.timer = setTimeout(() => this.step(), Math.max(1, this.state.settleAt - now) + 1);
  }

  stop() {
    clearTimeout(this.timer);
  }
}
