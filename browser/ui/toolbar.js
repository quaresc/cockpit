// The toolbar: the row of tabs, the address bar with its suggestions, and the buttons. It draws the state the shell
// sends and sends back what the person does; it decides nothing.

const $ = (id) => document.getElementById(id);
const send = (type, more = {}) => window.cockpit.send({ type, ...more });
const tabs = $("tabs");
const address = $("address");
const suggestions = $("suggestions");
let state = { tabs: [], nav: {}, suggestions: [] };
let typing = false;
let picked = -1;
let dragged = null;
/** A note's intent and priority, as they are shown. */
const words = { fix: "Fix", change: "Change", question: "Question", approve: "Approve", blocking: "Blocking", important: "Important", suggestion: "Suggestion" };

const active = () => state.tabs.find((t) => t.active) || null;

/** Whether the toolbar needs the page's room too: its suggestions, the rename field, a note's card or the notes are open. */
function setOpen() {
  send("open", { open: !suggestions.hidden || !$("rename").hidden || !$("draft").hidden || !$("recap").hidden });
}

function el(tag, props = {}, children = []) {
  const node = document.createElement(tag);
  for (const [key, value] of Object.entries(props)) {
    if (key === "class") node.className = value;
    else if (key.startsWith("on")) node.addEventListener(key.slice(2), value);
    else if (value !== null && value !== undefined) node.setAttribute(key, value);
  }
  for (const child of children) node.append(child);
  return node;
}

function drawTabs() {
  tabs.replaceChildren(
    ...state.tabs.map((tab, at) => {
      const classes = ["tab", tab.active && "active", tab.asleep && "asleep", tab.loading && "loading"].filter(Boolean).join(" ");
      const icon = tab.favicon && !tab.loading ? el("img", { src: tab.favicon, alt: "" }) : el("span", { class: "dot" });
      const close = el("button", { class: "x", title: "Close tab (⌘W)", "aria-label": "Close tab", onclick: (e) => (e.stopPropagation(), send("closeTab", { id: tab.id })) }, ["×"]);
      return el(
        "div",
        {
          class: classes,
          role: "tab",
          draggable: "true",
          title: tab.title || tab.url || "New tab",
          "aria-selected": tab.active ? "true" : "false",
          onclick: () => send("activate", { id: tab.id }),
          onauxclick: (e) => e.button === 1 && send("closeTab", { id: tab.id }),
          oncontextmenu: (e) => (e.preventDefault(), send("menu", { name: "tab", id: tab.id })),
          ondragstart: () => (dragged = tab.id),
          ondragover: (e) => e.preventDefault(),
          ondrop: () => dragged && dragged !== tab.id && send("moveTab", { id: dragged, to: at }),
        },
        [icon, el("span", { class: "title" }, [tab.title || tab.url || "New tab"]), close],
      );
    }),
    el("button", { id: "new-tab", title: "New tab (⌘T)", "aria-label": "New tab", onclick: () => send("newTab") }, ["+"]),
    el("span", { id: "worktree", title: state.worktree?.root || "" }, [state.worktree?.name || ""]),
  );
}

function drawSuggestions() {
  const list = typing ? state.suggestions || [] : [];
  suggestions.hidden = list.length === 0;
  suggestions.replaceChildren(
    ...list.map((s, at) =>
      el("li", { class: at === picked ? "on" : "", onmousedown: (e) => (e.preventDefault(), go(s.url)) }, [el("span", {}, [s.title || s.url]), el("span", { class: "url" }, [s.url])]),
    ),
  );
  setOpen();
}

