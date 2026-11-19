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
