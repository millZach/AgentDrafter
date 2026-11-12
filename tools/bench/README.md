# The speed bench

`bench.py` measures the three numbers of the speed baseline ([`docs/speed.md`](../../docs/speed.md)) against any `agentdrafter`, on any machine, and prints them as one line of JSON. It needs Python 3.11 or newer and nothing outside its standard library; the viewer's number also needs a Chromium, Chrome or Edge, which it drives itself over the browser's DevTools protocol. Nothing is held to a number: it exits 1 only when it could not measure.

```
python3 tools/bench/bench.py [--bin <agentdrafter>] [--only cold,set,viewer] [--browser <path | chromium | chrome | edge>]
                             [--runs 10] [--sheets 100] [--set-runs 3] [--changes 10] [--gl software|gpu]
                             [--plant <demo plant>] [--work <empty folder>] [--keep]
```

`--bin` is the `agentdrafter` to measure: by default `target/release/agentdrafter`, which `cargo build --release -p agentdrafter-cli` builds, with the viewer's page built in first (`npm ci` and `npm run build` in `viewer/`, [`docs/viewer.md`](../../docs/viewer.md#building-it)). It can be the pip package's command too, such as `.venv/try/bin/agentdrafter`. The bench passes on neither `AGENTDRAFTER_FONTS` nor `AGENTDRAFTER_PLOT_STYLES`, so every machine draws with the same look-alike fonts, in monochrome.

## What it measures

1. **`cold`: one command from a cold start.** `agentdrafter check` on a copy of the demo plant, which lays out its 8 sheets through the drawing pipeline and checks them, as an agent checks its work. Each run is a fresh process, with nothing of AgentDrafter running or loaded before it. The file system's caches are as warm as the system leaves them, since emptying them needs an administrator: one run first, not counted, warms them, as it does the virus scanner's verdict on Windows. It reports the median of `--runs` runs, the fastest, the slowest and every run.
2. **`set`: a 100-sheet set built to DWG and PDF.** The bench makes the set in its working folder from the demo plant: its templates and block library, and its 8 rows used again, each with its loop renumbered in its tags, cables, wires, points and sheet name, keeping its area and three digits (loop 101 becomes 105, 109, ...), and each sheet numbered `n OF 100`. Then it times `agentdrafter build` on the set, then `agentdrafter plot`, each one command over all 100 sheets, and checks that every DWG and PDF was written. It does this `--set-runs` times, each in a fresh copy, and reports build, plot and their total. The sheets are never kept.
3. **`viewer`: a change showing in the viewer.** It builds a copy of the demo plant, starts `agentdrafter serve` on a free port of the local machine, and opens the viewer on `DP-LD-101` in a headless browser with a profile of its own, in a 1440 by 900 window. For each change it moves the sheet's device by half an inch, right and then left, with `agentdrafter change` run in another process, as an agent would. It times each from just before the change is sent until the end of the page's first draw of the canvas after the page loaded the sheet with the device at its new place. It learns both from the page's own probe, `window.agentdrafterViewer` (`viewer/src/probe.ts`): the sheet's `summary`, set as the page loads a display list, and `takeDraws()`, each draw of the canvas. One change first is not counted; it reports the median of `--changes` changes, with how long the change command itself took and when the page had loaded the sheet. WebGL is software, SwiftShader, by default, the same on every machine, as the viewer's browser tests draw; `--gl gpu` draws with the machine's GPU.

Every file it makes is in one working folder, a new temporary one or `--work`, which it removes at the end unless `--keep` is given.

## What it prints

```json
{"ok": true, "result": {
  "format": 1, "measured": "2026-11-12T05:10:00+00:00", "agentdrafter": "0.0.0",
  "machine": {"cpu": "...", "cores": 10, "threads": 16, "ram_gb": 15.4, "os": "...", "disk": "NVMe SSD", "power": "mains, no battery", "python": "3.14.7"},
  "cold": {"command": "agentdrafter check demo-plant", "sheets": 8, "runs": 10, "median_ms": 45.6, "min_ms": 39.0, "max_ms": 71.7, "all_ms": [...]},
  "set": {"sheets": 100, "build": {"median_ms": ...}, "plot": {"median_ms": ...}, "total": {"median_ms": ...}, "dwg_mb": 3.07, "pdf_mb": 4.11},
  "viewer": {"sheet": "DP-LD-101", "gl": "software", "runs": 10, "median_ms": 176.3, "min_ms": ..., "max_ms": ..., "change_command_median_ms": 52.7, "loaded_median_ms": 172.3},
  "browser": {"name": "chromium", "product": "Chrome/152.0.7977.82", "webgl": "ANGLE (... SwiftShader driver)"}}}
```

