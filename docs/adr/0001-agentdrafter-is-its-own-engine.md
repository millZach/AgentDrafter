# AgentDrafter is its own engine, built for agents first

AgentDrafter is a CAD engine whose first user is an agent: it creates, revises and checks drawings with no window open, and a person reviews the result. Open-source work close to this existed when the project started (November 2026): AgentCAD, a Python runtime on a DXF library that needs an installed AutoCAD for DWG and has no 3D; and Open CAD Studio, a GPL-3.0 Rust drafting application with native DWG that is built for a person at a mouse. We build our own engine because the requirements that matter most here (DWG without AutoCAD, untouched means unchanged, the drawing set as the unit of work, and a later drafting app on the same core) are not what either was designed around.

## Considered options

- **Build on AgentCAD.** Working operations, undo and a viewer from day one. Rejected: its core is a DXF library, DWG depends on AutoCAD, and a trial on a made-up loop-sheet set showed no templates, no shared block library and no undo across a set.
- **Build on or fork Open CAD Studio.** DWG, plotting, 3D and a drafting app already exist. Rejected: it would inherit its licence, roadmap and weekly churn, and an agent layer would sit on top of an application shaped for hand drafting.
- **Own engine, shared parts (chosen).** We may use the same libraries those projects prove out, and we read both as references.

## Consequences

- Hand drafting is not in the first versions. A person drafts in AutoCAD until the drafting app is built on this engine.
- Ideas are taken from those projects; code is taken only where its licence fits ours.
