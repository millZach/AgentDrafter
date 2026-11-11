"""Score AutoCAD's readings and write the reports.

A readings folder holds, per drawing ID: <id>.status (what the judge script recorded), <id>.log
(AutoCAD's console output, with the audit report), <id>.dxf.gz (AutoCAD's DXF of the drawing), and
for the run: ids.txt, versions.tsv and results1.tsv / results2.tsv (the laptop driver's record).
Reports name drawings by ID and give counts only.
"""

from __future__ import annotations

import gzip
import json
import re
import statistics
from pathlib import Path

from . import dxf

AUDIT_RX = re.compile(r"Total errors found\s+(\d+)\s+fixed\s+(\d+)")
NOT_JUDGED = {"LOWMEM": "not judged: low memory", "BUSY": "not judged: AutoCAD busy"}


def read_status(d: Path, i: str) -> dict:
    st = {}
    p = d / (i + ".status")
    if p.exists():
        for line in p.read_text(encoding="utf-8", errors="replace").splitlines():
            if "\t" in line:
                k, v = line.split("\t", 1)
                st[k] = v
    return st


def read_runs(d: Path) -> dict:
    """The driver's record per ID: one entry per pass (1 = open, 2 = RECOVER)."""
    res: dict = {}
    for n in ("1", "2"):
        p = d / ("results%s.tsv" % n)
        if not p.exists():
            continue
        for line in p.read_text(encoding="utf-8", errors="replace").splitlines():
            f = line.split("\t")
            if len(f) >= 4 and f[0] not in ("id", "DRIVER DONE"):
                res.setdefault(f[0], []).append({"pass": n, "status": f[1], "exit": f[2], "seconds": f[3]})
    return res


def read_versions(d: Path) -> dict:
    """id -> (DWG version of the original or '-', DWG version of the judged file)."""
    out = {}
    p = d / "versions.tsv"
    if p.exists():
        for line in p.read_text().splitlines():
            f = line.split("\t")
            if len(f) == 3:
                out[f[0]] = (f[1], f[2])
    return out


def audit_errors(d: Path, i: str, st: dict):
    """The error count from AutoCAD's audit report in the console log, or None if there is none."""
    p = d / (i + (".rec.log" if st.get("MODE") == "recovered" else ".log"))
    if not p.exists():
        return None
    m = AUDIT_RX.findall(p.read_text(encoding="utf-8", errors="replace"))
    return int(m[-1][0]) if m else None


def opened(st: dict, i: str, runs=()) -> str:
    """yes: opened without recovery; recovered: only through RECOVER; no: not even then;
    or 'not judged: ...' when the driver never started AutoCAD on it."""
    if st.get("DXFOUT_DONE") != "1" or st.get("DWGNAME", "").lower() != (i + ".dwg").lower():
        # (a DWGNAME that is not this file means AutoCAD worked on some other drawing, such as a blank one)
        for r in reversed(list(runs)):
            if r["status"] in NOT_JUDGED:
                return NOT_JUDGED[r["status"]]
        return "no"
    return "yes" if st.get("MODE") == "clean" else "recovered"


def gz_ok(path: Path) -> bool:
    try:
        with gzip.open(path, "rb") as f:
            return len(f.read(64)) > 0 and f.read() is not None
    except (OSError, EOFError):
        return False


def _lane_ids(d: Path) -> list[str]:
    p = d / "ids.txt"
    return p.read_text().split() if p.exists() else []


def score_built(lane_dir: Path, names: dict | None = None) -> dict:
    """For each built DWG: did AutoCAD open it without recovery, and how many audit errors."""
    runs = read_runs(lane_dir)
    versions = read_versions(lane_dir)
    files = {}
    for i in _lane_ids(lane_dir):
        st = read_status(lane_dir, i)
        e = {"dwg_version": versions.get(i, ("-", "?"))[1], "opened": opened(st, i, runs.get(i, ()))}
        e["audit_errors"] = audit_errors(lane_dir, i, st) if e["opened"] in ("yes", "recovered") else None
        e["pass"] = e["opened"] == "yes" and e["audit_errors"] == 0
        e["runs"] = runs.get(i, [])
        if names and i in names:
            e["name"] = names[i]
        files[i] = e
    totals = {
        "drawings": len(files),
        "pass": sum(1 for e in files.values() if e["pass"]),
        "opened_clean": sum(1 for e in files.values() if e["opened"] == "yes"),
        "opened_with_recover": sum(1 for e in files.values() if e["opened"] == "recovered"),
        "failed_to_open": sum(1 for e in files.values() if e["opened"] == "no"),
        "not_judged": sum(1 for e in files.values() if e["opened"].startswith("not judged")),
        "with_audit_errors": sum(1 for e in files.values() if e["audit_errors"]),
    }
    return {"kind": "built", "totals": totals, "files": files}


