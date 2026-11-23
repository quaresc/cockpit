// Paths and links in what a conversation says, made clickable: a bare URL or file path in its words, or alone in a code
// span, becomes a Markdown link (a path as `file://`), and the images it names are listed to be drawn under it.

const SEGMENT = String.raw`[\w.@+%~-]+`;
// A path starts at ~ or /, has two segments at least (so a slash command is none), and may end with :<line>.
const PATH = String.raw`(?:~|\/${SEGMENT})(?:\/${SEGMENT})+(?::\d+)?`;
const BARE_PATH = new RegExp(String.raw`(?<![\w:/.~@%+\-\]])${PATH}`, "g");
const BARE_URL = /(?<![\w/])https?:\/\/[^\s<>()[\]`"']+/g;
const ONLY_PATH = new RegExp(String.raw`^${PATH}\/?$`);
const ONLY_URL = /^https?:\/\/\S+$/;
const IMAGE = new RegExp(String.raw`(?<![\w:/.~@%+\-])(?:~|\/${SEGMENT})(?:\/${SEGMENT})*\.(?:png|jpe?g|gif|webp)(?![\w.])`, "gi");
// What Markdown already holds as code or links is left as it is.
const KEPT = /(`[^`\n]+`|\[[^\]\n]*\]\([^)\n]*\)|<[a-z][a-z0-9+.-]*:[^>\s]+>)/i;
// Sentence punctuation after a path or a URL is not part of it.
const TRAILING = /[.,;:!?'"]+$/;

const cut = (s) => {
  const tail = TRAILING.exec(s)?.[0] ?? "";
  return [s.slice(0, s.length - tail.length), tail];
};

/** The link a path or URL opens: the URL itself, or the path as `file://`. */
export const linkOf = (target) => (ONLY_URL.test(target) ? target : `file://${encodeURI(target)}`);

/** The path a `file://` link opens, or null for any other link. */
export function pathOf(url) {
  if (!String(url ?? "").startsWith("file://")) return null;
  try {
    return decodeURI(url.slice(7));
  } catch {
    return url.slice(7);
  }
}

/** `path:line` split: the file, and the line as a number when one is given. */
export function fileAndLine(path) {
  const m = /^(.*?)(?::(\d+))?$/.exec(path);
  return { file: m[1], line: m[2] ? Number(m[2]) : null };
}

/** `~/x` as the absolute path under `home`. */
export const expand = (path, home) => (path.startsWith("~/") ? `${home}${path.slice(1)}` : path);

/** Markdown with its bare URLs and paths as links, and a code span holding only one wrapped in a link. */
export function linkify(text) {
  return String(text ?? "")
    .split(KEPT)
    .map((part, i) => {
      if (i % 2 === 1) {
        if (!part.startsWith("`")) return part;
        const inner = part.slice(1, -1).trim();
        return ONLY_URL.test(inner) || ONLY_PATH.test(inner) ? `[${part}](${linkOf(inner.replace(/\/$/, "") || inner)})` : part;
      }
      return part
        .replace(BARE_URL, (url) => {
          const [clean, tail] = cut(url);
          return `[${clean}](${clean})${tail}`;
        })
        .replace(BARE_PATH, (path) => {
          const [clean, tail] = cut(path);
          return `[${clean}](${linkOf(clean)})${tail}`;
        });
    })
    .join("");
}

/** The images a text names by path, absolute (`~` under `home`), each once, at most `max`. */
export function imagesIn(text, home, max = 6) {
  const out = [];
  for (const m of String(text ?? "").matchAll(IMAGE)) {
    const path = expand(m[0], home);
    if (path.startsWith("/") && !out.includes(path)) out.push(path);
    if (out.length >= max) break;
  }
  return out;
}
