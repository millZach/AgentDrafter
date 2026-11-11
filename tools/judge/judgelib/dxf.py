"""Read AutoCAD's own ASCII DXF of a drawing and compare two readings.

Both readings are written by AutoCAD (DXFOUT, 2018 format, 16 decimal places), so a difference
comes from the DWG, not from a DXF writer. The reader keeps every group code, including ones a
DXF library's document model would drop.

Main entry points: parse(path) -> Dxf; drawn_objects(orig, lane) -> dict; compare(orig, lane) -> dict.
"""

from __future__ import annotations

import collections
import gzip
import hashlib
import io

# Header values that change on every save or every open; ignored on purpose.
VOLATILE_HEADER = {
    "$TDCREATE", "$TDUCREATE", "$TDUPDATE", "$TDUUPDATE", "$TDINDWG", "$TDUSRTIMER",
    "$HANDSEED", "$VERSIONGUID",
}

# xdata values AutoCAD regenerates every time it loads a drawing (two DXFOUTs of one unchanged DWG
# differ only here). Replaced by a placeholder before comparing.
VOLATILE_XDATA_APPS = {"AcDbDynamicBlockGUID"}

FLOAT_CODES = (set(range(10, 60)) | set(range(110, 150)) | set(range(210, 240)) | set(range(460, 470))
               | set(range(1010, 1060)))
# group codes that hold handles or pointers to other objects
POINTER_CODES = ({5, 105, 320, 330, 340, 350, 360, 390, 480, 481, 1005} | set(range(321, 330))
                 | set(range(331, 340)) | set(range(341, 350)) | set(range(351, 360))
                 | set(range(361, 370)) | set(range(391, 400)))

# Records in ENTITIES and BLOCKS that mark structure rather than draw anything.
NOT_DRAWN = {"BLOCK", "ENDBLK", "SEQEND"}

# Subclasses whose binary group 310 is an embedded OLE object's own data, compared as content (ole_digest).
OLE_SUBCLASSES = {"AcDbOle2Frame", "AcDbOleFrame"}
CFB_MAGIC = bytes.fromhex("D0CF11E0A1B11AE1")  # an OLE compound file's first 8 bytes


def ole_digest(hex_lines: list) -> str:
    """An embedded OLE object's data as one comparable value: a digest of all its bytes except the
    modified time of its compound file's root storage, which AutoCAD stamps each time it writes the
    object (two reads of the same unchanged drawing differ there and nowhere else)."""
    text = "".join(hex_lines)
    try:
        data = bytearray(bytes.fromhex(text))
    except ValueError:
        return "<OLE data, not hex, sha1 %s>" % hashlib.sha1(text.encode("utf-8", "replace")).hexdigest()
    s = data.find(CFB_MAGIC, 0, 4096)
    if s >= 0 and len(data) >= s + 52:
        shift = int.from_bytes(data[s + 30:s + 32], "little")        # sector size is 1 << shift
        first_dir = int.from_bytes(data[s + 48:s + 52], "little")    # sector of the first directory entries
        if 9 <= shift <= 12:
            root = s + ((first_dir + 1) << shift)                    # entry 0: the root storage
            if root + 116 <= len(data):
                data[root + 108:root + 116] = bytes(8)               # its modified time
    return "<OLE data, %d bytes, sha1 %s>" % (len(data), hashlib.sha1(data).hexdigest())


class DxfError(Exception):
    pass


def iter_tags(stream):
    """(group code, value) pairs from an ASCII DXF text stream. The group code is an int, the value
    a string without its trailing newline; comments (999) are skipped and nothing after EOF is read.
    This is the same reading as ezdxf's ascii_tags_loader, which the bake-off used."""
    readline = stream.readline
    line = 1
    while True:
        code = readline()
        if not code:
            return
        try:
            group = int(code)
        except ValueError:
            raise DxfError("invalid group code at line %d" % line) from None
        value = readline()
        if not value:
            return
        value = value.rstrip("\n")
        if group != 999:
            yield group, value
        if group == 0 and value == "EOF":
            return
        line += 2


def _sort_groups(tags, start, members, inner_sort=None, after=None):
    """Sort runs of tag groups that AutoCAD writes in no fixed order (sets stored as lists).
    A group starts at code `start` and takes the following tags whose codes are in `members`.
    `after`: only tags after the first (100, after) marker are touched."""
    out = list(tags)
    i = 0
    if after is not None:
        i = next((k + 1 for k, t in enumerate(out) if t == (100, after)), len(out))
    n = len(out)
    while i < n:
        if out[i][0] != start:
            i += 1
            continue
        j = i
        groups = []
        while j < n and out[j][0] == start:
            k = j + 1
            while k < n and out[k][0] in members:
                k += 1
            g = out[j:k]
            if inner_sort is not None:
                head = [t for t in g if t[0] != inner_sort]
                g = head + sorted(t for t in g if t[0] == inner_sort)
            groups.append(tuple(g))
            j = k
        groups.sort()
        out[i:j] = [t for g in groups for t in g]
        i = j
    return out


