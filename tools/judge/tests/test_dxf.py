import io
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))

from judgelib import dxf  # noqa: E402
from dxfmaker import compound_file, dxf_text, line, ole2frame, text  # noqa: E402


def parse(s: str) -> dxf.Dxf:
    return dxf.parse_stream(io.StringIO(s.replace("\r\n", "\n")))


class TagReading(unittest.TestCase):
    def test_codes_are_ints_values_keep_leading_spaces_comments_skipped_nothing_after_eof(self):
        s = "999\nnote\n  0\nLINE\n  1\n  two spaces\n  0\nEOF\n  0\nAFTER\n"
        self.assertEqual(list(dxf.iter_tags(io.StringIO(s))), [(0, "LINE"), (1, "  two spaces"), (0, "EOF")])

    def test_a_bad_group_code_is_an_error(self):
        with self.assertRaises(dxf.DxfError):
            list(dxf.iter_tags(io.StringIO("zero\nLINE\n")))


class Parsing(unittest.TestCase):
    def test_records_know_their_section_space_handle_and_owner(self):
        d = parse(dxf_text(drawn=[line("A1")], in_block=[line("A2", owner="B1")],
                           objects=[["DICTIONARY", (5, "C"), (330, "0"), (100, "AcDbDictionary")]]))
        by_h = {r.handle: r for r in d.recs}
        self.assertEqual((by_h["A1"].section, by_h["A1"].space, by_h["A1"].owner), ("ENTITIES", "model", "1F"))
        self.assertEqual((by_h["A2"].section, by_h["A2"].space), ("BLOCKS", "block"))
        self.assertEqual(by_h["C"].section, "OBJECTS")
        self.assertEqual(d.header["$ACADVER"], ((1, "AC1032"),))

    def test_paper_space_objects_are_marked(self):
        d = parse(dxf_text(drawn=[line("A1", extra=[(67, "1")])]))
        self.assertEqual(d.recs[-1].space, "paper")


class Normalizing(unittest.TestCase):
    def test_what_autocad_changes_between_two_reads_is_ignored(self):
        def insert(guid):
            return [(5, "D1"), (330, "1F"), (100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockReference"),
                    (2, "DEMO"), (1001, "AcDbDynamicBlockGUID"), (1000, guid)]
        self.assertEqual(dxf.normalize("INSERT", "ENTITIES", insert("{A}")), dxf.normalize("INSERT", "ENTITIES", insert("{B}")))
        # proxy graphics: only their length counts
        a = dxf.normalize("ACAD_TABLE", "ENTITIES", [(5, "E"), (310, "00FF")])
        self.assertEqual(a, dxf.normalize("ACAD_TABLE", "ENTITIES", [(5, "E"), (310, "FF00")]))
        self.assertNotEqual(a, dxf.normalize("ACAD_TABLE", "ENTITIES", [(5, "E"), (310, "FF0000")]))
        # a block record's insert list is a set
        self.assertEqual(dxf.normalize("BLOCK_RECORD", "TABLES", [(5, "F"), (331, "2"), (331, "1")]),
                         dxf.normalize("BLOCK_RECORD", "TABLES", [(5, "F"), (331, "1"), (331, "2")]))

    def test_a_fields_cached_value_is_ignored_but_its_code_is_not(self):
        def field(code, cached):
            return [(5, "F1"), (1, "AcVar"), (2, code), (7, "ACFD_FIELD_VALUE"), (93, "1"), (1, cached),
                    (304, "ACVALUE_END"), (301, cached)]
        self.assertEqual(dxf.normalize("FIELD", "OBJECTS", field("%<\\AcVar Date>%", "1/1/2026")),
                         dxf.normalize("FIELD", "OBJECTS", field("%<\\AcVar Date>%", "2/2/2026")))
        self.assertNotEqual(dxf.normalize("FIELD", "OBJECTS", field("%<\\AcVar Date>%", "x")),
                            dxf.normalize("FIELD", "OBJECTS", field("%<\\AcVar Author>%", "x")))


class DrawnObjects(unittest.TestCase):
    def test_identical_lost_and_changed_are_counted_by_handle_and_content(self):
        orig = parse(dxf_text(drawn=[line("A1"), line("A2"), text("A3", "PUMP 1")], in_block=[line("A4", owner="B1")]))
        lane = parse(dxf_text(drawn=[line("A1"), line("A2", x2="11.0")],
                              in_block=[line("A4", owner="B1", x2="10.000000000001")]))
        r = dxf.drawn_objects(orig, lane)
        # A1 same; A4 same within 1e-9; A2 moved; A3 gone. BLOCK and ENDBLK are not drawn objects.
        self.assertEqual((r["drawn"], r["identical"], r["lost"], r["changed"]), (4, 2, 1, 1))
        self.assertEqual(r["pct_identical"], 50.0)
        self.assertEqual(r["lost_by_type"], {"TEXT": 1})
        self.assertEqual(r["changed_by_type"], {"LINE": 1})

    def test_an_object_saved_under_a_new_handle_is_lost_and_found(self):
        orig = parse(dxf_text(drawn=[line("A1"), text("A3", "PUMP 1")]))
        lane = parse(dxf_text(drawn=[line("A1"), text("A9", "PUMP 1")]))
        r = dxf.drawn_objects(orig, lane)
        self.assertEqual((r["lost"], r["added"], r["lost_found_under_new_handle"]), (1, 1, 1))

    def test_an_ole_objects_data_is_compared_except_the_stamp_autocad_rewrites(self):
        def reading(payload, modified):
            return parse(dxf_text(drawn=[ole2frame("A5", compound_file(payload, modified))]))
        orig = reading(b"picture one", b"\x01" * 8)
        # AutoCAD stamps the root storage's modified time on every write: not a change
        self.assertEqual(dxf.drawn_objects(orig, reading(b"picture one", b"\x02" * 8))["identical"], 1)
        # the same length with other content is a change
        self.assertEqual(dxf.drawn_objects(orig, reading(b"picture two", b"\x01" * 8))["changed_by_type"], {"OLE2FRAME": 1})

    def test_a_reading_against_itself_is_identical(self):
        s = dxf_text(drawn=[line("A1"), text("A3", "PUMP 1")], in_block=[line("A4", owner="B1")])
        r = dxf.drawn_objects(parse(s), parse(s))
        self.assertEqual((r["identical"], r["drawn"]), (3, 3))


class AllRecords(unittest.TestCase):
    def test_header_changes_count_except_the_ones_every_save_makes(self):
        orig = parse(dxf_text(drawn=[line("A1")]))
        lane = parse(dxf_text(drawn=[line("A1")], header={"$ACADVER": [(1, "AC1032")], "$HANDSEED": [(5, "1")]}))
        self.assertEqual(dxf.compare(orig, lane)["header_vars_changed"], [])
        lane = parse(dxf_text(drawn=[line("A1")], header={"$ACADVER": [(1, "AC1027")], "$HANDSEED": [(5, "1")]}))
        self.assertEqual(dxf.compare(orig, lane)["header_vars_changed"], ["$ACADVER"])

    def test_a_lost_dictionary_shows_outside_drawn_objects(self):
        d = ["DICTIONARY", (5, "C"), (330, "0"), (100, "AcDbDictionary")]
        orig = parse(dxf_text(drawn=[line("A1")], objects=[d]))
        lane = parse(dxf_text(drawn=[line("A1")]))
        self.assertEqual(dxf.drawn_objects(orig, lane)["lost"], 0)
        self.assertEqual(dxf.compare(orig, lane)["lost_by_type"], {"DICTIONARY": 1})


if __name__ == "__main__":
    unittest.main()
