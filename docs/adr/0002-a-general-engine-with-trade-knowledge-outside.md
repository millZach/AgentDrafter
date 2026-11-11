# AgentDrafter is a general engine; trade knowledge lives outside it

AgentDrafter knows lines, blocks, attributes, layers and sheets. It does not know what a loop, a wire, a terminal or an I/O point is. That knowledge lives in a generator, a separate program that turns project data into sheet descriptions and calls AgentDrafter. We chose this over a built-in controls layer (the AutoCAD Electrical approach) so the open-source engine is useful to any trade, stays small, and never needs a client's standards inside a public repository.

## Consequences

- A check that needs trade knowledge, such as "every wire lands on a terminal", is written by the generator against AgentDrafter's query results. AgentDrafter's own checks are about drawings: nothing off the sheet, no text overflowing its cell.
- Compatibility with AutoCAD Electrical is a matter of attribute names and hidden data inside an ordinary DWG, which a generator can write. The engine carries them like any other content.