def score_roundtrip(ref_dir: Path, lane_dir: Path, sample_ids: list[str]) -> dict:
    """For each sample drawing the run supplied (a lane, in the bake-off's word): AutoCAD's reading of
    the original (ref_dir) against its reading of the file judged, counted in drawn objects, with every
    record as a second view."""
    lane_ids = set(_lane_ids(lane_dir))
    runs = read_runs(lane_dir)
    versions = read_versions(lane_dir)
    files = {}
    for i in sample_ids:
        rst = read_status(ref_dir, i)
        e = {"ref_opened": opened(rst, i), "ref_audit_errors": audit_errors(ref_dir, i, rst)}
        files[i] = e
        if i not in lane_ids:
            e["opened"] = "not supplied"
            continue
        ref_dxf = ref_dir / (i + ".dxf.gz")
        if not ref_dxf.exists() or not gz_ok(ref_dxf):
            e["opened"] = "n/a"
            e["note"] = ("AutoCAD could not read the original; not scored" if not ref_dxf.exists()
                         else "the reference reading is damaged; rebuild the reference")
            continue
        st = read_status(lane_dir, i)
        e["dwg_version_orig"], e["dwg_version_lane"] = versions.get(i, ("?", "?"))
        e["opened"] = opened(st, i, runs.get(i, ()))
        e["audit_errors"] = audit_errors(lane_dir, i, st) if e["opened"] in ("yes", "recovered") else None
        e["runs"] = runs.get(i, [])
        lane_dxf = lane_dir / (i + ".dxf.gz")
        if e["opened"] == "no":
            e["note"] = ("AutoCAD could not open the file; RECOVER not tried (too many failures in this run)"
                         if st.get("MODE") == "not-tried" else "AutoCAD could not open the file, even with RECOVER")
            continue
        if e["opened"] not in ("yes", "recovered"):
            continue
        if not lane_dxf.exists():
            e["note"] = "AutoCAD's reading did not come back; run again"
            continue
        if not gz_ok(lane_dxf):
            e["opened"] += " (reading damaged)"
            e["note"] = "the copy of AutoCAD's reading is empty or cut off; run again"
            continue
        o, l_ = dxf.parse(ref_dxf), dxf.parse(lane_dxf)
        e["drawn_objects"] = dxf.drawn_objects(o, l_)
        # R13 and R14 files carry no fingerprint GUID; AutoCAD makes a new one each time it opens them
        extra = {"$FINGERPRINTGUID"} if e["dwg_version_orig"] in ("AC1012", "AC1014") else ()
        e["all_records"] = dxf.compare(o, l_, extra)

    scored = [e for e in files.values() if "drawn_objects" in e]
    drawn = sum(e["drawn_objects"]["drawn"] for e in scored)
    same = sum(e["drawn_objects"]["identical"] for e in scored)
    pcts = [e["drawn_objects"]["pct_identical"] for e in scored]
    totals = {
        "drawings_in_sample": len(sample_ids),
        "drawings_scored": len(scored),
        "original_unreadable": sum(1 for e in files.values() if e["opened"] == "n/a"),
        "not_supplied": sum(1 for e in files.values() if e["opened"] == "not supplied"),
        "opened_clean": sum(1 for e in files.values() if e["opened"] == "yes"),
        "opened_with_recover": sum(1 for e in files.values() if e["opened"] == "recovered"),
        "failed_to_open": sum(1 for e in files.values() if e["opened"] == "no"),
        "not_judged": sum(1 for e in files.values() if e["opened"].startswith("not judged")),
        "gaining_audit_errors": sum(1 for e in scored if (e.get("audit_errors") or 0) > (e.get("ref_audit_errors") or 0)),
        "drawn_objects": drawn,
        "drawn_identical": same,
        "drawn_lost": sum(e["drawn_objects"]["lost"] for e in scored),
        "drawn_changed": sum(e["drawn_objects"]["changed"] for e in scored),
        "drawn_pct_identical_pooled": round(100.0 * same / drawn, 3) if drawn else None,
        "drawn_pct_identical_median": round(statistics.median(pcts), 3) if pcts else None,
        "drawn_pct_identical_worst": round(min(pcts), 3) if pcts else None,
        "drawings_drawn_identical": sum(1 for e in scored
                                        if e["drawn_objects"]["identical"] == e["drawn_objects"]["drawn"]),
        "all_records_pct_identical_pooled": _pooled(scored, "identical_within_1e-9"),
        "all_records_pct_handles_kept_pooled": _pooled(scored, "handles_kept_same_type"),
    }
    return {"kind": "roundtrip", "totals": totals, "files": files}


