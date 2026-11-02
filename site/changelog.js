// Renders CHANGELOG.md (copied beside the page at deploy time) into #changelog.
(function () {
  const target = document.getElementById("changelog");
  const repo = "https://github.com/quaresc/cockpit";
  const sources = ["changelog.md", "https://raw.githubusercontent.com/quaresc/cockpit/main/CHANGELOG.md"];

  const escape = (s) => s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");

  function inline(text) {
    const codes = [];
    let s = text.replace(/`([^`]+)`/g, (_, c) => `\u0000${codes.push(c) - 1}\u0000`);
    s = escape(s)
      .replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>")
      .replace(/\[([^\]]+)\]\((https?:\/\/[^)\s]+)\)/g, '<a href="$2">$1</a>')
      .replace(/#(\d+)\b/g, `<a href="${repo}/issues/$1">#$1</a>`);
    return s.replace(/\u0000(\d+)\u0000/g, (_, i) => `<code>${escape(codes[Number(i)])}</code>`);
  }

  function render(md) {
    const out = [];
    let item = null;
    let para = null;
    let inList = false;
    const flushItem = () => { if (item !== null) { out.push(`<li>${inline(item)}</li>`); item = null; } };
    const flushPara = () => { if (para !== null) { out.push(`<p>${inline(para)}</p>`); para = null; } };
    const closeList = () => { flushItem(); if (inList) { out.push("</ul>"); inList = false; } };

    for (const line of md.split("\n")) {
      const heading = line.match(/^## (\S+)\s*(?:\((.*)\))?\s*$/);
      if (/^# /.test(line)) continue;
      if (heading) {
        closeList(); flushPara();
        const id = "v" + heading[1].replace(/[^\w.-]/g, "");
        const date = heading[2] ? ` <span class="date">${escape(heading[2])}</span>` : "";
        out.push(`<h2 id="${id}"><a class="anchor" href="#${id}">${escape(heading[1])}</a>${date}</h2>`);
      } else if (/^\s*$/.test(line)) {
        closeList(); flushPara();
      } else if (/^- /.test(line)) {
        flushPara(); flushItem();
        if (!inList) { out.push("<ul>"); inList = true; }
        item = line.slice(2).trim();
      } else if (item !== null && /^\s/.test(line)) {
        item += " " + line.trim();
      } else {
        closeList();
        para = para === null ? line.trim() : para + " " + line.trim();
      }
    }
    closeList(); flushPara();
    return out.join("\n");
  }

  async function load() {
    for (const url of sources) {
      try {
        const res = await fetch(url, { cache: "no-cache" });
        if (res.ok) return await res.text();
      } catch (_) { /* try the next source */ }
    }
    return null;
  }

  load().then((md) => {
    if (!md) return;
    target.innerHTML = render(md);
    if (location.hash) document.getElementById(location.hash.slice(1))?.scrollIntoView();
  });
})();
