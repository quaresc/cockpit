// Next steps: up to three prompts the agent the chat follows suggests after its turn, drawn above the box. A suggestion
// only fills the box, for the person to read, change and send themselves; its words are the agent's, untrusted.

import { div } from "gpui-kit";
import { h_flex } from "gpui-base";
import { TEXT, Icon, Spinner } from "./ui.js";

/** The keys that pick a suggestion (1, 2, 3) or put them away (0), bound in the chat's key context while they show. */
export const NEXT_KEYS = ["1", "2", "3", "0"];

/** The most characters a button and a prompt hold, as the hub caps them. */
const LABEL_MAX = 48;
const PROMPT_MAX = 600;

/**
 * What the hub sent, cleaned again before it is drawn or put in the box: escape sequences, control and format
 * characters (zero widths, bidi), private use and variation selectors out, whitespace folded, capped; text holding
 * Unicode tag characters is refused whole.
 */
export function cleanText(text, max) {
  const s = String(text ?? "");
  if (/[\u{E0000}-\u{E007F}]/u.test(s)) return "";
  const out = s
    .replace(/\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]/g, "")
    .replace(/\s+/g, " ")
    .replace(/[\p{Cc}\p{Cf}\p{Co}\p{Cs}\p{Variation_Selector}ᅟᅠㅤﾠ]/gu, "")
    .replace(/ {2,}/g, " ")
    .trim();
  const points = [...out];
  return points.length > max ? `${points.slice(0, max - 1).join("").trimEnd()}…` : out;
}

/**
 * Whether a prompt is safe to put in the box: not one that runs something the moment it is sent (a shell line `!` or a
 * memory note `#` in Claude Code, Codex and Gemini CLI), nor a `/` command unless the hub checked it (`commands`).
 */
export const sendable = (prompt, commands = false) => Boolean(prompt) && !/^[!#]/.test(prompt) && (commands === true || !prompt.startsWith("/"));

/**
 * What the strip shows now, or null: the hub's suggestions for this turn (or its run going), while the agent rests,
 * no question waits, nothing is typed or being sent, and the person did not put away this turn's.
 */
export function nextShown(next, { working = false, question = false, typed = "", sending = false, dismissed = null } = {}) {
  if (!next || !next.turn || working || question || sending || String(typed ?? "").trim() || dismissed === next.turn) return null;
  if (next.state === "unavailable") return { turn: next.turn, state: "unavailable", reason: cleanText(next.reason || "Next steps are unavailable for this agent", 220), items: [] };
  const by = cleanText(next.by, 40) || undefined;
  if (next.state === "loading") return { turn: next.turn, state: "loading", by, items: [] };
  const items = (Array.isArray(next.items) ? next.items : [])
    .filter((i) => typeof i?.prompt === "string")
    .map((i) => ({ label: typeof i.label === "string" ? cleanText(i.label, LABEL_MAX) : "", prompt: cleanText(i.prompt, PROMPT_MAX) }))
    .filter((i) => sendable(i.prompt, next.commands))
    .map((i) => ({ ...i, label: i.label || cleanText(i.prompt, LABEL_MAX) }))
    .slice(0, 3);
  return next.state === "ready" && items.length ? { turn: next.turn, state: "ready", by, items } : null;
}

/**
 * What a key does with suggestions showing: `{fill}` with the prompt for 1, 2 or 3, `{dismiss}` for 0, else null. A key
 * held down or sent from outside the window does nothing, nor any while a window command put the chat on its agent.
 */
export function nextKey(key, shown, { held = false, injected = false, pointed = false } = {}) {
  if (!shown || shown.state !== "ready" || held || injected || pointed) return null;
  if (key === "0") return { dismiss: true };
  const item = /^[1-3]$/.test(String(key)) ? shown.items[Number(key) - 1] : null;
  return item ? { fill: item.prompt } : null;
}

/** Who writes the suggestions, in the strip's dim words: the agent the hub named, billed to its account. */
const byLine = (by) => (by ? `by ${by}` : "");

/**
 * The strip above the box: a dim "Next", a button per suggestion (its prompt and key in its tooltip, as the chat says
 * keys), a dim "by <agent>" naming who wrote them, and a way to put them away; "Next steps by <agent>…" while it is asked.
 */
export function NextSteps(shown, k, { onPick, onDismiss }) {
  const row = h_flex().id("chat-next").w_full().gap(6).items_center().flex_wrap().text_size(TEXT.sm);
  const heading = div().flex_none().text_size(TEXT.xs).text_color(k.dim).child("Next");
  const billed = shown.by ? `Written by ${shown.by} and billed to its account` : null;
  if (shown.state === "unavailable") return row.child(div().text_size(TEXT.xs).text_color(k.dim).child(shown.reason));
  if (shown.state === "loading") {
    return row.child(Spinner({ size: 12, color: k.dim })).child(div().text_size(TEXT.xs).text_color(k.dim).child(shown.by ? `Next steps ${byLine(shown.by)}…` : "Next steps…"));
  }
  const strip = row
    .child(heading)
    .children(
      shown.items.map((item, i) =>
        div()
          .id(`chat-next-${i + 1}`)
          .h(26)
          .px(10)
          .flex()
          .items_center()
          .max_w(260)
          .rounded(13)
          .border_1()
          .border_color(k.border)
          .cursor_pointer()
          .text_color(k.mutedFg)
          .hover((s) => s.bg(k.accent).text_color(k.foreground))
          .tooltip(`${item.prompt}  ${i + 1}`)
          .on_click((_e, cx) => onPick(item.prompt, cx))
          .child(div().min_w(0).truncate().child(item.label)),
      ),
    );
  return (billed ? strip.child(div().id("chat-next-by").flex_none().text_size(TEXT.xs).text_color(k.dim).tooltip(billed).child(byLine(shown.by))) : strip)
    .child(
      div()
        .id("chat-next-dismiss")
        .size(22)
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .rounded(11)
        .cursor_pointer()
        .text_color(k.dim)
        .hover((s) => s.bg(k.accent).text_color(k.foreground))
        .tooltip("Put these away  0")
        .on_click((_e, cx) => onDismiss(cx))
        .child(Icon("x", { size: 13 })),
    );
}
