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
| `check` | Checks a drawing set's saved files. | [check.md](check.md) |

**The input** is a JSON object. Its keys are the command line's options, with `-` written `_`: `--dry-run` is `dry_run`, `--plot-style` is `plot_style`, and an option given more than once, such as `--fonts`, is a list. What the command line takes as a plain argument has a name of its own: the file or folder is `source` (`set` for `change`, `journal`, `undo` and `accept`, and `output` for `stale`), the text `measure-text` measures is `contents`, `schema`'s kind is `kind`, `undo`'s change set is `change_set`, `display-list`'s journal entry `entry`, `accept`'s acknowledgement of failed checks `despite_failed_checks`, `query`'s kind and selection are `query: {"kind": "find", "name": ...}`, and `change` takes the change set itself under `change_set`, not a file. Each command's input has a JSON Schema, which `GET /commands` lists and the MCP server gives as each tool's input schema. A key the schema does not name is refused.

**The reply** is the one the command line prints: `{"ok": true, "result": ...}`, or `{"ok": false, "error": {"code", "message", ...}}` with the `result` of what was done, if anything was. The same input gives the same reply through every interface, and the same change set gives the same journal entry, apart from its time. A test runs every command of the registry through all three and compares the replies, and the Python library's test runs the same calls through it too.

**The interface a command came through** is handed to the core with it, and an accept records it in the journal (`via`), so the reviewer can tell an agent's accept from their own: `cli` for the command line, the Python package's command among it, `http` for the HTTP API, `mcp` for the MCP server, `python` for the Python library, and `viewer` for the viewer's page. The input never says it, so a caller cannot choose it. An accept's reply and entry name it, so they differ there between interfaces, and only there; the tests compare them apart from it, and hold each accept to its own interface.

**Paths** are taken as the command line takes them: relative to the folder the server was started in, or absolute. Replies name files from the paths sent, as the command line's do. A server reaches every file its user can, so a server that listens beyond the local machine needs a token (below).

**Fonts and plot styles** are read from the folders the input's `fonts` and `plot_styles` name, then from those the server's own environment names in `AGENTDRAFTER_FONTS` and `AGENTDRAFTER_PLOT_STYLES` when it started.

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
| 422 | The core reported an error, such as a file it could not read or a sheet that failed. |
| 503 | `POST /events` when 16 event streams are open already. |

**A command the viewer's page sends is the viewer's.** A browser marks each request a page makes to its own server `Sec-Fetch-Site: same-origin`, a header no script can set or change, so the HTTP API hands a request that carries it to the core as the viewer's, and any other as its own. It tells the reviewer's accept from an agent's that uses the API with its own tools; it is not a lock against a program that sends the header on purpose, as any program on a machine that reaches the API could.

### Watching a set

The viewer shows change sets as they land, whichever process sent them and through whichever interface: an agent's command line in another terminal, its MCP server, the Python library, or the HTTP API itself. `serve` learns of them from the set's saved files, not from its own requests: every change set, failed attempt, undo and accept adds an entry to the set's journal, `.agentdrafter/journal.jsonl`, and `POST /events` watches that file.

```
curl -sN -H 'Content-Type: application/json' -d '{"set": "examples/demo-set"}' http://127.0.0.1:7316/events
event: journal
data: {"ok":true,"result":{"set":"examples/demo-set","entries":[...],"can_undo":[...],...}}
```

- The response is `text/event-stream`, Server-Sent Events. Each event is named `journal`, and its data is the `journal` command's reply on the set, on one line: one when the stream opens, and one each time the journal file's length or time changes, which the stream looks at five times a second. Every 15 seconds without an event it sends a comment, so a stream the page has let go of is found closed.
- **Why Server-Sent Events**: the server only has to push, the page only to listen, and a stream of text over the one HTTP connection needs nothing more than the server already speaks: no WebSocket handshake or framing, and no polling round trips that each wait behind a long plot.
- **The token.** A browser's `EventSource` cannot send a header, so the page reads the stream with `fetch`, which can: beyond the local machine the stream needs `Authorization: Bearer <token>`, as every command does, and a token in a query string is not taken. On the local machine it is guarded as a command is: addressed to the local machine, its body JSON, which a page of another site cannot send without asking first.
- Each stream is watched on a thread of its own, which only reads, so commands go on being answered one at a time while streams are open; at most 16 are open at once.

### Guarding the port

The HTTP API is an open port to guard (ADR-0007). Zach decided on 2026-11-11 that **a token is needed only when the HTTP API listens beyond the local machine**:

- **On the local machine** (a loopback address such as `127.0.0.1` or `::1`), no token is needed. A request must still be addressed to the local machine: its `Host` names `localhost` or a loopback address, and an `Origin`, when a browser sends one, is a page of the local machine. A body must be sent as `application/json`, which a web page cannot send to another site without the browser asking it first. So a web page the reviewer opens cannot reach the API, by a name of its own or from its own site.
- **On any other address**, the unspecified address `0.0.0.0` among them, every request carries a token. A new one is made from the system's randomness each time the server starts, and printed in the line it starts with, under `token`, and in the fragment of the `viewer` address, `#token=…`. A request carries it as `Authorization: Bearer <token>`; it is compared in constant time, and a request without it, or with another, is refused with status 401 and the error `unauthorized`. The line the server starts with is the only one it writes the token in.
- **The viewer's page** and its files are served without the token, since a browser cannot send one to open a page: they are the page's own files, embedded in the binary, and give nothing of the machine's, and the page sends the token with every command it runs ([viewer.md](viewer.md#the-token)). On the local machine they too are served only to a request addressed to it.

### How HTTP is read

Plainly. Each connection carries one request and is closed after its response. A body is sized by `Content-Length`; a chunked body is refused, so no request can be read as part of another. Requests are taken one at a time, in the order they come, so two change sets never run on a drawing set at once; a long plot keeps the next request waiting. A connection has 30 seconds to send its whole request, however slowly it sends it, and 30 seconds to take its reply; then it is closed. A request that is refused is closed at once, its body unread. So no client can hold the server.

## The MCP server

```
agentdrafter mcp
```

`agentdrafter mcp` is an MCP server over standard input and output, for an agent's MCP client to start, such as Claude Code with this in its `.mcp.json`:

```json
{"mcpServers": {"agentdrafter": {"command": "agentdrafter", "args": ["mcp"]}}}
```

Every command of the registry is a tool of the same name. Its description is what the command does, and its input schema is the command's. A call's result carries the reply twice: as text, the line the command line prints, and as `structuredContent`. It `isError` when the reply is not ok, so an agent reads the error and can fix its call. A tool that does not exist, or arguments that are not an object, are protocol errors (`-32602`).

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