def normalize(type_, section, tags) -> tuple:
    """Remove differences that AutoCAD itself produces when it reads the same unchanged DWG twice
    (found by judging the originals against themselves). Applied to both sides when parsing."""
    out = []
    app = subclass = None
    in_value = seen_value = False
    ole_lines: list = []   # an OLE object's binary data, compared as one digest (see ole_digest)
    ole_at = None
    for code, val in tags:
        if code == 100:
            subclass = val
        if code == 1001:
            app = val
        elif app in VOLATILE_XDATA_APPS and code == 1000:
            val = "<volatile>"
        if type_ == "FIELD":
            # cached evaluation result (date, file path, ...) and the checksum of the shown text are
            # re-computed on every open; the field code and its parameters are kept
            if (code == 7 and val == "ACFD_FIELD_VALUE") or (code == 6 and val == "ACFD_FIELDTEXT_CHECKSUM"):
                in_value = True
                seen_value = seen_value or code == 7
                out.append((code, val))
                continue
            if in_value:
                if code == 304 and val == "ACVALUE_END":
                    in_value = False
                continue
            if seen_value and code in (301, 98):
                continue
        if code == 310 and section in ("ENTITIES", "BLOCKS") and subclass in OLE_SUBCLASSES:
            # an embedded OLE object's data is its content: compared, less the one stamp AutoCAD rewrites
            if ole_at is None:
                ole_at = len(out)
                out.append(None)
            ole_lines.append(val)
            continue
        if code == 310 and section in ("ENTITIES", "BLOCKS") and type_ != "ACAD_PROXY_ENTITY":
            # proxy graphics (the display cache AutoCAD writes for tables, multileaders, ...): rebuilt on
            # every save with uninitialised padding and timestamps, so only its length counts
            val = "<binary %d>" % len(val)
        out.append((code, val))
    if ole_at is not None:
        out[ole_at] = (310, ole_digest(ole_lines))
    if type_ == "BLOCK_RECORD":
        out = _sort_groups(out, 331, ())
    elif type_ == "SORTENTSTABLE":
        out = _sort_groups(out, 331, (5,))
    elif type_ == "FIELDLIST":
        out = _sort_groups(out, 330, (), after="AcDbIdSet")
    elif type_.startswith("BLOCK") and type_.endswith("ACTION"):
        out = _sort_groups(out, 331, (74, 94), inner_sort=94)
    return tuple(out)


class Rec:
    """One record with a handle: its type, section, space, handle, owner and normalized tags."""

    __slots__ = ("type", "section", "space", "handle", "owner", "tags")

    def __init__(self, type_, section, space, tags):
        self.type = type_
        self.section = section
        self.space = space  # 'model' | 'paper' | 'block' | None
        self.tags = tags    # tuple of (code, value-string), without the leading (0, type)
        self.handle = None
        self.owner = None
        hcode = 105 if type_ == "DIMSTYLE" else 5
        depth = 0
        for code, val in tags:
            if code == 102:
                depth += 1 if val.startswith("{") else -1
                continue
            if self.handle is None and code == hcode:
                self.handle = val.upper()
            elif self.owner is None and code == 330 and depth == 0:
                self.owner = val.upper()
                break


class Dxf:
    def __init__(self):
        self.header: dict[str, tuple] = {}
        self.classes: list[tuple] = []
        self.recs: list[Rec] = []          # every record with a handle, file order
        self.unkeyed: dict[str, list[tuple]] = collections.defaultdict(list)  # section -> records without handles
        self.sections: list[str] = []


def _open_text(path):
    with open(path, "rb") as f:
        raw = f.read()
    if str(path).endswith(".gz"):
        raw = gzip.decompress(raw)
    # AutoCAD writes 2007 and later DXF as UTF-8
    return io.StringIO(raw.decode("utf-8", errors="replace").replace("\r\n", "\n"))


def parse(path) -> Dxf:
    """Read one DXF (plain or gzipped) into header, classes and records."""
    return parse_stream(_open_text(path))


