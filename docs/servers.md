# The HTTP API and the MCP server

The command line, the HTTP API and the MCP server are three of AgentDrafter's four interfaces (see [`CONTEXT.md`](../CONTEXT.md)); the Python library comes later. Each is a thin layer over one core (ADR-0007): it turns what it was sent into a command's input, hands it to the core, and sends back the core's reply. None of them holds drawing logic.

## One registry of commands

The core keeps one registry of commands, and every interface takes its commands from it, so a command added there reaches all three at once and none can be missed. Each command has one name everywhere, takes one input and gives one reply:

| Command | What it does | Its own page |
|---|---|---|
| `build` | Builds a sheet description, or a drawing set's folder, into DWGs. | [drawing-set.md](drawing-set.md) |
| `schema` | Gives the JSON Schema of a kind of file. | [sheet-description.md](sheet-description.md) |
| `measure-text` | Measures how wide a line of text draws. | [sheet-description.md](sheet-description.md) |
| `render` | Draws a sheet to a PNG. | [render.md](render.md) |
| `plot` | Plots a sheet, a DWG or a drawing set to PDF. | [plot.md](plot.md) |
| `stale` | Says whether a PNG or PDF is stale. | [render.md](render.md) |
| `query` | Asks a drawing set what is on its sheets. | [query.md](query.md) |
| `operations` | Lists the operations a change set can hold. | [change-sets.md](change-sets.md) |
| `change` | Applies a change set to a drawing set. | [change-sets.md](change-sets.md) |
| `journal` | Lists a drawing set's journal. | [journal.md](journal.md) |
| `undo` | Undoes a change set and every later one. | [journal.md](journal.md) |
| `check` | Checks a drawing set's saved files. | [check.md](check.md) |

**The input** is a JSON object. Its keys are the command line's options, with `-` written `_`: `--dry-run` is `dry_run`, `--plot-style` is `plot_style`, and an option given more than once, such as `--fonts`, is a list. What the command line takes as a plain argument has a name of its own: the file or folder is `source` (`set` for `change`, `journal` and `undo`, and `output` for `stale`), the text `measure-text` measures is `contents`, `schema`'s kind is `kind`, `undo`'s change set is `change_set`, `query`'s kind and selection are `query: {"kind": "find", "name": ...}`, and `change` takes the change set itself under `change_set`, not a file. Each command's input has a JSON Schema, which `GET /commands` lists and the MCP server gives as each tool's input schema. A key the schema does not name is refused.

**The reply** is the one the command line prints: `{"ok": true, "result": ...}`, or `{"ok": false, "error": {"code", "message", ...}}` with the `result` of what was done, if anything was. The same input gives the same reply through every interface, and the same change set gives the same journal entry, apart from its time. A test runs every command of the registry through all three and compares the replies.

**Paths** are taken as the command line takes them: relative to the folder the server was started in, or absolute. Replies name files from the paths sent, as the command line's do. A server reaches every file its user can, so a server that listens beyond the local machine needs a token (below).

**Fonts and plot styles** are read from the folders the input's `fonts` and `plot_styles` name, then from those the server's own environment names in `AGENTDRAFTER_FONTS` and `AGENTDRAFTER_PLOT_STYLES` when it started.

## The HTTP API

```
agentdrafter serve [--listen <address:port>]
```

`agentdrafter serve` listens on `127.0.0.1:7316`, the local machine only, unless `--listen` names another address and port, such as `0.0.0.0:7316` for every network or `127.0.0.1:0` for any free port. It prints one line of JSON when it starts, then serves until it is stopped:

```json
{"ok":true,"result":{"address":"127.0.0.1:7316","listening":"http://127.0.0.1:7316","local_only":true,"token_required":false}}
```

| Route | What it does |
|---|---|
| `GET /commands` | Lists every command: its `name`, what it `does` and its `input_schema`. |
| `POST /commands/<name>` | Runs the command. The body is its input, sent with `Content-Type: application/json`; an empty body is `{}`. |

```
curl -s -H 'Content-Type: application/json' -d '{"set": "examples/demo-set"}' http://127.0.0.1:7316/commands/journal
```

The status says what the command line's exit code says, and every response, a refused one too, is a JSON reply:

| Status | When |
|---|---|
| 200 | The reply is ok. |
| 400 | The request is wrong (`usage`): the body is not JSON, or does not fit the command's input. |
| 401, 403 | The request was refused before it reached a command (`unauthorized`). |
| 404, 405 | No such route or command, or the wrong method. |
| 411, 413, 415, 431 | A chunked body, a body over 64 MiB, a body that is not sent as JSON, or headers over 64 KiB. |
| 422 | The core reported an error, such as a file it could not read or a sheet that failed. |

### Guarding the port

The HTTP API is an open port to guard (ADR-0007). Zach decided on 2026-11-11 that **a token is needed only when the HTTP API listens beyond the local machine**:

- **On the local machine** (a loopback address such as `127.0.0.1` or `::1`), no token is needed. A request must still be addressed to the local machine: its `Host` names `localhost` or a loopback address, and an `Origin`, when a browser sends one, is a page of the local machine. A body must be sent as `application/json`, which a web page cannot send to another site without the browser asking it first. So a web page the reviewer opens cannot reach the API, by a name of its own or from its own site.
- **On any other address**, the unspecified address `0.0.0.0` among them, every request carries a token. A new one is made from the system's randomness each time the server starts, and printed in the line it starts with, under `token`. A request carries it as `Authorization: Bearer <token>`; it is compared in constant time, and a request without it, or with another, is refused with status 401 and the error `unauthorized`.

### How HTTP is read

Plainly. Each connection carries one request and is closed after its response. A body is sized by `Content-Length`; a chunked body is refused, so no request can be read as part of another. Requests are taken one at a time, in the order they come, so two change sets never run on a drawing set at once; a long plot keeps the next request waiting. A connection that sends nothing for 30 seconds is closed.

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

It takes requests one at a time, in order, and stops when its standard input closes.

## Errors the servers add

| Code | Meaning |
|---|---|
| `unauthorized` | The HTTP API refused a request before it reached a command: it lacked the token a server listening beyond the local machine needs, or it was not addressed to a server listening on the local machine. |
| `listen_failed` | `agentdrafter serve` could not listen on the address it was given, such as a port another program holds. |
