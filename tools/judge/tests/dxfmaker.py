"""Made-up DXF text for the judge's tests: a header, drawn objects in model space and in one block, and a
record or two in the OBJECTS section."""

from __future__ import annotations

import gzip
from pathlib import Path


def line(handle, owner="1F", x1="0.0", y1="0.0", x2="10.0", y2="0.0", layer="0", extra=()):
    return ["LINE", (5, handle), (330, owner), (100, "AcDbEntity"), (8, layer), *extra, (100, "AcDbLine"),
            (10, x1), (20, y1), (30, "0.0"), (11, x2), (21, y2), (31, "0.0")]


def text(handle, value, owner="1F"):
    return ["TEXT", (5, handle), (330, owner), (100, "AcDbEntity"), (8, "0"), (100, "AcDbText"),
            (10, "1.0"), (20, "2.0"), (30, "0.0"), (40, "2.5"), (1, value)]


def compound_file(payload: bytes, modified: bytes) -> bytes:
    """A made-up OLE object's data: a 128-byte lead, then a compound file with 512-byte sectors whose
    first directory sector (sector 0) starts with the root storage's entry, modified time at byte 108."""
    head = bytearray(512)
    head[0:8] = bytes.fromhex("D0CF11E0A1B11AE1")
    head[30:32] = (9).to_bytes(2, "little")   # sector size 1 << 9
    head[48:52] = (0).to_bytes(4, "little")   # first directory sector
    root = bytearray(128)
    root[0:20] = "Root Entry".encode("utf-16-le")
    root[108:116] = modified
    return bytes(128) + bytes(head) + bytes(root) + bytes(384) + payload


def ole2frame(handle, data: bytes, owner="1F"):
    hexed = data.hex().upper()
    return ["OLE2FRAME", (5, handle), (330, owner), (100, "AcDbEntity"), (8, "0"), (100, "AcDbOle2Frame"),
            (70, "2"), (3, "Demo Picture"), (10, "0.0"), (20, "10.0"), (30, "0.0"), (11, "10.0"), (21, "0.0"),
            (31, "0.0"), (71, "2"), (72, "0"), (90, str(len(data))),
            *[(310, hexed[k:k + 64]) for k in range(0, len(hexed), 64)], (1, "OLE")]


def dxf_text(drawn=(), in_block=(), objects=(), header=None) -> str:
    """A small DXF: HEADER, BLOCKS (one block 'DEMO' holding `in_block`), ENTITIES (holding `drawn`), OBJECTS."""
    header = header or {"$ACADVER": [(1, "AC1032")], "$HANDSEED": [(5, "FFFF")]}
    tags = [(0, "SECTION"), (2, "HEADER")]
    for var, vals in header.items():
        tags.append((9, var))
        tags.extend(vals)
    tags += [(0, "ENDSEC"), (0, "SECTION"), (2, "BLOCKS"),
             (0, "BLOCK"), (5, "B0"), (330, "B1"), (100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockBegin"), (2, "DEMO")]
    for rec in in_block:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDBLK"), (5, "B2"), (330, "B1"), (100, "AcDbEntity"), (8, "0"), (100, "AcDbBlockEnd"),
             (0, "ENDSEC"), (0, "SECTION"), (2, "ENTITIES")]
    for rec in drawn:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDSEC"), (0, "SECTION"), (2, "OBJECTS")]
    for rec in objects:
        tags += [(0, rec[0]), *rec[1:]]
    tags += [(0, "ENDSEC"), (0, "EOF")]
    # AutoCAD right-aligns group codes in three columns and ends lines with CRLF
    return "".join("%3d\r\n%s\r\n" % (c, v) for c, v in tags)


def write_gz(path: Path, text_: str) -> None:
    path.write_bytes(gzip.compress(text_.encode("utf-8")))