def _pooled(scored: list, key: str):
    total = sum(e["all_records"]["records_orig"] for e in scored)
    return round(100.0 * sum(e["all_records"][key] for e in scored) / total, 3) if total else None


def _top(d: dict, n=3) -> str:
    items = sorted(d.items(), key=lambda kv: -kv[1])[:n]
    s = ", ".join("%s %d" % kv for kv in items)
    more = len(d) - n
    return (s + (" +%d more" % more if more > 0 else "")) if s else "-"


def _version(e: dict) -> str:
    a, b = e.get("dwg_version_orig"), e.get("dwg_version_lane")
    if not a:
        return "-"
    return a if a == b else "%s -> %s" % (a, b)


def render_built(name: str, started: str, doc: dict, show_names: bool) -> str:
    t = doc["totals"]
    lines = [
        "# Judge: built %s" % name,
        "",
        "Started %s. A drawing passes when AutoCAD opens it without recovery and its audit finds no errors." % started,
        "",
        "| drawing | DWG version | opened | audit errors | verdict |",
        "|---|---|---|---|---|",
    ]
    for i, e in doc["files"].items():
        shown = e.get("name", i) if show_names else i
        ae = "-" if e["audit_errors"] is None else str(e["audit_errors"])
        lines.append("| %s | %s | %s | %s | %s |" % (shown, e["dwg_version"], e["opened"], ae, "pass" if e["pass"] else "FAIL"))
    lines += ["", "%d of %d pass. Opened clean %d, with RECOVER %d, failed %d, not judged %d; with audit errors %d." % (
        t["pass"], t["drawings"], t["opened_clean"], t["opened_with_recover"], t["failed_to_open"], t["not_judged"],
        t["with_audit_errors"])]
    return "\n".join(lines) + "\n"


def render_roundtrip(name: str, started: str, doc: dict) -> str:
    t = doc["totals"]
    lines = [
        "# Judge: round trip %s" % name,
        "",
        "Started %s. AutoCAD's reading of each original against its reading of the file judged. "
        "Drawn objects are the records in the ENTITIES and BLOCKS sections (lines, text, block inserts, "
        "attributes, hatches, dimensions, ...); identical means the same handle with the same content. "
        "Read these against the baseline `autocad-noop` (AutoCAD's own plain save), not against 100." % started,
        "",
        "Pooled over %d drawings: %d drawn objects, %d identical (%s%%), %d lost, %d changed; "
        "%d drawings with every drawn object identical. Opened clean %d, with RECOVER %d, failed %d, "
        "not judged %d; not supplied %d; original unreadable %d. Drawings gaining audit errors: %d." % (
            t["drawings_scored"], t["drawn_objects"], t["drawn_identical"], t["drawn_pct_identical_pooled"],
            t["drawn_lost"], t["drawn_changed"], t["drawings_drawn_identical"], t["opened_clean"],
            t["opened_with_recover"], t["failed_to_open"], t["not_judged"], t["not_supplied"],
            t["original_unreadable"], t["gaining_audit_errors"]),
        "",
        "Every record with a handle, not only drawn objects (table entries, dictionaries, layouts, ...): "
        "%s%% identical, %s%% kept under the same handle." % (
            t["all_records_pct_identical_pooled"], t["all_records_pct_handles_kept_pooled"]),
        "",
        "| id | DWG version (orig -> file) | opened | audit errors (orig -> file) | drawn objects | identical | lost | changed "
        "| identical % | lost (by type) | changed (by type) | all records identical % |",
        "|---|---|---|---|---|---|---|---|---|---|---|---|",
    ]
    for i, e in doc["files"].items():
        if e["opened"] == "not supplied":
            continue
        ae = "%s -> %s" % (e.get("ref_audit_errors"), "-" if e.get("audit_errors") is None else e["audit_errors"])
        if "drawn_objects" not in e:
            lines.append("| %s | %s | %s | %s | - | - | - | - | - | - | - | %s |" % (
                i, _version(e), e["opened"], ae, e.get("note", "")))
            continue
        d = e["drawn_objects"]
        lines.append("| %s | %s | %s | %s | %d | %d | %d | %d | %.2f | %s | %s | %.2f |" % (
            i, _version(e), e["opened"], ae, d["drawn"], d["identical"], d["lost"], d["changed"], d["pct_identical"],
            _top(d["lost_by_type"]), _top(d["changed_by_type"]), e["all_records"]["pct_identical"]))
    return "\n".join(lines) + "\n"


def write(results_dir: Path, name: str, doc: dict, md: str) -> None:
    results_dir.mkdir(parents=True, exist_ok=True)
    for suffix, text in ((".json", json.dumps(doc, indent=1)), (".md", md)):
        tmp = results_dir / (name + suffix + ".tmp")
        tmp.write_text(text)
        tmp.replace(results_dir / (name + suffix))
