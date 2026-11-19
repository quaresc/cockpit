// The toolbar's and the new-tab page's bridge to the shell: messages out, state in.
const { contextBridge, ipcRenderer } = require("electron");

contextBridge.exposeInMainWorld("cockpit", {
  send: (message) => ipcRenderer.send("ui", message),
  onState: (handler) => ipcRenderer.on("state", (_event, state) => handler(state)),
});
