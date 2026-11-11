"""AutoCAD reads drawings on the Windows machine that has it, over ssh, with no window.

For each drawing: copy it to one fixed folder on that machine, open it with the AutoCAD console
(one at a time, after the free-memory check, 120 s limit), have AutoCAD write its own ASCII DXF and
run AUDIT without fixing. A drawing that will not open gets one try through RECOVER (at most 8 per
run). The DXFs come back gzipped with the console logs and status files.

Every drawing is opened from the same folder (<scratch>\\lanes\\judging), so fields that show the
file path read the same in every run, including the reference.
"""

from __future__ import annotations

import base64
import subprocess
import time
from pathlib import Path

from .report import NOT_JUDGED, gz_ok, read_runs, read_status

LAPTOP = Path(__file__).resolve().parent.parent / "laptop"
SLOT = "judging"
TIMEOUT_SEC = 120     # per drawing
MAX_WAIT_SEC = 1200   # per drawing, for memory or for another AutoCAD console to finish
MAX_RECOVER = 8       # a file RECOVER cannot read hangs until the time limit; cap the cost of a broken run
SSH_OPTS = ["-o", "BatchMode=yes", "-o", "ServerAliveInterval=30", "-o", "ConnectTimeout=20"]


class RemoteError(Exception):
    pass


def lisp_forms(src: str) -> list[str]:
    """Top-level forms of a LISP file, one per line, comments and line breaks removed."""
    out, cur, depth, i, n = [], [], 0, 0, len(src)
    in_str = False
    while i < n:
        c = src[i]
        if in_str:
            cur.append(c)
            if c == "\\" and i + 1 < n:
                cur.append(src[i + 1])
                i += 2
                continue
            if c == '"':
                in_str = False
            i += 1
            continue
        if c == ";":
            while i < n and src[i] != "\n":
                i += 1
            continue
        if c == '"':
            in_str = True
            cur.append(c)
            i += 1
            continue
        if c in " \t\r\n":
            if cur and cur[-1] != " " and depth > 0:
                cur.append(" ")
            i += 1
            continue
        if c == "(":
            depth += 1
        cur.append(c)
        if c == ")":
            depth -= 1
            if depth == 0:
                out.append("".join(cur).strip())
                cur = []
        i += 1
    if depth != 0 or in_str:
        raise ValueError("unbalanced LISP")
    return out


def judge_script(lsp: str) -> str:
    """The console script for the open pass: FILEDIA saved and restored, judge.lsp pasted in."""
    lines = ['(setq adb-fd (getvar "FILEDIA"))', '(setvar "FILEDIA" 0)']
    lines += lisp_forms(lsp)
    lines += ['(adb-judge "clean")', '(setvar "FILEDIA" adb-fd)', "_.QUIT", "_Y"]
    return "\r\n".join(lines) + "\r\n"


def recover_script(folder: str, i: str) -> str:
    """The console script for one RECOVER try on <folder>/<i>.dwg (folder with forward slashes).
    Plain script lines follow RECOVER, because LISP state does not carry into the recovered drawing;
    FILEDIA is kept in a file across it."""
    def st(expr):
        return '(progn (setq adb-f (open "%s/%s.status" "a")) (write-line %s adb-f) (close adb-f) (princ))' % (folder, i, expr)
    lines = [
        '(progn (setq adb-f (open "%s/%s.filedia" "w")) (write-line (itoa (getvar "FILEDIA")) adb-f) '
        '(close adb-f) (setvar "FILEDIA" 0) (princ))' % (folder, i),
        "_.RECOVER", "%s/%s.dwg" % (folder, i),
        st('(strcat "DWGNAME\\t" (getvar "DWGNAME"))'),
        st('"MODE\\trecovered"'),
        "_.DXFOUT", "%s/%s.dxf" % (folder, i), "_V", "2018", "16",
        st('"DXFOUT_DONE\\t1"'),
        "_.AUDIT", "_N",
        st('"AUDIT_DONE\\t1"'),
        '(progn (setq adb-f (open "%s/%s.filedia" "r")) (setvar "FILEDIA" (atoi (read-line adb-f))) '
        '(close adb-f) (princ))' % (folder, i),
        "_.QUIT", "_Y",
    ]
    return "\r\n".join(lines) + "\r\n"