def parse_stream(stream) -> Dxf:
    dxf = Dxf()
    section = None
    cur_type = None
    cur_tags: list = []
    block_space = None   # space for the records in the current BLOCKS block

    def flush():
        nonlocal cur_type, cur_tags, block_space
        if cur_type is None:
            return
        t = cur_type
        tags = normalize(t, section, cur_tags)
        cur_type, cur_tags = None, []
        if t in ("SECTION", "ENDSEC", "EOF"):
            return
        if section == "HEADER":
            return
        if section == "CLASSES":
            dxf.classes.append(tags)
            return
        space = None
        if section == "ENTITIES":
            space = "paper" if any(c == 67 and v.strip() == "1" for c, v in tags) else "model"
        elif section == "BLOCKS":
            if t == "BLOCK":
                name = next((v for c, v in tags if c == 2), "").upper()
                block_space = ("paper" if name.startswith("*PAPER_SPACE")
                               else ("model" if name == "*MODEL_SPACE" else "block"))
            elif t != "ENDBLK":
                space = block_space
        rec = Rec(t, section, space, tags)
        if rec.handle is None:
            dxf.unkeyed[section or "?"].append((t,) + tags)
        else:
            dxf.recs.append(rec)

    header_var = None
    header_vals: list = []
    expect_section_name = False
    for code, value in iter_tags(stream):
        if code == 0:
            if section == "HEADER" and header_var is not None:
                dxf.header[header_var] = tuple(header_vals)
                header_var = None
            flush()
            if value == "SECTION":
                expect_section_name = True
                cur_type = None
                continue
            if value == "ENDSEC":
                section = None
                continue
            cur_type = value
            continue
        if expect_section_name and code == 2:
            section = value
            dxf.sections.append(value)
            expect_section_name = False
            continue
        if section == "HEADER":
            if code == 9:
                if header_var is not None:
                    dxf.header[header_var] = tuple(header_vals)
                header_var, header_vals = value, []
            else:
                header_vals.append((code, value))
            continue
        if section == "THUMBNAILIMAGE" and cur_type is None:
            dxf.unkeyed["THUMBNAILIMAGE"].append((code, value))
            continue
        if cur_type is not None:
            cur_tags.append((code, value))
    flush()
    return dxf


def _feq(a: str, b: str) -> bool:
    try:
        x, y = float(a), float(b)
    except ValueError:
        return a == b
    return abs(x - y) <= 1e-9 * max(1.0, abs(x), abs(y))


def tags_equal(a: tuple, b: tuple, approx: bool) -> bool:
    """Same tags. With `approx`, floats may differ by 1e-9 relative."""
    if a == b:
        return True
    if not approx or len(a) != len(b):
        return False
    for (ca, va), (cb, vb) in zip(a, b):
        if ca != cb:
            return False
        if va != vb and not (ca in FLOAT_CODES and _feq(va, vb)):
            return False
    return True


def changed_codes(a: tuple, b: tuple) -> list[int]:
    """Group codes that differ between two tag lists (approximate floats count as equal)."""
    out = set()
    if len(a) == len(b):
        for (x, vx), (y, vy) in zip(a, b):
            if x != y:
                out.add(x)
                out.add(y)
            elif vx != vy and not (x in FLOAT_CODES and _feq(vx, vy)):
                out.add(x)
        return sorted(out)
    ca = collections.Counter(a)
    cb = collections.Counter(b)
    for t in (ca - cb) + (cb - ca):
        out.add(t[0])
    return sorted(out)


def content_key(rec: Rec) -> str:
    """Hash of a record's data with handles and pointers removed (finds objects saved under new handles)."""
    h = hashlib.sha1(rec.type.encode())
    for c, v in rec.tags:
        if c in POINTER_CODES:
            continue
        if c in FLOAT_CODES:
            try:
                v = "%.9g" % float(v)
            except ValueError:
                pass
        h.update(b"\x00%d\x01" % c)
        h.update(v.encode("utf-8", "replace"))
    return h.hexdigest()


def _found_under_new_handle(o_by_h: dict, l_by_h: dict) -> int:
    """How many records whose handle vanished reappear with the same data under a new handle."""
    pool = collections.Counter(content_key(r) for h, r in l_by_h.items() if h not in o_by_h)
    found = 0
    for h, o in o_by_h.items():
        if h in l_by_h:
            continue
        k = content_key(o)
        if pool[k] > 0:
            pool[k] -= 1
            found += 1
    return found


def drawn(dxf: Dxf) -> dict:
    """The drawing's drawn objects by handle: every record in ENTITIES and BLOCKS (lines, text, block
    inserts, attributes, hatches, dimensions, ...) except the BLOCK, ENDBLK and SEQEND markers."""
    return {r.handle: r for r in dxf.recs
            if r.handle and r.section in ("ENTITIES", "BLOCKS") and r.type not in NOT_DRAWN}


