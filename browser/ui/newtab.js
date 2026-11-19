// The new-tab page: the servers listening in this worktree, read from the page's own address.
const params = new URLSearchParams(location.search);
const worktree = params.get("worktree") || "";
let servers = [];
try {
  servers = JSON.parse(params.get("servers") || "[]");
} catch {}
document.getElementById("name").textContent = worktree.split("/").filter(Boolean).pop() || "New tab";
const list = document.getElementById("servers");
for (const server of servers) {
  if (!Number.isInteger(server.port)) continue;
  const link = document.createElement("a");
  link.href = `http://localhost:${server.port}/`;
  const name = document.createElement("strong");
  name.textContent = `localhost:${server.port}`;
  const what = document.createElement("span");
  what.textContent = String(server.command || "");
  link.append(name, what);
  const item = document.createElement("li");
  item.append(link);
  list.append(item);
}
if (!list.children.length) document.getElementById("hint").textContent = "Type an address above (⌘L). Servers listening in this worktree show here.";
