"""The judge command with AutoCAD's pass replaced by a stand-in, on made-up drawings."""

import contextlib
import io
import json
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

import judge  # noqa: E402
from judgelib import autocad, report  # noqa: E402
from dxfmaker import dxf_text, line, write_gz  # noqa: E402


class StandIn:
    """Takes the place of autocad.read_drawings: AutoCAD reads every drawing it is given, except the
    ones in `busy`, which the driver skips as it does when another program holds AutoCAD."""

    def __init__(self, busy=()):
        self.busy = set(busy)
        self.asked = []

    def __call__(self, remote, files, out, name, log=print):
        self.asked.append(sorted(files))
        out.mkdir(parents=True, exist_ok=True)
        rows = []
        for i in sorted(files):
            if i in self.busy:
                rows.append("%s\tBUSY\t\t0\t8000000\t" % i)
                continue
            (out / (i + ".status")).write_text("DWGNAME\t%s.dwg\nMODE\tclean\nDXFOUT_DONE\t1\nAUDIT_DONE\t1\n" % i)
            (out / (i + ".log")).write_text("Total errors found 0 fixed 0\n")
            write_gz(out / (i + ".dxf.gz"), dxf_text(drawn=[line("A1")]))
            rows.append("%s\tOK\t0\t5\t8000000\t1" % i)
        (out / "results1.tsv").write_text("id\tstatus\texit\tseconds\tfree_kb_before\tpid\n"
                                          + "".join(r + "\n" for r in rows) + "DRIVER DONE\n")


class Reference(unittest.TestCase):
    def test_a_skipped_original_is_read_on_the_next_run_and_only_then_is_the_reference_complete(self):
        with tempfile.TemporaryDirectory() as t, contextlib.redirect_stdout(io.StringIO()):
            jd = Path(t)
            ref = jd / "ref"
            ids = ["m001", "m002", "m003"]
            acad = StandIn(busy={"m002"})
            with mock.patch.object(autocad, "read_drawings", acad):
                judge.ensure_reference(None, jd, ids)
                self.assertFalse((ref / "COMPLETE").exists())
                self.assertEqual(sorted(p.name for p in ref.glob("*.dxf.gz")), ["m001.dxf.gz", "m003.dxf.gz"])
                kept = (ref / "m001.dxf.gz").read_bytes()

                # meanwhile a round trip of m002 is not judged, not scored as an unreadable original
                lane = jd / "lane"
                lane.mkdir()
                (lane / "ids.txt").write_text("m002\n")
                doc = report.score_roundtrip(ref, lane, ids)
                self.assertEqual(doc["files"]["m002"]["opened"], "not judged: AutoCAD busy")
                self.assertEqual((doc["totals"]["not_judged"], doc["totals"]["original_unreadable"]), (1, 0))

                acad.busy.clear()
                judge.ensure_reference(None, jd, ids)
                self.assertEqual(acad.asked, [ids, ["m002"]])
                self.assertTrue((ref / "COMPLETE").exists())
                self.assertFalse((ref / "SKIPPED").exists())
                self.assertTrue((ref / "m002.dxf.gz").exists())
                self.assertEqual((ref / "m001.dxf.gz").read_bytes(), kept)

                judge.ensure_reference(None, jd, ids)
                self.assertEqual(len(acad.asked), 2)


class Names(unittest.TestCase):
    def test_both_reports_hold_neutral_ids_and_names_only_with_show_names(self):
        with tempfile.TemporaryDirectory() as t, contextlib.redirect_stdout(io.StringIO()):
            jd, folder = Path(t, "judge"), Path(t, "built")
            jd.mkdir()
            folder.mkdir()
            Path(folder, "demo-sheet-one.dwg").write_bytes(b"AC1032" + bytes(64))
            cfg = types.SimpleNamespace(dir=jd, host="cad-box", scratch="C:\\Temp\\judge", accoreconsole="accore.exe",
                                        require=lambda *names: None)
            with mock.patch.object(autocad, "read_drawings", StandIn()), \
                    mock.patch.object(judge, "judge_lock", lambda d: contextlib.nullcontext()):
                for show in (False, True):
                    args = types.SimpleNamespace(folder=str(folder), name="b", show_names=show, overwrite=True)
                    self.assertEqual(judge.cmd_built(args, cfg), 0)
                    for suffix in (".json", ".md"):
                        shown = "demo-sheet-one" in (jd / "results" / ("b" + suffix)).read_text()
                        self.assertEqual(shown, show, "%s with show_names=%s" % (suffix, show))
            # the map stays apart, in the run's readings folder
            self.assertNotIn("demo-sheet-one", (jd / "work" / "b" / "run.json").read_text())
            self.assertEqual(report.read_names(jd / "work" / "b"), {"b001": "demo-sheet-one"})
            self.assertIn("b001", json.loads((jd / "results" / "b.json").read_text())["files"])


if __name__ == "__main__":
    unittest.main()
