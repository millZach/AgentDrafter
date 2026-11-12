#!/usr/bin/env python3
"""The speed bench: the three numbers the speed baseline is made of, measured against any agentdrafter.

  1. cold:   one command from a cold start, `agentdrafter check` on the demo plant's 8 sheets;
  2. set:    a 100-sheet set, made from the demo plant's rows, built to DWG and then plotted to PDF;
  3. viewer: a change set sent through the command line, in another process, until the open
             viewer has drawn it.

It prints one line of JSON on standard output, `{"ok": true, "result": {...}}`, with the machine it
ran on (no hostname, user name or path), and its progress on standard error. Nothing is held to a
number: it exits 1 only when it could not measure. Every file it makes is in one folder it removes
at the end. See tools/bench/README.md and docs/speed.md. Python 3.11 or newer, standard library only.
"""

from __future__ import annotations

import argparse
import base64
import ctypes
import datetime
import json
import os
import platform
import re
import shutil
import socket
import statistics
import struct
import subprocess
import sys
import tempfile
import threading
import time
import urllib.parse
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
PARTS = ("cold", "set", "viewer")
# The bench's output, in case its shape changes: a reader compares like with like.
FORMAT = 1
# The object the viewer's changes move, and the sheet it is on.
SHEET = "DP-LD-101"
DEVICE = "DP-LD-101/device"
STEP = 0.5  # inches each change moves it, right then left, so the sheet stays as it was
# Every request goes to the local machine, never through a proxy the system or environment names.
LOCAL = urllib.request.build_opener(urllib.request.ProxyHandler({}))


class BenchError(Exception):
    pass


def say(text: str) -> None:
    print("bench: %s" % text, file=sys.stderr, flush=True)


def summary(values: list[float]) -> dict:
    """The median of the runs and their spread, in milliseconds, with every run."""
    return {
        "runs": len(values),
        "median_ms": round(statistics.median(values), 1),
        "min_ms": round(min(values), 1),
        "max_ms": round(max(values), 1),
        "all_ms": [round(v, 1) for v in values],
    }


# ---------------------------------------------------------------- agentdrafter


class Agentdrafter:
    """The agentdrafter measured: a binary, or the pip package's command. Font and plot style
    folders are not passed on, so every machine draws with the same look-alikes, in monochrome."""

    def __init__(self, path: str):
        p = Path(path)
        if not p.is_file() and Path(path + ".exe").is_file():
            p = Path(path + ".exe")
        if not p.is_file():
            raise BenchError("no agentdrafter at --bin; build it (cargo build --release -p agentdrafter-cli) or name it")
        self.path = str(p.resolve())
        self.env = {k: v for k, v in os.environ.items() if k not in ("AGENTDRAFTER_FONTS", "AGENTDRAFTER_PLOT_STYLES")}

    def run(self, cwd: Path, *args: str) -> tuple[float, dict]:
        """Runs one command in a fresh process; gives the milliseconds it took and its reply."""
        began = time.perf_counter()
        r = subprocess.run([self.path, *args], cwd=cwd, env=self.env, stdin=subprocess.DEVNULL,
                           capture_output=True, timeout=600)
        ms = (time.perf_counter() - began) * 1000
        try:
            reply = json.loads(r.stdout.decode("utf-8").strip().splitlines()[-1])
        except (ValueError, IndexError):
            raise BenchError("agentdrafter %s printed no reply (exit %d): %s"
                             % (args[0], r.returncode, r.stderr.decode("utf-8", "replace")[-400:])) from None
        if not reply.get("ok"):
            raise BenchError("agentdrafter %s failed: %s" % (args[0], json.dumps(reply.get("error"))[:400]))
        return ms, reply["result"]

    def version(self, cwd: Path) -> str:
        return self.run(cwd, "--version")[1]["version"]

    def popen(self, cwd: Path, *args: str, stderr) -> subprocess.Popen:
        return subprocess.Popen([self.path, *args], cwd=cwd, env=self.env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=stderr)


def copy_plant(plant: Path, dest: Path) -> Path:
    """A copy of the demo plant's records, without anything built from them or its journal."""
    shutil.copytree(plant, dest, ignore=shutil.ignore_patterns("dwg", ".agentdrafter"))
    return dest


# ---------------------------------------------------------------- 1. one command from a cold start


