# AgentDrafter is licensed under Apache-2.0

AgentDrafter's own code is under the Apache License 2.0. Anyone can use it, including inside a closed product, and it carries a patent grant. The engine is meant to be called by other people's generators and tools, most of them private, so the licence asks nothing of the code around it. acadrust stays MPL-2.0, which Apache-2.0 code can build on.

## Considered options

- **MIT.** The same freedom in fewer words, but no patent grant.
- **MPL-2.0, the same as acadrust.** Anyone who changes AgentDrafter's files must publish those files. Rejected: it adds a duty for adopters and buys little, since code can already move to acadrust by its author contributing it there.
- **GPL-3.0.** The only choice that would let us copy code from Open CAD Studio. Rejected: companies are cautious about adopting it, and that project is a reference for us, not a source (ADR-0001).
- **Apache-2.0 (chosen).**

## Consequences

- No code is copied from a GPL project, including Open CAD Studio, LibreDWG, LibreCAD and QCAD. Reading them for ideas is fine.
- Fixes to acadrust are contributed under its licence. An acadrust file we change stays published, upstream or in a public fork.
- A dependency's licence is checked before it is added. Permissive and MPL-2.0 are fine. LGPL or stronger needs its own decision, which will come up with the 3D kernel.
