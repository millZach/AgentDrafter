# The drafting app

**Status: builds 1 to 7 are done.** The records are written, the [viewer](viewer.md) is in the frame (#150), the ribbon, a tab for each sheet, the command bar, the status bar and quick properties, round dark model space, the cursor catches [object snaps](viewer.md#object-snaps) (#151), and the first [drawing tools](viewer.md#drawing-tools) draw and edit through change sets (#152): Line, Circle, Rectangle, Text, Move and Erase, and [quick properties edit](viewer.md#editing-in-quick-properties) a layer, a text's contents, attribute values and a radius (#153), and an edit to a template's object is [drawn before it lands](viewer.md#a-change-set-drawn-before-it-lands) on each sheet it reaches (#154), and [proposals](viewer.md#proposals) are drawn dashed on the drawing with Keep and Discard (#155). The assistant is not built. This page sets out the drafting app the viewer grows into (issue [#144](https://github.com/millZach/AgentDrafter/issues/144)), in the order it will be built. Its terms are in [`CONTEXT.md`](../CONTEXT.md): drafting app, drafter, drawing tool, command bar, assistant, proposal and guide.

## What it is

The viewer with drawing tools and an assistant. A drafter draws and edits in it the way they use AutoCAD, and an agent drafts with them in the same session. It opens in a browser, on a laptop and on a phone, and as an installed desktop app.

## Decided

Zach decided these on 2026-11-17, from question cards and a prototype, and the last on 2026-11-19, from a prototype's card.

- **The stack is ADR-0011's.** The drawing is drawn by the Rust pipeline compiled to WebAssembly, with TypeScript and React around it. An all-Rust UI was put to him again and not chosen.
- **The desktop shell is Tauri** ([ADR-0011](adr/0011-one-rust-drawing-pipeline-shown-in-the-browser.md)).
- **The look** was picked from a prototype, kept as the tag [`prototype/drafting-app-144`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-144/prototypes/drafting-app). Three other directions were turned down first. His words for what he wanted: "closer to the look of AutoCAD but a simplier UI That makes it easier for beginners to use and somewhere you can have the agent interactlvly draft with you and help you".
- **The assistant runs on the drafter's own agent first.** The app starts an agent the drafter already has, such as Claude Code or Codex, pointed at `agentdrafter mcp`, and passes the chat to it. A built-in assistant that calls a model's API with the drafter's own key can follow.
- **The build order.** The first five builds below come after #101. After them, the work alternates: one capability from [the ranking](capabilities.md), one build of the app.
- **How a kept proposal reads in the journal** (J1, 2026-11-19, from the proposals prototype's card): a kept proposal is one journal entry, `Move service · proposed by Assistant · kept by you`, undone and accepted as any change set, and undoing it does not bring the proposal back. A discard leaves a line, `Discarded proposal N · by you`, with no undo, and an undo passes over it. On the same card he picked the proposals' layout, variant B, on the drawing.

## The look

- **AutoCAD's look.** Dark model space with a grid and a crosshair, a ribbon, a tab for each sheet, the command bar at the bottom of the drawing, Model and layout tabs, and a status bar with Grid, Object snap and Ortho.
- **Cut down for beginners.**
  - The ribbon holds a few labelled drawing tools in Draw, Modify and Layer panels. Each has a tip in plain words and its AutoCAD alias.
  - The command bar prompts in plain words with options to click, such as "LINE Click the next point [ Done ]". It still takes aliases and typed coordinates: `L`, `120,80`, `@40,0`.
  - Snap markers are named (Endpoint, Midpoint, Centre), and the length and angle show at the cursor.
  - Quick properties open beside the selected object.
- **The assistant** has a panel on the right, which is a sheet that slides up on a phone.
- **Unchanged from the viewer:** the journal, Undo and Accept.

## How a drafter's edit travels

Every edit is a change set. A drafter's edit and the assistant's go through the same `change` command, the same journal, Undo and Accept, and an agent on any interface sees them land.

1. The drafter picks a drawing tool and clicks points on the drawing. The tool runs in the Rust renderer in the page: its rubber band, its snaps and its prompts.
2. When the tool has what it needs, the page sends one change set with the matching [operation](change-sets.md), such as `add_line`, `move`, `delete`, `change` or `set_attribute`.
3. The core writes the set's records, builds the sheet's DWG again and records the change set in the journal, as it does today.
4. The page draws the sheet again.

**How fast.** A change shows in the viewer in 169 ms on forge and 239 ms on the laptop today ([speed.md](speed.md)). Up to 200 ms of that is the wait for the journal check. A drafter's own edit need not wait for it: the page can ask for the sheet as soon as `change` replies, which takes 40 ms on forge and 86 ms on the laptop.

**Where it is written** follows the rules of [change sets](change-sets.md):

- An object added on one sheet goes in that sheet's row.
- An edit to an object that comes from a template changes the template, and every sheet built from it follows. The app says so before the edit lands, such as "This is part of the template. Moving it changes 8 sheets."

**No drawing logic goes in the page** (ADR-0007). What a drawing tool does on the drawing runs in the Rust renderer, on the core's code, and what an edit means is the core's.

## What the core gains

Each addition is a command, or an option of one, in the registry. So every interface gets it, and an agent on any of them can use it.

| # | Addition | Why | Today |
|---|---|---|---|
| 1 | **Object snaps with the display list.** *Built (#151).* `display-list` can give each object's snaps: the snap, which of the object's points it is, and where it is. | The cursor jumps to ends, middles and centres on every move of the mouse, with no round trip. The page finds the nearest snap in reach. The operation is still sent with a [snapped point](change-sets.md#snapped-points), so the core works out the exact place. | The snap code is in the core and reads no files. The page holds a display list, not a sheet. |
| 2 | **A change set drawn before it lands.** *Built (#154).* `display-list` takes a change set, applies it in memory, and gives the sheet as it would be, with the objects it adds and changes, what it changes and deletes as it draws now, whether each sheet it reaches would take it, and the checks they would fail. Nothing is written. | A proposal shown dashed, and the template warning above. | `change --dry-run` applies in memory and builds, and its reply holds no drawing. |
| 3 | **Proposals.** *Built (#155).* A proposal is a change set that waits. It is made with `propose`, listed with `proposals`, and ended with `keep` or `discard`. `keep` lands it as a change set, and the journal says who proposed it and who kept it. The event stream reports a proposal as it reports a change set. | An agent that wants its work looked at first calls `propose` where it would call `change`. It works for any agent, through any interface. | A change set lands or it does not. |
| 4 | **What the drafter is looking at.** A command gives the open sheet and the selected objects. | An agent can answer "what is this?" and act on "move this". | The page knows. The core does not. |
| 5 | **Guides.** A guide is a list of steps as data: what to say, which drawing tool, where to click (a snapped point), and the operation that completes the step. The page runs it: the tool's button pulses, a ring marks the point, and each step is checked against what the drafter does. | The assistant teaches on the drafter's own drawing. | Nothing. |

A proposal is made against the set as it stood. If the set has changed by the time it is kept, `keep` applies it to the set as it stands then, and each sheet takes its part fully or not at all, as with any change set.

## What the page gains

- **The frame:** the ribbon, a tab for each sheet, the command bar, the status bar, and quick properties in place of the inspector.
- **Drawing tools in the Rust renderer:** the tool's state, its rubber band, the snap lookup, grips, the crosshair and the size at the cursor. The page's React asks the renderer for the prompt and draws the frame round it.
- **The command bar.** It takes a drawing tool's AutoCAD alias, typed coordinates and lengths. The aliases are the drawing tools' own, in the renderer, not the operations': `L`, `C`, `REC`, `T` and the rest start tools, which may send operations whose AutoCAD names differ (Rectangle sends `add_polyline`), and `T` starts TEXT here where AutoCAD's starts MTEXT. Words that are not a drawing tool's alias will go to the assistant.
- **The assistant's panel:** the chat, proposals with Keep and Discard, the guide's banner, and the journal.

## The assistant

The assistant is an agent, and it reaches the core through the registry as any agent does. Proposals, guides and what the drafter is looking at are the core's commands, so what runs behind the chat can change without changing them.

- **First: the drafter's own agent.** The app starts an agent's command line, points it at `agentdrafter mcp`, and passes the chat to it. It needs no API key, and it uses whatever the drafter already pays for. A drafter must have an agent installed, and each agent's command line needs its own small adapter.
- **Later: built in.** The app calls a model's API itself with the drafter's own key and runs the tool loop on the drafter's machine, with the registry's commands as its tools. It works with nothing else installed, and it is billed by use.
- **Not chosen: a hosted agent loop.** It would send the session, with drawing content in it, to be kept by the host.

## The builds, in order

Each is one ticket, proven as [`AGENTS.md`](../AGENTS.md) asks.

1. **The records:** this page, the plan, ADR-0011 and the glossary. *Done (#145).*
2. **The frame** in the viewer, with selecting and quick properties that only show. *Done (#150): [the viewer](viewer.md#the-frame) sets out what was built. The prototype Zach picked on 2026-11-18 is kept as the tag [`prototype/drafting-app-frame-150`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-frame-150/prototypes/drafting-app-frame).*
3. **Object snaps with the display list**, and the crosshair and snap markers. *Done (#151): [the viewer](viewer.md#object-snaps) sets out what was built. The prototype Zach picked on 2026-11-18 is kept as the tag [`prototype/drafting-app-snaps-151`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-snaps-151/prototypes/drafting-app-snaps). The intersections are listed by the core in the reply; the length and angle at the cursor wait for build 4 (#152).*
4. **The first drawing tools:** Line, Circle, Rectangle, Text, Move and Erase, with the command bar, typed coordinates and the template warning. *Done (#152): [the viewer](viewer.md#drawing-tools) sets out what was built. The prototype Zach picked on 2026-11-19 is kept as the tag [`prototype/drafting-app-tools-152`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-tools-152/prototypes/drafting-app-tools). The core gained `display-list --origins`, where each object is written and the sheets its template reaches, for the template warning.*
5. **Editing in quick properties:** layer, text, attribute values and radius. *Done (#153): [the viewer](viewer.md#editing-in-quick-properties) sets out what was built. The prototype Zach picked on 2026-11-19 is kept as the tag [`prototype/drafting-app-props-153`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-props-153/prototypes/drafting-app-props). The core gained `display-list --values`, each object's contents, radius and attribute values and where each is written, for the fields and what they say before an edit. One sheet's own value for an attribute the template leaves unset waits on an operation for it.*
6. **A change set drawn before it lands.** *Done (#154): [the viewer](viewer.md#a-change-set-drawn-before-it-lands) sets out what was built. The prototype Zach picked on 2026-11-19, its variant B, on the drawing, is kept as the tag [`prototype/drafting-app-preview-154`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-preview-154/prototypes/drafting-app-preview). The core gained `display-list --change-set`, the sheet as a change set would leave it, applied in memory by the dry run's code, with its `preview`. The template warning shows it; proposals (build 7) will be drawn with it.*
7. **Proposals,** shown dashed with Keep and Discard. *Done (#155): [the viewer](viewer.md#proposals) sets out what was built, and [change sets](change-sets.md#proposals) the commands. The prototype Zach picked on 2026-11-19, its variant B, on the drawing, is kept as the tag [`prototype/drafting-app-proposals-155`](https://github.com/millZach/AgentDrafter/tree/prototype/drafting-app-proposals-155/prototypes/drafting-app-proposals). The core gained `propose`, `proposals`, `keep` and `discard`, and the event stream's `proposals`; how a kept proposal reads in the journal is decided below.*
8. **The assistant's panel,** on the drafter's own agent, with what the drafter is looking at.
9. **Guides.**
10. **The Tauri shell:** an installed app that runs the core and shows the page, with a folder picker.

Drawing tools join the ribbon as their operations are built: Copy with #66, Rotate, Scale and Mirror with #67, then Offset, Trim, Fillet and the rest (#80 to #85).

## Still open

| Question | Waits on |
|---|---|
| What the app opens beyond a drawing set: one sheet description, or a plant's own DWG | #70 and #71 for a plant's DWG |
| Which agents get an adapter first, and how the app finds them | the assistant's panel |
| Code signing and releases | the app shipping ([plan.md](plan.md)) |