def measure_cold(tool: Agentdrafter, plant: Path, work: Path, runs: int) -> dict:
    """`check` on the demo plant, each run a fresh process. Cold means nothing of agentdrafter is
    running or loaded before each run; the file system's caches are as warm as the system leaves
    them, since emptying them needs an administrator: one run first, not counted, warms them."""
    folder = work / "cold"
    folder.mkdir()
    copy_plant(plant, folder / "demo-plant")
    say("cold: agentdrafter check on the demo plant, 1 run to warm the caches, then %d" % runs)
    _, first = tool.run(folder, "check", "demo-plant")
    times = [tool.run(folder, "check", "demo-plant")[0] for _ in range(runs)]
    return {"command": "agentdrafter check demo-plant", "sheets": len(first["sheets"]), **summary(times)}


# ---------------------------------------------------------------- 2. a 100-sheet set


ROW_START = re.compile(r"^  - sheet: ", re.M)


def make_set(plant: Path, dest: Path, sheets: int) -> list[str]:
    """A drawing set of `sheets` sheets, made from the demo plant's: its templates and block library,
    and its 8 rows, each used again with its loop renumbered, in its tags, cables, wires and sheet
    name, and its sheet numbered `n OF <sheets>`. Loop 101 of area 100 becomes 105, 109, ..., so
    every number keeps its area and its three digits. Gives the sheets' names."""
    copy_plant(plant, dest)
    text = (plant / "rows.yaml").read_text(encoding="utf-8")
    starts = [m.start() for m in ROW_START.finditer(text)]
    if not starts:
        raise BenchError("the demo plant's rows.yaml holds no rows")
    head = text[:starts[0]]
    rows = []
    for a, b in zip(starts, starts[1:] + [len(text)]):
        # each row without the comments that introduce the next group of rows
        rows.append("".join(l for l in text[a:b].splitlines(True) if not l.lstrip().startswith("#")).rstrip() + "\n")
    loops = []
    for row in rows:
        m = re.search(r"^      LOOP: '(\d{3})'$", row, re.M)
        if not m:
            raise BenchError("a demo plant row has no three-digit LOOP value to renumber")
        loops.append(m.group(1))
    areas: dict[str, list[str]] = {}
    for loop in loops:
        areas.setdefault(loop[0], []).append(loop)
    out, names = [], []
    for n in range(sheets):
        row, loop = rows[n % len(rows)], loops[n % len(rows)]
        same_area = areas[loop[0]]
        number = int(loop[0]) * 100 + (n // len(rows)) * len(same_area) + same_area.index(loop) + 1
        if number % 100 == 0 or number // 100 != int(loop[0]):
            raise BenchError("too many sheets to renumber within the demo plant's areas")
        new = re.sub(r"(?<!\d)%s(?!\d)" % loop, "%03d" % number, row)
        new = re.sub(r"^      SHEET: \d+ OF \d+$", "      SHEET: %d OF %d" % (n + 1, sheets), new, flags=re.M)
        name = re.search(r"^  - sheet: (\S+)$", new, re.M).group(1)
        names.append(name)
        out.append(new)
    if len(set(names)) != len(names):
        raise BenchError("the renumbered sheets' names are not all different")
    (dest / "rows.yaml").write_text(head + "\n".join(out), encoding="utf-8")
    return names


def measure_set(tool: Agentdrafter, plant: Path, work: Path, sheets: int, runs: int) -> dict:
    """Builds the set to DWG and then plots it to PDF, each in one command, `runs` times, each time
    into a fresh copy, so every build writes every DWG anew."""
    say("set: %d sheets from the demo plant's rows, built then plotted, %d times" % (sheets, runs))
    builds, plots, totals = [], [], []
    for k in range(runs):
        folder = work / ("set-%d" % (k + 1))
        folder.mkdir()
        names = make_set(plant, folder / "plant", sheets)
        build_ms, built = tool.run(folder, "build", "plant")
        plot_ms, plotted = tool.run(folder, "plot", "plant")
        out = folder / "plant" / "dwg"
        dwgs, pdfs = sorted(out.glob("*.dwg")), sorted(out.glob("*.pdf"))
        if built.get("built") != sheets or plotted.get("plotted") != sheets or len(dwgs) != sheets or len(pdfs) != sheets:
            raise BenchError("the set did not build and plot all %d sheets: built %s, plotted %s, %d DWGs, %d PDFs"
                             % (sheets, built.get("built"), plotted.get("plotted"), len(dwgs), len(pdfs)))
        if {p.stem for p in dwgs} != set(names):
            raise BenchError("the DWGs built are not the set's sheets")
        size = (sum(p.stat().st_size for p in dwgs), sum(p.stat().st_size for p in pdfs))
        builds.append(build_ms)
        plots.append(plot_ms)
        totals.append(build_ms + plot_ms)
        say("set: run %d: build %.0f ms, plot %.0f ms" % (k + 1, build_ms, plot_ms))
        shutil.rmtree(folder, ignore_errors=True)
    return {
        "sheets": sheets,
        "commands": ["agentdrafter build <set>", "agentdrafter plot <set>"],
        "build": summary(builds),
        "plot": summary(plots),
        "total": summary(totals),
        "dwg_mb": round(size[0] / 1e6, 2),
        "pdf_mb": round(size[1] / 1e6, 2),
    }


# ---------------------------------------------------------------- 3. a change showing in the viewer


class WebSocket:
    """Just enough of a WebSocket client (RFC 6455) to speak to a browser's DevTools: text messages,
    masked as a client's must be, with pings answered."""

    def __init__(self, url: str, timeout: float = 60):
        u = urllib.parse.urlsplit(url)
        self.sock = socket.create_connection((u.hostname, u.port), timeout=timeout)
        key = base64.b64encode(os.urandom(16)).decode()
        self.sock.sendall(("GET %s HTTP/1.1\r\nHost: %s:%d\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                           "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n"
                           % (u.path or "/", u.hostname, u.port, key)).encode())
        self.buf = bytearray()
        while b"\r\n\r\n" not in self.buf:
            self._fill()
        end = self.buf.index(b"\r\n\r\n")
        head = bytes(self.buf[:end])
        del self.buf[:end + 4]
        if not head.startswith(b"HTTP/1.1 101"):
            raise BenchError("the browser refused the DevTools connection: %s" % head.split(b"\r\n")[0].decode("latin-1"))

    def _fill(self) -> None:
        chunk = self.sock.recv(1 << 16)
        if not chunk:
            raise BenchError("the browser closed the DevTools connection")
        self.buf += chunk

    def _take(self, n: int) -> bytes:
        while len(self.buf) < n:
            self._fill()
        out = bytes(self.buf[:n])
        del self.buf[:n]
        return out

    def _frame(self, opcode: int, data: bytes) -> None:
        n = len(data)
        head = bytearray([0x80 | opcode])
        if n < 126:
            head.append(0x80 | n)
        elif n < 1 << 16:
            head += bytes([0x80 | 126]) + struct.pack(">H", n)
        else:
            head += bytes([0x80 | 127]) + struct.pack(">Q", n)
        mask = os.urandom(4)
        keys = (mask * (n // 4 + 1))[:n]
        masked = (int.from_bytes(data, "big") ^ int.from_bytes(keys, "big")).to_bytes(n, "big") if n else b""
        self.sock.sendall(bytes(head) + mask + masked)

    def send(self, text: str) -> None:
        self._frame(0x1, text.encode("utf-8"))

    def recv(self) -> str:
        parts = []
        while True:
            b0, b1 = self._take(2)
            n = b1 & 0x7F
            if n == 126:
                n = struct.unpack(">H", self._take(2))[0]
            elif n == 127:
                n = struct.unpack(">Q", self._take(8))[0]
            mask = self._take(4) if b1 & 0x80 else b""
            data = self._take(n)
            if mask:
                data = bytes(c ^ mask[i % 4] for i, c in enumerate(data))
            opcode = b0 & 0x0F
            if opcode == 0x9:
                self._frame(0xA, data)
            elif opcode == 0x8:
                raise BenchError("the browser closed the DevTools connection")
            elif opcode in (0x0, 0x1, 0x2):
                parts.append(data)
                if b0 & 0x80:
                    return b"".join(parts).decode("utf-8")

    def close(self) -> None:
        try:
            self.sock.close()
        except OSError:
            pass


class DevTools:
    """One target of the browser, over the Chrome DevTools Protocol."""

    def __init__(self, url: str):
        self.ws = WebSocket(url)
        self.last = 0

    def call(self, method: str, params: dict | None = None) -> dict:
        self.last += 1
        n = self.last
        self.ws.send(json.dumps({"id": n, "method": method, "params": params or {}}))
        while True:
            message = json.loads(self.ws.recv())
            if message.get("id") == n:
                if "error" in message:
                    raise BenchError("DevTools %s: %s" % (method, message["error"].get("message")))
                return message.get("result", {})

    def eval(self, expression: str):
        r = self.call("Runtime.evaluate", {"expression": expression, "returnByValue": True, "awaitPromise": True})
        if "exceptionDetails" in r:
            d = r["exceptionDetails"]
            raise BenchError("the page threw: %s" % (d.get("exception", {}).get("description") or d.get("text")))
        return r.get("result", {}).get("value")

    def wait_for(self, expression: str, what: str, timeout: float = 60):
        deadline = time.monotonic() + timeout
        while True:
            value = self.eval(expression)
            if value:
                return value
            if time.monotonic() > deadline:
                raise BenchError("the page did not %s within %d s" % (what, timeout))
            time.sleep(0.01)


def find_browser(given: str | None) -> str:
    """A Chromium-family browser: a path, or `edge`, `chrome` or `chromium`, or the first found."""
    pf = os.environ.get("ProgramFiles", r"C:\Program Files")
    pf86 = os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")
    known = {
        "edge": [pf86 + r"\Microsoft\Edge\Application\msedge.exe", pf + r"\Microsoft\Edge\Application\msedge.exe",
                 "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge", "/usr/bin/microsoft-edge"],
        "chrome": [pf + r"\Google\Chrome\Application\chrome.exe", pf86 + r"\Google\Chrome\Application\chrome.exe",
                   "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "/usr/bin/google-chrome",
                   "/usr/bin/google-chrome-stable"],
        "chromium": ["/usr/bin/chromium", "/usr/bin/chromium-browser", "/Applications/Chromium.app/Contents/MacOS/Chromium"],
    }
    given = given or os.environ.get("CHROME")
    if given and given not in known:
        if not Path(given).is_file():
            raise BenchError("no browser at --browser")
        return given
    for name in [given] if given else ["chromium", "chrome", "edge"]:
        for path in known[name]:
            if Path(path).is_file():
                return path
    raise BenchError("no Chromium, Chrome or Edge found; name one with --browser")


class Browser:
    """A headless Chromium-family browser with a profile of its own, driven over DevTools."""

    def __init__(self, path: str, profile: Path, gl: str, log):
        flags = ["--use-angle=swiftshader", "--enable-unsafe-swiftshader"] if gl == "software" else (
            ["--use-angle=vulkan", "--enable-features=Vulkan", "--ignore-gpu-blocklist"] if sys.platform.startswith("linux")
            else ["--ignore-gpu-blocklist"])
        args = [path, "--headless", "--remote-debugging-port=0", "--user-data-dir=%s" % profile,
                "--no-first-run", "--no-default-browser-check", "--disable-extensions", "--disable-sync",
                "--disable-background-networking", "--disable-component-update",
                "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
                "--disable-backgrounding-occluded-windows", "--window-size=1440,900", *flags]
        if sys.platform.startswith("linux"):
            args.append("--no-sandbox")
        self.proc = subprocess.Popen(args + ["about:blank"], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=log)
        port_file = profile / "DevToolsActivePort"
        deadline = time.monotonic() + 60
        while not (port_file.is_file() and len(port_file.read_text().split()) >= 2):
            if self.proc.poll() is not None:
                raise BenchError("the browser stopped as it started (exit %d)" % self.proc.returncode)
            if time.monotonic() > deadline:
                raise BenchError("the browser did not open DevTools within 60 s")
            time.sleep(0.05)
        port, path_ = port_file.read_text().split()[:2]
        self.http = "http://127.0.0.1:%s" % port
        self.browser_ws = "ws://127.0.0.1:%s%s" % (port, path_)
        self.version = json.loads(LOCAL.open(self.http + "/json/version", timeout=30).read())

    def page(self) -> DevTools:
        deadline = time.monotonic() + 30
        while True:
            targets = json.loads(LOCAL.open(self.http + "/json/list", timeout=30).read())
            pages = [t for t in targets if t.get("type") == "page" and t.get("webSocketDebuggerUrl")]
            if pages:
                return DevTools(pages[0]["webSocketDebuggerUrl"])
            if time.monotonic() > deadline:
                raise BenchError("the browser opened no page")
            time.sleep(0.1)

    def close(self) -> None:
        try:
            DevTools(self.browser_ws).ws.send(json.dumps({"id": 1, "method": "Browser.close"}))
            self.proc.wait(timeout=15)
        except Exception:
            pass
        if self.proc.poll() is None:
            self.proc.kill()
            self.proc.wait(timeout=15)


class Server:
    """`agentdrafter serve` on a free port of the local machine."""

    def __init__(self, tool: Agentdrafter, cwd: Path, log):
        self.proc = tool.popen(cwd, "serve", "--listen", "127.0.0.1:0", stderr=log)
        lines: list[bytes] = []
        got = threading.Event()

        def read():
            for line in self.proc.stdout:  # drained to the end, so the server never blocks on it
                if not lines:
                    lines.append(line)
                    got.set()
            got.set()

        threading.Thread(target=read, daemon=True).start()
        if not got.wait(60) or not lines:
            self.stop()
            raise BenchError("agentdrafter serve did not start")
        started = json.loads(lines[0])
        if not started.get("ok"):
            self.stop()
            raise BenchError("agentdrafter serve did not start: %s" % json.dumps(started.get("error"))[:300])
        self.base = started["result"]["listening"]

    def stop(self) -> None:
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=15)


# Set in the page: from now on, when the sheet is loaded with the device at `want`, note the time,
# and then the end of the first draw of the canvas after it. The viewer's probe sets `summary` as it
# loads a sheet's display list; `takeDraws` gives each draw of the canvas since it was last called.
ARM = """(() => {
  const p = window.agentdrafterViewer, place = %d, want = %r;
  const b = (window.__agentdrafterBench = { start: 0, loaded: null, drawn: null });
  const moved = () => { const x = p.bounds([place]); return x.length === 4 && Math.abs(x[0] - want) < 0.05; };
  let s = p.summary;
  Object.defineProperty(p, 'summary', { configurable: true, enumerable: true, get: () => s,
    set: (v) => { s = v; if (b.loaded === null && moved()) b.loaded = performance.now(); } });
  p.takeDraws();
  const step = () => {
    if (b.loaded !== null) for (const d of p.takeDraws()) if (b.drawn === null && d.at >= b.loaded) b.drawn = d.at + d.ms;
    if (b.drawn === null) requestAnimationFrame(step);
  };
  requestAnimationFrame(step);
  b.start = performance.now();
  return true;
})()"""
DISARM = """(() => {
  const p = window.agentdrafterViewer, s = p.summary;
  Object.defineProperty(p, 'summary', { value: s, writable: true, configurable: true, enumerable: true });
  return true;
})()"""
DEVICE_NOW = """(() => {
  const p = window.agentdrafterViewer;
  const place = p.summary.objects.findIndex((o) => o.id === %r);
  return { place, bounds: place >= 0 ? p.bounds([place]) : [] };
})()""" % DEVICE
WEBGL = """(() => {
  const gl = document.createElement('canvas').getContext('webgl2');
  if (!gl) return null;
  const info = gl.getExtension('WEBGL_debug_renderer_info');
  return gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER);
})()"""


def measure_viewer(tool: Agentdrafter, plant: Path, work: Path, browser_path: str, changes: int, gl: str) -> tuple[dict, dict]:
    """The viewer open on a demo plant sheet, at laptop size; each change set moves one insert, sent
    through the command line in another process, and is timed from just before it is sent until the
    end of the page's first draw of the canvas after the page loaded the sheet with the insert at
    its new place. One change first, not counted, as the cold command's first run is not. The
    changes are spaced so that they reach the server at every point of its watch on the journal."""
    folder = work / "viewer"
    folder.mkdir()
    copy_plant(plant, folder / "demo-plant")
    tool.run(folder, "build", "demo-plant")
    for k, dx in enumerate((STEP, -STEP)):
        (folder / ("move-%d.json" % k)).write_text(json.dumps(
            {"operations": [{"op": "move", "target": {"id": DEVICE}, "by": [dx, 0, 0]}]}))
    log = open(work / "viewer.log", "wb")
    server = browser = None
    try:
        server = Server(tool, folder, log)
        page_text = LOCAL.open(server.base + "/", timeout=30).read()
        if b"viewer: not built" in page_text:
            raise BenchError("this agentdrafter was built without the viewer's page (docs/viewer.md); "
                             "build the page in, or leave the viewer out with --only cold,set")
        say("viewer: %s, headless, %s WebGL" % (Path(browser_path).stem, gl))
        browser = Browser(browser_path, work / "browser-profile", gl, log)
        page = browser.page()
        page.call("Page.navigate", {"url": "%s/?source=demo-plant&sheet=%s" % (server.base, SHEET)})
        page.wait_for("window.agentdrafterViewer?.summary?.sheet === %r && !!document.querySelector('.va-agent.working')"
                      % SHEET, "draw the sheet and open its live stream")
        # The browser's own name for its version; a user agent string hides all but the first number.
        about = {"name": Path(browser_path).stem, "product": browser.version.get("Browser", ""),
                 "webgl": page.eval(WEBGL) or "none"}
        say("viewer: 1 change to warm up, then %d" % changes)
        shown, sent, sending = [], [], []
        for k in range(changes + 1):
            now = page.eval(DEVICE_NOW)
            if now["place"] < 0 or len(now["bounds"]) != 4:
                raise BenchError("the device is not on the sheet shown")
            dx = STEP if k % 2 == 0 else -STEP
            page.eval(ARM % (now["place"], now["bounds"][0] + dx))
            took, reply = tool.run(folder, "change", "demo-plant", "move-%d.json" % (k % 2))
            if reply.get("landed") != 1 or reply.get("failed"):
                raise BenchError("a change set did not land on its sheet: %s" % reply.get("message"))
            b = page.wait_for("(() => { const b = window.__agentdrafterBench; return b.drawn !== null && b; })()",
                              "draw the change", timeout=30)
            page.eval(DISARM)
            if k > 0:
                shown.append(b["drawn"] - b["start"])
                sending.append(took)
                sent.append(b["loaded"] - b["start"])
            # Let the page finish what a change set brings, its checks and its halos, before the next;
            # and send each change at another point of the server's watch on the journal, which looks
            # at it a few times a second, by waiting a second and a share of a quarter second that
            # differs every time (k times the golden ratio, its fraction), so the median is not the
            # wait of one point of it but of all of them.
            time.sleep(1.0 + 0.25 * ((k * 0.6180339887) % 1.0))
        result = {
            "sheet": SHEET,
            "change": "move one insert %g in, by `agentdrafter change` in another process" % STEP,
            "window": "1440 by 900, headless",
            "gl": gl,
            **summary(shown),
            "change_command_median_ms": round(statistics.median(sending), 1),
            "loaded_median_ms": round(statistics.median(sent), 1),
        }
        return result, about
    finally:
        if browser:
            browser.close()
        if server:
            server.stop()
        log.close()


# ---------------------------------------------------------------- the machine


def run_text(args: list[str], timeout: float = 60) -> str:
    try:
        r = subprocess.run(args, stdin=subprocess.DEVNULL, capture_output=True, timeout=timeout)
        return r.stdout.decode("utf-8", "replace").strip() if r.returncode == 0 else ""
    except (OSError, subprocess.TimeoutExpired):
        return ""


WINDOWS_MACHINE = r"""
$ErrorActionPreference = 'SilentlyContinue'
$p = @(Get-CimInstance Win32_Processor)
$os = Get-CimInstance Win32_OperatingSystem
$cs = Get-CimInstance Win32_ComputerSystem
$disk = Get-Partition -DriveLetter '%s' | Get-Disk
$media = (Get-PhysicalDisk | Where-Object { $_.DeviceId -eq [string]$disk.Number }).MediaType
[pscustomobject]@{
  cpu = $p[0].Name.Trim(); cores = ($p | Measure-Object NumberOfCores -Sum).Sum
  threads = ($p | Measure-Object NumberOfLogicalProcessors -Sum).Sum; ram = $cs.TotalPhysicalMemory
  os = ($os.Caption + ' ' + $os.Version); bus = [string]$disk.BusType; media = [string]$media
} | ConvertTo-Json -Compress
"""


def machine(work: Path) -> dict:
    """What the numbers were measured on: no hostname, user name or path."""
    info = {"cpu": "unknown", "cores": None, "threads": os.cpu_count(), "ram_gb": None, "os": platform.platform(terse=True),
            "disk": "unknown", "power": "unknown", "python": platform.python_version()}
    try:
        if sys.platform == "win32":
            drive = str(work.resolve())[0]
            script = base64.b64encode((WINDOWS_MACHINE % drive).encode("utf-16-le")).decode()
            got = run_text(["powershell", "-NoProfile", "-NonInteractive", "-EncodedCommand", script], 120)
            w = json.loads(got) if got else {}
            info.update(cpu=w.get("cpu") or "unknown", cores=int(w["cores"]) if w.get("cores") else None,
                        threads=int(w.get("threads") or os.cpu_count()), os=w.get("os") or info["os"])
            if w.get("ram"):
                info["ram_gb"] = round(int(w["ram"]) / 2**30, 1)
            kind = {"4": "SSD", "3": "HDD", "SSD": "SSD", "HDD": "HDD"}.get(w.get("media", ""), "")
            bus = {"17": "NVMe", "11": "SATA", "7": "USB", "NVMe": "NVMe", "SATA": "SATA", "USB": "USB"}.get(w.get("bus", ""), "")
            info["disk"] = " ".join(x for x in (bus, kind) if x) or "unknown"
            info["power"] = windows_power()
        elif sys.platform.startswith("linux"):
            cpuinfo = Path("/proc/cpuinfo").read_text()
            names = re.findall(r"^model name\s*:\s*(.+)$", cpuinfo, re.M)
            info["cpu"] = names[0].strip() if names else "unknown"
            cores = set(re.findall(r"^physical id\s*:\s*(\d+)$[\s\S]*?^core id\s*:\s*(\d+)$", cpuinfo, re.M))
            info["cores"] = len(cores) or None
            mem = re.search(r"^MemTotal:\s*(\d+) kB$", Path("/proc/meminfo").read_text(), re.M)
            info["ram_gb"] = round(int(mem.group(1)) / 2**20, 1) if mem else None
            release = Path("/etc/os-release").read_text() if Path("/etc/os-release").is_file() else ""
            pretty = re.search(r'^PRETTY_NAME="?([^"\n]+)"?$', release, re.M)
            info["os"] = "%s, Linux %s" % (pretty.group(1) if pretty else "Linux", platform.release())
            info["disk"] = linux_disk(work)
            info["power"] = linux_power()
        elif sys.platform == "darwin":
            info["cpu"] = run_text(["sysctl", "-n", "machdep.cpu.brand_string"]) or "unknown"
            info["cores"] = int(run_text(["sysctl", "-n", "hw.physicalcpu"]) or 0) or None
            info["threads"] = int(run_text(["sysctl", "-n", "hw.logicalcpu"]) or 0) or os.cpu_count()
            info["ram_gb"] = round(int(run_text(["sysctl", "-n", "hw.memsize"]) or 0) / 2**30, 1) or None
            info["os"] = "macOS %s" % platform.mac_ver()[0]
            disk = run_text(["diskutil", "info", "/"])
            solid = re.search(r"Solid State:[ \t]*(\w+)", disk)
            proto = re.search(r"Protocol:[ \t]*(\S[^\n]*)", disk)
            info["disk"] = " ".join(x for x in ((proto.group(1).strip() if proto else ""),
                                                ("SSD" if solid and solid.group(1) == "Yes" else "")) if x) or "unknown"
    except Exception as e:  # what the machine is never stops the bench
        say("could not read all of the machine: %s" % e)
    return info


def linux_disk(work: Path) -> str:
    source = run_text(["findmnt", "-n", "-o", "SOURCE", "--target", str(work)]).split("[")[0]
    if not source.startswith("/dev/"):
        return "unknown"
    for line in run_text(["lsblk", "-nsro", "TYPE,ROTA,TRAN", source]).splitlines():
        f = line.split()
        if f and f[0] == "disk":
            kind = "HDD" if len(f) > 1 and f[1] == "1" else "SSD"
            tran = f[2].upper() if len(f) > 2 else ""
            return ("NVMe SSD" if tran == "NVME" else "%s %s" % (tran, kind)).strip()
    return "unknown"


def linux_power() -> str:
    supplies = list(Path("/sys/class/power_supply").glob("*")) if Path("/sys/class/power_supply").is_dir() else []
    kinds = {p: (p / "type").read_text().strip() for p in supplies if (p / "type").is_file()}
    if "Battery" not in kinds.values():
        return "mains, no battery"
    mains = [p for p, k in kinds.items() if k == "Mains" and (p / "online").is_file()]
    return "mains" if any((p / "online").read_text().strip() == "1" for p in mains) else "battery"


def windows_power() -> str:
    class Status(ctypes.Structure):
        _fields_ = [("ac", ctypes.c_ubyte), ("flag", ctypes.c_ubyte), ("percent", ctypes.c_ubyte),
                    ("saver", ctypes.c_ubyte), ("life", ctypes.c_ulong), ("full", ctypes.c_ulong)]
    s = Status()
    if not ctypes.windll.kernel32.GetSystemPowerStatus(ctypes.byref(s)):
        return "unknown"
    if s.flag == 128:
        return "mains, no battery"
    return {0: "battery", 1: "mains"}.get(s.ac, "unknown")


# ---------------------------------------------------------------- the run


def main(argv=None) -> int:
    p = argparse.ArgumentParser(prog="bench.py", description=__doc__.split("\n\n")[0],
                                formatter_class=argparse.RawDescriptionHelpFormatter, epilog=__doc__.split("\n\n", 1)[1])
    p.add_argument("--bin", default=str(REPO / "target" / "release" / "agentdrafter"),
                   help="the agentdrafter to measure (default: target/release/agentdrafter)")
    p.add_argument("--only", default=",".join(PARTS), help="which numbers, of cold, set and viewer (default: all three)")
    p.add_argument("--runs", type=int, default=10, help="cold: runs counted, after one that warms the caches (default 10)")
    p.add_argument("--sheets", type=int, default=100, help="set: how many sheets (default 100)")
    p.add_argument("--set-runs", type=int, default=3, help="set: how many times it is built and plotted (default 3)")
    p.add_argument("--changes", type=int, default=10, help="viewer: changes counted, after one to warm up (default 10)")
    p.add_argument("--browser", help="viewer: a Chromium, Chrome or Edge, by path or as chromium, chrome or edge "
                                     "(default: $CHROME, else the first found)")
    p.add_argument("--gl", choices=("software", "gpu"), default="software",
                   help="viewer: WebGL in software (SwiftShader, the same on every machine; default) or on the GPU")
    p.add_argument("--plant", default=str(REPO / "examples" / "demo-plant"), help="the demo plant's folder")
    p.add_argument("--work", help="an empty folder to work in, removed at the end (default: a new temporary one)")
    p.add_argument("--keep", action="store_true", help="keep the working folder")
    args = p.parse_args(argv)
    only = [x.strip() for x in args.only.split(",") if x.strip()]
    work = None
    try:
        unknown = [x for x in only if x not in PARTS]
        if unknown or not only:
            raise BenchError("--only takes cold, set and viewer")
        if min(args.runs, args.sheets, args.set_runs, args.changes) < 1:
            raise BenchError("--runs, --sheets, --set-runs and --changes are at least 1")
        plant = Path(args.plant)
        if not (plant / "rows.yaml").is_file():
            raise BenchError("no demo plant at --plant")
        if args.work:
            given = Path(args.work)
            # Refused before it is ours: a folder that holds anything is never removed at the end.
            if given.exists() and (not given.is_dir() or any(given.iterdir())):
                raise BenchError("--work must be an empty folder")
            given.mkdir(parents=True, exist_ok=True)
            work = given
        else:
            work = Path(tempfile.mkdtemp(prefix="agentdrafter-bench-"))
        tool = Agentdrafter(args.bin)
        browser = find_browser(args.browser) if "viewer" in only else None
        result = {
            "format": FORMAT,
            "measured": datetime.datetime.now(datetime.timezone.utc).isoformat(timespec="seconds"),
            "agentdrafter": tool.version(work),
            "machine": machine(work),
        }
        if "cold" in only:
            result["cold"] = measure_cold(tool, plant, work, args.runs)
        if "set" in only:
            result["set"] = measure_set(tool, plant, work, args.sheets, args.set_runs)
        if "viewer" in only:
            result["viewer"], result["browser"] = measure_viewer(tool, plant, work, browser, args.changes, args.gl)
        print(json.dumps({"ok": True, "result": result}), flush=True)
        return 0
    except BenchError as e:
        say(str(e))
        print(json.dumps({"ok": False, "error": {"message": str(e)}}), flush=True)
        return 1
    finally:
        if work is not None and not args.keep:
            for _ in range(10):
                shutil.rmtree(work, ignore_errors=True)
                if not work.exists():
                    break
                time.sleep(1)  # a browser that has just closed may still hold its profile on Windows


if __name__ == "__main__":
    sys.exit(main())
