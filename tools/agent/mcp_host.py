"""A minimal MCP host for MurmelCraft: runs a list of tool calls through
`murmel mcp` (JSON-RPC 2.0 on stdin and stdout, MCP 2025-11-25) and writes
a journal of every call and its result. It replays the example chats of
docs/chats.md.

    python3 -I tools/agent/mcp_host.py MURMEL WORK CALLS.json [JOURNAL.jsonl]

MURMEL is the `murmel` binary, WORK the one folder the server may read and
write (`--root`), CALLS a JSON list of [tool, arguments] pairs. Relative
paths in the arguments start at WORK, as they do for an agent. The journal
(default: CALLS with .jsonl) has one line per call: the tool, its
arguments, isError and the result envelope, with WORK written as <work>.
A summary of each call goes to standard output.
"""
import json
import subprocess
import sys


def main():
    if len(sys.argv) not in (4, 5):
        sys.exit(__doc__)
    murmel, work, calls = sys.argv[1:4]
    journal = sys.argv[4] if len(sys.argv) == 5 else calls.rsplit(".", 1)[0] + ".jsonl"
    server = subprocess.Popen([murmel, "mcp", "--root", work], stdin=subprocess.PIPE,
                              stdout=subprocess.PIPE, text=True, bufsize=1)
    ids = iter(range(1, 1 << 30))

    def send(msg):
        server.stdin.write(json.dumps(msg) + "\n")
        server.stdin.flush()

    def rpc(method, params):
        n = next(ids)
        send({"jsonrpc": "2.0", "id": n, "method": method, "params": params})
        while True:
            line = server.stdout.readline()
            if not line:
                sys.exit(f"the server ended during {method}")
            msg = json.loads(line)
            if msg.get("id") == n:
                if "error" in msg:
                    sys.exit(f"{method}: {msg['error']}")
                return msg["result"]

    rpc("initialize", {"protocolVersion": "2025-11-25", "capabilities": {},
                       "clientInfo": {"name": "murmel-chat-host", "version": "1"}})
    send({"jsonrpc": "2.0", "method": "notifications/initialized"})
    with open(calls) as f:
        steps = json.load(f)
    with open(journal, "w") as out:
        for k, (tool, args) in enumerate(steps, 1):
            res = rpc("tools/call", {"name": tool, "arguments": args})
            env = res.get("structuredContent", {})
            images = sum(1 for c in res.get("content", []) if c.get("type") == "image")
            row = {"call": k, "tool": tool, "arguments": args, "isError": bool(res.get("isError")),
                   "images": images, "envelope": env}
            out.write(json.dumps(row).replace(work.rstrip("/"), "<work>") + "\n")
            codes = ", ".join(d.get("code", "") for d in env.get("diagnostics") or [])
            state = "refused" if res.get("isError") else "ok"
            print(f"{k:2d} {tool:20s} {state}{' ' + codes if codes else ''}")
    server.stdin.close()
    server.wait()


if __name__ == "__main__":
    main()
