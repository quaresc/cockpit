// Code in a conversation or a note, drawn by Herdr's read-only editor: coloured by its language and selectable. Each
// block keeps its editor while it is drawn; until the editor is made, and where Herdr cannot make one, plain lines show.

import { div } from "gpui-kit";
import { v_flex, Editor, EditorState } from "gpui-base";
import { theme, settings } from "herdr";

/** Herdr's editor language for a file extension or a fence's tag, where the name differs. */
export const LANG = {
  ts: "js", tsx: "js", mts: "js", cts: "js", jsx: "js", mjs: "js", cjs: "js", typescript: "js", javascript: "js",
  toml: "yaml", yml: "yaml", jsonc: "json", json5: "json", svelte: "html", vue: "html", htm: "html", md: "markdown",
  py: "python", rs: "rust", sh: "bash", zsh: "bash", shell: "bash", console: "bash", shellscript: "bash",
  ex: "elixir", exs: "elixir", heex: "elixir",
};

/** The editor language a fence's tag names: its first word, lowercased and mapped; "text" when it names none. */
export function languageOf(tag) {
  const word = String(tag ?? "").trim().split(/[\s{,]/)[0].toLowerCase();
  return LANG[word] ?? (word || "text");
}

/** The theme's colours for each kind of token, the code view's. */
export function syntaxColors(t = theme()) {
  return { keyword: t.ansi5, string: t.ansi2, comment: t.dim, function: t.ansi4, type: t.ansi3, number: t.ansi1, constant: t.ansi6, boolean: t.ansi6 };
}

/** A message's words split on fences, so code is drawn in a box of its own: Herdr's Markdown draws it square. */
export function segments(text) {
  const out = [];
  const fence = /```([^\n`]*)\n([\s\S]*?)```/g;
  let last = 0;
  let m;
  while ((m = fence.exec(text))) {
    if (m.index > last) out.push({ code: false, text: text.slice(last, m.index) });
    out.push({ code: true, lang: m[1].trim(), text: m[2] });
    last = fence.lastIndex;
  }
  // A fence still being written is code already, up to where the words stop.
  const rest = text.slice(last);
  const open = /```([^\n`]*)\n/.exec(rest);
  if (open) {
    out.push({ code: false, text: rest.slice(0, open.index) });
    out.push({ code: true, lang: open[1].trim(), text: rest.slice(open.index + open[0].length) });
  } else if (rest) out.push({ code: false, text: rest });
  return out.filter((s) => s.text.trim() !== "");
}

/** Text size and line height of a code block, in pixels. */
const SIZE = 12;
const LINE = 18;
/** The tallest a block grows before it scrolls, in lines. */
const MAX_LINES = 40;

/** The block's lines as plain text, while its editor is not there. */
function plainLines(text) {
  return v_flex().children(text.split("\n").map((line) => div().whitespace_nowrap().child(line.replace(/\t/g, "    ") || " ")));
}

/**
 * The code blocks a view draws, each with its editor. `view` returns a block's body; `make` creates the editors asked
 * for since its last call and runs outside render, as Herdr wants; `sweep` releases those no longer drawn.
 */
export class CodeBlocks {
  constructor() {
    this.made = new Map();
    this.wanted = new Map();
    this.drawn = new Set();
  }

  view(id, lang, text, k) {
    this.drawn.add(id);
    const got = this.made.get(id);
    const lines = Math.min(MAX_LINES, text.split("\n").length);
    const box = div().w_full().px(12).py(10).font_family("JetBrains Mono").text_size(SIZE).text_color(k.foreground);
    if (got?.state) {
      if (got.text !== text) {
        got.state.set_value(text);
        got.text = text;
      }
      return box.h(lines * LINE + 20).child(Editor.new(got.state).size_full());
    }
    if (!got) this.wanted.set(id, { lang, text });
    return box.id(`${id}-lines`).overflow_x_scroll().child(plainLines(text));
  }

  /** Creates the editors asked for since the last call: true when one was made. */
  make() {
    if (!this.wanted.size || typeof EditorState?.new !== "function") return false;
    let any = false;
    for (const [id, { lang, text }] of this.wanted) {
      try {
        const state = EditorState.new({
          language: languageOf(lang),
          value: text,
          readonly: true,
          line_numbers: false,
          folding: false,
          soft_wrap: false,
          font_family: settings().font_family,
          font_size: SIZE,
          syntax: syntaxColors(),
        });
        this.made.set(id, { state, text });
        any = true;
      } catch {
        // Kept as plain lines: a block Herdr cannot draw as code is not asked again.
        this.made.set(id, { state: null, text });
      }
    }
    this.wanted.clear();
    return any;
  }

  /** Releases the editors of blocks the last frame did not draw, and starts counting the next frame's. */
  sweep() {
    for (const [id, got] of this.made) {
      if (this.drawn.has(id)) continue;
      got.state?.release();
      this.made.delete(id);
    }
    this.drawn = new Set();
  }

  /** Releases every editor. */
  clear() {
    for (const got of this.made.values()) got.state?.release();
    this.made.clear();
    this.wanted.clear();
    this.drawn = new Set();
  }
}
