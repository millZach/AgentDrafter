#!/usr/bin/env python3
"""Runs the speed bench once on the Windows laptop, from forge, under the judge's lock.

  laptop.py --binary <agentdrafter.exe> [--wheel <agentdrafter-...-win_amd64.whl>]
            [--browser edge|chrome] [--wait <minutes>] [-- <bench options>]

It takes the judge's lock, so no judge run uses the laptop meanwhile, and waits, as the judge does,
until no AutoCAD console runs there and at least 3 GB of memory is free. Then it makes one
temporary folder on the laptop and copies the bench, the demo plant and the Windows binary CI built
with the viewer's page (the `Windows binary` workflow) into it, and runs the bench there with the
laptop's own Python and its own Edge or Chrome: all three numbers, against the binary. Given the
Windows wheel CI built too, it installs it into a virtual environment inside the folder and measures
the cold command and the set through the pip package's command as well. At the end it stops
anything still running from the folder and removes it. It installs nothing outside that folder and
builds nothing there. It prints the bench's JSON, the wheel's numbers under `wheel` and what it
found on the laptop under `laptop`. The laptop is the judge's host, from the judge's settings
(tools/judge/README.md). Never run in CI.
"""

from __future__ import annotations

import argparse
import base64
import contextlib
import json
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO / "tools" / "judge"))
import judge  # noqa: E402  the judge's lock
from judgelib import settings as settings_mod  # noqa: E402
from judgelib.autocad import SSH_OPTS  # noqa: E402

MIN_FREE_KB = 3 * 1024 * 1024  # as the judge's driver asks before each AutoCAD console
RUN_TIMEOUT = 900  # the bench takes a few minutes; this is far past it


class LaptopError(Exception):
    pass


def say(text: str) -> None:
    print("laptop: %s" % text, file=sys.stderr, flush=True)


def q(text: str) -> str:
    """A PowerShell string in single quotes."""
    return "'%s'" % text.replace("'", "''")


class Laptop:
    def __init__(self, host: str):
        self.host = host

    def ps(self, code: str, timeout: float, stderr=subprocess.PIPE) -> str:
        """Runs PowerShell on the laptop; gives what it printed. Raises if it failed."""
        enc = base64.b64encode(code.encode("utf-16-le")).decode()
        try:
            r = subprocess.run(["ssh", *SSH_OPTS, self.host, "powershell -NoProfile -NonInteractive -EncodedCommand " + enc],
                               stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=stderr, timeout=timeout)
        except subprocess.TimeoutExpired:
            raise LaptopError("timed out after %d s on the laptop" % timeout) from None
        out = r.stdout.decode("utf-8", "replace").replace("\r", "")
        if r.returncode != 0:
            raise LaptopError("PowerShell on the laptop failed (exit %d)%s" % (
                r.returncode, ": " + r.stderr.decode("utf-8", "replace")[-300:] if r.stderr else ""), out)
        return out

    def put(self, files: list[Path], folder: str) -> None:
        r = subprocess.run(["scp", "-q", "-r", *SSH_OPTS, *map(str, files), "%s:/%s/" % (self.host, folder.replace("\\", "/"))],
                           stdin=subprocess.DEVNULL, capture_output=True, timeout=600)
        if r.returncode != 0:
            raise LaptopError("copying to the laptop failed (exit %d)" % r.returncode)