def drawn_objects(orig: Dxf, lane: Dxf) -> dict:
    """Drawn objects of the original against the lane: identical (same handle, same content with
    floats to 1e-9 relative), lost (handle gone) and changed (handle kept, content differs)."""
    a, b = drawn(orig), drawn(lane)
    lost = collections.Counter()
    changed = collections.Counter()
    for h, r in a.items():
        other = b.get(h)
        if other is None:
            lost[r.type] += 1
        elif other.type != r.type or not tags_equal(r.tags, other.tags, approx=True):
            changed[r.type] += 1
    n_lost, n_changed = sum(lost.values()), sum(changed.values())
    identical = len(a) - n_lost - n_changed
    return {
        "drawn": len(a),
        "identical": identical,
        "lost": n_lost,
        "changed": n_changed,
        "added": sum(1 for h in b if h not in a),
        "pct_identical": round(100.0 * identical / len(a), 3) if a else 100.0,
        "lost_found_under_new_handle": _found_under_new_handle(a, b) if n_lost else 0,
        "lost_by_type": dict(lost.most_common()),
        "changed_by_type": dict(changed.most_common()),
    }


def compare(orig: Dxf, lane: Dxf, volatile_header_extra=()) -> dict:
    """Every record with a handle (drawn objects, table entries, dictionaries, layouts, ...), not only
    drawn objects: kept, identical, lost, added and changed, plus header variables and classes."""
    o_by_h = {r.handle: r for r in orig.recs}
    l_by_h = {r.handle: r for r in lane.recs}
    total = len(o_by_h)
    same_type = identical = identical_approx = 0
    lost = collections.Counter()
    retyped = collections.Counter()
    changed = collections.defaultdict(lambda: {"n": 0, "codes": collections.Counter()})
    for h, o in o_by_h.items():
        other = l_by_h.get(h)
        if other is None:
            lost[o.type] += 1
            continue
        if other.type != o.type:
            retyped["%s->%s" % (o.type, other.type)] += 1
            continue
        same_type += 1
        if o.tags == other.tags:
            identical += 1
            identical_approx += 1
            continue
        if tags_equal(o.tags, other.tags, approx=True):
            identical_approx += 1
            continue
        c = changed[o.type]
        c["n"] += 1
        for code in changed_codes(o.tags, other.tags):
            c["codes"][code] += 1
    added = collections.Counter(r.type for h, r in l_by_h.items() if h not in o_by_h)

    # object order inside each owner (block record or layout), over handles kept in both
    o_order = collections.defaultdict(list)
    l_order = collections.defaultdict(list)
    for r in orig.recs:
        if r.space and r.handle in l_by_h:
            o_order[r.owner].append(r.handle)
    for r in lane.recs:
        if r.space and r.handle in o_by_h:
            l_order[r.owner].append(r.handle)
    order_changed = sum(1 for k, v in o_order.items() if l_order.get(k) != v)

    hdr_changed = sorted(k for k in set(orig.header) | set(lane.header)
                         if k not in VOLATILE_HEADER and k not in volatile_header_extra
                         and orig.header.get(k) != lane.header.get(k))
    o_cls = collections.Counter(next((v for c, v in t if c == 1), "?") for t in orig.classes)
    l_cls = collections.Counter(next((v for c, v in t if c == 1), "?") for t in lane.classes)
    unkeyed = {}
    for sec in sorted(set(orig.unkeyed) | set(lane.unkeyed)):
        a = collections.Counter(orig.unkeyed.get(sec, []))
        b = collections.Counter(lane.unkeyed.get(sec, []))
        if a != b:
            unkeyed[sec] = {"orig": sum(a.values()), "lane": sum(b.values()),
                            "missing": sum((a - b).values()), "extra": sum((b - a).values())}

    def pct(n):
        return round(100.0 * n / total, 3) if total else 100.0

    return {
        "records_orig": total,
        "records_lane": len(l_by_h),
        "handles_kept_same_type": same_type,
        "identical_exact": identical,
        "identical_within_1e-9": identical_approx,
        "pct_handles_kept": pct(same_type),
        "pct_identical": pct(identical_approx),
        "pct_identical_exact": pct(identical),
        "lost_by_type": dict(lost.most_common()),
        "lost_found_under_new_handle": _found_under_new_handle(o_by_h, l_by_h) if lost else 0,
        "retyped": dict(retyped),
        "added_by_type": dict(added.most_common()),
        "changed_by_type": {t: {"n": v["n"], "codes": {str(k): n for k, n in v["codes"].most_common(12)}}
                            for t, v in sorted(changed.items(), key=lambda kv: -kv[1]["n"])},
        "object_order_changed_in_owners": order_changed,
        "header_vars_changed": hdr_changed,
        "classes_lost": dict(o_cls - l_cls),
        "classes_added": dict(l_cls - o_cls),
        "unkeyed_sections_changed": unkeyed,
    }
