# The judge

The judge is AutoCAD, run with no window on a machine that has a licence, saying what it made of DWGs. It is used in tests only: never in CI and never in a job (ADR-0008). This folder holds the tool and no drawings. The sample of real drawings, AutoCAD's readings of them and every result stay in a private folder outside the repository, named by a setting. Reports name drawings by neutral ID and give counts only.

It was the referee of the DWG bake-off (ADR-0009) and reproduces its numbers from the readings stored then.

## Commands

Run from forge (Linux) with Python 3.11 or newer; it needs nothing outside the standard library. It reaches the Windows machine that has AutoCAD 2026 with `ssh` and `scp`, which must log in with a key and no prompt.

```
python3 tools/judge/judge.py built <folder>       [--name <run>] [--show-names] [--overwrite]
python3 tools/judge/judge.py roundtrip <folder>   [--name <run>] [--overwrite]
python3 tools/judge/judge.py rescore <run>        [--show-names]
```

- **`built`** judges every DWG in the folder: did AutoCAD open it without recovery, and how many errors did its audit find. A drawing passes with both: opened `yes` and 0 audit errors. The command exits 1 when any drawing fails. Both reports, Markdown and JSON, name drawings by neutral ID (`b001`, ...); the map to file names is `work/<run>/names.tsv` in the private folder, and no report embeds it. `--show-names` puts file names in both reports instead and is only for public drawings, such as the demo plant.
- **`roundtrip`** judges a revision or a round trip. The folder holds `<id>.dwg` files, each named after a drawing in the sample. AutoCAD reads each one, and its reading is compared with its reading of the original (the reference) in drawn objects: identical, lost and changed, per drawing and pooled, beside opened and audit errors. Every other record with a handle (table entries, dictionaries, layouts) is compared as well, as a second view.
- **`rescore`** scores a run again from its stored readings, without AutoCAD.

A run takes minutes: 13 small drawings took under a minute and the whole sample takes 5 to 8 when the laptop is free, longer while another program uses AutoCAD. Start it in the background with its output in a log:

```
nohup python3 tools/judge/judge.py roundtrip <folder> --name <run> > <log> 2>&1 &
```

The log starts with the time the run started and ends with the report. The report is also written to `results/<run>.md` and `results/<run>.json` in the private folder.

## Settings

Nothing private has a default. Each setting comes from an environment variable or from the config file, and the environment wins.

| Config key | Environment variable | What it is |
|---|---|---|
| `dir` | `AGENTDRAFTER_JUDGE_DIR` | The judge's private folder on this machine. Refused if it is inside any working tree of the repository: the main checkout or any linked worktree. |
| `host` | `AGENTDRAFTER_JUDGE_HOST` | The ssh name of the Windows machine that has AutoCAD. |
| `scratch` | `AGENTDRAFTER_JUDGE_SCRATCH` | A scratch folder on that machine, such as `C:\Users\<you>\AppData\Local\Temp\agentdrafter-judge`: letters, digits, `.`, `_`, `-` and backslashes. |
| `accoreconsole` | `AGENTDRAFTER_JUDGE_ACCORECONSOLE` | The AutoCAD console. Default: `C:\Program Files\Autodesk\AutoCAD 2026\accoreconsole.exe`. |

The config file is TOML at `$AGENTDRAFTER_JUDGE_CONFIG`, else `$XDG_CONFIG_HOME/agentdrafter/judge.toml`, else `~/.config/agentdrafter/judge.toml`. Write the scratch folder in single quotes so its backslashes stay as they are.

The private folder holds:

| Path | What |
|---|---|
| `orig/<id>.dwg` | The sample: one original per neutral ID. Read-only. |
| `ref/` | AutoCAD's reading of every original. The first `roundtrip` builds it. An original skipped because AutoCAD was busy or memory was low is listed in `ref/SKIPPED` and read by the next `roundtrip`; `ref/COMPLETE` is written once none is left. Delete `ref/COMPLETE` to build it again. |
| `work/<run>/` | AutoCAD's readings for one run: its DXF of each drawing, the console log with the audit report, and status files. |
| `results/<run>.md`, `.json` | The reports. |
| `judge.lock` | Held while a run uses AutoCAD. |

## One AutoCAD at a time

