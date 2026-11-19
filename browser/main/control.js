// The control socket: the hub and `cockpit browse` reach the shell here, one JSON request per line.

import fs from "node:fs";
import net from "node:net";
import path from "node:path";
import { parse, trusted, GUARDED, HUB_ONLY } from "./protocol.js";

/** Starts the server; `handle(request, {trusted})` answers a request, `subscribe(send)` returns the way to stop. */
export function serve(socketPath, { handle, subscribe, token, log }) {
  fs.mkdirSync(path.dirname(socketPath), { recursive: true });
  fs.rmSync(socketPath, { force: true });
  const server = net.createServer((socket) => {
    let buffer = "";
    let stop = null;
    const send = (value) => !socket.destroyed && socket.write(`${JSON.stringify(value)}\n`);
    socket.setEncoding("utf8");
    socket.on("data", async (chunk) => {
      buffer += chunk;
      let at;
      while ((at = buffer.indexOf("\n")) >= 0) {
        const line = buffer.slice(0, at).trim();
        buffer = buffer.slice(at + 1);
        if (!line) continue;
        const { request, error } = parse(line);
        if (error) {
          send({ error });
          continue;
        }
        if (request.op === "subscribe") {
          stop ||= subscribe(send);
          send({ ok: true });
          continue;
        }
        if (HUB_ONLY.has(request.op) && !trusted(request, token)) {
          send({ error: `${request.op} is the hub's to ask` });
          continue;
        }
        try {
          const answer = await handle(request, { trusted: trusted(request, token), guarded: GUARDED.has(request.op) });
          send(answer === undefined ? { ok: true } : answer);
        } catch (failure) {
          log?.(`${request.op}: ${failure.message}`);
          send({ error: failure.message });
        }
      }
    });
    socket.on("error", () => {});
    socket.on("close", () => stop?.());
  });
  server.listen(socketPath, () => fs.chmodSync(socketPath, 0o600));
  return server;
}

/** Whether a shell already answers on the socket. */
export function alive(socketPath, timeout = 500) {
  return new Promise((resolve) => {
    const socket = net.createConnection(socketPath);
    const done = (answer) => (socket.destroy(), resolve(answer));
    socket.setTimeout(timeout, () => done(false));
    socket.on("error", () => done(false));
    socket.on("connect", () => socket.write('{"op":"ping"}\n'));
    socket.on("data", () => done(true));
  });
}
