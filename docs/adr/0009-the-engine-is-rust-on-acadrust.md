# The engine is written in Rust and reads and writes DWG through acadrust

No open DWG library had been proven against AutoCAD, and the library decides the engine's language, so three were tested in November 2026 with AutoCAD 2026 as the judge. Each library read 53 drawings (made-up ones and real ones under neutral IDs) and wrote them back. AutoCAD audited every result, and AutoCAD's own reading of each original was compared with its reading of the result. AutoCAD's own plain save was the baseline. The engine is written in Rust on acadrust (MPL-2.0), the only library whose files AutoCAD accepted at close to its own standard.

"Drawn objects" are lines, text, block inserts, attributes, hatches and dimensions. One counts as identical when AutoCAD reads the same object with the same content before and after.

| Run over 53 drawings | Opened clean in AutoCAD | Drawn objects identical | Drawings gaining audit errors |
|---|---|---|---|
| AutoCAD alone, plain save (baseline) | 50 | 99.6% | 0 |
| acadrust, newest code, same version | 52 | 98.4% | 5 |
| acadrust, newest code, converted to 2018 | 44, 8 refused | 95.4% | 5 |
| acadrust, last release, same version | 43 | 97.6% | 6 |
| ACadSharp, converted to 2018 | 40 | 93.8% | 18 |
| LibreDWG | 0 | not scorable | not scorable |

With one attribute or text changed and one line added across 22 drawings, the only drawn objects acadrust changed were the edited ones. A sheet it wrote from scratch opened in AutoCAD with no audit errors.

## Considered options

- **ACadSharp, with the engine in .NET.** Rejected: it discards what it does not understand when it reads a file, so "untouched means unchanged" (ADR-0004) would need its reader and writer reworked. It lost 640 attributes and 322 block inserts across the sample, and its new sheet opened with 20 audit errors.
- **LibreDWG, with the engine in C.** Rejected: AutoCAD opened none of its files, and its GPL-3.0 licence would set ours.
- **A paid library (ODA Drawings SDK, RealDWG).** Rejected: proprietary code cannot sit inside an open-source engine.
- **Our own reader and writer.** Rejected: months of work on an undocumented format before the first sheet builds, repeating faults acadrust has already fixed.
- **acadrust, with the engine in Rust (chosen).**

## Consequences

- acadrust was nine months old at the test, and the fixes this result depends on were not in a published release. AgentDrafter pins a commit, and moving the pin means running the judge again.
- Its known faults are ours to fix and send upstream: one real drawing did not reopen after a round trip, four lost a layout and gained audit errors, one lost its tables, and old-style polylines had vertices renumbered and a few dropped. Until they are fixed, a drawing the reader cannot fully decode is not revised (ADR-0004).
- Converting between DWG versions is its weak path, which is why a revision keeps its version (ADR-0006).
- The Python library is a binding over the Rust core. Every interface stays a thin layer (ADR-0007).
- The core compiles to WebAssembly: a round trip run that way gave byte-identical output. The front end relies on this (ADR-0011).
- If acadrust stalls or moves away from what this engine needs, whether to fork it is a new decision.
- Not covered by the test: an R12 drawing, and acadrust built on Windows or macOS (its own CI covers both).
