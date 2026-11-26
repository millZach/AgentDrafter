"""A made-up agent for the assistant's tests (#156): it stands in for Claude
Code (`claude`) or Codex (`codex`) on the PATH of `agentdrafter serve`, so no
test needs a real agent.

It is run as `fake_agent.py <claude|codex> <the agent's own arguments>` by a
small shim the tests write under the agent's name. It reads its prompt from
standard input, as both agents do when `serve` runs them, starts the MCP
server its arguments name, as the real agent would, and prints its work in
that agent's own JSON lines, so the adapter that reads them is tested too.

It knows a few things the drafter may say:
- `move this N to the right` (or left, up, down): `looking-at`, `query` for
  the object selected, then `propose` a move of it by N;
- `what is this`: `looking-at` and `query`, and says what it is;
- `change it directly`: tries the `change` tool, which the assistant's
  server withholds, and says it could not;
- `what do I see`: `looking-at` alone, and says the sheet and selection it
  gave;
- `guide me`: `looking-at`, then `guide` with a 2-step guide on the sheet
  shown;
- `look at <folder>`: `looking-at` on another folder, which the assistant's
  server, bound to the conversation's set, refuses;
- anything else: says what it heard.

FAKE_AGENT_DELAY, in seconds, waits before each step. FAKE_AGENT_LOG names a
file each run appends its arguments and prompt to, as one JSON line.
FAKE_AGENT_HANG=1 makes it start a process of its own that holds its output
open and then never end, as a hung agent would. FAKE_AGENT_STRAY=1 makes it
answer and end, leaving a process of its own running that holds its output
and its standard error open.
"""

import json
import os
import re
import subprocess
import sys
import time
import uuid


def main():
    kind = sys.argv[1]
    args = sys.argv[2:]
    if kind == "codex" and handle_codex_probe(args):
        return
    prompt = sys.stdin.read()
    log = os.environ.get("FAKE_AGENT_LOG")
    if log:
        with open(log, "a", encoding="utf-8") as f:
            f.write(json.dumps({"agent": kind, "args": args, "prompt": prompt}) + "\n")
    delay = float(os.environ.get("FAKE_AGENT_DELAY", "0"))
    if os.environ.get("FAKE_AGENT_STRAY") == "1":
        out = Claude(args) if kind == "claude" else Codex(args)
        out.begin()
        subprocess.Popen([sys.executable, "-c", "import time; time.sleep(60)"],
                         stdin=subprocess.DEVNULL, stdout=sys.stdout, stderr=sys.stderr)
        out.words("(fake agent) I left a helper running.")
        out.end()
        return
    if os.environ.get("FAKE_AGENT_HANG") == "1":
        print(json.dumps({"type": "system", "subtype": "init", "session_id": "hung"}), flush=True)
        helper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(600)"])
        pid_file = os.environ.get("FAKE_AGENT_PID_FILE")
        if pid_file:
            tmp = f"{pid_file}.tmp"
            with open(tmp, "w", encoding="utf-8") as f:
                f.write(f"{os.getpid()}\n{helper.pid}\n")
            os.replace(tmp, pid_file)
        time.sleep(600)
        return
    server = Server(server_command(kind, args))
    out = Claude(args) if kind == "claude" else Codex(args)
    out.begin()
    try:
        act(prompt, server, out, delay)
    finally:
        server.close()
    out.end()


def handle_codex_probe(args):
    """Answers the Codex preflight probes (#229) without reading stdin or
    logging a turn."""
    old = os.environ.get("FAKE_CODEX_OLD")
    if args == ["--version"]:
        ver = "0.120.0" if old == "1" else (old or "0.172.0")
        print(f"codex-cli {ver}", flush=True)
        return True
    if args == ["exec", "--help"]:
        print("Usage: codex exec [OPTIONS] [PROMPT]", flush=True)
        if not old:
            print("      --ignore-user-config  Do not load config.toml", flush=True)
        return True
    if len(args) >= 2 and args[0] == "login" and args[1] == "status":
        auth = os.environ.get("FAKE_CODEX_AUTH", "file")
        forces_file = any(
            "cli_auth_credentials_store" in a and "file" in a for a in args[2:]
        )
        if auth == "none":
            print("Not logged in", flush=True)
            sys.exit(1)
        if auth == "keyring" and forces_file:
            print("Not logged in", flush=True)
            sys.exit(1)
        print("Logged in", flush=True)
        return True
    return False


