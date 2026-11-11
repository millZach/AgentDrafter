"""What the Python library adds to the core, which is only the form of a
call: an input as keywords or as one dict, paths as `pathlib.Path`, and a
value no JSON can hold refused before it reaches the core. A command's own
error is its reply, as through every interface; and the installed command
runs, as `agentdrafter` and as `python -m agentdrafter`.

Run against the installed package, as test_interfaces.py is.
"""

import contextlib
import json
import math
import subprocess
import sys
import tempfile
import unittest
from importlib.metadata import version
from pathlib import Path

import agentdrafter
from test_interfaces import COMMAND, copy_demo_set


class Library(unittest.TestCase):
    def setUp(self):
        scratch = tempfile.TemporaryDirectory(prefix="agentdrafter-library-")
        self.addCleanup(scratch.cleanup)
        self.folder = Path(scratch.name)
        copy_demo_set(self.folder / "demo-set")

    def test_an_input_is_keywords_or_one_dict(self):
        by_keywords = agentdrafter.schema(kind="set")
        self.assertTrue(by_keywords["ok"], by_keywords)
        self.assertEqual(by_keywords["result"]["title"], "AgentDrafter drawing set settings")
        self.assertEqual(agentdrafter.schema({"kind": "set"}), by_keywords)
        self.assertEqual(agentdrafter.operations(), agentdrafter.operations({}))
        with self.assertRaises(TypeError):
            agentdrafter.schema({"kind": "set"}, kind="set")

    def test_a_path_is_taken_as_its_text(self):
        query = {"kind": "find", "name": "title-block"}
        with contextlib.chdir(self.folder):
            by_text = agentdrafter.query(source="demo-set", query=query)
            by_path = agentdrafter.query(source=Path("demo-set"), query=query)
        self.assertTrue(by_text["ok"], by_text)
        self.assertEqual(by_path, by_text)

    def test_a_value_no_json_can_hold_is_refused_before_the_core(self):
        with self.assertRaises(TypeError):
            agentdrafter.query(source=object(), query={"kind": "blocks"})
        with self.assertRaises(ValueError):
            agentdrafter.measure_text(contents="PUMP 101", font="romans.shx", height=math.nan)

    def test_a_command_s_error_is_its_reply(self):
        # A key the schema does not name is refused by the core, as through
        # every interface.
        reply = agentdrafter.journal(set="demo-set", folder="x")
        self.assertFalse(reply["ok"])
        self.assertEqual(reply["error"]["code"], "usage")
        # A list is not an input; the core says so.
        reply = agentdrafter.journal(["demo-set"])
        self.assertEqual(reply["error"]["code"], "usage")
        with contextlib.chdir(self.folder):
            self.assertTrue(agentdrafter.build(source="demo-set")["ok"])
            again = agentdrafter.build(source="demo-set")
        self.assertFalse(again["ok"], "the DWGs are there and no overwrite is asked")
        self.assertTrue(again["result"]["sheets"], again)

    def test_the_installed_command_runs(self):
        for command in ([COMMAND], [sys.executable, "-m", "agentdrafter"]):
            done = subprocess.run([*command, "--version"], capture_output=True, timeout=60)
            self.assertEqual(done.returncode, 0, done)
            self.assertEqual(json.loads(done.stdout)["result"]["version"], version("agentdrafter"))
            done = subprocess.run([*command, "nope"], capture_output=True, timeout=60)
            self.assertEqual(done.returncode, 2, "wrong arguments exit 2, as the binary's do")
            self.assertEqual(json.loads(done.stdout)["error"]["code"], "usage")
        # Both servers are the command's: the help lists them.
        done = subprocess.run([COMMAND, "help"], capture_output=True, timeout=60)
        listed = json.loads(done.stdout)["result"]["commands"]
        self.assertEqual(listed["serve"]["usage"], "agentdrafter serve [--listen <address:port>]")
        self.assertEqual(listed["mcp"]["usage"], "agentdrafter mcp")


if __name__ == "__main__":
    unittest.main()
