#!/usr/bin/env python3
"""A fake MCP server over stdio for cockpit's tests: answers each tool from a JSON file of canned answers.

An answer {"_list": [...]} is that list; {"_range": true} answers a Prometheus-style range query over the asked window.
A "_versions" list makes `initialize` refuse any other protocol version with -32601, as Google's servers do. Every call
is appended to <answers>.calls as {"tool", "args"}.
"""
import datetime
import json
import sys

answers = json.load(open(sys.argv[1]))
versions = answers.pop("_versions", None)
log = open(sys.argv[1] + ".calls", "a")


def send(message):
    sys.stdout.write(json.dumps(message) + "\n")
    sys.stdout.flush()


def range_answer(args):
    parse = lambda s: datetime.datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()
    start, end = parse(args["startTime"]), parse(args["endTime"])
    step = max(int(args.get("stepSeconds") or 60), 60)
    expr = args.get("expr", "")
    value = "0.2" if "quantile" in expr else ("1" if "error" in expr else "50")
    points, t = [], start
    while t < end:
        points.append([int(t), value])
        t += step
    return {"data": {"result": [{"metric": {"__name__": "series"}, "values": points}]}}


for line in sys.stdin:
    if not line.strip():
        continue
    message = json.loads(line)
    if "id" not in message:
        continue
    method, ident = message.get("method"), message["id"]
    if method == "initialize":
        version = message["params"].get("protocolVersion", "2025-06-18")
        if versions is not None and version not in versions:
            send({"jsonrpc": "2.0", "id": ident, "error": {"code": -32601, "message": "Method not supported"}})
            continue
        send({"jsonrpc": "2.0", "id": ident, "result": {"protocolVersion": version, "capabilities": {"tools": {}}, "serverInfo": {"name": "fake", "version": "1"}}})
    elif method == "tools/list":
        tools = [{"name": name, "description": "fake " + name, "inputSchema": {"type": "object"}} for name in answers]
        send({"jsonrpc": "2.0", "id": ident, "result": {"tools": tools}})
    elif method == "tools/call":
        name, args = message["params"]["name"], message["params"].get("arguments", {})
        log.write(json.dumps({"tool": name, "args": args}) + "\n")
        log.flush()
        answer = answers.get(name)
        if isinstance(answer, dict) and answer.get("_range"):
            answer = range_answer(args)
        elif isinstance(answer, dict) and "_list" in answer:
            answer = answer["_list"]
        if answer is None:
            send({"jsonrpc": "2.0", "id": ident, "result": {"content": [{"type": "text", "text": "no such tool"}], "isError": True}})
        else:
            text = answer if isinstance(answer, str) else json.dumps(answer)
            send({"jsonrpc": "2.0", "id": ident, "result": {"content": [{"type": "text", "text": text}], "isError": False}})
    else:
        send({"jsonrpc": "2.0", "id": ident, "result": {}})
