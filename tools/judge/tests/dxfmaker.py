"""Made-up DXF text for the judge's tests: a header, entities, one block and an object or two."""

from __future__ import annotations

import gzip
from pathlib import Path


def line(handle, owner="1F", x1="0.0", y1="0.0", x2="10.0", y2="0.0", layer="0", extra=()):
    return ["LINE", (5, handle), (330, owner), (100, "AcDbEntity"), (8, layer), *extra, (100, "AcDbLine"),
            (10, x1), (20, y1), (30, "0.0"), (11, x2), (21, y2), (31, "0.0")]


def text(handle, value, owner="1F"):
    return ["TEXT", (5, handle), (330, owner), (100, "AcDbEntity"), (8, "0"), (100, "AcDbText"),
            (10, "1.0"), (20, "2.0"), (30, "0.0"), (40, "2.5"), (1, value)]


def dxf_text(entities=(), block_entities=(), objects=(), header=None) -> str:
    """A small DXF: HEADER, BLOCKS (one block 'DEMO' holding block_entities), ENTITIES, OBJECTS."""
    header = header or {"$ACADVER": [(1, "AC1032")], "$HANDSEED": [(5, "FFFF")]}
    tags = [(0, "SECTION"), (2, "HEADER")]
    for var, vals in header.items():
        tags.append((9, var))
        tags.extend(vals)
    tags += [(0, "ENDSEC"), (0, "SECTION"), (2, "BLOCKS"),
             (0, "BLOCK"), (5, "B0"), (330, "B1"), (100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockBegin"), (2, "DEMO")]
    for rec in block_entities:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDBLK"), (5, "B2"), (330, "B1"), (100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockEnd"),
             (0, "ENDSEC"), (0, "SECTION"), (2, "ENTITIES")]
    for rec in entities:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDSEC"), (0, "SECTION"), (2, "OBJECTS")]
    for rec in objects:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDSEC"), (0, "EOF")]
    # AutoCAD right-aligns group codes in three columns and ends lines with CRLF
    return "".join("%3d\r\n%s\r\n" % (c, v) for c, v in tags)


def write_gz(path: Path, text_: str) -> None:
    path.write_bytes(gzip.compress(text_.encode("utf-8")))