- A run holds `judge.lock` in the private folder while it uses AutoCAD. A second run prints that the judge is busy and waits for it.
- On the AutoCAD machine, before each drawing, it waits until no AutoCAD console is running, because other programs use AutoCAD there too. Then it checks that at least 3 GB of memory is free and waits while it is not. A run waits 20 minutes at most for all of this together, not per drawing. Once those 20 minutes are spent, every drawing left is reported as `not judged` at once, rather than failed, and the run ends as usual. The run's ssh calls allow for that wait plus each drawing's own time, so a busy machine never cuts a run off before it reports.
- Each AutoCAD console is started and stopped by its process ID, with a 120-second limit per drawing.
- If a run is stopped (an error, Ctrl-C, `kill`), it stops its own AutoCAD console and empties its work folder there. Stop a run with a plain `kill`, not `kill -9`, so this can happen.

## What it does with each drawing

1. Copies the DWG to one fixed folder on the AutoCAD machine, `<scratch>\lanes\judging\<id>.dwg`. Every drawing in every run is opened from there, so fields that show the file path read the same as in the reference.
2. Opens it with `accoreconsole /i`, writes AutoCAD's own ASCII DXF (`DXFOUT`, 2018 format, 16 decimal places) and runs `AUDIT` without fixing. Nothing is saved. The script restores `FILEDIA`; no settings, profiles, trusted paths or licence files are touched.
3. A drawing that will not open gets one try through `RECOVER`, at most 8 a run. It then counts as `recovered`, which fails a built drawing.
4. The DXFs come back gzipped and are checked, with the console logs. Because AutoCAD writes both readings, a difference comes from the DWG, not from a DXF writer.

**Drawn objects** are the records in a drawing's ENTITIES and BLOCKS sections: lines, text, block inserts, attributes, hatches, dimensions and the rest, but not the markers that open and close a block. One is **identical** when AutoCAD reads it under the same handle with the same content before and after (floats to 1e-9 relative). **Lost** means its handle is gone. **Changed** means the handle is kept and the content differs. The JSON also counts lost objects whose content came back under a new handle.

## Reading a round trip

Read the numbers against what AutoCAD alone does, not against 100. AutoCAD's own plain save of the sample, the baseline run `autocad-noop`, keeps 99.64% of drawn objects identical over 50 drawings. It rebuilds dimension and other anonymous blocks under new handles. The originals judged against themselves score 100%. acadrust saving in the same version scored 98.41% over 52 drawings in the bake-off.

## What it cannot see

1. **AutoCAD's view, not the bytes.** Anything AutoCAD repairs or rebuilds silently when it opens a file shows only in the audit count. A file only AutoCAD's forgiving reader accepts can still score well.
2. **Only what DXF carries.** The preview image, file security and signatures, and the DWG's internal layout are not compared.
3. **Floats to 16 places**, and "identical" forgives a 1e-9 relative difference. The JSON has the byte-exact share beside it.
4. **Opaque binary**, such as proxy data, 3D solid data and the data of embedded OLE objects, is compared as bytes: a change is seen, its meaning is not.
5. **Ignored on purpose**, because AutoCAD changes them between two reads of the same unchanged DWG: the header dates, `$HANDSEED`, `$VERSIONGUID`, `$FINGERPRINTGUID` of R13 and R14 files, `AcDbDynamicBlockGUID` xdata, the cached value inside a field, the proxy graphics (binary group 310) AutoCAD writes for tables, multileaders and other objects that are not proxies (only their length counts), the modified time of an embedded OLE object's root storage, and handle lists AutoCAD keeps as sets.
6. **Not looked at**: plots and on-screen appearance, external files such as xref targets and fonts, record order outside each block or layout, and the DWG version written (shown, not scored).
7. **One AutoCAD.** This is AutoCAD 2026's console with the object enablers that machine has. Another release, or the full AutoCAD, could read some files differently.

## Files

- `judge.py`: the command.
- `judgelib/settings.py`: the settings. `judgelib/autocad.py`: the AutoCAD pass over ssh. `judgelib/dxf.py`: reading and comparing AutoCAD's DXF. `judgelib/report.py`: scores and reports.
- `laptop/`: what runs on the AutoCAD machine. `driver.ps1` runs the consoles one at a time, `tasks.ps1` prepares and cleans the work folder, and `judge.lsp` is the judge's pass, pasted into each console script.
- `tests/`: tests of the reading and scoring on made-up data, with no AutoCAD. CI runs them: `python3 -m unittest discover -s tools/judge/tests`.