WAIT = """
$deadline = (Get-Date).AddSeconds(%d); $waited = [Diagnostics.Stopwatch]::StartNew()
while ($true) {
  $busy = @(Get-Process -Name accoreconsole -ErrorAction SilentlyContinue).Count -gt 0
  $free = [int64](Get-CimInstance Win32_OperatingSystem).FreePhysicalMemory
  $turn = if ($busy) { 'BUSY' } elseif ($free -lt %d) { 'LOWMEM' } else { 'GO' }
  if ($turn -eq 'GO' -or (Get-Date) -gt $deadline) { "$turn $free $([int]$waited.Elapsed.TotalSeconds)"; break }
  Start-Sleep -Seconds 5
}
"""
MAKE = """
$d = Join-Path ([IO.Path]::GetTempPath()) ('agentdrafter-bench-' + [guid]::NewGuid().ToString('N').Substring(0, 12))
New-Item -ItemType Directory -Path $d | Out-Null
[Console]::Out.Write($d)
"""
RUN = """
$ErrorActionPreference = 'Continue'
$d = %(dir)s
$env:TEMP = Join-Path $d 'tmp'; $env:TMP = $env:TEMP
New-Item -ItemType Directory -Force -Path $env:TEMP | Out-Null
$env:PIP_DISABLE_PIP_VERSION_CHECK = '1'; $env:PIP_NO_CACHE_DIR = '1'; $env:PYTHONDONTWRITEBYTECODE = '1'
function SysPy { if (Get-Command py -ErrorAction SilentlyContinue) { & py -3 @args } else { & python @args } }
function Bench($bin, $name, $more) {
  SysPy (Join-Path $d 'bench.py') --bin $bin --plant (Join-Path $d 'demo-plant') --work (Join-Path $d ('work-' + $name)) `
    --browser %(browser)s @more > (Join-Path $d ($name + '.json'))
}
if (%(binary)s) { Bench (Join-Path $d 'agentdrafter.exe') 'binary' @(%(extra)s) }
if (%(wheel)s) {
  $venv = Join-Path $d 'venv'
  SysPy -m venv $venv *> (Join-Path $d 'venv.log')
  & (Join-Path $venv 'Scripts\\python.exe') -m pip install --no-index --no-deps (Join-Path $d %(wheel)s) *> (Join-Path $d 'pip.log')
  if ($LASTEXITCODE) { [Console]::Error.WriteLine('the wheel did not install') }
  else { Bench (Join-Path $venv 'Scripts\\agentdrafter.exe') 'wheel' @(%(wheel_extra)s) }
}
foreach ($name in 'binary', 'wheel') {
  $p = Join-Path $d ($name + '.json')
  if (Test-Path -LiteralPath $p) { $t = Get-Content -LiteralPath $p -Raw; if ($t) { [Console]::Out.WriteLine($name + ' ' + $t.Trim()) } }
}
"""
CLEAN = """
$d = %s
$ours = @(Get-CimInstance Win32_Process | Where-Object { $_.CommandLine -and $_.CommandLine.IndexOf($d, [StringComparison]::OrdinalIgnoreCase) -ge 0 })
foreach ($p in $ours) { Stop-Process -Id $p.ProcessId -Force -ErrorAction SilentlyContinue }
for ($i = 0; $i -lt 10 -and (Test-Path -LiteralPath $d); $i++) {
  Remove-Item -LiteralPath $d -Recurse -Force -ErrorAction SilentlyContinue
  if (Test-Path -LiteralPath $d) { Start-Sleep -Seconds 2 }
}
$left = Test-Path -LiteralPath $d
$consoles = @(Get-Process -Name accoreconsole -ErrorAction SilentlyContinue).Count
"STOPPED $($ours.Count) LEFT $left CONSOLES $consoles"
"""


def stage(binary: Path | None, wheel: Path | None, into: Path) -> list[Path]:
    """What goes to the laptop: the bench, the demo plant's records, the binary and the wheel."""
    shutil.copy2(REPO / "tools" / "bench" / "bench.py", into / "bench.py")
    shutil.copytree(REPO / "examples" / "demo-plant", into / "demo-plant", ignore=shutil.ignore_patterns("dwg", ".agentdrafter"))
    files = [into / "bench.py", into / "demo-plant"]
    if binary:
        shutil.copy2(binary, into / "agentdrafter.exe")
        files.append(into / "agentdrafter.exe")
    if wheel:
        shutil.copy2(wheel, into / wheel.name)
        files.append(into / wheel.name)
    return files


