#!/usr/bin/env python3
"""The judge: AutoCAD, run with no window on a machine that has it, says what it made of DWGs.

  judge.py built <folder>      every DWG in the folder: opened without recovery? audit errors?
  judge.py roundtrip <folder>  each <id>.dwg against the sample original with the same ID:
                               drawn objects identical, lost and changed, per drawing and pooled
  judge.py rescore <name>      score a run again from AutoCAD's stored readings, without AutoCAD

Settings, the sample and every reading stay in a private folder outside the repository; reports
name drawings by neutral ID and give counts only. See tools/judge/README.md. Never run in CI.
"""

from __future__ import annotations

import argparse
import contextlib
import datetime
import json
import re
import shutil
import signal
import sys
import time
from pathlib import Path

from judgelib import autocad, report, settings as settings_mod

REPO = Path(__file__).resolve().parents[2]
NAME_RX = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


class UsageError(Exception):
    pass


def say(text: str) -> None:
    print("%s %s" % (time.strftime("%H:%M:%S"), text), flush=True)


def dwg_version(path: Path) -> str:
    with open(path, "rb") as f:
        return "".join(c for c in f.read(6).decode("ascii", "replace") if c.isalnum()) or "?"


def sample_ids(judge_dir: Path) -> list[str]:
    orig = judge_dir / "orig"
    ids = sorted(p.stem for p in orig.glob("*.dwg")) if orig.is_dir() else []
    if not ids:
        raise UsageError("no sample: the judge folder has no orig/<id>.dwg files")
    return ids


def run_name(given: str | None, kind: str) -> str:
    name = given or "%s-%s" % (kind, datetime.datetime.now().strftime("%Y%m%d-%H%M%S"))
    if not NAME_RX.match(name):
        raise UsageError("a run name is letters, digits, '.', '_' and '-', starting with a letter or digit")
    return name