The machine is described with no hostname, user name or path: the processor, its cores and threads, the memory, the system, the kind of disk the working folder is on, whether it runs on mains or battery, and the browser's version. When it could not measure, it prints `{"ok": false, "error": {"message": ...}}` and exits 1. Its progress goes to standard error.

## On the laptop

`laptop.py` runs the bench once on Zach's Windows laptop from forge, as the speed baseline is measured at a release or when Zach asks ([`docs/speed.md`](../../docs/speed.md)). It is never run in CI, and builds nothing on the laptop. It measures a Windows `agentdrafter.exe` that CI built, with the viewer's page in: the artifact `agentdrafter-windows-latest` of the `Windows binary` workflow (`.github/workflows/windows-binary.yml`), which runs on each push to `main` and when asked, and is not on the merge bar. CI's wheel is built without the page, since the required jobs need no Node ([`docs/viewer.md`](../../docs/viewer.md#building-it)), so it cannot measure the viewer; given too, the wheel is measured for the cold command and the set through the pip package's command.

```
run=$(gh run list -R millZach/AgentDrafter --workflow "Windows binary" --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" -R millZach/AgentDrafter --name agentdrafter-windows-latest --dir target/bench-binary
run=$(gh run list -R millZach/AgentDrafter --workflow CI --branch main --status success --limit 1 --json databaseId --jq '.[0].databaseId')
gh run download "$run" -R millZach/AgentDrafter --name wheel-windows-latest --dir target/bench-wheel
python3 tools/bench/laptop.py --binary target/bench-binary/agentdrafter.exe --wheel target/bench-wheel/agentdrafter-*-win_amd64.whl \
  > target/bench-laptop.json 2> target/bench-laptop.log
```

(`gh workflow run "Windows binary" -R millZach/AgentDrafter --ref main` builds it when main's newest run has expired; artifacts are kept seven days.) It reaches the laptop as the judge does, with the judge's settings (`host`, and `dir` for its lock; [`tools/judge/README.md`](../judge/README.md)), and keeps the laptop to one job at a time as the judge does:

- It holds the judge's lock while it runs, so no judge run starts meanwhile, and waits for a judge run that holds it.
- It waits until no AutoCAD console runs on the laptop and at least 3 GB of memory is free, 20 minutes at most (`--wait`); after that it runs nothing and says why.
- It makes one temporary folder on the laptop and puts everything in it: the bench, the demo plant, the binary, the wheel, a virtual environment made with the laptop's own Python that the wheel is installed into from the file alone, without the network or pip's cache, and the bench's working folders, the browser's profile among them. It installs nothing outside that folder.
- It runs the bench with the laptop's own Python and its own Edge (`--browser chrome` for Chrome), headless: all three numbers against the binary, then the cold command and the set against the wheel's command.
- At the end, whatever happened, it stops anything still running from that folder and removes it, and says whether it could.

It prints the binary's JSON, with the wheel's `cold` and `set` under `wheel`, and `laptop`: how long it waited, the memory free when it began, whether an AutoCAD console was running when it ended, what it measured and whether the folder was removed. Options after `--` go to the bench, such as `-- --runs 20`.

The wheel's command is a Python entry point that runs the command line's code inside the extension ([`docs/python.md`](../../docs/python.md#the-command)), so each command it runs starts Python first. The binary's numbers are the baseline, as forge's are of the plain binary.

## In CI

CI runs the bench small, to check that it still works, and never holds it to a number: each `check` job measures `cold` and `set` with 16 sheets on its system, and the `viewer` job measures `viewer` with the page built in, in headless Chrome. Runners' speed varies too much for a number to mean anything there.