def server_command(kind, args):
    """The MCP server the agent was given: from Claude Code's --mcp-config
    file, or Codex's -c settings."""
    if kind == "claude":
        path = args[args.index("--mcp-config") + 1]
        with open(path, encoding="utf-8") as f:
            config = json.load(f)["mcpServers"]["agentdrafter"]
        return [config["command"], *config["args"]]
    command, listed = None, None
    for i, arg in enumerate(args):
        if arg == "-c":
            key, _, value = args[i + 1].partition("=")
            if key == "mcp_servers.agentdrafter.command":
                command = toml_text(value)
            if key == "mcp_servers.agentdrafter.args":
                listed = [toml_text(v) for v in split_list(value)]
    return [command, *listed]


def toml_text(value):
    value = value.strip()
    if value.startswith("'"):
        return value[1:-1]
    return json.loads(value)


def split_list(value):
    inner = value.strip()[1:-1]
    return [part for part in re.findall(r"'[^']*'|\"(?:[^\"\\\\]|\\\\.)*\"", inner)]


class Server:
    """The MCP server, spoken to over its standard input and output."""

    def __init__(self, command):
        self.process = subprocess.Popen(
            command,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
            encoding="utf-8",
        )
        self.next_id = 0
        self.ask("initialize", {"protocolVersion": "2025-06-18"})
        self.tools = [t["name"] for t in self.ask("tools/list", {})["result"]["tools"]]

    def ask(self, method, params):
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def call(self, name, arguments):
        """The tool's reply, as text, and whether it was refused."""
        answer = self.ask("tools/call", {"name": name, "arguments": arguments})
        if "error" in answer:
            return answer["error"]["message"], True
        result = answer["result"]
        return result["content"][0]["text"], result.get("isError", False)

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=30)


class Claude:
    """Claude Code's stream-json lines."""

    def __init__(self, args):
        self.session = args[args.index("--resume") + 1] if "--resume" in args else str(uuid.uuid4())

    def say(self, message):
        message["session_id"] = self.session
        print(json.dumps(message), flush=True)

    def begin(self):
        self.say({"type": "system", "subtype": "init", "tools": []})

    def words(self, text):
        self.say({"type": "assistant", "message": {"content": [{"type": "text", "text": text}]}})

    def started(self, id, tool, arguments):
        self.say({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "id": id, "name": f"mcp__agentdrafter__{tool}", "input": arguments}]}})

    def done(self, id, text, refused):
        self.say({"type": "user", "message": {"content": [
            {"type": "tool_result", "tool_use_id": id, "content": [{"type": "text", "text": text}],
             "is_error": refused}]}})

    def end(self):
        self.say({"type": "result", "subtype": "success", "is_error": False, "result": ""})


class Codex:
    """Codex's exec --json lines."""

    def __init__(self, args):
        self.thread = args[args.index("resume") + 1] if "resume" in args else str(uuid.uuid4())
        self.pending = {}

    def say(self, event):
        print(json.dumps(event), flush=True)

    def begin(self):
        self.say({"type": "thread.started", "thread_id": self.thread})
        self.say({"type": "turn.started"})

    def words(self, text):
        self.say({"type": "item.completed", "item": {"id": str(uuid.uuid4()), "type": "agent_message", "text": text}})

    def started(self, id, tool, arguments):
        self.pending[id] = (tool, arguments)
        self.say({"type": "item.started", "item": {"id": id, "type": "mcp_tool_call", "server": "agentdrafter",
                                                    "tool": tool, "arguments": arguments, "status": "in_progress"}})

    def done(self, id, text, refused):
        tool, arguments = self.pending.pop(id)
        item = {"id": id, "type": "mcp_tool_call", "server": "agentdrafter", "tool": tool,
                "arguments": arguments, "status": "failed" if refused else "completed"}
        if refused and not text.startswith("{"):
            item["error"] = {"message": text}
        else:
            item["result"] = {"content": [{"type": "text", "text": text}]}
        self.say({"type": "item.completed", "item": item})

    def end(self):
        self.say({"type": "turn.completed", "usage": {}})