@contextlib.contextmanager
def judge_lock(judge_dir: Path):
    """One judge run at a time from this machine, so one AutoCAD at a time on the AutoCAD machine."""
    import fcntl

    with open(judge_dir / "judge.lock", "a") as f:
        try:
            fcntl.flock(f, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            say("judge busy (another run holds the lock); waiting")
            fcntl.flock(f, fcntl.LOCK_EX)
        yield


def stop_on_signals() -> None:
    """A kill or hang-up ends the run through the normal clean-up, which stops the judge's AutoCAD."""
    def handler(signum, frame):
        raise SystemExit(128 + signum)
    for s in (signal.SIGTERM, signal.SIGHUP):
        if signal.getsignal(s) is not signal.SIG_IGN:  # under nohup, a hang-up stays ignored
            signal.signal(s, handler)


def prepare_work(judge_dir: Path, name: str, overwrite: bool) -> Path:
    work = judge_dir / "work" / name
    if work.exists() and not overwrite:
        raise UsageError("a run named %s already has readings; pick another --name or pass --overwrite" % name)
    work.mkdir(parents=True, exist_ok=True)
    return work


def write_run(work: Path, kind: str, name: str, started: str, versions: dict, extra: dict | None = None) -> None:
    ids = sorted(versions)
    (work / "ids.txt").write_text("".join(i + "\n" for i in ids))
    (work / "versions.tsv").write_text("".join("%s\t%s\t%s\n" % (i, *versions[i]) for i in ids))
    (work / "run.json").write_text(json.dumps({"kind": kind, "name": name, "started": started, **(extra or {})}, indent=1))


def ensure_reference(remote: autocad.Remote, judge_dir: Path, ids: list[str]) -> None:
    """AutoCAD's reading of every original, in ref/. An original the AutoCAD machine skipped (AutoCAD
    busy, memory low) is listed in ref/SKIPPED and read on the next run; ref/COMPLETE is written only
    when none was skipped."""
    ref = judge_dir / "ref"
    if (ref / "COMPLETE").exists():
        return
    pending = report.read_skipped(ref)
    todo = [i for i in ids if i in pending] if pending else list(ids)
    if pending:
        say("reading the %d originals the reference skipped last time" % len(todo))
    else:
        say("building the reference first: AutoCAD's reading of every original")
    ref.mkdir(parents=True, exist_ok=True)
    reading = ref / "_reading"   # read apart, so readings already in ref/ stay as they are
    shutil.rmtree(reading, ignore_errors=True)
    if todo:
        autocad.read_drawings(remote, {i: judge_dir / "orig" / (i + ".dwg") for i in todo}, reading, "reference",
                              log=print_flush)
    left = report.skipped(reading, todo)
    for i in todo:
        for f in ref.glob(i + ".*"):
            f.unlink()
        if i not in left:
            for f in reading.glob(i + ".*"):
                f.replace(ref / f.name)
    shutil.rmtree(reading, ignore_errors=True)
    if left:
        (ref / "SKIPPED").write_text("".join("%s\t%s\n" % kv for kv in sorted(left.items())))
        say("%d originals were not read (AutoCAD busy or low memory); the next run reads them" % len(left))
    else:
        (ref / "SKIPPED").unlink(missing_ok=True)
        (ref / "COMPLETE").touch()


def print_flush(text: str) -> None:
    print(text, flush=True)


def finish(judge_dir: Path, name: str, started: str, doc: dict, md: str) -> None:
    doc = {"name": name, "started": started, "scored": datetime.datetime.now().isoformat(timespec="seconds"), **doc}
    report.write(judge_dir / "results", name, doc, md)
    print(md, flush=True)
    say("results written to results/%s.md and results/%s.json in the judge folder" % (name, name))


def cmd_built(args, cfg) -> int:
    cfg.require("dir", "host", "scratch")
    folder = Path(args.folder)
    dwgs = sorted(p for p in folder.iterdir() if p.is_file() and p.suffix.lower() == ".dwg") if folder.is_dir() else []
    if not dwgs:
        raise UsageError("no DWG files in that folder")
    name = run_name(args.name, "built")
    # neutral IDs on the AutoCAD machine and in the reports; the map to file names is names.tsv in the
    # run's readings folder, which the reports never embed
    files = {"b%03d" % n: p for n, p in enumerate(dwgs, 1)}
    work = prepare_work(cfg.dir, name, args.overwrite)
    started = datetime.datetime.now().isoformat(timespec="seconds")
    say("[%s] started: %d built drawings to judge" % (name, len(files)))
    write_run(work, "built", name, started, {i: ("-", dwg_version(p)) for i, p in files.items()})
    (work / "names.tsv").write_text("".join("%s\t%s\n" % (i, p.stem) for i, p in files.items()))
    remote = autocad.Remote(cfg.host, cfg.scratch, cfg.accoreconsole)
    with judge_lock(cfg.dir):
        autocad.read_drawings(remote, files, work, name, log=print_flush)
    return score_and_finish(cfg, name, "built", started, work, [], args.show_names)


def cmd_roundtrip(args, cfg) -> int:
    cfg.require("dir", "host", "scratch")
    folder = Path(args.folder)
    if not folder.is_dir():
        raise UsageError("no such folder")
    ids = sample_ids(cfg.dir)
    files = {i: folder / (i + ".dwg") for i in ids if (folder / (i + ".dwg")).is_file()}
    if not files:
        raise UsageError("the folder has no <id>.dwg named after a drawing in the sample")
    others = sum(1 for p in folder.glob("*.dwg") if p.stem not in files)
    name = run_name(args.name, "roundtrip")
    work = prepare_work(cfg.dir, name, args.overwrite)
    started = datetime.datetime.now().isoformat(timespec="seconds")
    say("[%s] started: %d of the sample's %d drawings to judge%s" % (
        name, len(files), len(ids), "; %d other DWGs in the folder ignored" % others if others else ""))
    versions = {i: (dwg_version(cfg.dir / "orig" / (i + ".dwg")), dwg_version(p)) for i, p in files.items()}
    write_run(work, "roundtrip", name, started, versions)
    remote = autocad.Remote(cfg.host, cfg.scratch, cfg.accoreconsole)
    with judge_lock(cfg.dir):
        ensure_reference(remote, cfg.dir, ids)
        # a drawing whose original AutoCAD cannot read is not scored, so it is not sent either
        readable = {i: p for i, p in files.items() if (cfg.dir / "ref" / (i + ".dxf.gz")).exists()}
        unread = [i for i in files if i not in readable]
        pending = sum(1 for i in unread if i in report.read_skipped(cfg.dir / "ref"))
        if pending:
            say("[%s] %d not sent: their originals have not been read yet; the next run reads them" % (name, pending))
        if len(unread) > pending:
            say("[%s] %d not sent: AutoCAD cannot read their originals" % (name, len(unread) - pending))
        if readable:
            autocad.read_drawings(remote, readable, work, name, log=print_flush)
    return score_and_finish(cfg, name, "roundtrip", started, work, ids, show_names=False)


def score_and_finish(cfg, name, kind, started, work, ids, show_names) -> int:
    """Score the readings in `work`, write the results and print the report. A built run exits 1
    when any drawing fails. Both reports hold neutral IDs; file names only with `show_names`."""
    say("[%s] scoring" % name)
    if kind == "built":
        doc = report.score_built(work, report.read_names(work) if show_names else None)
        finish(cfg.dir, name, started, doc, report.render_built(name, started, doc))
        return 0 if doc["totals"]["pass"] == doc["totals"]["drawings"] else 1
    doc = report.score_roundtrip(cfg.dir / "ref", work, ids)
    finish(cfg.dir, name, started, doc, report.render_roundtrip(name, started, doc))
    return 0


def cmd_rescore(args, cfg) -> int:
    cfg.require("dir")
    name = run_name(args.name, "rescore")
    work = cfg.dir / "work" / name
    if not (work / "ids.txt").exists():
        raise UsageError("no stored readings for a run named %s" % name)
    run = json.loads((work / "run.json").read_text()) if (work / "run.json").exists() else {}
    kind = run.get("kind", "roundtrip")
    started = run.get("started", "(not recorded)")
    ids = sample_ids(cfg.dir) if kind == "roundtrip" else []
    return score_and_finish(cfg, name, kind, started, work, ids, args.show_names)


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="judge.py", description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__.split("\n\n", 1)[1])
    sub = p.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("built", help="judge a folder of built DWGs: opened without recovery, audit errors")
    b.add_argument("folder")
    b.add_argument("--name", help="run name (default: built-<date>-<time>)")
    b.add_argument("--show-names", action="store_true",
                   help="print file names instead of neutral IDs; only for public drawings, such as the demo plant")
    b.add_argument("--overwrite", action="store_true", help="replace the stored readings of a run with this name")
    r = sub.add_parser("roundtrip", help="judge a round trip: drawn objects identical, lost and changed")
    r.add_argument("folder", help="a folder of <id>.dwg files, each named after a drawing in the sample")
    r.add_argument("--name", help="run name (default: roundtrip-<date>-<time>)")
    r.add_argument("--overwrite", action="store_true", help="replace the stored readings of a run with this name")
    s = sub.add_parser("rescore", help="score a run again from AutoCAD's stored readings, without AutoCAD")
    s.add_argument("name")
    s.add_argument("--show-names", action="store_true", help="for a built run: print file names (public drawings only)")
    args = p.parse_args(argv)
    try:
        cfg = settings_mod.load(repo_root=REPO)
        if args.cmd != "rescore":
            stop_on_signals()
        return {"built": cmd_built, "roundtrip": cmd_roundtrip, "rescore": cmd_rescore}[args.cmd](args, cfg)
    except (UsageError, settings_mod.SettingsError) as e:
        print("judge: %s" % e, file=sys.stderr)
        return 2
    except autocad.RemoteError as e:
        print("judge: %s" % e, file=sys.stderr)
        return 3


if __name__ == "__main__":
    sys.exit(main())
