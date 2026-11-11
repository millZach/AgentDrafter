"""The Python library over the same core as the command line, the HTTP API
and the MCP server: every command of the registry is a function of the
library and gives the reply the other three give, from the calls their own
test makes (crates/agentdrafter-cli/tests/calls.json); the same change set
gives the same reply and journal entry through all four, apart from its
time.

It tests the installed package: the library imported from it and its
`agentdrafter` command, which runs the command line and both servers. Build
the wheel, install it into a virtual environment, and from the checkout run
`python -m unittest discover -s python/tests` with that environment's
Python (docs/python.md).
"""

import contextlib
import http.client
import json
import os
import shutil
import subprocess
import sysconfig
import tempfile
import unittest
from pathlib import Path

import agentdrafter
from agentdrafter import _native

ROOT = Path(__file__).resolve().parents[2]
DEMO_SET = ROOT / "examples" / "demo-set"
SHARED = json.loads(
    (ROOT / "crates" / "agentdrafter-cli" / "tests" / "calls.json").read_text(encoding="utf-8")
)
JOURNAL_FILE = Path(".agentdrafter", "journal.jsonl")
UNDO_STATES_FOLDER = Path(".agentdrafter", "undo-states")


def installed_command():
    """The `agentdrafter` command the package installed beside this Python."""
    name = "agentdrafter.exe" if os.name == "nt" else "agentdrafter"
    path = Path(sysconfig.get_path("scripts")) / name
    assert path.is_file(), f"the package installs the command {name} in {path.parent}"
    return str(path)


COMMAND = installed_command()


def copy_demo_set(to):
    """Every YAML file of examples/demo-set, in its folders, as the Rust
    tests copy it; DWGs a local build left in the example are not."""
    for path in sorted(DEMO_SET.rglob("*.yaml")):
        target = to / path.relative_to(DEMO_SET)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(path, target)


def interface_folder(scratch, interface):
    """A folder for one interface, holding a copy of the demo set as
    `demo-set` and the change sets as files, so every interface is given the
    same relative paths."""
    folder = Path(scratch) / interface
    copy_demo_set(folder / "demo-set")
    for file, change_set in SHARED["change_sets"].items():
        (folder / file).write_text(json.dumps(change_set), encoding="utf-8")
    return folder


def snapshot(set_folder):
    """Every file and folder of the set, each file with its bytes, but the
    journal's own, which is compared entry by entry."""
    found = {}
    for path in set_folder.rglob("*"):
        relative = path.relative_to(set_folder)
        if relative == JOURNAL_FILE or relative.is_relative_to(UNDO_STATES_FOLDER):
            continue
        if path.is_dir():
            if relative != JOURNAL_FILE.parent:
                found[relative] = None
        else:
            found[relative] = path.read_bytes()
    return found


def journal(set_folder):
    lines = (set_folder / JOURNAL_FILE).read_text(encoding="utf-8").splitlines()
    return [json.loads(line) for line in lines]


def without_times(value):
    """`value` with every journal time taken out, the one thing two runs of
    the same change set may differ in."""
    if isinstance(value, dict):
        return {key: without_times(item) for key, item in value.items() if key != "at"}
    if isinstance(value, list):
        return [without_times(item) for item in value]
    return value


def function_of(name):
    """The library's function for the command `name`."""
    return getattr(agentdrafter, name.replace("-", "_"))


def library(name, input, folder):
    """Runs a command through the library, in `folder`, as a program whose
    current folder it is."""
    with contextlib.chdir(folder):
        return function_of(name)(input)


def cli(args, folder):
    """Runs the installed command in `folder`; its exit code and reply."""
    done = subprocess.run([COMMAND, *args], cwd=folder, capture_output=True, timeout=200)
    assert done.stderr == b"", f"nothing on standard error: {done.stderr!r}"
    lines = done.stdout.splitlines()
    assert len(lines) == 1, f"one line of JSON: {done.stdout!r}"
    return done.returncode, json.loads(lines[0])


