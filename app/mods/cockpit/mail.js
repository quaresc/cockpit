// The mail: the `mail` view the hub pushes, asked again each minute so the ages stay true. Messages grouped "Needs you"
// above "Updates", newest first; a double click or See opens the whole thread on the side (the `mail-thread` view, its
// HTML sanitized by the hub and drawn natively), with Reply (an agent drafts it, nothing is sent), Archive, Mark read
// and Open in the mail client, and every key of the TUI app.

import { div } from "gpui-kit";
import { h_flex, v_flex, TextView } from "gpui-base";
import { settings } from "herdr";
import * as hub from "./hub.js";
import {
  tokens, TEXT, RADIUS, Icon, Button, ListRow, PageHeader, KeyHints, Empty, ErrorState, Skeleton, Toaster, Flashes, Scroller, Hover,
  SidePanel, Avatar, Spinner, StageHeading, openMenu, alpha, personColor,
} from "./ui.js";

const ICON = { see: "eye", reply: "reply", archive: "archive", read: "mail-check", open: "external" };
const HUB_ACTION = { reply: "mail_reply", archive: "mail_archive", read: "mail_read" };
/** After this long without the thread, the panel says it is still loading. */
const SLOW_MS = 20_000;

const escape = (s) => String(s ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

/** A plain text mail as HTML: a paragraph per line (TextView skips `<br>`), its links clickable. */
function textHtml(text) {
  const linked = escape(text).replace(/https?:\/\/[^\s<>"]+/g, (url) => `<a href="${url}">${url}</a>`);
  return linked.split(/\n+/).map((line) => `<p>${line}</p>`).join("");
}

export default class Mail {
  init(_p, cx) {
    this.model = null;
    this.status = "connecting";
    this.sel = null;
    this.reading = null;
    this.thread = null;
    this.threadSub = null;
    this.askedAt = 0;
    this.width = 1000;
    this.closed = false;
    this.hover = new Hover();
    this.toasts = new Toaster();
    this.flashes = new Flashes();
    this.scroller = new Scroller();
    this.handle = cx.focus_handle();
    this.sub = hub.subscribe(
      cx,
      { views: ["mail"] },
      (m, cx) => {
        if (m.view === "mail") this.take(m.value, cx);
        if (m.focus === "mail") this.focusOn(m.target?.key, cx);
        cx.notify();
      },
      (state, cx) => {
        this.status = state;
        cx.notify();
      },
    );
    cx.spawn(async (cx) => {
      while (!this.closed) {
        await hub.sleep(cx, 60000 - (Date.now() % 60000) + 300);
        if (this.closed) break;
        try {
          this.take(await hub.view("mail"), cx);
          cx.notify();
        } catch {}
      }
    });
  }

  close() {
    this.closed = true;
    this.sub?.stop();
    this.threadSub?.stop();
  }

  command(c, cx) {
    if (c === "focus" || c === "show:mail") this.handle.focus();
    const [scope, action, ...rest] = c.split(":");
    if (scope === "mail") {
      const m = this.message(rest.join(":"));
      if (m) {
        this.sel = m.id;
        this.run(cx, action, m);
      }
    }
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // The model

  messages() {
    return this.model?.messages ?? [];
  }

  message(id) {
    return this.messages().find((m) => m.id === id);
  }

  selected() {
    return this.message(this.sel);
  }

  ids() {
    return (this.model?.groups ?? []).flatMap((g) => g.ids);
  }

  take(model, cx) {
    const before = this.model;
    this.model = model;
    if (before?.messages?.length) {
      const was = new Set(before.messages.map((m) => m.id));
      for (const m of this.messages()) if (!was.has(m.id)) this.flashes.start(cx, m.id, tokens().ring);
    }
    if (!this.message(this.sel)) this.sel = this.ids()[0] ?? null;
    if (this.reading && !this.message(this.reading)) this.closeReading();
  }

  focusOn(id, cx) {
    const m = this.message(id);
    if (!m) {
      if (id) this.toasts.push(cx, "That message is not in the inbox any more");
      return;
    }
    this.sel = m.id;
    this.flashes.start(cx, m.id, tokens().ring);
    this.openReading(cx, m);
    this.handle.focus();
  }

  move(cx, by) {
    const ids = this.ids();
    if (!ids.length) return;
    const at = Math.max(0, ids.indexOf(this.sel));
    this.sel = ids[Math.max(0, Math.min(ids.length - 1, at + by))];
    cx.notify();
  }

  /** The first message of the other group: `]` to the updates, `[` back to what needs you. */
  group(cx, by) {
    const groups = (this.model?.groups ?? []).filter((g) => g.ids.length);
    const at = groups.findIndex((g) => g.ids.includes(this.sel));
    const next = groups[Math.max(0, Math.min(groups.length - 1, at + by))];
    if (next) this.sel = next.ids[0];
    cx.notify();
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Actions

  async hubAct(cx, action, args) {
    try {
      const r = await hub.act("mail", action, args);
      if (r.error) this.toasts.push(cx, r.error, { kind: "error" });
      else if (r.note && !r.note.startsWith("opened ")) this.toasts.push(cx, r.note, { kind: "success" });
      return r;
    } catch (e) {
      this.toasts.push(cx, "The hub did not answer", { kind: "error", description: String(e?.message ?? e) });
    } finally {
      cx.notify();
    }
  }

  /** The thread on the side: the `mail-thread` view followed, and the hub asked to read it (at once, or by a session). */
  openReading(cx, m) {
    if (this.reading !== m.id) {
      this.threadSub?.stop();
      this.thread = null;
      this.reading = m.id;
      this.threadSub = hub.subscribe(cx, { views: [{ name: "mail-thread", args: { id: m.id } }] }, (msg, cx) => {
        if (msg.view === "mail-thread" && msg.args?.id === this.reading) this.thread = msg.value;
        cx.notify();
      });
    }
    this.askedAt = Date.now();
    cx.spawn(async (cx) => this.hubAct(cx, "mail_thread", [m.raw]));
    // Reading it marks it read, quietly: the dot goes at once, the mail server follows in the background.
    if (m.unread) cx.spawn(async () => hub.act("mail", "mail_read", [m.raw]).catch(() => undefined));
    cx.notify();
  }

  closeReading() {
    this.threadSub?.stop();
    this.threadSub = null;
    this.reading = null;
    this.thread = null;
  }

  run(cx, id, m = this.selected()) {
    if (!m) return;
    if (id === "see") this.openReading(cx, m);
    else if (id === "open") {
      if (m.url) cx.spawn(async (cx) => this.hubAct(cx, "open_url", [m.url]));
      else this.toasts.push(cx, "The mail tool gives no link for this one");
    } else if (HUB_ACTION[id]) {
      if (id === "reply") this.toasts.push(cx, `An agent drafts a reply to ${m.fromName}`, { description: "Nothing is sent: the draft waits for you in the actions space." });
      if (id === "archive" && this.reading === m.id) this.closeReading();
      cx.spawn(async (cx) => this.hubAct(cx, HUB_ACTION[id], [m.raw]));
    }
    cx.notify();
  }

  menu(m) {
    openMenu(
      (m.acts ?? []).map((a) => ({ label: a.label, icon: ICON[a.id], kbd: a.key === "enter" ? "⏎" : a.key, command: `mail:${a.id}:${m.id}` })),
      { width: 220 },
    );
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Keys

  key(e, cx) {
    const k = e.keystroke;
    const m = this.selected();
    switch (k) {
      case "j":
      case "down":
        this.move(cx, 1);
        break;
      case "k":
      case "up":
        this.move(cx, -1);
        break;
      case "g":
      case "home":
        this.move(cx, -1e6);
        break;
      case "shift-g":
      case "end":
        this.move(cx, 1e6);
        break;
      case "]":
        this.group(cx, 1);
        break;
      case "[":
        this.group(cx, -1);
        break;
      case "enter":
        this.run(cx, "see", m);
        break;
      case "escape":
        if (!this.reading) return false;
        this.closeReading();
        break;
      case "shift-r":
        cx.spawn(async (cx) => this.hubAct(cx, "request_refresh", []));
        break;
      case "shift-f10":
        if (m) this.menu(m);
        break;
      default: {
        const act = (m?.acts ?? []).find((a) => a.key === k);
        if (!act) return false;
        this.run(cx, act.id, m);
      }
    }
    cx.notify();
    return true;
  }

  // -------------------------------------------------------------------------------------------------------------------
  // Drawing

  row(m, k, narrow) {
    const id = `mail-${m.id}`;
    const name = narrow ? 110 : 160;
    return ListRow(id, {
      height: 52,
      selected: this.sel === m.id,
      flash: this.flashes.of(m.id),
      muted: !m.unread,
      leading: div().w(8).flex_none().child(m.unread ? div().size(8).rounded(4).bg(k.warning) : ""),
      title: v_flex()
        .flex_1()
        .min_w(0)
        .gap(2)
        .child(
          h_flex()
            .gap(8)
            .min_w(0)
            .items_center()
            .child(div().w(name).flex_none().truncate().text_size(TEXT.sm).when(m.unread, (e) => e.font_semibold()).text_color(m.unread ? personColor(m.fromName) : k.mutedFg).child(m.fromName))
            .child(div().flex_1().min_w(0).truncate().when(m.unread, (e) => e.font_medium()).text_color(m.unread ? k.foreground : k.mutedFg).child(m.subject)),
        )
        .child(div().pl(name + 8).truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(m.snippet ?? "")),
      meta: div().w(36).flex_none().text_right().text_size(TEXT.xs).text_color(k.mutedFg).child(m.age ?? ""),
      hover: this.hover,
      actions: [
        Button(`${id}-see`, { label: narrow ? undefined : "See", icon: "eye", variant: "secondary", size: narrow ? "icon-sm" : "sm", tooltip: "See: read it on the side  ⏎", onClick: (_e, cx) => (cx.stop_propagation(), (this.sel = m.id), this.run(cx, "see", m)) }),
        ...(m.acts ?? [])
          .filter((a) => a.id === "archive" || a.id === "read")
          .map((a) => Button(`${id}-${a.id}`, { icon: ICON[a.id], variant: "ghost", size: "icon-sm", tooltip: `${a.label}  ${a.key}`, onClick: (_e, cx) => (cx.stop_propagation(), (this.sel = m.id), this.run(cx, a.id, m)) })),
      ],
      onClick: (_e, cx) => {
        this.sel = m.id;
        this.handle.focus();
        cx.notify();
      },
      onDoubleClick: (_e, cx) => ((this.sel = m.id), this.run(cx, "see", m)),
      onRightClick: () => this.menu(m),
    });
  }

  list(k, narrow) {
    const groups = (this.model?.groups ?? []).filter((g) => g.ids.length);
    if (!groups.length) return Empty({ icon: "mail-check", title: "Inbox zero", description: "Nothing unread or important in the inbox." });
    const rows = [];
    for (const g of groups) {
      rows.push({ height: 32, el: div().h(32).px(10).pt(6).child(StageHeading(`mail-group-${g.key}`, g.label, { color: g.key === "needs" ? k.warning : k.dim, count: g.ids.length })) });
      for (const id of g.ids) {
        const m = this.message(id);
        if (m) rows.push({ height: 52, el: this.row(m, k, narrow), id });
      }
    }
    return this.scroller.view("mail-list", rows, rows.findIndex((r) => r.id === this.sel));
  }

  /** The thread's messages, each under its sender and date, its HTML drawn as it is. */
  threadBody(m, k) {
    const t = this.thread;
    if (!t || t.state === "unread" || t.state === "loading") {
      const slow = Date.now() - this.askedAt > SLOW_MS && t?.state === "loading";
      return [
        h_flex().gap(8).items_center().text_size(TEXT.sm).text_color(k.mutedFg).child(Spinner({ size: 12 })).child(slow ? "Still loading…" : "Loading…"),
        ...[0, 1, 2, 3].map((i) => Skeleton({ w: 300 - i * 50, h: 12 })),
        div().pt(8).text_size(TEXT.sm).text_color(k.mutedFg).child(m.snippet ?? ""),
      ];
    }
    if (t.state === "failed") {
      return [
        h_flex().gap(8).items_center().px(12).py(8).rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).text_size(TEXT.sm).text_color(k.destructive).child(Icon("alert", { size: 14 })).child(div().flex_1().child(t.error ?? "")),
        Button("mail-retry", { label: "Try again", icon: "refresh", variant: "outline", size: "sm", onClick: (_e, cx) => this.openReading(cx, m) }),
      ];
    }
    return (t.messages ?? []).map((msg, i) =>
      v_flex()
        .gap(10)
        .pb(16)
        .when(i > 0, (e) => e.pt(14).border_t_1().border_color(k.border))
        .child(
          h_flex()
            .gap(10)
            .items_center()
            .child(Avatar(msg.fromName, { size: 28 }))
            .child(v_flex().flex_1().min_w(0).child(div().truncate().font_semibold().child(msg.fromName)).child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(msg.from)))
            .child(div().flex_none().text_size(TEXT.xs).text_color(k.mutedFg).child(msg.when ?? "")),
        )
        .child(
          TextView.html(`mail-body-${m.id}-${i}`, msg.html || textHtml(msg.text))
            .selectable(true)
            .text_size(TEXT.base)
            .text_color(k.foreground)
            .on_link_click((url, cx) => cx.spawn(async (cx) => this.hubAct(cx, "open_url", [url]))),
        ),
    );
  }

  panelWidth() {
    return Math.min(720, Math.max(360, this.width * 0.55));
  }

  reader(m, k) {
    return SidePanel("mail-read", {
      title: m.subject,
      description: `${m.fromName} · ${m.age}`,
      width: this.panelWidth(),
      onClose: (_e, cx) => (this.closeReading(), cx.notify()),
      children: this.threadBody(m, k),
      footer: h_flex()
        .flex_1()
        .gap(6)
        .flex_wrap()
        .children(
          (m.acts ?? [])
            .filter((a) => a.id !== "see")
            .map((a) => Button(`mail-read-${a.id}`, { label: a.label, icon: ICON[a.id], kbd: a.key, size: "sm", variant: a.id === "reply" ? "default" : "outline", onClick: (_e, cx) => this.run(cx, a.id, m) })),
        ),
    });
  }

  render(cx) {
    const k = tokens();
    const v = this.model;
    const counts = v?.counts ?? {};
    const description = !v ? "Loading…" : v.waiting ? "Waiting for the first fetch" : `${counts.needs ? `${counts.needs} need you` : "Nothing needs you"}${v.updated ? ` · updated ${v.updated}` : ""}`;
    let root = v_flex()
      .id("mail")
      .track_focus(this.handle)
      .size_full()
      .relative()
      .bg(k.background)
      .text_color(k.foreground)
      .text_size(TEXT.base)
      .on_size_change((size, cx) => {
        const w = size.width / (settings().zoom ?? 1);
        if (Math.abs(w - this.width) > 1) {
          this.width = w;
          cx.notify();
        }
      })
      .on_mouse_down("left", () => this.handle.focus())
      .on_key_down((e, cx) => {
        if (this.key(e, cx)) cx.stop_propagation();
      })
      .child(
        PageHeader({
          icon: "mail",
          title: "Mail",
          description,
          actions: [Button("mail-refresh", { icon: "refresh", variant: "ghost", size: "icon", tooltip: "Fetch now  shift+r", onClick: (_e, cx) => cx.spawn(async (cx) => this.hubAct(cx, "request_refresh", [])) })],
        }),
      );
    const toasts = this.toasts.render();
    if (!v) {
      const inner = this.status.startsWith("error")
        ? ErrorState({ title: "Cannot reach the cockpit hub", message: this.status.slice(7) })
        : v_flex().p(24).gap(12).children([0, 1, 2, 3].map((i) => h_flex().gap(10).child(Skeleton({ w: 140, h: 32 })).child(Skeleton({ w: 180 + ((i * 97) % 220), h: 32 }))));
      root = root.child(inner);
      return toasts ? root.child(toasts) : root;
    }
    if (v.waiting) {
      root = root.child(
        Empty({
          icon: "mail",
          title: "No mail yet",
          description: v.enabled
            ? "The hub fetches it from your mail server, or a Claude Code session with the mail connector does, within two minutes."
            : "Pick mail in cockpit setup to see the messages that need you here.",
        }),
      );
      return toasts ? root.child(toasts) : root;
    }
    const reading = this.message(this.reading);
    const wide = this.width >= 860;
    const main = v_flex()
      .flex_1()
      .min_w(0)
      .h_full()
      .when(Boolean(v.error), (e) =>
        e.child(h_flex().mx(24).mt(12).px(12).py(8).gap(8).items_center().rounded(RADIUS.md).bg(alpha(k.destructive, 0.1)).text_size(TEXT.sm).text_color(k.destructive).child(Icon("alert", { size: 14 })).child(div().flex_1().truncate().child(v.error ?? ""))),
      )
      .child(v_flex().flex_1().min_h(0).px(14).pt(8).child(this.list(k, this.width - (wide && reading ? this.panelWidth() : 0) < 900)));
    root = root
      .child(h_flex().flex_1().min_h(0).child(main).when(wide && Boolean(reading), (e) => e.child(this.reader(reading, k))))
      .child(KeyHints([["j k", "move"], ["] [", "group"], ["⏎", "see"], ["c", "reply"], ["b", "archive"], ["m", "mark read"], ["o", "open in mail"], ["esc", "close"]]));
    if (!wide && reading) {
      root = root.child(
        div()
          .id("mail-sheet")
          .absolute()
          .inset_0()
          .bg("#00000044")
          .on_mouse_down("left", (_e, cx) => (this.closeReading(), cx.notify()))
          .child(h_flex().absolute().top(0).bottom(0).right(0).on_mouse_down("left", (_e, cx) => cx.stop_propagation()).child(this.reader(reading, k).shadow_lg())),
      );
    }
    return toasts ? root.child(toasts) : root;
  }
}
