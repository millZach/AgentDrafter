# The speed baseline

The speed baseline is the fixed set of numbers later work reports its speed against (see [`CONTEXT.md`](../CONTEXT.md)): one command from a cold start, a 100-sheet set built to DWG and PDF, and a change showing in the viewer (`docs/plan.md`, Platforms and shipping). One script measures all three, on any machine and against any `agentdrafter`, and prints them as JSON: `tools/bench/bench.py`, whose [README](../tools/bench/README.md) says exactly what it times.

Zach decided on 2026-11-11 that the baseline is measured on the laptop, in one short scripted run of a few minutes, under the judge's lock. Forge's numbers, from the same script, sit beside the laptop's, so that an agent on forge can compare its own with them.

## The numbers

Measured on commit `75657a9` of `main`. Each is a median, with the fastest and the slowest run after it.

| Number | What is timed | Laptop | Forge |
|---|---|---|---|
| One command from a cold start | `agentdrafter check` on the demo plant's 8 sheets, a fresh process each time; 10 runs after one that warms the caches | not yet measured | **41.9 ms** (41.6 to 48.1) |
| A 100-sheet set, built | `agentdrafter build` of the set's 100 sheets to DWG, one command; 3 runs, each into a fresh copy | not yet measured | **1.00 s** (0.97 to 1.02) |
| A 100-sheet set, plotted | then `agentdrafter plot` of the same 100 sheets to PDF, one command | not yet measured | **1.39 s** (1.38 to 1.41) |
| A 100-sheet set, in all | the build and the plot together | not yet measured | **2.38 s** (2.36 to 2.43) |
| A change showing in the viewer | from sending a change set through the command line, in another process, until the open page has drawn it; 10 changes after one that warms up | not yet measured | **169 ms** (65 to 267) |

- **Cold** means a fresh process with nothing of AgentDrafter running or loaded before it, and the file system's caches as warm as the system leaves them: emptying them needs an administrator, so one run first, not counted, warms them. `check` reads the set's records and lays out every sheet through the drawing pipeline, text in its fonts, as an agent checking its work does.
- **The 100-sheet set** is made by the bench from the demo plant's 8 loop sheets, each used again with its loop renumbered, in a temporary folder; it is never kept. Its DWGs come to 3.07 MB and its PDFs to 4.11 MB.
- **A change showing in the viewer** moves one insert on the sheet shown, `DP-LD-101`, by half an inch, and is timed until the end of the page's first draw of the canvas after it loaded the sheet with the insert at its new place, as the page's own probe reports them. In the page, at 1440 by 900 in a headless browser with software WebGL. On forge, of the 169 ms, the `change` command itself took 40 ms and the page had loaded the changed sheet at 166 ms. The server looks at the set's journal five times a second ([servers.md](servers.md#watching-a-set)), so a change waits up to 200 ms before the page hears of it, depending on when it lands; the bench spaces its changes so that they land at every point of that wait, which is most of the spread from 65 to 267 ms.

## The machines

| | Laptop | Forge |
|---|---|---|
| Processor | not yet measured | Intel Core i5-14400F |
| Cores | | 10 (16 threads) |
| Memory | | 15.4 GB |
| System | | Omarchy, Linux 7.2.5 |
| Disk | | NVMe SSD |
| Power | | mains |
| Browser | | Chromium 152.0.7977.82, headless, WebGL in software (SwiftShader) |
| `agentdrafter` | | built from the commit with `cargo build --release -p agentdrafter-cli`, the viewer's page built in |
| Python running the bench | | 3.14.7 |

Forge is shared with other agents' work. While its numbers were measured, its load average stood between 6 and 9, other programs taking about 2.5 of its 16 threads. Three runs of the bench in a row gave medians within 10% of one another for each number, and within 4% for the cold command and the plot; the one recorded here is the middle run.

## Reporting in a pull request

`AGENTS.md` asks a pull request that could move these numbers to report them. **It could move them** when it changes:

- code that runs when `agentdrafter` runs: anything in `crates/` or `viewer/` beyond their tests (`tests/`, `#[cfg(test)]` modules, `viewer/test/`, `viewer/e2e/`);
- a dependency, the toolchain or a build setting: `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `viewer/package.json`, `viewer/package-lock.json`, `pyproject.toml`;
- what the bench measures, or how: `examples/demo-plant/`, `tools/bench/`.

A pull request that changes only documents, tests, CI workflows, the judge or other examples need not.

**Measure on forge**, against `agentdrafter` built from the pull request's head as forge's baseline was built:

```
cd viewer && npm ci && npm run build && cd ..
cargo build --release -p agentdrafter-cli -j4
python3 tools/bench/bench.py > target/bench.json
```

The page's build needs `wasm-bindgen-cli` of the version the viewer crate pins ([viewer.md](viewer.md#building-it)). Run it when forge is not busy building, and give the numbers in the pull request beside forge's baseline:

| Number | Forge's baseline | This pull request |
|---|---|---|
| One command from a cold start | 41.9 ms | ... |
| A 100-sheet set, built | 1.00 s | ... |
| A 100-sheet set, plotted | 1.39 s | ... |
| A 100-sheet set, in all | 2.38 s | ... |
| A change showing in the viewer | 169 ms | ... |

When a number moves by more than 10%, run the bench again, give both runs and say what in the pull request moved it. A pull request that changes what the bench measures, or how, gives forge's numbers from before its change and after it, measured the same way. No number is a gate.

## When the baseline is measured again

The laptop's numbers are a manual step, done at a release or when Zach asks, never in CI. `tools/bench/laptop.py` runs the bench once on the laptop from forge, under the judge's lock, with the Windows wheel CI built, and leaves nothing behind there ([its README](../tools/bench/README.md#on-the-laptop)). Forge's numbers are measured again with them, from the same commit, and this page is updated with both.

## In CI

CI does not hold the bench to any number, since runners' speed varies too much. It runs the bench small, to check that it still works: each `check` job measures one command from a cold start and a 16-sheet set on its system, and the `viewer` job measures a change showing in the viewer, with the page built in, in headless Chrome.