class Http:
    """`agentdrafter serve`, started in a folder on any free local port."""

    def __init__(self, folder):
        self.process = subprocess.Popen(
            [COMMAND, "serve", "--listen", "127.0.0.1:0"],
            cwd=folder,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        started = json.loads(self.process.stdout.readline())
        assert started["ok"], started
        self.host, port = started["result"]["address"].rsplit(":", 1)
        self.port = int(port)

    def send(self, method, path, body=None):
        connection = http.client.HTTPConnection(self.host, self.port, timeout=200)
        try:
            data = None if body is None else json.dumps(body).encode("utf-8")
            connection.request(method, path, body=data, headers={"Content-Type": "application/json"})
            response = connection.getresponse()
            assert response.getheader("Content-Type").startswith("application/json")
            return response.status, json.loads(response.read())
        finally:
            connection.close()

    def run(self, name, input):
        return self.send("POST", f"/commands/{name}", input)

    def stop(self):
        self.process.kill()
        self.process.wait()
        self.process.stdout.close()


class Mcp:
    """`agentdrafter mcp`, started in a folder, after the handshake."""

    def __init__(self, folder):
        self.process = subprocess.Popen(
            [COMMAND, "mcp"],
            cwd=folder,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
        )
        self.next_id = 0
        self.request(
            "initialize",
            {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "python-interfaces-test", "version": "0"},
            },
        )
        self.write({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def write(self, message):
        self.process.stdin.write(json.dumps(message).encode("utf-8") + b"\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.next_id += 1
        self.write({"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params})
        answer = json.loads(self.process.stdout.readline())
        assert answer["id"] == self.next_id, answer
        return answer

    def call(self, name, arguments):
        """A tool's reply, as its text carries it: the line the command line
        prints. Its structured content is the same reply read back by the
        server, whose numbers may differ in their last digit, so the Rust
        test (tests/servers.rs) compares it."""
        result = self.request("tools/call", {"name": name, "arguments": arguments})["result"]
        reply = json.loads(result["content"][0]["text"])
        assert result["structuredContent"]["ok"] == reply["ok"]
        assert result["isError"] == (not reply["ok"])
        return reply

    def finish(self):
        """Closes the server's input, which stops it; its exit code."""
        self.process.stdin.close()
        code = self.process.wait(timeout=60)
        self.process.stdout.close()
        return code

    def stop(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()
        for stream in (self.process.stdin, self.process.stdout):
            if not stream.closed:
                stream.close()


class Interfaces(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="agentdrafter-interfaces-")
        self.addCleanup(scratch.cleanup)
        self.folders = {
            interface: interface_folder(scratch.name, interface)
            for interface in ("library", "cli", "http", "mcp")
        }

    def start_servers(self):
        http_server = Http(self.folders["http"])
        self.addCleanup(http_server.stop)
        mcp = Mcp(self.folders["mcp"])
        self.addCleanup(mcp.stop)
        return http_server, mcp

    def test_every_command_of_the_registry_is_a_function_with_the_same_reply_as_every_interface(self):
        registry = json.loads(_native.commands())
        http_server, mcp = self.start_servers()

        # The library has one function per command of the registry, and
        # nothing else public; each says what its command does.
        self.assertEqual(agentdrafter.__all__, [c["name"].replace("-", "_") for c in registry])
        public = sorted(name for name in dir(agentdrafter) if not name.startswith("_"))
        self.assertEqual(public, sorted(agentdrafter.__all__))
        for command in registry:
            doc = " ".join(function_of(command["name"]).__doc__.split())
            self.assertTrue(doc.startswith(" ".join(command["does"].split())), command["name"])

        # Each server lists the same commands, with the same schemas.
        status, listed = http_server.send("GET", "/commands")
        self.assertEqual(status, 200, listed)
        self.assertEqual(listed["result"]["commands"], registry)
        tools = mcp.request("tools/list", {})["result"]["tools"]
        self.assertEqual(
            [(t["name"], t["description"], t["inputSchema"]) for t in tools],
            [(c["name"], c["does"], c["input_schema"]) for c in registry],
        )

        # Each command, run through each interface, gives the same reply.
        calls = SHARED["calls"]
        for command in registry:
            name = command["name"]
            self.assertIn(name, calls, f"`{name}` is in the registry: add a call of it to calls.json")
            args, input = calls[name]["args"], calls[name]["input"]
            by_library = library(name, input, self.folders["library"])
            code, by_cli = cli(args, self.folders["cli"])
            status, by_http = http_server.run(name, input)
            by_mcp = mcp.call(name, input)
            self.assertTrue(by_library["ok"], f"{name}: {by_library}")
            self.assertEqual((code, status), (0, 200), name)
            for interface, reply in (("command line", by_cli), ("HTTP API", by_http), ("MCP server", by_mcp)):
                self.assertEqual(
                    without_times(reply),
                    without_times(by_library),
                    f"`{name}` through the library and the {interface}",
                )
        self.assertEqual(mcp.finish(), 0, "the MCP server stops when its input closes")

        # Every file of the set is the same after the same commands.
        by_library = snapshot(self.folders["library"] / "demo-set")
        for interface in ("cli", "http", "mcp"):
            self.assertEqual(snapshot(self.folders[interface] / "demo-set"), by_library, interface)

    def test_the_same_change_set_gives_the_same_reply_and_journal_entry_through_every_interface(self):
        for folder in self.folders.values():
            code, reply = cli(["build", "demo-set"], folder)
            self.assertEqual(code, 0, reply)
        http_server, mcp = self.start_servers()

        # A template edit, which lands on every sheet, then a change set that
        # lands on one sheet and fails on another.
        for file, ok in (("move.json", True), ("partial.json", False)):
            input = {"set": "demo-set", "change_set": SHARED["change_sets"][file]}
            with contextlib.chdir(self.folders["library"]):
                by_library = agentdrafter.change(**input)
            code, by_cli = cli(["change", "demo-set", file], self.folders["cli"])
            status, by_http = http_server.run("change", input)
            by_mcp = mcp.call("change", input)
            self.assertEqual(by_library["ok"], ok, by_library)
            self.assertEqual((code, status), (0, 200) if ok else (1, 422))
            self.assertEqual(by_cli, by_library, "the command line's reply is the library's")
            self.assertEqual(by_http, by_library, "the HTTP API's reply is the library's")
            self.assertEqual(by_mcp, by_library, "the MCP server's reply is the library's")
        self.assertEqual(mcp.finish(), 0)
        http_server.stop()

        by_library = journal(self.folders["library"] / "demo-set")
        self.assertEqual(len(by_library), 2)
        for entry in by_library:
            self.assertTrue(entry["at"].endswith("Z"), entry)
        for interface in ("cli", "http", "mcp"):
            entries = journal(self.folders[interface] / "demo-set")
            self.assertEqual(
                [without_times(e) for e in entries],
                [without_times(e) for e in by_library],
                f"the same journal entries through the {interface}, apart from their times",
            )


if __name__ == "__main__":
    unittest.main()
