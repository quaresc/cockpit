// The clipboard's image as a file, for a box to send with its words: a mod reads only the clipboard's text, so the
// system's own tool writes the image (osascript on macOS, wl-paste or xclip elsewhere) to a new private file.

import { exec } from "herdr";

/** The largest image a paste keeps, in bytes: past it, the file is the way. */
export const MAX_IMAGE = 20 * 1024 * 1024;

/**
 * Writes the clipboard's image to a new file only its owner reads and prints its path, after deleting this person's
 * pastes older than a day. Exits 2 when the clipboard holds no image, 3 when no tool reads one, 4 past MAX_IMAGE.
 */
export const PASTE_IMAGE = `umask 077
find "\${TMPDIR:-/tmp}/" -maxdepth 1 -type d -name 'cockpit-paste.*' -user "$(id -u)" -mtime +0 -exec rm -rf {} + 2>/dev/null
d=$(mktemp -d "\${TMPDIR:-/tmp}/cockpit-paste.XXXXXX") || exit 1
f="$d/pasted-\${d##*.}.png"
gone() { rm -rf "$d"; exit "$1"; }
if command -v osascript >/dev/null 2>&1; then
  osascript -e 'clipboard info' 2>/dev/null | grep -qE 'PNGf|TIFF' || gone 2
  osascript -e 'on run argv' -e 'set f to open for access (POSIX file (item 1 of argv)) with write permission' -e 'write (the clipboard as «class PNGf») to f' -e 'close access f' -e 'end run' "$f" >/dev/null || gone 1
elif [ -n "\${WAYLAND_DISPLAY:-}" ] && command -v wl-paste >/dev/null 2>&1; then
  wl-paste --list-types 2>/dev/null | grep -qx 'image/png' || gone 2
  wl-paste --type image/png >"$f" || gone 1
elif command -v xclip >/dev/null 2>&1; then
  xclip -selection clipboard -t TARGETS -o 2>/dev/null | grep -qx 'image/png' || gone 2
  xclip -selection clipboard -t image/png -o >"$f" || gone 1
else
  gone 3
fi
[ -s "$f" ] || gone 1
[ "$(wc -c <"$f")" -le ${MAX_IMAGE} ] || gone 4
chmod 600 "$f"
printf '%s\\n' "$f"
`;

/** What a run of the script says: `{path}` of the image written, `{none}` when there was no image, else `{error, detail}`. */
export function pasted(r) {
  const path = String(r?.stdout ?? "").trim().split("\n").pop();
  if (r?.code === 0 && path) return { path };
  if (r?.code === 2) return { none: true };
  if (r?.code === 3) return { error: "this machine has neither wl-paste nor xclip to read it: drop the file on the box", detail: "" };
  if (r?.code === 4) return { error: `the clipboard's image is over ${MAX_IMAGE / 1024 / 1024} MB: save it and drop the file on the box`, detail: "" };
  return { error: "the clipboard's image could not be read", detail: String(r?.stderr ?? "").trim() };
}

/** Reads the clipboard's image into a file: `{path}`, `{none}` or `{error, detail}`. */
export async function pasteImage(run = exec) {
  try {
    return pasted(await run("sh", ["-c", PASTE_IMAGE]));
  } catch (e) {
    return { error: "the clipboard's image could not be read", detail: String(e?.message ?? e) };
  }
}

/** A whole line of a prompt that is an image's path, the way the box sends the files riding with it. */
export const IMAGE_LINE = /^\/.*\.(png|jpe?g|gif|webp)$/i;

/**
 * Copies an image into the app's folder, the only place a mod draws pictures from, and prints its size: `sips` on
 * macOS, `file` elsewhere. Copies untouched for a day go; exits non-zero when the image itself is gone.
 */
export const THUMB = `d=$(dirname "$1")
mkdir -p "$d" && find "$d" -maxdepth 1 -type f -mtime +0 -exec rm -f {} + 2>/dev/null
{ [ -s "$1" ] && touch "$1"; } || cp "$2" "$1" || exit 1
if command -v sips >/dev/null 2>&1; then sips -g pixelWidth -g pixelHeight "$1"; else file -b "$1" || true; fi
`;

/** The app-relative path of an image's copy, named after where it came from so each image is copied once. */
export function thumbPath(path) {
  let h = 5381;
  for (const c of String(path)) h = ((h << 5) + h + c.charCodeAt(0)) | 0;
  return `data/cockpit/thumbs/${(h >>> 0).toString(36)}.${String(path).split(".").pop().toLowerCase()}`;
}

/** What a run of THUMB says: `{rel, w, h}` of the copy, or null when the image could not be read. */
export function thumbed(r, rel) {
  if (r?.code !== 0) return null;
  const out = String(r.stdout ?? "");
  const dims = [...out.matchAll(/(\d+) ?x ?(\d+)/g)].pop();
  const w = Number(/pixelWidth: (\d+)/.exec(out)?.[1] ?? dims?.[1]), h = Number(/pixelHeight: (\d+)/.exec(out)?.[1] ?? dims?.[2]);
  return { rel, w: w || 1, h: h || 1 };
}

/** An image's copy in the app's folder `home`: `{rel, w, h}`, or null when it is gone. */
export async function thumbnail(path, home, run = exec) {
  const rel = thumbPath(path);
  try {
    return thumbed(await run("sh", ["-c", THUMB, "sh", `${home}/${rel}`, path]), rel);
  } catch {
    return null;
  }
}