class Remote:
    """The Windows machine that has AutoCAD, reached with ssh and scp."""

    def __init__(self, host: str, scratch: str, accore: str):
        self.host, self.scratch, self.accore = host, scratch, accore

    def win(self, *parts: str) -> str:
        return "\\".join((self.scratch,) + parts)

    def scp_target(self, *parts: str) -> str:
        return "%s:/%s/%s" % (self.host, self.scratch.replace("\\", "/"), "/".join(parts))

    def ssh(self, command: str, timeout: float) -> subprocess.CompletedProcess:
        try:
            return subprocess.run(["ssh", *SSH_OPTS, self.host, command], stdin=subprocess.DEVNULL,
                                  capture_output=True, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise RemoteError("timed out after %d s talking to the AutoCAD machine" % timeout) from None

    def ps_file(self, script: str, args: list[str], timeout: float = 180) -> str:
        """Run one of the judge's PowerShell scripts in the scratch folder; parameter values are quoted
        (the settings allow no quotes in the scratch folder, and no value ends in a backslash)."""
        quoted = " ".join(a if a.startswith("-") else '"%s"' % a for a in args)
        cmd = 'powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass -File "%s" %s' % (
            self.win("scripts", script), quoted)
        r = self.ssh(cmd, timeout)
        if r.returncode != 0:
            raise RemoteError("%s on the AutoCAD machine failed (exit %d)" % (script, r.returncode))
        return r.stdout.decode("utf-8", "replace").replace("\r", "")

    def ps_inline(self, code: str, timeout: float = 60) -> None:
        enc = base64.b64encode(code.encode("utf-16-le")).decode()
        r = self.ssh("powershell -NoProfile -NonInteractive -EncodedCommand " + enc, timeout)
        if r.returncode != 0:
            raise RemoteError("could not reach the AutoCAD machine over ssh (exit %d); check the host setting "
                              "and that `ssh <host> echo ok` works without a password" % r.returncode)

    def put(self, files: list[Path], *parts: str) -> None:
        r = subprocess.run(["scp", "-q", *SSH_OPTS, *map(str, files), self.scp_target(*parts) + "/"],
                           stdin=subprocess.DEVNULL, capture_output=True, timeout=600)
        if r.returncode != 0:
            raise RemoteError("copying %d files to the AutoCAD machine failed (exit %d)" % (len(files), r.returncode))

    def get(self, pattern: str, dest: Path, *parts: str) -> bool:
        """Copy files matching `pattern` from the scratch folder; False when none came."""
        r = subprocess.run(["scp", "-q", *SSH_OPTS, self.scp_target(*parts, pattern), str(dest) + "/"],
                           stdin=subprocess.DEVNULL, capture_output=True, timeout=600)
        return r.returncode == 0


def _say(name, log, text):
    log("[%s] %s %s" % (name, time.strftime("%H:%M:%S"), text))


def driver_timeout(n: int) -> int:
    """How long the ssh call running the driver on n drawings may take. The driver may wait up to
    MAX_WAIT_SEC for its turn before every drawing, then gives each up to TIMEOUT_SEC and 10 s to stop;
    30 s per drawing covers that and its own checks, so busy drawings end as not judged, not in a timeout."""
    return n * (MAX_WAIT_SEC + TIMEOUT_SEC + 30) + 300


def _run_driver(remote: Remote, jobs_file: Path, results: str, n: int) -> None:
    remote.put([jobs_file], "lanes", SLOT)
    remote.ps_file("driver.ps1", ["-Jobs", remote.win("lanes", SLOT, jobs_file.name),
                                  "-Results", remote.win("lanes", SLOT, results),
                                  "-Accore", remote.accore,
                                  "-TimeoutSec", str(TIMEOUT_SEC), "-MaxWaitSec", str(MAX_WAIT_SEC)],
                   timeout=driver_timeout(n))


def read_drawings(remote: Remote, files: dict, out: Path, name: str, log=print) -> None:
    """AutoCAD reads each drawing in `files` (id -> local DWG path). Leaves in `out`: <id>.status,
    <id>.log, <id>.dxf.gz, results1.tsv (and results2.tsv after RECOVER). The caller holds the lock."""
    ids = sorted(files)
    out.mkdir(parents=True, exist_ok=True)
    for pat in ("*.dxf.gz", "*.status", "*.log", "results*.tsv", "*.rec.scr", "jobs*.tsv"):
        for f in out.glob(pat):
            f.unlink()
    send = out / "_send"
    if send.exists():
        for f in send.iterdir():
            f.unlink()
    send.mkdir(exist_ok=True)
    try:
        # scripts to the machine (cheap, keeps its copy current)
        (send / "judge.scr").write_text(judge_script((LAPTOP / "judge.lsp").read_text()), newline="")
        remote.ps_inline("New-Item -ItemType Directory -Force -Path '%s' | Out-Null" % remote.win("scripts"))
        remote.put([LAPTOP / "driver.ps1", LAPTOP / "tasks.ps1", send / "judge.scr"], "scripts")
        remote.ps_file("tasks.ps1", ["-Action", "prep", "-Root", remote.scratch])

        # the drawings, each as <id>.dwg
        for i in ids:
            (send / (i + ".dwg")).symlink_to(Path(files[i]).resolve())
        remote.put([send / (i + ".dwg") for i in ids], "lanes", SLOT)
        lw = remote.win("lanes", SLOT)

        _say(name, log, "pass 1 (open, DXFOUT, AUDIT): %d drawings, one at a time" % len(ids))
        jobs = out / "jobs1.tsv"
        jobs.write_text("".join("%s\t%s\\%s.dwg\t%s\t%s\\%s\r\n" % (i, lw, i, remote.win("scripts", "judge.scr"), lw, i)
                                for i in ids), newline="")
        _run_driver(remote, jobs, "results1.tsv", len(ids))
        remote.get("results1.tsv", out, "lanes", SLOT)
        remote.get("*.status", out, "lanes", SLOT)

        # pass 2: RECOVER for anything that did not open (not for drawings the driver never started)
        runs = read_runs(out)
        rec = [i for i in ids if read_status(out, i).get("DXFOUT_DONE") != "1"
               and not any(r["status"] in NOT_JUDGED for r in runs.get(i, []))]
        if len(rec) > MAX_RECOVER:
            _say(name, log, "%d did not open; RECOVER is tried on the first %d only" % (len(rec), MAX_RECOVER))
            for i in rec[MAX_RECOVER:]:
                (out / (i + ".status")).write_text("MODE\tnot-tried\n")
            rec = rec[:MAX_RECOVER]
        if rec:
            _say(name, log, "pass 2: %d did not open; trying RECOVER" % len(rec))
            folder = lw.replace("\\", "/")
            scripts = []
            for i in rec:
                s = out / (i + ".rec.scr")
                s.write_text(recover_script(folder, i), newline="")
                scripts.append(s)
                (out / (i + ".status")).unlink(missing_ok=True)
            remote.put(scripts, "lanes", SLOT)
            remote.ps_file("tasks.ps1", ["-Action", "forget", "-Root", remote.scratch, "-Ids", ",".join(rec)])
            jobs = out / "jobs2.tsv"
            jobs.write_text("".join("%s\t-\t%s\\%s.rec.scr\t%s\\%s.rec\r\n" % (i, lw, i, lw, i) for i in rec), newline="")
            _run_driver(remote, jobs, "results2.tsv", len(rec))
            remote.get("results2.tsv", out, "lanes", SLOT)
            for i in rec:
                remote.get(i + ".status", out, "lanes", SLOT)

        # AutoCAD's readings back, gzipped on the machine, each checked
        remote.ps_file("tasks.ps1", ["-Action", "pack", "-Root", remote.scratch])
        remote.get("*.dxf.gz", out, "lanes", SLOT)
        bad = [i for i in ids if (out / (i + ".dxf.gz")).exists() and not gz_ok(out / (i + ".dxf.gz"))]
        for attempt in range(1, 4):
            if not bad:
                break
            _say(name, log, "%d readings came back empty or cut off (disk full?); retry %d in 30 s" % (len(bad), attempt))
            time.sleep(30)
            for i in bad:
                (out / (i + ".dxf.gz")).unlink(missing_ok=True)
                remote.get(i + ".dxf.gz", out, "lanes", SLOT)
            bad = [i for i in bad if not gz_ok(out / (i + ".dxf.gz"))]
        if bad:
            raise RemoteError("could not copy %d of AutoCAD's readings back; free disk space and run again" % len(bad))
        remote.get("*.out", out, "lanes", SLOT)
        for f in out.glob("*.out"):
            # the console writes UTF-16-like output; keep the text
            f.with_suffix(".log").write_bytes(f.read_bytes().replace(b"\x00", b"").replace(b"\r", b""))
            f.unlink()
        cleaned = remote.ps_file("tasks.ps1", ["-Action", "clean", "-Root", remote.scratch]).strip()
        _say(name, log, "AutoCAD pass done: %d readings back; machine: %s" % (len(list(out.glob("*.dxf.gz"))), cleaned))
    except BaseException:
        # whatever stopped us (an error, Ctrl-C, a kill), leave nothing of ours running on the machine
        _say(name, log, "stopping; making sure nothing of the judge's is left running on the AutoCAD machine")
        try:
            remote.ps_file("tasks.ps1", ["-Action", "stop", "-Root", remote.scratch], timeout=120)
            remote.ps_file("tasks.ps1", ["-Action", "clean", "-Root", remote.scratch], timeout=120)
        except Exception:
            _say(name, log, "could not reach the machine to stop the judge there; check it by hand")
        raise
    finally:
        for f in send.iterdir():
            f.unlink()
        send.rmdir()