def script_for(binary: Path | None, wheel: Path | None, browser: str, extra: list[str]) -> str:
    """The run's PowerShell, with the folder still to fill in as `%(dir)s`."""
    return RUN % {
        "dir": "%(dir)s", "browser": q(browser), "binary": "$true" if binary else "$false",
        "wheel": q(wheel.name) if wheel else "$false", "extra": ", ".join(q(a) for a in extra),
        "wheel_extra": ", ".join(q(a) for a in extra + ["--only", "cold,set"]),
    }


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="laptop.py", description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__.split("\n\n", 1)[1])
    p.add_argument("--binary", help="agentdrafter.exe with the viewer's page, from CI's `Windows binary` workflow")
    p.add_argument("--wheel", help="also the Windows wheel from CI's `wheel (windows-latest)` job, for the cold command "
                                   "and the set through the pip package's command")
    p.add_argument("--browser", choices=("edge", "chrome"), default="edge", help="the laptop's own browser (default: edge)")
    p.add_argument("--wait", type=float, default=20, help="minutes to wait for AutoCAD and memory to be free (default 20)")
    p.add_argument("bench", nargs=argparse.REMAINDER, help="after --: options for bench.py, such as --runs 20")
    args = p.parse_args(argv)
    extra = args.bench[1:] if args.bench[:1] == ["--"] else args.bench
    binary = Path(args.binary) if args.binary else None
    wheel = Path(args.wheel) if args.wheel else None
    try:
        if not binary and not wheel:
            raise LaptopError("name the binary with --binary, the wheel with --wheel, or both")
        if binary and (not binary.is_file() or binary.suffix.lower() != ".exe" or binary.read_bytes()[:2] != b"MZ"):
            raise LaptopError("--binary must be agentdrafter.exe for Windows")
        if wheel and (not wheel.is_file() or not wheel.name.endswith("win_amd64.whl")):
            raise LaptopError("--wheel must be agentdrafter's Windows wheel, a file ending win_amd64.whl")
        cfg = settings_mod.load(repo_root=REPO)
        cfg.require("dir", "host")
        laptop = Laptop(cfg.host)
        script = script_for(binary, wheel, args.browser, extra)
        # The judge's lock says when it waits, on standard output, which here holds only the JSON.
        with tempfile.TemporaryDirectory(prefix="agentdrafter-bench-stage-") as staged, \
                contextlib.redirect_stdout(sys.stderr), judge.judge_lock(cfg.dir):
            files = stage(binary, wheel, Path(staged))
            say("holding the judge's lock; waiting until no AutoCAD console runs and 3 GB is free")
            turn, free_kb, waited = laptop.ps(WAIT % (int(args.wait * 60), MIN_FREE_KB), args.wait * 60 + 120).split()
            if turn != "GO":
                raise LaptopError("the laptop is %s after %s s; nothing was run" % (
                    "running an AutoCAD console" if turn == "BUSY" else "short of memory", waited))
            say("the laptop is free after %s s; making one temporary folder there" % waited)
            folder = laptop.ps(MAKE, 60).strip()
            if not folder or " " in folder or "'" in folder:
                raise LaptopError("the laptop's temporary folder cannot be copied to by scp")
            replies = {}
            try:
                laptop.put(files, folder)
                say("running the bench there")
                try:
                    out = laptop.ps(script % {"dir": q(folder)}, RUN_TIMEOUT, stderr=None)
                except LaptopError as e:
                    out = e.args[1] if len(e.args) > 1 else ""
                for line in out.splitlines():
                    name, _, text = line.partition(" ")
                    if name in ("binary", "wheel") and text.startswith("{"):
                        replies[name] = json.loads(text)
            finally:
                cleaned = laptop.ps(CLEAN % q(folder), 180).split()
                removed = cleaned[3] == "False" if len(cleaned) > 3 else False
                say("the temporary folder %s; %s of the bench's processes stopped" % (
                    "is removed" if removed else "could NOT be removed", cleaned[1] if len(cleaned) > 1 else "?"))
        wanted = [n for n, given in (("binary", binary), ("wheel", wheel)) if given]
        missing = [n for n in wanted if n not in replies]
        if missing:
            raise LaptopError("the bench printed nothing on the laptop for the %s" % " and the ".join(missing))
        reply = replies.get("binary") or replies["wheel"]
        if reply.get("ok"):
            result = reply["result"]
            if binary and wheel:
                w = replies["wheel"]
                result["wheel"] = ({k: w["result"][k] for k in ("agentdrafter", "cold", "set")} if w.get("ok")
                                   else {"error": w.get("error")})
            result["laptop"] = {
                "how": "from forge over ssh, under the judge's lock",
                "binary": "agentdrafter.exe" if binary else None,
                "wheel": wheel.name if wheel else None,
                "waited_s": int(waited),
                "free_gb_at_start": round(int(free_kb) / 2**20, 1),
                "autocad_consoles_at_end": int(cleaned[5]) if len(cleaned) > 5 else None,
                "folder_removed": removed,
            }
        print(json.dumps(reply), flush=True)
        return 0 if all(replies[n].get("ok") for n in wanted) else 1
    except (LaptopError, settings_mod.SettingsError) as e:
        say(str(e.args[0]))
        return 2


if __name__ == "__main__":
    sys.exit(main())
