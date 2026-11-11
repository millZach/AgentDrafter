# Four interfaces on one core; scripts are sandboxed and off by default

An agent reaches AgentDrafter through the command line (JSON on standard output), an MCP server, a Python library or a local HTTP API. All four are thin layers over one core, so an operation behaves the same through each and none of them holds drawing logic. Operations have their own typed names, with the AutoCAD command name as an alias where one exists. Agents may also send a script for an edit no operation covers; it runs sandboxed as one change set, and only when the reviewer has switched scripts on, because an unguarded script tool is the common weak point of today's CAD agent integrations.

## Consequences

- Four surfaces to keep in step: a test proves each operation gives the same result through every interface.
- The HTTP API is local and is what the viewer uses; it is an open port to guard.
