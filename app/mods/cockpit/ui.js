// cockpit's design kit: shadcn/ui's components and proportions drawn with GPUI elements, colored from Herdr's theme()
// so every screen follows the person's theme, light and dark. Build screens from these; add a missing piece here.
//
// Rhythm: a 4px grid, 13px body, 12px secondary, 11px captions; radius 8 (cards), 6 (controls), 4 (checkbox, kbd);
// 1px borders; muted-foreground for what is secondary; color only for state.

import { div, svg, PathBuilder, Background } from "gpui-kit";
import { h_flex, v_flex, Input, HoverCard as BaseHoverCard } from "gpui-base";
import { theme, settings, openOverlay } from "herdr";

export const RADIUS = { sm: 4, md: 6, lg: 8, xl: 12 };
export const TEXT = { xs: 11, sm: 12, base: 13, lg: 15, xl: 18 };

/** The kit's color tokens, shadcn's names on Herdr's theme. */
export function tokens() {
  const t = theme();
  return {
    t,
    background: t.background,
    foreground: t.foreground,
    card: t.background,
    canvas: t.canvas ?? t.background,
    chrome: t.chrome ?? t.surface,
    popover: t.popover ?? t.surface,
    muted: t.raised ?? t.hover,
    mutedFg: t.muted,
    dim: t.dim,
    accent: t.hover,
    selected: t.selected ?? t.hover,
    border: t.border,
    input: t.input ?? t.border,
    ring: t.accent,
    primary: t.solid ?? t.foreground,
    primaryFg: t.solid_foreground ?? t.background,
    destructive: t.ansi1,
    success: t.ansi2 ?? t.done,
    warning: t.ansi3 ?? t.working,
    info: t.accent,
    working: t.working,
    blocked: t.blocked,
    done: t.done,
    idle: t.idle,
  };
}

/** `color` at this opacity, for tints (a 15% destructive behind its text). */
export function alpha(color, a) {
  if (!color?.startsWith("#")) return color;
  const hex = color.length === 9 ? color.slice(0, 7) : color.slice(0, 7);
  return hex + Math.round(Math.max(0, Math.min(1, a)) * 255).toString(16).padStart(2, "0");
}

// ---------------------------------------------------------------------------------------------------------------------
// Primitives

/** A lucide icon from icons/, tinted with `color` (else the text color). */
export function Icon(name, { size = 16, color } = {}) {
  const el = svg(`mods/cockpit/icons/${name}.svg`).size(size).flex_none();
  return color ? el.text_color(color) : el;
}

/** A turning ring: loading. */
export function Spinner({ size = 14, color } = {}) {
  return Icon("loader", { size, color }).spin(900);
}

export function Separator({ vertical = false, my = 0, mx = 0 } = {}) {
  const k = tokens();
  return vertical ? div().w(1).h_full().flex_none().mx(mx).bg(k.border) : div().h(1).w_full().flex_none().my(my).bg(k.border);
}

const MODIFIER = { ctrl: "⌃", cmd: "⌘", alt: "⌥", opt: "⌥", shift: "⇧" };

/** A key as the Mac writes it: "ctrl p", "ctrl-p" and "ctrl+p" read "⌃P", "cmd-shift-p" "⌘⇧P"; the rest as given. */
export function keyLabel(key) {
  const s = String(key);
  if (!/^(ctrl|cmd|alt|opt|shift)[ +-]./i.test(s)) return s;
  const parts = s.split(/[ +-](?=.)/);
  const last = parts.pop();
  return parts.map((p) => MODIFIER[p.toLowerCase()] ?? p).join("") + (last.length === 1 ? last.toUpperCase() : last);
}

/** Keys as shadcn's <Kbd>: "⌘K", ["⌘", "K"], or "ctrl-p" (written ⌃P). */
export function Kbd(keys) {
  const k = tokens();
  const list = (Array.isArray(keys) ? keys : [keys]).map(keyLabel);
  return h_flex()
    .gap(2)
    .flex_none()
    .children(
      list.map((key) =>
        h_flex()
          .h(20)
          .min_w(20)
          .px(5)
          .items_center()
          .justify_center()
          .rounded(RADIUS.sm)
          .border_1()
          .border_color(k.border)
          .bg(k.muted)
          .text_size(TEXT.xs)
          .font_medium()
          .text_color(k.mutedFg)
          .child(String(key)),
      ),
    );
}

const BADGE = {
  default: (k) => [k.primary, k.primaryFg, null],
  secondary: (k) => [k.muted, k.foreground, null],
  outline: (k) => [null, k.foreground, k.border],
  destructive: (k) => [alpha(k.destructive, 0.15), k.destructive, null],
  success: (k) => [alpha(k.success, 0.15), k.success, null],
  warning: (k) => [alpha(k.warning, 0.15), k.warning, null],
  info: (k) => [alpha(k.info, 0.15), k.info, null],
};

/** A small label: default, secondary, outline, destructive, success, warning, info; `dot` puts a color dot first. */
export function Badge(text, variant = "secondary", { icon, dot, id, tooltip, onClick } = {}) {
  const k = tokens();
  const [bg, fg, border] = (BADGE[variant] ?? BADGE.secondary)(k);
  let el = h_flex().h(20).px(8).gap(4).flex_none().items_center().rounded(RADIUS.md).text_size(TEXT.xs).font_medium().text_color(fg);
  if (bg) el = el.bg(bg);
  if (border) el = el.border_1().border_color(border);
  if (dot) el = el.child(div().size(6).rounded(3).bg(dot));
  if (icon) el = el.child(Icon(icon, { size: 12 }));
  el = el.child(String(text));
  if (id) el = el.id(id);
  if (onClick && id) el = el.cursor_pointer().hover((s) => s.opacity(0.85)).on_click(onClick);
  if (tooltip && id) el = el.tooltip(tooltip);
  return el;
}

const BUTTON = {
  default: (k) => ({ bg: k.primary, fg: k.primaryFg, hover: (s) => s.opacity(0.9) }),
  secondary: (k) => ({ bg: k.muted, fg: k.foreground, hover: (s) => s.bg(k.selected) }),
  outline: (k) => ({ bg: k.background, fg: k.foreground, border: k.input, hover: (s) => s.bg(k.accent) }),
  ghost: (k) => ({ fg: k.foreground, hover: (s) => s.bg(k.accent) }),
  // The background's color on the red reads in both themes: dark themes' red is light, white on it washes out.
  destructive: (k) => ({ bg: k.destructive, fg: k.background, hover: (s) => s.opacity(0.9) }),
  link: (k) => ({ fg: k.foreground, hover: (s) => s.underline() }),
};
const SIZE = { default: [32, 12, TEXT.base, 16], sm: [28, 10, TEXT.sm, 14], lg: [36, 16, TEXT.base, 16], icon: [32, 0, TEXT.base, 16], "icon-sm": [28, 0, TEXT.sm, 14] };

/**
 * A button: variant default | secondary | outline | ghost | destructive | link, size default | sm | lg | icon |
 * icon-sm; `kbd` shows its key; `tooltip` on hover. `onClick(event, cx)`.
 */
