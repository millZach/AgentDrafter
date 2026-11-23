# The HTTP API and the MCP server

The command line, the HTTP API and the MCP server are three of AgentDrafter's four interfaces (see [`CONTEXT.md`](../CONTEXT.md)); the Python library is the fourth ([`python.md`](python.md)). Each is a thin layer over one core (ADR-0007): it turns what it was sent into a command's input, hands it to the core, and sends back the core's reply. None of them holds drawing logic.

## One registry of commands

The core keeps one registry of commands, and every interface takes its commands from it, so a command added there reaches all four at once and none can be missed. Each command has one name everywhere, takes one input and gives one reply:

| Command | What it does | Its own page |
|---|---|---|
| `build` | Builds a sheet description, or a drawing set's folder, into DWGs. | [drawing-set.md](drawing-set.md) |
| `schema` | Gives the JSON Schema of a kind of file. | [sheet-description.md](sheet-description.md) |
| `measure-text` | Measures how wide a line of text draws. | [sheet-description.md](sheet-description.md) |
| `render` | Draws a sheet to a PNG. | [render.md](render.md) |
| `display-list` | Draws a sheet to its display list, as JSON, for the viewer. | [viewer.md](viewer.md) |
| `plot` | Plots a sheet, a DWG or a drawing set to PDF. | [plot.md](plot.md) |
| `stale` | Says whether a PNG or PDF is stale. | [render.md](render.md) |
| `query` | Asks a drawing set what is on its sheets. | [query.md](query.md) |
| `operations` | Lists the operations a change set can hold. | [change-sets.md](change-sets.md) |
| `change` | Applies a change set to a drawing set. | [change-sets.md](change-sets.md) |
| `journal` | Lists a drawing set's journal. | [journal.md](journal.md) |
| `undo` | Undoes a change set and every later one. | [journal.md](journal.md) |
| `accept` | Issues the working revision: every change set in place. | [journal.md](journal.md#accept) |
| `propose` | Records a change set that waits, a proposal, to be kept or discarded. | [change-sets.md](change-sets.md#proposals) |
| `proposals` | Lists the proposals waiting. | [change-sets.md](change-sets.md#proposals) |
| `keep` | Lands a proposal as a change set, on the set as it stands. | [change-sets.md](change-sets.md#proposals) |
| `discard` | Ends a proposal without landing it. | [change-sets.md](change-sets.md#proposals) |
| `guide` | Gives the drafter a guide: steps they do with the drawing tools on their own drawing. | [change-sets.md](change-sets.md#guides) |
| `guides` | Lists the guides given, each step worked out on the set as it stands. | [change-sets.md](change-sets.md#guides) |
| `look-at` | Records what the drafter is looking at: the sheet open, its layout and the objects selected. | [below](#what-the-drafter-is-looking-at) |
| `looking-at` | Gives what the drafter is looking at, as the viewer last recorded it. | [below](#what-the-drafter-is-looking-at) |
| `check` | Checks a drawing set's saved files. | [check.md](check.md) |
| `import-blocks` | Imports a DWG's or a DXF's blocks into a block library of its own. | [import-blocks.md](import-blocks.md) |
| `read` | Reads an original DWG whole: its decode report, what the core cannot draw, and its text mirror. | [originals.md](originals.md) |

**The input** is a JSON object. Its keys are the command line's options, with `-` written `_`: `--dry-run` is `dry_run`, `--plot-style` is `plot_style`, and an option given more than once, such as `--fonts`, is a list. What the command line takes as a plain argument has a name of its own: the file or folder is `source` (`set` for `change`, `journal`, `undo`, `accept`, `propose`, `proposals`, `keep`, `discard`, `guide`, `guides`, `look-at` and `looking-at`, and `output` for `stale`), `keep`'s and `discard`'s proposal is `proposal`, `look-at`'s sheet `sheet`, its `--layout` `layout` and its `--selected` IDs `selected`, and their `--by`, and `propose`'s and `guide`'s, `by`, `guide`'s guide is `guide`, the text `measure-text` measures is `contents`, `schema`'s kind is `kind`, `undo`'s change set is `change_set`, `display-list`'s journal entry `entry`, its `--snaps` `snaps`, its `--origins` `origins`, its `--values` `values`, its `--change-set` `change_set`, its `--all-or-none` `all_or_none` and its `--read-past-damage` `read_past_damage`, the change set itself, as `change` takes it, `accept`'s acknowledgement of failed checks `despite_failed_checks`, `query`'s kind and selection are `query: {"kind": "find", "name": ...}`, `import-blocks`'s drawing is `drawing` and its `--block` names `blocks`, `read`'s `--mirror` is `mirror` and its `--no-mirror` `no_mirror`, and `change` takes the change set itself under `change_set`, not a file (and for original DWGs, optional `output`, `allow_loss`, `expected_original_sha256`, `plot_style`, `plot_styles` and `no_pdf`). Each command's input has a JSON Schema, which `GET /commands` lists and the MCP server gives as each tool's input schema. A key the schema does not name is refused.

**The reply** is the one the command line prints: `{"ok": true, "result": ...}`, or `{"ok": false, "error": {"code", "message", ...}}` with the `result` of what was done, if anything was. A command that first rolled back a commit a killed process never finished on its drawing set says so under `rolled_back`, after `ok` ([journal.md](journal.md#a-commit-that-never-finished)). A reply in which a look-alike stood in for a font warns of it under `warnings`, after `ok` and `rolled_back` and before `result`: `{"code": "font_look_alike", "font", "look_alike", "sheets"}`, one per font ([fonts](sheet-description.md#fonts)). The same input gives the same reply through every interface, and the same change set gives the same journal entry, apart from its time. A test runs every command of the registry through all three and compares the replies, and the Python library's test runs the same calls through it too.

**The interface a command came through** is handed to the core with it, and an accept records it in the journal (`via`), so the reviewer can tell an agent's accept from their own; a proposal made, kept or discarded with no `by` is recorded as made by it, such as `agent (MCP)` ([change-sets.md](change-sets.md#proposals)): `cli` for the command line, the Python package's command among it, `http` for the HTTP API, `mcp` for the MCP server, `python` for the Python library, and `viewer` for the viewer's page. The input never says it, so a caller cannot choose it. An accept's reply and entry name it, so they differ there between interfaces, and only there; the tests compare them apart from it, and hold each accept to its own interface.

**Paths** are taken as the command line takes them: relative to the folder the server was started in, or absolute. Replies name files from the paths sent, as the command line's do. A server reaches every file its user can, so a server that listens beyond the local machine needs a token (below).

**Fonts and plot styles** are read from the folders the input's `fonts` and `plot_styles` name, then from those the server's own environment names in `AGENTDRAFTER_FONTS` and `AGENTDRAFTER_PLOT_STYLES` when it started, then, for a TrueType font, from the system's own font folders, unless the input's `no_system_fonts` is true or the server's environment held `AGENTDRAFTER_SYSTEM_FONTS=0` when it started.

## The HTTP API

```
agentdrafter serve [--listen <address:port>]
```

`agentdrafter serve` listens on `127.0.0.1:7316`, the local machine only, unless `--listen` names another address and port, such as `0.0.0.0:7316` for every network or `127.0.0.1:0` for any free port. It prints one line of JSON when it starts, with the viewer's address under `viewer`, then serves until it is stopped:

```json
{"ok":true,"result":{"address":"127.0.0.1:7316","listening":"http://127.0.0.1:7316","local_only":true,"token_required":false,"viewer":"http://127.0.0.1:7316/"}}
```

| Route | What it does |
|---|---|
| `GET /commands` | Lists every command: its `name`, what it `does` and its `input_schema`. |
| `POST /commands/<name>` | Runs the command. The body is its input, sent with `Content-Type: application/json`; an empty body is `{}`. |
| `POST /events` | Watches a drawing set's journal: the body names the set, `{"set": "<folder>"}`, and the response is a stream of Server-Sent Events that stays open ([below](#watching-a-set)). |
| `GET /assistant`, `POST /assistant/...` | The drafter's assistant: the agents found, a message to the drafter's own agent, and its conversation as it goes ([below](#the-assistant)). |
| `GET /`, `GET /assets/<file>` | The viewer's page and its files ([viewer.md](viewer.md)), embedded in the binary. `HEAD` gives their headers. |

```
curl -s -H 'Content-Type: application/json' -d '{"set": "examples/demo-set"}' http://127.0.0.1:7316/commands/journal
```

The status says what the command line's exit code says, and every response but the viewer's files, a refused one too, is a JSON reply:

| Status | When |
|---|---|
| 200 | The reply is ok. |
| 400 | The request is wrong (`usage`): the body is not JSON, or does not fit the command's input. |
| 401, 403 | The request was refused before it reached a command (`unauthorized`). |
| 404, 405 | No such route or command, or the wrong method. |
| 408, 411, 413, 415, 431 | A request that did not arrive within 30 seconds, a chunked body, a body over 64 MiB, a body that is not sent as JSON, or headers over 64 KiB. |
| 409 | A message to the assistant, or a pick of its agent, while it is still working on the last message (`assistant_busy`). |
| 422 | The core reported an error, such as a file it could not read or a sheet that failed; or no agent was found for the assistant (`no_agent`). |
| 503 | `POST /events` when 16 event streams are open already. |

**A command the viewer's page sends is the viewer's.** A browser marks each request a page makes to its own server `Sec-Fetch-Site: same-origin`, a header no script can set or change, so the HTTP API hands a request that carries it to the core as the viewer's, and any other as its own. It tells the reviewer's accept from an agent's that uses the API with its own tools; it is not a lock against a program that sends the header on purpose, as any program on a machine that reaches the API could.

### Watching a set

The viewer shows change sets as they land, whichever process sent them and through whichever interface: an agent's command line in another terminal, its MCP server, the Python library, or the HTTP API itself. `serve` learns of them from the set's saved files, not from its own requests: every change set, failed attempt, undo and accept adds an entry to the set's journal, `.agentdrafter/journal.jsonl`, and `POST /events` watches that file.

```
curl -sN -H 'Content-Type: application/json' -d '{"set": "examples/demo-set"}' http://127.0.0.1:7316/events
event: journal
data: {"ok":true,"result":{"set":"examples/demo-set","entries":[...],"can_undo":[...],...}}
```

- The response is `text/event-stream`, Server-Sent Events. An event named `journal` holds the `journal` command's reply on the set, on one line: one when the stream opens, and one each time the journal file's length or time changes and the reply with it, which the stream looks at five times a second. One named `proposals` holds the `proposals` command's reply, the [proposals](change-sets.md#proposals) waiting: one when the stream opens, after the journal, and one each time a proposal is made, which adds a file beside the journal, or kept or discarded, which adds to the journal, and the reply with it. One named `guides` holds the `guides` command's reply, the [guides](change-sets.md#guides) given, each step worked out on the set as it stands: one when the stream opens, after the proposals, and one each time the reply changes: a guide given, which adds a file beside the journal, or a change set sent for a guide's step landing or undone, which moves its steps on or back. A change set no guide's step names leaves the reply as it was, and sends none. A proposal kept comes as a change set does, in a `journal` event, naming the proposal; a discard as a `discard` entry. A change set's entry is written pending and then marked done ([journal.md](journal.md#what-it-records)); a look between the two waits for the commit to finish, so the stream sends the change set once, done. Every 15 seconds without an event it sends a comment, so a stream the page has let go of is found closed.
- **Why Server-Sent Events**: the server only has to push, the page only to listen, and a stream of text over the one HTTP connection needs nothing more than the server already speaks: no WebSocket handshake or framing, and no polling round trips that each wait behind a long plot.
- **The token.** A browser's `EventSource` cannot send a header, so the page reads the stream with `fetch`, which can: beyond the local machine the stream needs `Authorization: Bearer <token>`, as every command does, and a token in a query string is not taken. On the local machine it is guarded as a command is: addressed to the local machine, its body JSON, which a page of another site cannot send without asking first.
- Each stream is watched on a thread of its own, which only reads, so commands go on being answered one at a time while streams are open; at most 16 are open at once.

### What the drafter is looking at

The drafting app's page records what the drafter is looking at with `look-at`, as they move about: the sheet open, the layout shown, and the objects selected on it, by their IDs (`{"set", "sheet", "layout", "selected"}`), on a drawing set's sheet as it is. An agent on any interface reads it with `looking-at` (`{"set"}`): the sheet, the layout, the IDs, when it was recorded, and a line saying it, `the drafter has sheet DP-LD-101 open, with DP-LD-101/device selected`; before the page has recorded anything the sheet is null and nothing is selected. So an agent can answer "what is this?" and act on "move this", querying the objects by their IDs.

It is kept in the set's folder, beside the journal, in `.agentdrafter/looking-at.json`, since the agent asking runs in another process than the page's server. It is not one of the set's records: a change set, an undo and an accept never read or write it, the journal never sees it, and neither command rolls back or waits on an unfinished commit. The sheet must be one of the set's (`unknown_name`, naming them); a selection holds at most 1000 objects, each named once; a folder that is not a drawing set's is `read_failed`.

### The assistant

`agentdrafter serve` runs the drafting app's assistant on the drafter's own agent (#156): Claude Code or Codex, whichever it finds on the PATH when it starts, both when both are there. Zach picked these two on 2026-11-20, from the assistant's prototype ([drafting-app.md](drafting-app.md#decided)). The page reaches it through these routes, guarded as every command is:

| Route | What it does |
|---|---|
| `GET /assistant` | The agents, `claude` and `codex`, each with its name and whether it was found, and how to install each: its line and its page. |
| `POST /assistant/find` | Looks on the PATH again, for an agent installed since, and replies as `GET /assistant` does. A conversation with no agent goes on the first one found, and one whose agent is gone on the first found, or none, as a new conversation, so the server and the panel agree. |
| `POST /assistant/events` | The conversation on a set, `{"set": "<folder>"}`, as Server-Sent Events: an event named `assistant` when the stream opens and each time the conversation changes, holding all of it, `{"set", "agent", "busy", "items"}`. Its streams count with `/events`'s towards the 16 open at once. |
| `POST /assistant/messages` | A message from the drafter, `{"set", "text", "via", "looking_at"}`: `via` is `chat` or `command_bar`, and `looking_at`, `{"sheet", "layout", "selected", "names"}`, what the drafter is looking at, or null when they left it off. It replies at once with the item it added; the agent's work comes on the stream. |
| `POST /assistant/agent` | The picker: `{"set", "agent"}`. The conversation on the set goes on with that agent, as a new conversation. |

**A conversation** is kept in `serve`'s memory, one for each drawing set, by the set's canonical path, so two spellings of one folder are one conversation, shown as first named; on one agent. Every route that names a set checks it is a drawing set's folder, as the core's `looking-at` does, a message with nothing of what the drafter looks at among them, and refuses another (`read_failed`). Its items, oldest first: a `note` (`On Claude Code: AgentDrafter started claude and pointed it at agentdrafter mcp, on examples/demo-plant.`), each of the drafter's messages (`drafter`, with its `text`, `via` and `looking_at`), and each answer (`assistant`, with its `parts` in the order the agent made them, `done` and `error`). A part is the agent's `words`, or a `command` it called, by the registry's name, with its `input`, and its `reply` and `ok` once it came; a tool that is not one of AgentDrafter's keeps its own name. A proposal the assistant made is a `propose` part whose reply holds its number. The assistant answers one message at a time: another, or a pick of the agent, while it works is refused (`assistant_busy`).

**A message** is recorded first, when it carries what the drafter is looking at, through the core's `look-at`, as the page records it. The agent's own `looking-at` answers for the message it is answering: what the message carried, as `look-at` replied, or, when the drafter left it off, no sheet and nothing selected, saying they left it off; never what the viewer records after it was sent. Every other caller of `looking-at` reads the live record. Then the agent's command line is run once for it, its prompt on standard input: the first in a conversation tells it what it is, the drafter's assistant on the set, and that it changes the set only by proposing; each says what the drafter is looking at, or that they left it out; then the drafter's words. The next message resumes the session the first began. A run is watched from its start and stopped after 20 minutes, with every process it started (its process group on Unix, its tree on Windows), so the chat leaves working. The files a run is started with, its MCP configuration and the message's `looking-at` reply, are written in a folder of the system's temporary folder made for that run alone, with a random name, created only if nothing has that name, private to its user on Unix, and removed when the run ends.

**The agents' adapters**, one for each agent's command line, in `crates/agentdrafter-cli/src/assistant/`:

- **Claude Code**: `claude -p --output-format stream-json --verbose`, with no built-in tool (`--tools ""`), the assistant's MCP server alone (`--mcp-config`, the run's file, and `--strict-mcp-config`, so none of the drafter's own servers loads), whose tools it calls without asking (`--allowedTools mcp__agentdrafter`), and none of the drafter's hooks (`--settings` with `disableAllHooks`); its sign-in is the drafter's own; the next message `--resume`s its session.
- **Codex**: `codex exec --json --skip-git-repo-check --ignore-user-config -`, which leaves the drafter's `config.toml`, and every MCP server it names, unloaded, so the assistant's server is the only one, while the drafter's sign-in still is; no shell (`--disable shell_tool` and `unified_exec`), so no command rule the drafter saved in Codex's `.rules`, which load even without its configuration, can let a command run; its apps, browser, computer use, hooks, image viewing and image making off too (`--disable`), so it has no tool that reads a file beyond the set; what else could write, such as `apply_patch`, held by the read-only sandbox (`--sandbox read-only`); asking no one (`approval_policy=never`), with the assistant's MCP server given by `-c mcp_servers.agentdrafter.command`, `.args` and `.default_tools_approval_mode=approve`; the next message is `codex exec ... resume <thread> -`. The feature names are Codex 0.171's.

Each reads the agent's own JSON lines back into the conversation's parts. The agents run in the folder `serve` started in, so paths mean what they mean to the page.

**The assistant's edits are always proposals.** The MCP server it is given is `agentdrafter mcp --assistant --set <the set> --looking-at <the message's reply>`, which serves only the commands the core gives the assistant: those that read, `propose` and `guide` ([below](#the-mcp-server)), on the conversation's set alone. It withholds `change`, `undo`, `accept`, `keep`, `discard`, `look-at`, `build`, `plot`, `render`, `import-blocks` and `read`, which writes a text mirror, and its instructions tell the agent to propose. A proposal it makes with no `by` is recorded as `Assistant`, so the journal reads `proposed by Assistant · kept by you`. It gives a [guide](change-sets.md#guides) with `guide`, which records nothing of the set either, when the drafter asks how to do something; one it gives with no `by` is `Assistant`'s too. What it would change waits, drawn dashed on the drafter's drawing, until the drafter keeps it.

**The command that runs AgentDrafter**, which the agent starts its MCP server with, is the one `serve` runs as: the `agentdrafter` binary, or, for the Python package's command, its Python with `-m agentdrafter`.

### Guarding the port

The HTTP API is an open port to guard (ADR-0007). Zach decided on 2026-11-11 that **a token is needed only when the HTTP API listens beyond the local machine**:

- **On the local machine** (a loopback address such as `127.0.0.1` or `::1`), no token is needed. A request must still be addressed to the local machine: its `Host` names `localhost` or a loopback address, and an `Origin`, when a browser sends one, is a page of the local machine. A body must be sent as `application/json`, which a web page cannot send to another site without the browser asking it first. So a web page the reviewer opens cannot reach the API, by a name of its own or from its own site.
- **On any other address**, the unspecified address `0.0.0.0` among them, every request carries a token. A new one is made from the system's randomness each time the server starts, and printed in the line it starts with, under `token`, and in the fragment of the `viewer` address, `#token=…`. A request carries it as `Authorization: Bearer <token>`; it is compared in constant time, and a request without it, or with another, is refused with status 401 and the error `unauthorized`. The line the server starts with is the only one it writes the token in.
- **The viewer's page** and its files are served without the token, since a browser cannot send one to open a page: they are the page's own files, embedded in the binary, and give nothing of the machine's, and the page sends the token with every command it runs ([viewer.md](viewer.md#the-token)). On the local machine they too are served only to a request addressed to it.

### How HTTP is read

Plainly. Each connection carries one request and is closed after its response. A body is sized by `Content-Length`; a chunked body is refused, so no request can be read as part of another. Requests are taken one at a time, in the order they come, so two change sets never run on a drawing set at once; a long plot keeps the next request waiting. A connection has 30 seconds to send its whole request, however slowly it sends it, and 30 seconds to take its reply; then it is closed. A request that is refused is closed at once, its body unread. So no client can hold the server.

## The MCP server

```
agentdrafter mcp [--assistant [--set <drawing-set-folder>] [--looking-at <reply.json>]]
```

`agentdrafter mcp` is an MCP server over standard input and output, for an agent's MCP client to start, such as Claude Code with this in its `.mcp.json`:

```json
{"mcpServers": {"agentdrafter": {"command": "agentdrafter", "args": ["mcp"]}}}
```

Every command of the registry is a tool of the same name. With `--assistant` (`agentdrafter mcp --assistant`), the server the drafter's assistant is given ([above](#the-assistant)), only the commands that read, `propose` and `guide` are tools, as the core lists them; another is refused as an unknown tool, saying to call `propose`, its instructions say the same, and a proposal or a guide with no `by` is recorded as `Assistant`. `--set <folder>` binds it to one drawing set: a call whose `set` is not that folder, however it is spelled (a set nested inside it among them), or whose `source` or `output` is neither that folder nor a file in it, is refused (`-32602`). `--looking-at <file>` names a file holding a `looking-at` reply, which the server gives in place of the live record. Its description is what the command does, and its input schema is the command's. A call's result carries the reply twice: as text, the line the command line prints, and as `structuredContent`. It `isError` when the reply is not ok, so an agent reads the error and can fix its call. A tool that does not exist, or arguments that are not an object, are protocol errors (`-32602`).

Messages are JSON-RPC 2.0, one to a line, and nothing else goes to standard output. The server speaks both kinds of client:

- a client that opens with `initialize`, the handshake of revisions 2025-11-25, 2025-06-18, 2025-03-26 and 2024-11-05, is answered in the revision it asks for, or else 2025-11-25; revisions before 2025-06-18 get no `structuredContent`;
- a request that names revision 2026-07-28 under `_meta`, `io.modelcontextprotocol/protocolVersion`, is served as that revision says, with no handshake, and `server/discover` answers it. A request that names another revision is refused with `-32022` and the revisions the server speaks.

A message holds at most 64 MiB, as the HTTP API's body may. A longer one is not read into memory: it is answered with `-32600` and passed over to the end of its line. The server takes requests one at a time, in order, and stops when its standard input closes.

## Errors the servers add

| Code | Meaning |
|---|---|
| `unauthorized` | The HTTP API refused a request before it reached a command: it lacked the token a server listening beyond the local machine needs, or it was not addressed to a server listening on the local machine. |
| `usage` | Also: `POST /events` whose body names no set, or more than the set; 16 event streams already open. |
| `listen_failed` | `agentdrafter serve` could not listen on the address it was given, such as a port another program holds. |
| `assistant_busy` | A message to the assistant, or a pick of its agent, while it is still working on the last message: it answers one at a time. |
| `no_agent` | A message to the assistant when no agent was found on the PATH, or a pick of an agent that was not found. |
