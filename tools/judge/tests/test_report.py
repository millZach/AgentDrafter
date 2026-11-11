import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from judgelib import report  # noqa: E402
from dxfmaker import dxf_text, line, text, write_gz  # noqa: E402

RESULTS_HEAD = "id\tstatus\texit\tseconds\tfree_kb_before\tpid\n"


def reading(d: Path, i: str, audit=0, mode="clean", dwgname=None, dxf=None):
    """What one AutoCAD pass leaves for a drawing that opened."""
    (d / (i + ".status")).write_text("DWGNAME\t%s\nMODE\t%s\nDXFOUT_DONE\t1\nAUDIT_DONE\t1\n" % (dwgname or i + ".dwg", mode))
    log = i + (".rec.log" if mode == "recovered" else ".log")
    (d / log).write_text("Command: _.AUDIT\nAuditing Header\n\nTotal errors found %d fixed 0\n" % audit)
    if dxf is not None:
        write_gz(d / (i + ".dxf.gz"), dxf)


def run_files(d: Path, ids, versions, results):
    (d / "ids.txt").write_text("".join(i + "\n" for i in ids))
    (d / "versions.tsv").write_text("".join("%s\t%s\t%s\n" % (i, *versions[i]) for i in ids))
    (d / "results1.tsv").write_text(RESULTS_HEAD + "".join("%s\t%s\t0\t5\t8000000\t1\n" % (i, results.get(i, "OK")) for i in ids)
                                    + "DRIVER DONE\n")


class Built(unittest.TestCase):
    def test_a_drawing_passes_only_when_opened_clean_with_no_audit_errors(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            ids = ["b001", "b002", "b003", "b004", "b005"]
            run_files(d, ids, {i: ("-", "AC1032") for i in ids}, {"b005": "LOWMEM"})
            reading(d, "b001")
            reading(d, "b002", audit=3)
            reading(d, "b003", mode="recovered")
            # b004 never opened: no status; b005 never started: low memory
            doc = report.score_built(d)
            got = {i: (e["opened"], e["audit_errors"], e["pass"]) for i, e in doc["files"].items()}
            self.assertEqual(got, {
                "b001": ("yes", 0, True),
                "b002": ("yes", 3, False),
                "b003": ("recovered", 0, False),
                "b004": ("no", None, False),
                "b005": ("not judged: low memory", None, False),
            })
            self.assertEqual((doc["totals"]["pass"], doc["totals"]["drawings"]), (1, 5))
            md = report.render_built("demo", "now", doc)
            self.assertIn("| b002 | AC1032 | yes | 3 | FAIL |", md)
            named = report.score_built(d, {i: "sheet-%s" % i for i in ids})
            self.assertIn("| sheet-b001 |", report.render_built("demo", "now", named))

    def test_autocad_working_on_some_other_drawing_is_not_an_open(self):
        with tempfile.TemporaryDirectory() as t:
            d = Path(t)
            run_files(d, ["b001"], {"b001": ("-", "AC1032")}, {})
            reading(d, "b001", dwgname="Drawing1.dwg")
            self.assertEqual(report.score_built(d)["files"]["b001"]["opened"], "no")


class RoundTrip(unittest.TestCase):
    def test_drawn_objects_are_counted_per_drawing_and_pooled(self):
        with tempfile.TemporaryDirectory() as t:
            ref, lane = Path(t, "ref"), Path(t, "lane")
            ref.mkdir()
            lane.mkdir()
            sample = ["m001", "m002", "m003", "m004"]
            orig1 = dxf_text(drawn=[line("A1"), line("A2"), text("A3", "TT-101")])
            orig2 = dxf_text(drawn=[line("A1")], in_block=[line("A4", owner="B1")])
            reading(ref, "m001", dxf=orig1)
            reading(ref, "m002", audit=1, dxf=orig2)
            reading(ref, "m004", dxf=orig1)
            # m003: AutoCAD could not read the original, so there is no reference reading
            ids = ["m001", "m002", "m003"]
            run_files(lane, ids, {i: ("AC1032", "AC1032") for i in ids}, {})
            reading(lane, "m001", dxf=dxf_text(drawn=[line("A1"), line("A2", y2="5.0")]))   # 1 changed, 1 lost
            reading(lane, "m002", audit=2, dxf=orig2)                                         # identical, audit worse
            doc = report.score_roundtrip(ref, lane, sample)
            f = doc["files"]
            self.assertEqual({k: f["m001"]["drawn_objects"][k] for k in ("drawn", "identical", "lost", "changed")},
                             {"drawn": 3, "identical": 1, "lost": 1, "changed": 1})
            self.assertEqual(f["m002"]["drawn_objects"]["identical"], 2)
            self.assertEqual(f["m003"]["opened"], "n/a")
            self.assertEqual(f["m004"]["opened"], "not supplied")
            t_ = doc["totals"]
            self.assertEqual((t_["drawings_scored"], t_["drawn_objects"], t_["drawn_identical"]), (2, 5, 3))
            self.assertEqual(t_["drawn_pct_identical_pooled"], 60.0)
            self.assertEqual((t_["gaining_audit_errors"], t_["original_unreadable"], t_["not_supplied"]), (1, 1, 1))
            md = report.render_roundtrip("demo", "now", doc)
            self.assertIn("Pooled over 2 drawings: 5 drawn objects, 3 identical (60.0%), 1 lost, 1 changed", md)
            self.assertIn("| m001 | AC1032 | yes | 0 -> 0 | 3 | 1 | 1 | 1 | 33.33 | TEXT 1 | LINE 1 |", md)
            self.assertNotIn("| m004 |", md)

    def test_a_lane_file_autocad_cannot_open_is_reported_not_scored(self):
        with tempfile.TemporaryDirectory() as t:
            ref, lane = Path(t, "ref"), Path(t, "lane")
            ref.mkdir()
            lane.mkdir()
            reading(ref, "m001", dxf=dxf_text(drawn=[line("A1")]))
            run_files(lane, ["m001"], {"m001": ("AC1032", "AC1032")}, {})
            doc = report.score_roundtrip(ref, lane, ["m001"])
            self.assertEqual(doc["files"]["m001"]["opened"], "no")
            self.assertNotIn("drawn_objects", doc["files"]["m001"])
            self.assertEqual((doc["totals"]["failed_to_open"], doc["totals"]["drawings_scored"]), (1, 0))


if __name__ == "__main__":
    unittest.main()