function applyTheme() {
  const root = document.documentElement;
  root.classList.toggle("dark", !!state.dark);
  for (const [name, value] of Object.entries(state.theme || {})) {
    if (/^(bg|panel|fg|muted|line|accent|hover)$/.test(name) && /^#[0-9a-f]{6,8}$/i.test(String(value))) root.style.setProperty(`--${name}`, value);
  }
}

function draw() {
  applyTheme();
  drawTabs();
  const tab = active();
  if (!typing) address.value = tab?.url || "";
  $("back").disabled = !state.nav.back;
  $("forward").disabled = !state.nav.forward;
  $("reload").title = state.nav.loading ? "Stop" : "Reload (⌘R)";
  $("reload").dataset.stop = state.nav.loading ? "1" : "";
  $("profile").textContent = state.profile?.name || "Default";
  $("viewport").classList.toggle("on", !!state.viewport);
  $("grab").classList.toggle("on", state.grab?.mode === "copy");
  $("annotate").classList.toggle("on", state.grab?.mode === "annotate");
  $("devtools").classList.toggle("on", !!state.devtools);
  const count = (state.notes || []).length;
  $("notes").hidden = !count;
  $("notes").textContent = count || "";
  drawRecap();
  for (const id of ["viewport", "grab", "annotate", "devtools", "reload"]) $(id).disabled = !tab;
  const finding = !!state.find || document.activeElement === $("find");
  $("finder").hidden = !finding;
  $("where").hidden = finding;
  $("found").textContent = state.find?.matches ? `${state.find.at}/${state.find.matches}` : state.find ? "0/0" : "";
  drawSuggestions();
  if (state.focusAddress) (address.focus(), address.select());
  if (state.focusFind) (($("finder").hidden = false), ($("where").hidden = true), $("find").focus(), $("find").select());
  if (state.draft) openDraft(state.draft);
  if (state.rename) {
    $("rename").hidden = false;
    $("rename-name").value = state.rename.name;
    $("rename").dataset.id = state.rename.id;
    $("rename-name").focus();
    $("rename-name").select();
    setOpen();
  }
}

function go(text) {
  typing = false;
  picked = -1;
  suggestions.hidden = true;
  send("navigate", { text });
  address.blur();
  setOpen();
}

address.addEventListener("focus", () => address.select());
address.addEventListener("input", () => {
  typing = true;
  picked = -1;
  send("suggest", { text: address.value });
});
address.addEventListener("blur", () => {
  typing = false;
  picked = -1;
  drawSuggestions();
  address.value = active()?.url || "";
});
address.addEventListener("keydown", (event) => {
  const list = state.suggestions || [];
  if (event.key === "ArrowDown" || event.key === "ArrowUp") {
    if (!list.length) return;
    event.preventDefault();
    picked = (picked + (event.key === "ArrowDown" ? 1 : -1) + list.length + 1) % (list.length + 1);
    if (picked < list.length) address.value = list[picked].url;
    drawSuggestions();
  } else if (event.key === "Escape") {
    event.preventDefault();
    address.blur();
    send("focusPage");
  }
});
$("where").addEventListener("submit", (event) => (event.preventDefault(), address.value.trim() && go(address.value)));

$("back").onclick = () => send("back");
$("forward").onclick = () => send("forward");
$("reload").onclick = () => send($("reload").dataset.stop ? "stop" : "reload");
$("devtools").onclick = () => send("devtools");
$("grab").onclick = () => send("grab", { mode: "copy" });
// With notes written, the button shows them; with none, it starts the first one.
$("annotate").onclick = () => {
  if (!(state.notes || []).length) return send("grab", { mode: "annotate" });
  if (state.grab) send("grab", { mode: state.grab.mode });
  showRecap($("recap").hidden);
};
for (const name of ["profile", "viewport", "more"]) $(name).onclick = () => send("menu", { name });

const recap = $("recap");
function showRecap(on) {
  recap.hidden = !on;
  drawRecap();
  setOpen();
}

/** The notes written so far, each with its number as the page badges it. */
function drawRecap() {
  const notes = state.notes || [];
  if (!notes.length && !recap.hidden) {
    recap.hidden = true;
    setOpen();
  }
  if (recap.hidden) return;
  $("recap-details").checked = !!state.details;
  $("recap-send").textContent = notes.length === 1 ? "Send the note" : `Send ${notes.length} notes`;
  $("recap-list").replaceChildren(
    ...notes.map((note, at) =>
      el("li", {}, [
        el("span", { class: "n" }, [String(at + 1)]),
        el("div", { class: "what" }, [el("div", { class: "comment" }, [note.comment]), el("div", { class: "element", title: note.label }, [note.label]), el("div", { class: "tags" }, [`${words[note.intent] || note.intent} · ${words[note.priority] || note.priority}`])]),
        el("button", { class: "x", title: "Remove this note", "aria-label": "Remove this note", onclick: () => send("notes", { do: "delete", id: note.id }) }, ["×"]),
      ]),
    ),
  );
}
$("recap-details").onchange = () => send("notes", { do: "details", on: $("recap-details").checked });
$("recap-copy").onclick = () => send("notes", { do: "copy" });
$("recap-clear").onclick = () => (send("notes", { do: "clear" }), showRecap(false));
$("recap-send").onclick = () => (send("notes", { do: "send" }), showRecap(false));
$("recap-add").onclick = () => (showRecap(false), send("grab", { mode: "annotate" }));
recap.addEventListener("keydown", (event) => event.key === "Escape" && showRecap(false));

const find = $("find");
const search = (forward, next) => send("find", { text: find.value, forward, next });
find.addEventListener("input", () => search(true, false));
$("finder").addEventListener("submit", (event) => (event.preventDefault(), search(true, true)));
$("find-next").onclick = () => search(true, true);
$("find-prev").onclick = () => search(false, true);
const closeFind = () => ((find.value = ""), send("findClose"), find.blur(), draw(), send("focusPage"));
$("find-close").onclick = closeFind;
find.addEventListener("keydown", (event) => event.key === "Escape" && (event.preventDefault(), closeFind()));
find.addEventListener("blur", () => !state.find && draw());

const rename = $("rename");
const endRename = (save) => {
  if (save && $("rename-name").value.trim()) send("rename", { id: rename.dataset.id, name: $("rename-name").value });
  rename.hidden = true;
  setOpen();
};
rename.addEventListener("submit", (event) => (event.preventDefault(), endRename(true)));
$("rename-name").addEventListener("keydown", (event) => event.key === "Escape" && endRename(false));
$("rename-name").addEventListener("blur", () => !rename.hidden && endRename(true));

const draft = $("draft");
const options = (select, values, picked) => select.replaceChildren(...values.map((value) => el("option", { value, ...(value === picked ? { selected: "" } : {}) }, [words[value] || value])));

/** Shows the card a note is written in, under the picked element and inside the window. */
function openDraft(asked) {
  $("draft-label").textContent = asked.label;
  $("draft-comment").value = "";
  // A pick is one line for the agent, sent at once; a note is kept with the others.
  $("draft-comment").placeholder = asked.ask ? "Ask the agent, or say what should change here" : "What should change here?";
  $("draft-save").textContent = asked.ask ? "Send" : "Add note";
  $("draft-intent").hidden = $("draft-priority").hidden = !!asked.ask;
  options($("draft-intent"), asked.intents, "change");
  options($("draft-priority"), asked.priorities, asked.priority);
  draft.hidden = false;
  draft.style.left = `${Math.max(8, Math.min(asked.x, innerWidth - 364))}px`;
  draft.style.top = `${Math.max(84, Math.min(asked.y + 8, innerHeight - draft.offsetHeight - 8))}px`;
  setOpen();
  $("draft-comment").focus();
}

function closeDraft(save) {
  if (draft.hidden) return;
  draft.hidden = true;
  send("note", { save, comment: $("draft-comment").value, intent: $("draft-intent").value, priority: $("draft-priority").value });
}
draft.addEventListener("submit", (event) => (event.preventDefault(), closeDraft(true)));
$("draft-cancel").onclick = () => closeDraft(false);
// Enter in the note saves it; with Shift or Option it is a new line, as in a chat box.
draft.addEventListener("keydown", (event) => {
  if (event.key === "Escape") return (event.preventDefault(), closeDraft(false));
  if (event.key !== "Enter" || event.target !== $("draft-comment") || event.isComposing) return;
  if (event.shiftKey || event.altKey) return;
  event.preventDefault();
  closeDraft(true);
});

window.cockpit.onState((next) => {
  state = next;
  draw();
});
send("ready");