export function Button(id, { label, icon, iconRight, variant = "default", size = "default", kbd, disabled = false, tooltip, onClick, grow = false } = {}) {
  const k = tokens();
  const v = (BUTTON[variant] ?? BUTTON.default)(k);
  const [h, px, text, iconSize] = SIZE[size] ?? SIZE.default;
  const square = size === "icon" || size === "icon-sm";
  let el = h_flex()
    .id(id)
    .h(h)
    .gap(6)
    .flex_none()
    .items_center()
    .justify_center()
    .rounded(RADIUS.md)
    .text_size(text)
    .font_medium()
    .text_color(v.fg)
    .whitespace_nowrap();
  el = square ? el.w(h) : el.px(px);
  if (grow) el = el.flex_1();
  if (v.bg) el = el.bg(v.bg);
  if (v.border) el = el.border_1().border_color(v.border);
  if (icon) el = el.child(Icon(icon, { size: iconSize }));
  if (label && !square) el = el.child(label);
  if (iconRight) el = el.child(Icon(iconRight, { size: iconSize }));
  if (kbd && !square) el = el.child(Kbd(kbd));
  if (disabled) return el.opacity(0.5);
  el = el.cursor_pointer().hover(v.hover);
  if (onClick) el = el.on_click((e, cx) => onClick(e, cx));
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

/** A shadcn checkbox: 16px, rounded 4, filled with a check when on. */
export function Checkbox(id, { checked = false, onClick, tooltip } = {}) {
  const k = tokens();
  let el = h_flex()
    .id(id)
    .size(16)
    .flex_none()
    .items_center()
    .justify_center()
    .rounded(RADIUS.sm)
    .border_1()
    .border_color(checked ? k.primary : k.input)
    .when(checked, (e) => e.bg(k.primary))
    .cursor_pointer()
    .hover((s) => s.border_color(k.ring))
    .when(checked, (e) => e.child(Icon("check", { size: 12, color: k.primaryFg })));
  if (onClick) el = el.on_click((e, cx) => {
    cx.stop_propagation();
    onClick(e, cx);
  });
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

/** A pulsing placeholder while data loads. */
export function Skeleton({ w, h = 16, radius = RADIUS.md } = {}) {
  const k = tokens();
  let el = div().h(h).rounded(radius).bg(k.muted).loop("opacity", { from: 0.45, to: 1, period: 1400 });
  return w ? el.w(w) : el.w_full();
}

// ---------------------------------------------------------------------------------------------------------------------
// Composites

/** A card: rounded 8, 1px border, title and description, an action on the right, content, a footer. */
export function Card({ id, title, description, action, children = [], footer, padding = 16, gap = 12 } = {}) {
  const k = tokens();
  let el = v_flex().rounded(RADIUS.lg).border_1().border_color(k.border).bg(k.card).p(padding).gap(gap);
  if (id) el = el.id(id);
  if (title || description || action) {
    el = el.child(
      h_flex()
        .items_start()
        .gap(12)
        .child(
          v_flex()
            .flex_1()
            .min_w(0)
            .gap(4)
            .when(Boolean(title), (e) => e.child(div().whitespace_normal().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(title)))
            .when(Boolean(description), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(description))),
        )
        .when(Boolean(action), (e) => e.child(action)),
    );
  }
  el = el.children(children);
  if (footer) el = el.child(footer);
  return el;
}

/** shadcn tabs: a muted track with the active trigger raised. items: [{ value, label, count, icon }]. */
export function Tabs(id, { items, value, onChange }) {
  const k = tokens();
  return h_flex()
    .id(id)
    .h(32)
    .p(3)
    .gap(2)
    .flex_none()
    .items_center()
    .rounded(RADIUS.lg)
    .bg(k.muted)
    .children(
      items.map((item) => {
        const on = item.value === value;
        return h_flex()
          .id(`${id}-${item.value}`)
          .h_full()
          .px(10)
          .gap(6)
          .items_center()
          .rounded(RADIUS.md)
          .text_size(TEXT.base)
          .font_medium()
          .cursor_pointer()
          .text_color(on ? k.foreground : k.mutedFg)
          .when(on, (e) => e.bg(k.background).shadow_sm())
          .when(!on, (e) => e.hover((s) => s.text_color(k.foreground)))
          .on_click((ev, cx) => onChange(item.value, cx))
          .when(Boolean(item.icon), (e) => e.child(Icon(item.icon, { size: 14 })))
          .child(item.label)
          .when(item.count !== undefined && item.count !== null, (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(String(item.count))));
      }),
    );
}

/**
 * A table: columns [{ key, label, width?, align?, sortable? }], rows [{ id, cells: {key: string | element}, selected?,
 * onClick?, onRightClick? }]. Widths in px; a column without one takes what is left. `sort` = { key, desc } marks the
 * sorted column; a click on a sortable header calls `onSort(key, cx)` (the caller sorts its rows).
 */
export function Table(id, { columns, rows, sort, onSort }) {
  const k = tokens();
  const cell = (col, content) => {
    let el = h_flex().px(8).items_center().min_w(0).text_size(TEXT.base);
    el = col.width ? el.w(col.width).flex_none() : el.flex_1();
    if (col.align === "right") el = el.justify_end();
    return el.child(typeof content === "string" || typeof content === "number" ? div().truncate().child(String(content)) : content ?? "");
  };
  const header = (c) => {
    const sorted = sort?.key === c.key;
    let label = h_flex().gap(4).items_center().text_size(TEXT.sm).font_medium().text_color(sorted ? k.foreground : k.mutedFg).child(c.label ?? "");
    if (c.sortable && onSort) {
      label = label
        .id(`${id}-sort-${c.key}`)
        .cursor_pointer()
        .hover((s) => s.text_color(k.foreground))
        .on_click((_e, cx) => onSort(c.key, cx))
        .child(Icon(sorted ? (sort.desc ? "chevron-down" : "chevron-up") : "chevrons-up-down", { size: 12 }));
    }
    return cell(c, label);
  };
  const head = h_flex().h(36).border_b_1().border_color(k.border).children(columns.map(header));
  const body = rows.map((r) =>
    h_flex()
      .id(`${id}-${r.id}`)
      .h(40)
      .border_b_1()
      .border_color(k.border)
      .text_color(k.foreground)
      .when(Boolean(r.selected), (e) => e.bg(k.selected))
      .hover((s) => s.bg(k.accent))
      .when(Boolean(r.onClick), (e) => e.cursor_pointer().on_click((ev, cx) => r.onClick(ev, cx)))
      .when(Boolean(r.onRightClick), (e) => e.on_mouse_down("right", (ev, cx) => r.onRightClick(ev, cx)))
      .children(columns.map((c) => cell(c, r.cells[c.key]))),
  );
  return v_flex().id(id).w_full().child(head).children(body);
}

/** Nothing to show: an icon in a muted tile, a title, a line, an action. */
export function Empty({ icon = "inbox", title, description, action } = {}) {
  const k = tokens();
  return v_flex()
    .flex_1()
    .items_center()
    .justify_center()
    .gap(12)
    .p(32)
    .child(h_flex().size(40).rounded(RADIUS.lg).items_center().justify_center().bg(k.muted).child(Icon(icon, { size: 20, color: k.mutedFg })))
    .child(
      v_flex()
        .items_center()
        .gap(4)
        .when(Boolean(title), (e) => e.child(div().text_size(TEXT.base).font_semibold().text_color(k.foreground).child(title)))
        .when(Boolean(description), (e) => e.child(div().max_w(360).text_size(TEXT.sm).text_color(k.mutedFg).text_center().child(description))),
    )
    .when(Boolean(action), (e) => e.child(action));
}

/** A failure the person can act on: the message and a retry. */
export function ErrorState({ title = "Something went wrong", message, onRetry } = {}) {
  const k = tokens();
  return Empty({
    icon: "alert",
    title,
    description: message,
    action: onRetry ? Button("error-retry", { label: "Try again", variant: "outline", size: "sm", onClick: onRetry }) : null,
  }).text_color(k.foreground);
}

/** A section label above a group: small, medium weight, muted; a count and an action at the right. */
export function SectionHeader(text, { count, action, id } = {}) {
  const k = tokens();
  let el = h_flex().h(28).px(8).gap(6).items_center().text_size(TEXT.sm).font_medium().text_color(k.mutedFg).child(text);
  if (count !== undefined) el = el.child(div().text_color(k.dim).child(String(count)));
  if (action) el = el.child(div().flex_1()).child(action);
  return id ? el.id(id) : el;
}

/**
 * The search field above a list: icon, an `Input` on the caller's InputState (created in init), a key hint. `active`
 * draws the focus ring.
 */
export function SearchField(state, { active = false, kbd, width } = {}) {
  const k = tokens();
  let el = h_flex()
    .h(32)
    .px(10)
    .gap(8)
    .items_center()
    .rounded(RADIUS.md)
    .border_1()
    .border_color(active ? k.ring : k.input)
    .bg(k.background)
    .child(Icon("search", { size: 14, color: k.mutedFg }))
    .child(Input.new(state).flex_1().h_full().text_size(TEXT.base).text_color(k.foreground));
  if (kbd) el = el.child(Kbd(kbd));
  return width ? el.w(width) : el.flex_1();
}

/** A text field on the caller's InputState, shadcn's <Input>. */
export function TextField(state, { active = true, leading } = {}) {
  const k = tokens();
  return h_flex()
    .h(32)
    .px(10)
    .gap(8)
    .flex_1()
    .items_center()
    .rounded(RADIUS.md)
    .border_1()
    .border_color(active ? k.ring : k.input)
    .bg(k.background)
    .when(Boolean(leading), (e) => e.child(leading))
    .child(Input.new(state).flex_1().h_full().text_size(TEXT.base).text_color(k.foreground));
}

/**
 * A list row: 36px, rounded 6, `leading` then title (string or element) then `meta` (muted) then `trailing`.
 * selected: the selected background; marked: a ring; flash: `flashes.of(key)` from a Flashes, a highlight that fades.
 * hover (a Hover) shows `actions` (small ghost buttons) and `grip` (a Reorder grip) while the pointer is on the row;
 * dropAbove draws the drop line of a drag, dimmed the dragged row; onRightClick opens a menu (openMenu).
 */
export function ListRow(id, { selected, marked, flash, onClick, onDoubleClick, onRightClick, leading, title, meta, trailing, actions, hover, grip, dropAbove, dimmed, muted, height = 36, tooltip } = {}) {
  const k = tokens();
  let el = h_flex()
    .id(id)
    .h(height)
    .px(10)
    .gap(10)
    .flex_none()
    .items_center()
    .rounded(RADIUS.md)
    .relative()
    .cursor_pointer()
    .text_size(TEXT.base)
    .text_color(muted ? k.mutedFg : k.foreground)
    .when(Boolean(selected), (e) => e.bg(k.selected))
    .when(!selected, (e) => e.hover((s) => s.bg(k.accent)))
    .when(Boolean(marked), (e) => e.border_1().border_color(k.ring));
  if (flash) {
    el = el.child(
      div()
        .id(`${id}-flash`)
        .absolute()
        .inset_0()
        .rounded(RADIUS.md)
        .bg(alpha(flash.color ?? k.ring, 0.28))
        .opacity(flash.on ? 1 : 0)
        .transition("opacity", { duration: 1100, easing: "ease-out" }),
    );
  }
  if (dimmed) el = el.opacity(0.4);
  if (dropAbove) el = el.child(div().absolute().left(0).right(0).top(-2).h(2).rounded(1).bg(k.ring));
  const showing = hover ? hover.is(id) : false;
  if (grip) el = el.child(div().w(12).flex_none().child(showing ? grip : ""));
  if (leading) el = el.child(leading);
  el = el.child(typeof title === "string" ? div().flex_1().min_w(0).truncate().child(title) : h_flex().flex_1().min_w(0).gap(6).items_center().child(title ?? ""));
  if (meta) el = el.child(typeof meta === "string" ? div().flex_none().text_size(TEXT.sm).text_color(k.mutedFg).child(meta) : meta);
  if (actions && (showing || selected)) el = el.child(h_flex().gap(2).flex_none().children(actions));
  if (trailing) el = el.child(trailing);
  if (hover) el = hover.track(el, id);
  if (onClick) el = el.on_click((e, cx) => (e.click_count > 1 && onDoubleClick ? onDoubleClick(e, cx) : onClick(e, cx)));
  if (onRightClick) el = el.on_mouse_down("right", (e, cx) => onRightClick(e, cx));
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

/** The selection's actions as buttons, each with its key. actions: [{ id, label, kbd, icon, variant, onClick }]. */
export function ActionBar(actions, { id = "actions" } = {}) {
  return h_flex()
    .id(id)
    .gap(6)
    .flex_wrap()
    .items_center()
    .children(actions.map((a, i) => Button(`${id}-${a.id ?? i}`, { label: a.label, icon: a.icon, kbd: a.kbd, variant: a.variant ?? (i === 0 ? "default" : "outline"), size: "sm", onClick: a.onClick, tooltip: a.tooltip })));
}

/** An app's header: icon tile, title and description, actions on the right. */
export function PageHeader({ icon, title, description, aside, actions = [] } = {}) {
  const k = tokens();
  return h_flex()
    .h(64)
    .px(24)
    .gap(12)
    .flex_none()
    .items_center()
    .border_b_1()
    .border_color(k.border)
    .when(Boolean(icon), (e) => e.child(h_flex().size(32).rounded(RADIUS.lg).items_center().justify_center().border_1().border_color(k.border).bg(k.muted).child(Icon(icon, { size: 16, color: k.foreground }))))
    .child(
      v_flex()
        .flex_1()
        .min_w(0)
        .gap(2)
        .child(div().text_size(TEXT.lg).font_semibold().text_color(k.foreground).child(title ?? ""))
        .when(Boolean(description), (e) => e.child(div().truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(description))),
    )
    .when(Boolean(aside), (e) => e.child(aside))
    .child(h_flex().gap(8).items_center().children(actions));
}

/** A footer line of key hints: [["⏎", "edit"], ["x", "done"], ...]. */
export function KeyHints(hints) {
  const k = tokens();
  return h_flex()
    .h(36)
    .px(24)
    .gap(14)
    .flex_none()
    .items_center()
    .border_t_1()
    .border_color(k.border)
    .text_size(TEXT.xs)
    .text_color(k.mutedFg)
    .children(hints.map(([key, what]) => h_flex().gap(6).items_center().child(Kbd(key)).child(what)));
}

/**
 * The palette's list (shadcn's <Command>): the search field, then groups [{ heading, items: [{ id, label, hint, icon,
 * shortcut, onSelect }] }]. `selected` is the selected item's id.
 */
export function Command(id, { state, groups, selected, empty = "No results.", maxHeight = 340 } = {}) {
  const k = tokens();
  const items = groups.flatMap((g) => g.items);
  return v_flex()
    .id(id)
    .w_full()
    .min_h(0)
    .child(
      h_flex()
        .h(44)
        .px(12)
        .gap(8)
        .flex_none()
        .items_center()
        .border_b_1()
        .border_color(k.border)
        .child(Icon("search", { size: 16, color: k.mutedFg }))
        .child(Input.new(state).flex_1().h_full().text_size(TEXT.base).text_color(k.foreground)),
    )
    .child(
      v_flex()
        .id(`${id}-list`)
        .max_h(maxHeight)
        .p(4)
        .overflow_y_scrollbar()
        .when(!items.length, (e) => e.child(div().py(24).text_center().text_size(TEXT.base).text_color(k.mutedFg).child(empty)))
        .children(
          groups
            .filter((g) => g.items.length)
            .map((g, gi) =>
              v_flex()
                .when(gi > 0, (e) => e.border_t_1().border_color(k.border).mt(4).pt(4))
                .child(div().px(8).py(6).text_size(TEXT.xs).font_medium().text_color(k.mutedFg).child(g.heading))
                .children(
                  g.items.map((item) =>
                    h_flex()
                      .id(`${id}-${item.id}`)
                      .h(32)
                      .px(8)
                      .gap(8)
                      .items_center()
                      .rounded(RADIUS.md)
                      .cursor_pointer()
                      .text_size(TEXT.base)
                      .text_color(k.foreground)
                      .when(item.id === selected, (e) => e.bg(k.accent))
                      .hover((s) => s.bg(k.accent))
                      .on_click((ev, cx) => item.onSelect?.(ev, cx))
                      .when(Boolean(item.icon), (e) => e.child(Icon(item.icon, { size: 16, color: k.mutedFg })))
                      .child(div().flex_1().min_w(0).truncate().child(item.label))
                      .when(Boolean(item.hint), (e) => e.child(div().max_w(220).truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(item.hint)))
                      .when(Boolean(item.shortcut), (e) => e.child(Kbd(item.shortcut))),
                  ),
                ),
            ),
        ),
    );
}

/**
 * A confirm over the view (shadcn's <AlertDialog>): a dimmed backdrop and a card with Cancel and the action. Esc and
 * Enter are the caller's to map onto onCancel and onConfirm.
 */
export function AlertDialog(id, { title, description, confirmLabel = "Continue", destructive = false, onConfirm, onCancel } = {}) {
  const k = tokens();
  return div()
    .id(id)
    .absolute()
    .inset_0()
    .bg("#00000066")
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .child(
      v_flex()
        .size_full()
        .items_center()
        .justify_center()
        .child(
          Card({
            id: `${id}-card`,
            title,
            description,
            padding: 24,
            gap: 16,
            footer: h_flex()
              .justify_end()
              .gap(8)
              .child(Button(`${id}-cancel`, { label: "Cancel", variant: "outline", kbd: "esc", onClick: onCancel }))
              .child(Button(`${id}-ok`, { label: confirmLabel, variant: destructive ? "destructive" : "default", kbd: "⏎", onClick: onConfirm })),
          })
            .w(420)
            .bg(k.popover)
            .shadow_lg(),
        ),
    );
}

/**
 * Short highlights on rows that changed (added, done, moved): `start(cx, key, color)`, then pass `of(key)` as a
 * ListRow's `flash`. The highlight shows at once and fades natively.
 */
export class Flashes {
  constructor() {
    this.on = new Map();
  }

  start(cx, key, color) {
    this.on.set(key, { on: true, color });
    cx.timer.after(40, (cx) => {
      const f = this.on.get(key);
      if (f) f.on = false;
      cx.notify();
    });
    cx.timer.after(1400, (cx) => {
      this.on.delete(key);
      cx.notify();
    });
  }

  of(key) {
    return this.on.get(key) ?? null;
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Toasts (Sonner-style): a stack at the bottom right of the view, each gone after a few seconds.

const TOAST = { success: "circle-check", error: "alert", info: "info", note: "info" };

export class Toaster {
  constructor() {
    this.items = [];
    this.next = 0;
  }

  /** Shows a toast: kind success | error | info; `description` under the title. */
  push(cx, title, { kind = "info", description, ms = 3500, action } = {}) {
    if (!title) return;
    const id = this.next++;
    this.items = [...this.items.slice(-2), { id, title, kind, description, action }];
    cx.timer.after(ms, (cx) => {
      this.items = this.items.filter((t) => t.id !== id);
      cx.notify();
    });
    cx.notify();
  }

  render() {
    if (!this.items.length) return null;
    const k = tokens();
    const color = { success: k.success, error: k.destructive, info: k.mutedFg, note: k.mutedFg };
    return v_flex()
      .absolute()
      .bottom(16)
      .right(16)
      .gap(8)
      .w(340)
      .children(
        this.items.map((t) =>
          h_flex()
            .id(`toast-${t.id}`)
            .p(14)
            .gap(10)
            .items_start()
            .rounded(RADIUS.lg)
            .border_1()
            .border_color(k.border)
            .bg(k.popover)
            .shadow_lg()
            .opacity(1)
            .transition("opacity", { duration: 200, easing: "ease-out" })
            .child(Icon(TOAST[t.kind] ?? "info", { size: 16, color: color[t.kind] ?? k.mutedFg }))
            .child(
              v_flex()
                .flex_1()
                .min_w(0)
                .gap(2)
                .child(div().text_size(TEXT.base).font_medium().text_color(k.foreground).child(t.title))
                .when(Boolean(t.description), (e) => e.child(div().text_size(TEXT.sm).text_color(k.mutedFg).child(t.description))),
            )
            .when(Boolean(t.action), (e) =>
              e.child(
                Button(`toast-${t.id}-action`, {
                  label: t.action?.label,
                  variant: "outline",
                  size: "sm",
                  onClick: (ev, cx) => {
                    this.items = this.items.filter((x) => x.id !== t.id);
                    t.action.onClick(ev, cx);
                    cx.notify();
                  },
                }),
              ),
            ),
        ),
      );
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Scrolling a list the keyboard drives

/**
 * A list of fixed-height rows that keeps the selected one in view: only the rows on screen are drawn, the wheel
 * scrolls, a thin bar shows where it is. Keep one per list on the view (created in init), and call `view()` in render:
 *
 *   this.scroller.view("todo-list", rows.map((r) => ({ height: 36, el: ListRow(...) })), selectedIndex)
 */
export class Scroller {
  constructor() {
    this.top = 0;
    this.height = 400;
    this.followed = -1;
  }

  view(id, rows, selected = -1, { gap = 2, padding = 8 } = {}) {
    const k = tokens();
    const tops = [];
    let y = padding;
    for (const r of rows) {
      tops.push(y);
      y += r.height + gap;
    }
    const total = y - gap + padding;
    const max = Math.max(0, total - this.height);
    if (selected >= 0 && selected < rows.length && selected !== this.followed) {
      const [a, b] = [tops[selected] - padding, tops[selected] + rows[selected].height + padding];
      if (a < this.top) this.top = a;
      if (b > this.top + this.height) this.top = b - this.height;
    }
    this.followed = selected;
    this.top = Math.max(0, Math.min(this.top, max));
    const visible = rows.map((r, i) => [r, i]).filter(([r, i]) => tops[i] + r.height > this.top && tops[i] < this.top + this.height);
    const bar = total > this.height ? Math.max(24, (this.height / total) * this.height) : 0;
    return div()
      .id(id)
      .relative()
      .flex_1()
      .min_h(0)
      .overflow_hidden()
      .on_size_change((size, cx) => {
        const h = size.height / (settings().zoom ?? 1);
        if (Math.abs(h - this.height) > 0.5) {
          this.height = h;
          cx.notify();
        }
      })
      .on_scroll_wheel((e, cx) => {
        this.top = Math.max(0, Math.min(this.top - e.delta.y, max));
        cx.notify();
      })
      .children(visible.map(([r, i]) => div().absolute().left(padding).right(padding).top(tops[i] - this.top).h(r.height).child(r.el)))
      .when(bar > 0, (e) =>
        e.child(
          div()
            .absolute()
            .right(2)
            .w(4)
            .rounded(2)
            .bg(alpha(k.mutedFg, 0.35))
            .top((this.top / Math.max(1, max)) * (this.height - bar))
            .h(bar),
        ),
      );
  }
}

// ---------------------------------------------------------------------------------------------------------------------
// Pointer patterns: hover, hover cards, right-click menus, drag to reorder, collapsible sections, side panels

/** Which element the pointer is on, for actions that show on hover: `hover.track(el, key)`, then `hover.is(key)`. */
export class Hover {
  constructor() {
    this.key = null;
  }

  track(el, key) {
    return el.on_hover((on, cx) => {
      if (on) this.key = key;
      else if (this.key === key) this.key = null;
      cx.notify();
    });
  }

  is(key) {
    return this.key === key;
  }
}

/** Details on resting the pointer (shadcn's <HoverCard>): `trigger` shows, `content` opens in a card beside it. */
export function HoverCard(id, trigger, content, { width = 320, openDelay = 500 } = {}) {
  const k = tokens();
  return BaseHoverCard.new(id)
    .open_delay(openDelay)
    .trigger(trigger)
    .content(v_flex().w(width).p(14).gap(8).rounded(RADIUS.lg).border_1().border_color(k.border).bg(k.popover).shadow_lg().text_size(TEXT.base).text_color(k.foreground).child(content));
}

/**
 * A right-click menu at the pointer: items [{ label, icon, kbd, command, destructive } | "separator"]. Choosing one
 * runs `cockpit.<command>`, which reaches the view's `command()` (give commands a prefix of the view's own).
 */
export function openMenu(items, { width = 220 } = {}) {
  const rows = items.filter(Boolean);
  const height = rows.reduce((h, i) => h + (i === "separator" ? 9 : 32), 10);
  openOverlay({ mod: "cockpit", props: { app: "menu", items: rows }, kind: "popup", at: "pointer", width, height, frame: "none" });
}

/** A section that folds (shadcn's <Collapsible>): a header with a chevron, count and action, then its children. */
export function Section(id, { title, count, open = true, onToggle, action, children = [] } = {}) {
  const k = tokens();
  return v_flex()
    .gap(2)
    .child(
      h_flex()
        .id(id)
        .h(28)
        .px(8)
        .gap(6)
        .items_center()
        .rounded(RADIUS.md)
        .cursor_pointer()
        .text_size(TEXT.sm)
        .font_medium()
        .text_color(k.mutedFg)
        .hover((s) => s.text_color(k.foreground))
        .on_click((e, cx) => onToggle?.(e, cx))
        .child(Icon(open ? "chevron-down" : "chevron-right", { size: 14 }))
        .child(title)
        .when(count !== undefined, (e) => e.child(div().text_color(k.dim).child(String(count))))
        .child(div().flex_1())
        .when(Boolean(action), (e) => e.child(action)),
    )
    .when(open, (e) => e.children(children));
}

/** A details panel beside a list: a header with a close button, then its content, scrolling. */
export function SidePanel(id, { title, description, onClose, width = 380, children = [], footer } = {}) {
  const k = tokens();
  return v_flex()
    .id(id)
    .w(width)
    .flex_none()
    .h_full()
    .border_l_1()
    .border_color(k.border)
    .bg(k.background)
    .child(
      h_flex()
        .h(56)
        .px(16)
        .gap(8)
        .flex_none()
        .items_center()
        .border_b_1()
        .border_color(k.border)
        .child(
          v_flex()
            .flex_1()
            .min_w(0)
            .child(div().truncate().text_size(TEXT.base).font_semibold().child(title ?? ""))
            .when(Boolean(description), (e) => e.child(div().truncate().text_size(TEXT.sm).text_color(k.mutedFg).child(description))),
        )
        .when(Boolean(onClose), (e) => e.child(Button(`${id}-close`, { icon: "x", variant: "ghost", size: "icon-sm", tooltip: "Close  esc", onClick: onClose }))),
    )
    .child(v_flex().id(`${id}-body`).flex_1().min_h(0).p(16).gap(12).overflow_y_scrollbar().children(children))
    .when(Boolean(footer), (e) => e.child(h_flex().p(12).gap(8).border_t_1().border_color(k.border).child(footer)));
}

/**
 * Drag to reorder rows: a grip's mouse down starts (`grip(key, index)` gives the grip element), the list root follows
 * the pointer (`track(el)`) and the drop calls `onDrop(key, toIndex)`. Rows are `rowHeight` apart in the list.
 */
export class Reorder {
  constructor(onDrop) {
    this.onDrop = onDrop;
    this.drag = null;
  }

  grip(key, index) {
    const k = tokens();
    return div()
      .id(`grip-${key}`)
      .cursor_grab()
      .text_color(k.dim)
      .hover((s) => s.text_color(k.foreground))
      .on_mouse_down("left", (e, cx) => {
        cx.stop_propagation();
        this.drag = { key, from: index, to: index, y: e.position.y };
        cx.notify();
      })
      .child(Icon("grip", { size: 12 }));
  }

  /** The index the dragged row would land on, or -1: draw a line above that row. */
  target() {
    return this.drag ? this.drag.to : -1;
  }

  dragging(key) {
    return this.drag?.key === key;
  }

  track(el, rowHeight, count) {
    return el
      .on_mouse_move((e, cx) => {
        if (!this.drag) return;
        const z = settings().zoom ?? 1;
        const moved = Math.round((e.position.y - this.drag.y) / (rowHeight * z));
        const to = Math.max(0, Math.min(count - 1, this.drag.from + moved));
        if (to !== this.drag.to) {
          this.drag.to = to;
          cx.notify();
        }
      })
      .on_mouse_up("left", (_e, cx) => {
        const d = this.drag;
        this.drag = null;
        if (d && d.to !== d.from) this.onDrop(d.key, d.to, cx);
        cx.notify();
      });
  }
}

/** A person's own color, the same wherever their name shows (avatar, author, attendee). */
export function personColor(name) {
  const palette = ["ansi1", "ansi2", "ansi3", "ansi4", "ansi5", "ansi6"];
  const hash = [...(name ?? "?")].reduce((h, c) => (h * 31 + c.charCodeAt(0)) >>> 0, 7);
  return theme()[palette[hash % palette.length]];
}

/** A round avatar: initials on a color taken from the name, or an image. */
export function Avatar(name, { size = 24, image: src } = {}) {
  const color = personColor(name);
  const initials = (name ?? "?").split(/[\s._-]+/).filter(Boolean).slice(0, 2).map((w) => w[0].toUpperCase()).join("");
  const el = h_flex().size(size).flex_none().rounded(size / 2).items_center().justify_center().overflow_hidden();
  if (src) return el.child(svg(src).size(size));
  return el.bg(alpha(color, 0.25)).text_color(color).text_size(Math.round(size * 0.42)).font_semibold().child(initials);
}

// ---------------------------------------------------------------------------------------------------------------------
// Charts, drawn natively: paths in percentages of their box (the box follows the zoom), strokes scaled by the zoom

const zoom = () => settings().zoom ?? 1;

/** A line through values (oldest first) filling its box: `Sparkline(values, { w: 80, h: 24, color, area: true })`. */
export function Sparkline(values, { w = 80, h = 24, color, area = false, stroke = 1.5 } = {}) {
  const k = tokens();
  const c = color ?? k.ring;
  const box = div().relative().w(w).h(h).flex_none();
  const points = (values ?? []).filter((v) => Number.isFinite(v));
  if (points.length < 2) return box;
  const [lo, hi] = [Math.min(...points), Math.max(...points)];
  const y = (v) => `${hi === lo ? 50 : 92 - ((v - lo) / (hi - lo)) * 84}%`;
  const x = (i) => `${(i / (points.length - 1)) * 100}%`;
  const line = PathBuilder.stroke(stroke * zoom());
  points.forEach((v, i) => (i ? line.line_to(x(i), y(v)) : line.move_to(x(i), y(v))));
  let el = box;
  if (area) {
    const fill = PathBuilder.fill().move_to("0%", "100%");
    points.forEach((v, i) => fill.line_to(x(i), y(v)));
    fill.line_to("100%", "100%").close();
    el = el.child(window.paint_path(fill.build(), Background.solid(c).opacity(0.15)).absolute().inset_0());
  }
  return el.child(window.paint_path(line.build(), c).absolute().inset_0());
}

/**
 * A line chart: series [{ label, values, color }] over the same x, grid lines, the range on the left, a legend.
 * `xLabels` (first and last shown) name the ends.
 */
export function LineChart(id, { series = [], height = 160, xLabels = [], format = (v) => String(Math.round(v)) } = {}) {
  const k = tokens();
  const all = series.flatMap((s) => s.values).filter((v) => Number.isFinite(v));
  if (!all.length) return Empty({ icon: "activity", title: "No data yet" });
  const [lo, hi] = [Math.min(0, ...all), Math.max(...all)];
  const colors = [k.ring, k.success, k.warning, k.destructive, k.t.ansi5];
  const plot = div().relative().flex_1().h(height);
  const grid = PathBuilder.stroke(zoom());
  for (const f of [0, 0.5, 1]) grid.move_to("0%", `${f * 100}%`).line_to("100%", `${f * 100}%`);
  let el = plot.child(window.paint_path(grid.build(), alpha(k.border, 0.5)).absolute().inset_0());
  series.forEach((s, si) => {
    const n = s.values.length;
    if (n < 2) return;
    const p = PathBuilder.stroke(2 * zoom());
    s.values.forEach((v, i) => {
      const [x, y] = [`${(i / (n - 1)) * 100}%`, `${100 - ((v - lo) / (hi - lo || 1)) * 100}%`];
      i ? p.line_to(x, y) : p.move_to(x, y);
    });
    el = el.child(window.paint_path(p.build(), s.color ?? colors[si % colors.length]).absolute().inset_0());
  });
  return v_flex()
    .id(id)
    .gap(8)
    .child(
      h_flex()
        .gap(8)
        .child(v_flex().h(height).justify_between().text_size(TEXT.xs).text_color(k.mutedFg).child(format(hi)).child(format((hi + lo) / 2)).child(format(lo)))
        .child(el),
    )
    .when(xLabels.length > 0, (e) => e.child(h_flex().pl(28).justify_between().text_size(TEXT.xs).text_color(k.mutedFg).child(xLabels[0] ?? "").child(xLabels[xLabels.length - 1] ?? "")))
    .when(series.length > 1, (e) =>
      e.child(h_flex().gap(12).flex_wrap().children(series.map((s, si) => h_flex().gap(6).items_center().text_size(TEXT.xs).text_color(k.mutedFg).child(div().size(8).rounded(2).bg(s.color ?? colors[si % colors.length])).child(s.label ?? "")))),
    );
}

/** Vertical bars: bars [{ label, value, color, tooltip }], scaled to the tallest, labels under them. */
export function BarChart(id, { bars = [], height = 120, format = (v) => String(v) } = {}) {
  const k = tokens();
  const max = Math.max(1, ...bars.map((b) => b.value ?? 0));
  return h_flex()
    .id(id)
    .h(height + 36)
    .gap(6)
    .items_end()
    .children(
      bars.map((b, i) =>
        v_flex()
          .id(`${id}-${i}`)
          .flex_1()
          .min_w(0)
          .gap(4)
          .items_center()
          .when(Boolean(b.tooltip), (e) => e.tooltip(b.tooltip))
          .child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(format(b.value ?? 0)))
          .child(div().w_full().rounded_t(RADIUS.sm).bg(b.color ?? k.ring).h(Math.max(2, ((b.value ?? 0) / max) * height)))
          .child(div().w_full().truncate().text_center().text_size(TEXT.xs).text_color(k.mutedFg).child(b.label ?? "")),
      ),
    );
}

/** A ring gauge, 0..1 filled, its text in the middle. */
export function Ring(value, { size = 48, stroke = 5, color, label } = {}) {
  const k = tokens();
  const z = zoom();
  const v = Math.max(0, Math.min(1, value ?? 0));
  const r = ((size - stroke) / 2) * z;
  const c = (size / 2) * z;
  const track = PathBuilder.stroke(stroke * z).move_to(c, c - r).arc_to(r, r, 0, false, true, c, c + r).arc_to(r, r, 0, false, true, c, c - r);
  let el = div().relative().size(size).flex_none().child(window.paint_path(track.build(), k.muted).absolute().inset_0());
  if (v > 0) {
    const a = Math.min(v, 0.9999) * 2 * Math.PI;
    const arc = PathBuilder.stroke(stroke * z).move_to(c, c - r).arc_to(r, r, 0, a > Math.PI, true, c + r * Math.sin(a), c - r * Math.cos(a));
    el = el.child(window.paint_path(arc.build(), color ?? k.ring).absolute().inset_0());
  }
  return el.child(h_flex().absolute().inset_0().items_center().justify_center().text_size(TEXT.xs).font_semibold().text_color(k.foreground).child(label ?? `${Math.round(v * 100)}%`));
}

/** A progress bar (shadcn's <Progress>): 0..1 of its width, 6px high. */
export function Progress(value, { color, h = 6, w } = {}) {
  const k = tokens();
  const v = Math.max(0, Math.min(1, value ?? 0));
  let el = div().h(h).rounded(h / 2).bg(k.muted).overflow_hidden().child(div().h_full().rounded(h / 2).bg(color ?? k.primary).w(`${v * 100}%`).transition("width", { duration: 300, easing: "ease-out" }));
  return w ? el.w(w) : el.w_full();
}

/** A dashboard number: label, value, a delta badge and a sparkline under it, in a card. */
export function StatTile(id, { label, value, delta, deltaGood, hint, spark, sparkColor, icon, onClick } = {}) {
  const k = tokens();
  let el = v_flex()
    .id(id)
    .flex_1()
    .min_w(160)
    .p(16)
    .gap(6)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(k.border)
    .bg(k.card)
    .child(
      h_flex()
        .items_center()
        .gap(6)
        .min_w(0)
        .text_size(TEXT.sm)
        .font_medium()
        .text_color(k.mutedFg)
        .when(Boolean(icon), (e) => e.child(Icon(icon, { size: 14 })))
        .child(div().flex_1().min_w(0).child(label ?? "")),
    )
    .child(
      h_flex()
        .items_end()
        .gap(8)
        .min_w(0)
        .child(div().min_w(0).truncate().text_size(24).font_semibold().text_color(k.foreground).child(String(value ?? "—")))
        .when(delta !== undefined && delta !== null, (e) => e.child(Badge(delta, deltaGood === undefined ? "secondary" : deltaGood ? "success" : "destructive"))),
    )
    .when(Boolean(hint), (e) => e.child(div().text_size(TEXT.xs).text_color(k.mutedFg).child(hint)))
    .when(Boolean(spark?.length), (e) => e.child(Sparkline(spark, { w: 200, h: 32, color: sparkColor, area: true }).w_full()));
  if (onClick) el = el.cursor_pointer().hover((s) => s.border_color(k.ring)).on_click(onClick);
  return el;
}

// ---------------------------------------------------------------------------------------------------------------------
// Boards: state chips, stage headings, metrics with mini bars, callouts (the Home board's pieces)

/** Text in the mono face, for what you would type: branches, keys, numbers. */
export function Mono(text, { size = TEXT.xs, color } = {}) {
  const k = tokens();
  return div().flex_none().font_family("JetBrains Mono").text_size(size).text_color(color ?? k.mutedFg).child(String(text ?? ""));
}

/** How long ago or how long: `<1m`, `12m`, `3h`, `2d`. */
export function dur(ms) {
  const m = Math.max(0, Math.floor((ms ?? 0) / 60000));
  if (m < 1) return "<1m";
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  return h < 48 ? `${h}h` : `${Math.floor(h / 24)}d`;
}

/** A fact as a chip: a faint tint of its state color behind it (12%, 22% on hover), mono for what you would type. */
export function Chip(text, { id, color, mono = false, icon, tooltip, onClick } = {}) {
  const k = tokens();
  let el = h_flex()
    .h(22)
    .px(8)
    .gap(5)
    .flex_none()
    .items_center()
    .rounded(RADIUS.md)
    .bg(color ? alpha(color, 0.12) : k.muted)
    .text_size(mono ? TEXT.xs : 11.5)
    .text_color(color ?? k.mutedFg)
    .when(mono, (e) => e.font_family("JetBrains Mono"))
    .when(!mono, (e) => e.font_medium());
  if (icon) el = el.child(Icon(icon, { size: 12 }));
  el = el.child(div().max_w(220).truncate().child(String(text ?? "")));
  if (id) el = el.id(id);
  if (id && onClick)
    el = el.cursor_pointer().hover((s) => s.bg(color ? alpha(color, 0.22) : k.selected)).on_click((e, cx) => {
      cx.stop_propagation();
      onClick(e, cx);
    });
  if (id && tooltip) el = el.tooltip(tooltip);
  return el;
}

/** A count in a pill wide enough from the start, so headings don't shift when counts land ("–" until known). */
export function CountPill(n, { on = true } = {}) {
  const k = tokens();
  return h_flex()
    .h(18)
    .min_w(22)
    .px(6)
    .flex_none()
    .items_center()
    .justify_center()
    .rounded(9)
    .bg(on ? k.muted : alpha(k.border, 0.6))
    .font_family("JetBrains Mono")
    .text_size(TEXT.xs)
    .font_medium()
    .text_color(on ? k.foreground : k.dim)
    .child(n === null || n === undefined ? "–" : String(n));
}

/**
 * A stage heading over a group: its mark (a dot in the stage color, `pulse` to breathe, `spin` for a working ring),
 * small caps, a count pill, a hairline to the end, an action.
 */
export function StageHeading(id, label, { color, count, pulse = false, spin = false, action } = {}) {
  const k = tokens();
  const c = color ?? k.mutedFg;
  const mark = spin
    ? Spinner({ size: 12, color: c })
    : div().id(`${id}-mark`).size(8).flex_none().rounded(4).bg(c).when(pulse, (e) => e.loop("opacity", { from: 0.35, to: 1, period: 1200 }));
  return h_flex()
    .id(id)
    .h(26)
    .gap(8)
    .flex_none()
    .items_center()
    .child(mark)
    .child(div().flex_none().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(String(label).toUpperCase()))
    .child(CountPill(count))
    .child(div().flex_1().h(1).bg(k.border))
    .when(Boolean(action), (e) => e.child(action));
}

/** Small bars for a few values (oldest first), the last one solid and the rest faint: a week at a glance. */
export function MiniBars(values, { color, h = 24, gap = 4, lastSolid = true } = {}) {
  const k = tokens();
  const c = color ?? k.ring;
  const max = Math.max(1, ...(values ?? []).map((v) => v ?? 0));
  return h_flex()
    .h(h)
    .gap(gap)
    .items_end()
    .children(
      (values ?? []).map((v, i) =>
        div()
          .flex_1()
          .min_w(4)
          .rounded(2)
          .bg(c)
          .opacity(lastSolid && i !== values.length - 1 ? 0.32 : 1)
          .h(Math.max(2, Math.round(((v ?? 0) / max) * h))),
      ),
    );
}

/**
 * A dashboard number in a card: a small-caps label, the value in its tone, a hint or mini bars under it; `tint` fills
 * the card with a faint wash of the tone (the one number to look at first).
 */
export function Metric(id, { label, value, tone, tint = false, hint, bars, icon, spin = false, onClick, tooltip } = {}) {
  const k = tokens();
  const c = tone ?? k.foreground;
  let el = v_flex()
    .id(id)
    .flex_1()
    .min_w(132)
    .px(16)
    .py(14)
    .gap(6)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(tint ? alpha(c, 0.35) : k.border)
    .bg(tint ? alpha(c, 0.08) : k.card)
    .child(
      h_flex()
        .gap(6)
        .items_center()
        .min_w(0)
        .when(spin, (e) => e.child(Spinner({ size: 12, color: c })))
        .when(Boolean(icon) && !spin, (e) => e.child(Icon(icon, { size: 13, color: k.mutedFg })))
        .child(div().flex_1().min_w(0).text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(String(label ?? "").toUpperCase())),
    )
    .child(div().min_w(0).truncate().text_size(26).font_semibold().text_color(c).child(String(value ?? "–")))
    .when(Boolean(bars?.length), (e) => e.child(MiniBars(bars, { color: c, h: 22 })))
    .when(Boolean(hint) && !bars?.length, (e) => e.child(div().truncate().text_size(TEXT.xs).text_color(k.mutedFg).child(hint)));
  if (onClick) el = el.cursor_pointer().hover((s) => s.border_color(alpha(c, 0.6))).on_click(onClick);
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

/**
 * A callout across the page in a state's tone (the Home's NEXT MOVE): a small-caps label, a title, a line under it, an
 * optional body (choices), actions on the right.
 */
export function Callout(id, { tone, label, icon, title, meta, body, actions = [], stack = false } = {}) {
  const k = tokens();
  const c = tone ?? k.ring;
  const tag = h_flex()
    .flex_none()
    .gap(6)
    .items_center()
    .text_color(c)
    .when(Boolean(icon), (e) => e.child(Icon(icon, { size: 14 })))
    .child(div().text_size(TEXT.xs).font_semibold().child(String(label ?? "").toUpperCase()));
  const text = v_flex()
    .flex_1()
    .min_w(0)
    .gap(4)
    .child(div().w_full().whitespace_normal().text_size(TEXT.lg).font_medium().text_color(k.foreground).child(title ?? ""))
    .when(Boolean(meta), (e) => e.child(typeof meta === "string" ? div().w_full().whitespace_normal().text_size(TEXT.sm).text_color(k.mutedFg).child(meta) : meta))
    .when(Boolean(body), (e) => e.child(body));
  const buttons = h_flex().flex_none().gap(8).items_center().when(stack, (e) => e.flex_wrap()).children(actions);
  const box = (stack ? v_flex().gap(10).items_start() : h_flex().gap(16).items_center())
    .id(id)
    .w_full()
    .flex_none()
    .px(18)
    .py(14)
    .rounded(RADIUS.lg)
    .border_1()
    .border_color(alpha(c, 0.45))
    .bg(alpha(c, 0.09));
  return box.child(tag).child(text).child(buttons);
}

/** A button in a state's color (solid tone, its text on it): the one action a callout or a card asks for. */
export function ToneButton(id, { label, tone, kbd, icon, onClick, tooltip, size = "sm" } = {}) {
  const k = tokens();
  const c = tone ?? k.primary;
  const h = size === "sm" ? 28 : 32;
  let el = h_flex()
    .id(id)
    .h(h)
    .px(size === "sm" ? 10 : 12)
    .gap(6)
    .flex_none()
    .items_center()
    .rounded(RADIUS.md)
    .bg(c)
    .text_size(size === "sm" ? TEXT.sm : TEXT.base)
    .font_semibold()
    .text_color(k.t.background)
    .whitespace_nowrap()
    .cursor_pointer()
    .hover((s) => s.opacity(0.88))
    .on_click((e, cx) => {
      cx.stop_propagation();
      onClick?.(e, cx);
    });
  if (icon) el = el.child(Icon(icon, { size: 14 }));
  if (label) el = el.child(label);
  if (kbd) el = el.child(div().px(4).rounded(RADIUS.sm).bg(alpha(k.t.background, 0.18)).text_size(TEXT.xs).child(kbd));
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

// ---------------------------------------------------------------------------------------------------------------------
// Dashboards: segmented meters, history strips, proportion bars, choice pills and switches (monitor, routines)

/**
 * A segmented meter (LED bar): parts [{ n, color }] lit in order, the rest of `total` segments faint. Every meter with
 * the same `total` and `segW` lines up with the others.
 */
export function Meter(parts, { total = 20, segW = 6, h = 10, gap = 2, id, tooltip } = {}) {
  const k = tokens();
  const colors = [];
  for (const p of parts ?? []) for (let i = 0; i < (p.n ?? 0) && colors.length < total; i++) colors.push(p.color);
  let el = h_flex()
    .flex_none()
    .h(h)
    .gap(gap)
    .children(Array.from({ length: total }, (_, i) => div().flex_none().w(segW).h_full().rounded(1.5).bg(colors[i] ?? alpha(k.border, 0.7))));
  if (id) el = el.id(id);
  if (id && tooltip) el = el.tooltip(tooltip);
  return el;
}

/**
 * A history strip: one 3px column per value (oldest first), its height the value against the largest, its color by
 * level (0 usual, muted; 1 up, warning; 2 far up, destructive).
 */
export function Strip(values, { levels = [], h = 24, w = 3, gap = 1, id, tooltip } = {}) {
  const k = tokens();
  const max = Math.max(...(values ?? []).map((v) => v ?? 0), 0);
  const color = (l) => (l >= 2 ? k.destructive : l === 1 ? k.warning : alpha(k.mutedFg, 0.45));
  let el = h_flex()
    .flex_none()
    .h(h)
    .gap(gap)
    .items_end()
    .children((values ?? []).map((v, i) => div().flex_none().w(w).rounded(1).bg(color(levels[i] ?? 0)).h(max > 0 ? Math.max(1, Math.round(((v ?? 0) / max) * h)) : 1)));
  if (id) el = el.id(id);
  if (id && tooltip) el = el.tooltip(tooltip);
  return el;
}

/** A bar split in proportion: parts [{ value, color, label }], each part's label as its tooltip; faint when empty. */
export function SegBar(id, parts, { h = 8 } = {}) {
  const k = tokens();
  const shown = (parts ?? []).filter((p) => (p.value ?? 0) > 0);
  return h_flex()
    .id(id)
    .w_full()
    .h(h)
    .gap(2)
    .flex_none()
    .rounded(h / 2)
    .overflow_hidden()
    .when(!shown.length, (e) => e.bg(alpha(k.border, 0.7)))
    .children(shown.map((p, i) => div().id(`${id}-${i}`).h_full().flex_grow(p.value).bg(p.color).when(Boolean(p.label), (e) => e.tooltip(p.label))));
}

/** One choice among a few, as pills: options [[value, label]], the chosen one ringed; `onPick(value, cx)`. */
export function Pills(id, options, value, onPick, { size = "sm" } = {}) {
  const k = tokens();
  const h = size === "sm" ? 26 : 30;
  return h_flex()
    .id(id)
    .gap(6)
    .flex_wrap()
    .children(
      options.map(([v, label, tip]) => {
        const on = v === value;
        return h_flex()
          .id(`${id}-${v}`)
          .h(h)
          .px(10)
          .flex_none()
          .items_center()
          .rounded(RADIUS.md)
          .border_1()
          .border_color(on ? k.ring : k.border)
          .bg(on ? k.selected : k.background)
          .text_size(TEXT.sm)
          .font_medium()
          .text_color(on ? k.foreground : k.mutedFg)
          .cursor_pointer()
          .hover((s) => s.bg(k.accent).text_color(k.foreground))
          .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
          .on_click((_e, cx) => onPick(v, cx))
          .when(Boolean(tip), (e) => e.tooltip(tip))
          .child(label);
      }),
    );
}

/** A switch (shadcn's <Switch>): on or off, `onToggle(cx)`. */
export function Toggle(id, on, onToggle, { tooltip } = {}) {
  const k = tokens();
  let el = h_flex()
    .id(id)
    .w(32)
    .h(18)
    .p(2)
    .flex_none()
    .rounded(9)
    .cursor_pointer()
    .bg(on ? k.primary : k.input)
    .when(on, (e) => e.justify_end())
    .on_mouse_down("left", (_e, cx) => cx.stop_propagation())
    .on_click((_e, cx) => onToggle(cx))
    .child(div().size(14).rounded(7).bg(on ? k.primaryFg : k.background).shadow_sm());
  if (tooltip) el = el.tooltip(tooltip);
  return el;
}

// ---------------------------------------------------------------------------------------------------------------------
// Corner counts and small captions

/** A caption smaller than TEXT.xs: chart axes, day letters under bars, the count on an icon. */
export const CAPTION = 10;

/**
 * A count on an icon's corner (the bell's, an activity button's, a nav icon's): solid in its tone, 15px high, at the
 * top right of its parent, which must be `relative()`. Nothing when `n` is 0 or unknown.
 */
export function CornerCount(n, { tone, top = 1, right = 1, id } = {}) {
  if (!n) return null;
  const k = tokens();
  const c = tone ?? k.blocked;
  let el = h_flex()
    .absolute()
    .top(top)
    .right(right)
    .h(15)
    .min_w(15)
    .px(4)
    .items_center()
    .justify_center()
    .rounded(8)
    .border_1()
    .border_color(k.chrome)
    .bg(c)
    .text_size(CAPTION)
    .font_semibold()
    .text_color(k.t.background)
    .child(n > 99 ? "99+" : String(n));
  if (id) el = el.id(id);
  return el;
}

/** A heading inside a card or side panel (small caps, muted): the level under `StageHeading`. */
export function Overline(text) {
  const k = tokens();
  return div().text_size(TEXT.xs).font_semibold().text_color(k.mutedFg).child(String(text ?? "").toUpperCase());
}

/** Dashboard tiles side by side, wrapping onto the next row once their minimum widths no longer fit; nulls are skipped. */
export function Tiles(tiles) {
  return h_flex().w_full().gap(12).flex_wrap().items_stretch().children(tiles.filter(Boolean));
}
