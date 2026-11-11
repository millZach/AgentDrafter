# AgentDrafter

AgentDrafter is an open-source CAD engine built for agents first. An agent creates, revises and checks technical drawings with no window open; a reviewer watches, accepts or undoes. It reads and writes DWG without AutoCAD, runs on Windows, Linux and macOS, and does 2D first and 3D after. The drawings it handles go to real plants, which keep them as master records.

This is a brief, not a specification. It names what is expensive to get wrong and leaves the rest to you. Where it does not reach, work out what AgentDrafter would do from `CONTEXT.md`, the ADRs and `docs/plan.md`, and say in the pull request what you worked out.

**Status: planned, not built.** `docs/plan.md` holds the agreed plan. No code exists yet. The stack is decided: a Rust core that reads and writes DWG through acadrust, a Python binding and thin interfaces over it, and a browser viewer that draws with the same Rust code compiled to WebAssembly, with TypeScript and React around it.

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

## Run, test and prove

Nothing to run yet. When the first code lands, this section names the commands for building, testing and proving a change on the real surface, and CI is added with the first test.

- CI runs on GitHub's Windows, Linux and macOS runners, never on Zach's laptop.
- A DWG that AgentDrafter wrote is proven by AutoCAD: it opens without recovery, its audit finds no errors, and AutoCAD's own reading of the result matches its reading of the original outside what was changed. AutoCAD is the judge in tests on a machine that has a licence. It is never part of a job and never part of CI.
- Anything a reviewer will see, starting with the viewer, is shown as a prototype that opens on a phone before it is built for real.

## The merge bar

To be named when CI exists: the jobs that must be green on a pull request's latest commit before it merges.

## Agent skills

### Issue tracker

Issues are GitHub Issues on this repository, through the `gh` CLI. See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: one `CONTEXT.md` and `docs/adr/` at the repository root. See `docs/agents/domain.md`.