def act(prompt, server, out, delay):
    said = prompt.split("The drafter says:\n", 1)[-1].strip()
    set_match = re.search(r"`([^`]+)`", prompt.split("The drafter says:\n", 1)[0] + "``")
    folder = set_match.group(1) if set_match and set_match.group(1) else "."
    for line in prompt.splitlines():
        found = re.search(r"of `([^`]+)`", line)
        if line.startswith("[The drafter is looking at") and found:
            folder = found.group(1)
    steps = [0]

    def call(tool, arguments):
        time.sleep(delay)
        steps[0] += 1
        id = f"call-{steps[0]}"
        out.started(id, tool, arguments)
        text, refused = server.call(tool, arguments)
        out.done(id, text, refused)
        try:
            return json.loads(text), refused
        except ValueError:
            return None, True

    moved = re.search(r"move (?:this|it) (-?\d*\.?\d+) (?:in |mm )?(?:to the )?(right|left|up|down)", said, re.I)
    if moved or re.search(r"what is this", said, re.I):
        looking, _ = call("looking-at", {"set": folder})
        selected = (looking or {}).get("result", {}).get("selected", [])
        if not selected:
            time.sleep(delay)
            out.words("Nothing is selected, so I don't know what you mean. Select it and ask again.")
            return
        id = selected[0]
        found, _ = call("query", {"source": folder, "query": {"kind": "find", "id": id}})
        objects = (found or {}).get("result", {}).get("answer", {}).get("objects", [])
        if moved:
            n = float(moved.group(1))
            dx, dy = {"right": (n, 0), "left": (-n, 0), "up": (0, n), "down": (0, -n)}[moved.group(2).lower()]
            reply, refused = call("propose", {"set": folder, "change_set": {"operations": [
                {"op": "move", "target": {"id": id}, "from": [0, 0, 0], "to": [dx, dy, 0]}]}})
            time.sleep(delay)
            if refused or not reply or not reply.get("ok"):
                message = ((reply or {}).get("error") or {}).get("message", "it was refused")
                out.words(f"I couldn't propose it: {message}")
                return
            number = reply["result"]["proposal"]
            out.words(f"Proposed: move {id} {moved.group(1)} {moved.group(2).lower()}. It's dashed on "
                      f"the drawing as proposal {number}; nothing changes until you keep it.")
        else:
            time.sleep(delay)
            kind = objects[0].get("type", "object") if objects else "object"
            layer = objects[0].get("layer", "?") if objects else "?"
            out.words(f"{id} is {'an' if kind[:1] in 'aeiou' else 'a'} {kind} on layer {layer}.")
        return
    if re.search(r"\bguide me\b", said, re.I):
        looking, _ = call("looking-at", {"set": folder})
        sheet = (looking or {}).get("result", {}).get("sheet") or "DP-LD-101"
        title = "Draw a note line"
        guide = {
            "title": title,
            "sheet": sheet,
            "steps": [
                {
                    "say": "Make notes the current layer.",
                    "title": "Make notes current",
                    "setting": {"current_layer": "notes"},
                },
                {
                    "say": "Draw a short line on notes.",
                    "title": "Draw a note line",
                    "tool": "line",
                    "points": [
                        {"at": [1, 3], "label": "start"},
                        {"at": [2, 3], "label": "end"},
                    ],
                    "completes": {
                        "op": "add_line",
                        "sheet": sheet,
                        "layer": "notes",
                        "from": [1, 3, 0],
                        "to": [2, 3, 0],
                    },
                },
            ],
        }
        reply, refused = call("guide", {"set": folder, "guide": guide, "by": "Assistant"})
        time.sleep(delay)
        if refused or not reply or not reply.get("ok"):
            message = ((reply or {}).get("error") or {}).get("message", "it was refused")
            out.words(f"I couldn't give a guide: {message}")
            return
        number = reply["result"]["guide"]
        out.words(f"I gave you guide {number}: {title}. Start it from the card below when you're ready.")
        return
    if re.search(r"what do I see", said, re.I):
        looking, _ = call("looking-at", {"set": folder})
        result = (looking or {}).get("result", {})
        time.sleep(delay)
        out.words(f"You were looking at {result.get('sheet')} with {result.get('selected')}: "
                  f"{result.get('message')}")
        return
    other = re.search(r"look at (\S+)", said, re.I)
    if other:
        reply, refused = call("looking-at", {"set": other.group(1)})
        time.sleep(delay)
        out.words(f"Refused: {str(refused).lower()}")
        return
    if re.search(r"change it directly", said, re.I):
        text, refused = server.call("change", {"set": folder, "change_set": {"operations": []}})
        call_id = "call-change"
        out.started(call_id, "change", {"set": folder})
        out.done(call_id, text, True)
        listed = "change" in server.tools
        out.words("I can't change the set myself: `change` is not one of my tools, so I propose "
                  f"changes for you to keep. (change listed: {str(listed).lower()})")
        return
    time.sleep(delay)
    out.words(f"(fake agent) You said: {said}")


if __name__ == "__main__":
    main()
