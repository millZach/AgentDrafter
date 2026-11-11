# AgentDrafter

AgentDrafter is an open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open; a reviewer watches, accepts or undoes. It reads and writes DWG without AutoCAD, runs on Windows, Linux and macOS, and does 2D first and 3D after. The drawings it handles go to real plants, which keep them as master records.

This is a brief, not a specification. It names what is expensive to get wrong and leaves the rest to you. Where it does not reach, work out what AgentDrafter would do from `CONTEXT.md`, the ADRs and `docs/plan.md`, and say in the pull request what you worked out.

**Status: walking skeleton.** `agentdrafter build` turns a sheet description that holds lines into a DWG in the 2018 format; nothing else is built yet. `docs/plan.md` holds the agreed plan. The stack is decided: a Rust core that reads and writes DWG through acadrust, a Python binding and thin interfaces over it, and a browser viewer that draws with the same Rust code compiled to WebAssembly, with TypeScript and React around it.

## What is expensive to get wrong

- **The repository is public.** No plant's drawing, block, tag or name enters it, ever, including in tests, fixtures, screenshots, logs and notes. Tests and examples use the demo plant, which is made up. A bug found on a real drawing is reproduced on a made-up drawing before its test is committed (ADR-0008).
- **Untouched means unchanged.** When AgentDrafter revises an existing DWG, everything it did not edit comes back exactly as it was, including content it does not understand. A revision is saved in the DWG version its original arrived in (ADR-0006). A drawing the reader cannot fully decode is not revised unless the reviewer allows it for that drawing. A change that touches DWG reading or writing proves this against AutoCAD, not by eye (ADR-0004).
- **An original is never overwritten.** An existing DWG is read-only. A change is saved as a new revision beside it.
- **The engine knows drawings, not trades.** Nothing about loops, wires, terminals or any trade goes into the engine. That belongs in a generator (ADR-0002).
- **Every interface behaves the same.** The command line, the MCP server, the Python library and the HTTP API are thin layers over one core. Drawing logic in an interface is a bug (ADR-0007).
- **A sheet is never left half-edited.** Each sheet takes its part of a change set fully or not at all (ADR-0005).
- **The licence is Apache-2.0.** No code is copied from a GPL project, such as Open CAD Studio or LibreDWG. Check a dependency's licence before adding it: permissive and MPL-2.0 are fine, anything stronger needs a decision (ADR-0010).
- **acadrust is pinned.** Moving the pin means running the judge again. Fixes to acadrust go upstream, not into a private patch (ADR-0009).

## Read first

- `CONTEXT.md` is the glossary. Write in its terms; when the work needs a word it has not got, add the entry in the same pull request.
- `docs/plan.md` is the agreed plan: the first usable version, what comes after, the rules of the engine and what is still open.
- `docs/adr/` records the decisions that are hard to reverse. Read the ones that touch your area before changing it, and amend or supersede one you contradict. `docs/agents/domain.md` has the rules.
- `docs/sheet-description.md` sets out the sheet description: its keys, its units and the strict YAML subset it is written in.

## Run, test and prove

The code is one Cargo workspace. `crates/agentdrafter-core` is the core, the library that knows drawings; its entry points take and return plain data, and acadrust sits behind its one seam, `src/dwg.rs`. `crates/agentdrafter-cli` is the `agentdrafter` command, a thin layer that prints the core's reply as JSON. The Rust toolchain is pinned in `rust-toolchain.toml`.

- Build: `cargo build --workspace`. On forge, add `-j4` and keep `target/` inside the checkout, not under `/tmp`.
- Run: `cargo run -p agentdrafter-cli -- build examples/one-line.yaml` writes `examples/one-line.dwg` and prints one line of JSON. Add `--overwrite` to build it again.
- Test: `cargo test --workspace`.
- Before pushing, run what CI runs: `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`, then `cargo test --workspace --locked`, then `python3 -m unittest discover -s tools/judge/tests`.
- Prove a change to DWG reading or writing with the judge, `tools/judge/judge.py`; its README says how it works and what it cannot see. Run it from forge. It drives AutoCAD on the laptop over ssh, one AutoCAD at a time, after a free-memory check. Its settings live outside the repository, in `~/.config/agentdrafter/judge.toml` or `AGENTDRAFTER_JUDGE_*` variables. A run takes minutes, so start it in the background with its output in a log, and tell Zach when it starts.
  - A new file: build it with `agentdrafter build`, copy the DWG to a folder outside the repository, and run `python3 tools/judge/judge.py built <folder>`. It passes when AutoCAD opens it without recovery and its audit finds no errors. Add `--show-names` only when every drawing in the folder is public, such as the demo plant.
  - A revision or round trip: write each sample drawing back as `<id>.dwg` into a folder inside the judge's private folder, never in the repository. Then run `python3 tools/judge/judge.py roundtrip <folder> --name <run>`. It reports drawn objects identical, lost and changed, per drawing and pooled. Read them against AutoCAD's own plain save, which keeps 99.64% identical over 50 drawings.
  - Say in the pull request what the judge found: neutral IDs and counts only, never a drawing's name, path or content. The judge never runs in CI; CI runs only its tests, on made-up data.
- CI runs on GitHub's Windows, Linux and macOS runners, never on Zach's laptop.
- A DWG that AgentDrafter wrote is proven by AutoCAD: it opens without recovery, its audit finds no errors, and AutoCAD's own reading of the result matches its reading of the original outside what was changed. AutoCAD is the judge in tests on a machine that has a licence. It is never part of a job and never part of CI.
- Anything a reviewer will see, starting with the viewer, is shown as a prototype that opens on a phone before it is built for real.

## The merge bar

A pull request merges only when these jobs of the `CI` workflow (`.github/workflows/ci.yml`) are green on its latest commit:

- `check (ubuntu-latest)`
- `check (windows-latest)`
- `check (macos-latest)`

Each runs formatting, lints with warnings as errors, and tests. A pull request that touches DWG reading or writing also states the judge's result before it merges.

## Agent skills

### Issue tracker

Issues are GitHub Issues on this repository, through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` and `docs/adr/` at the repository root. See `docs/agents/domain.md`.
