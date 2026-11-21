//! `agentdrafter mcp`: the MCP server, over standard input and output, a
//! thin layer over the core's registry of commands, as the command line is
//! (ADR-0007). It holds no drawing logic: each command is a tool of the
//! same name, whose input schema is the command's, and a call hands the
//! tool's arguments to the core and sends back the core's reply.
//!
//! Messages are JSON-RPC 2.0, one to a line. The server speaks both eras of
//! the protocol: a client that opens with `initialize` is served as the
//! handshake revisions say (2025-11-25 and earlier), and a request that
//! names revision 2026-07-28 in its `_meta` is served as that revision
//! says, which has `server/discover` in the handshake's place. A tool's
//! result carries the reply the command line prints, as text and as
//! structured content, and is an error when the reply is not ok.
//!
//! Requests are taken one at a time, in order, and the server stops when
//! its standard input closes. Nothing but MCP messages goes to standard
//! output.
//!
//! `agentdrafter mcp --assistant` is the server `agentdrafter serve` points
//! the drafter's own agent at (#156): it serves only the commands the core
//! gives the assistant, those that read and `propose`, so what the agent
//! would change waits as a proposal for the drafter to keep or discard; a
//! proposal it makes with no `by` is the assistant's, and its instructions
//! say so. `--set` binds it to the drawing set of the drafter's
//! conversation: a call that names another set, or a file outside it, is
//! refused. `--looking-at` names a file holding the `looking-at` reply for
//! the message the agent is answering, as the drafter sent it, which the
//! server gives in place of the live record.

use std::io::{BufRead, Read, Write};
use std::process::ExitCode;

use agentdrafter_core::{
    assistant_commands, commands, find_command, Access, Command, Error, Interface, Reply, Settings,
    ASSISTANT,
};
use serde_json::{json, Value};

use crate::{send, Args};

pub const MCP_USAGE: &str =
    "agentdrafter mcp [--assistant [--set <drawing-set-folder>] [--looking-at <reply.json>]]";

/// What `mcp` does, for the command line's help.
pub const MCP_DOES: &str =
    "Serves every command as a tool of an MCP server over standard input and output, one \
     JSON-RPC message to a line, for an agent's MCP client to start. Each tool takes the \
     command's input as its arguments and replies with the JSON the command line prints. Paths \
     are taken from the folder the server started in. It stops when its standard input closes. \
     With --assistant it serves only the commands that read and `propose`, for the drafter's \
     assistant, which proposes what it would change and never changes a set itself.";

/// The revision of the protocol served request by request, with no
/// handshake.
const MODERN: &str = "2026-07-28";

/// The handshake revisions, the newest first, which a client chooses with
/// `initialize`.
const HANDSHAKE: [&str; 4] = ["2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05"];

/// Where a request of the modern revision names its revision.
const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

/// What the server tells an agent's model about itself.
const INSTRUCTIONS: &str =
    "AgentDrafter makes, changes and checks technical drawings with no window open. Each tool \
     is one of the agentdrafter command line's commands, by the same name, and takes that \
     command's input as its arguments. It replies with the JSON the command line prints: \
     {\"ok\": true, \"result\": ...}, or {\"ok\": false, \"error\": {\"code\", \"message\", ...}} \
     and the result of what was done, if any. Paths are taken relative to the folder the server \
     started in. `operations` lists the operations a change set can hold, `change` applies one \
     to a drawing set, `journal` and `undo` read and undo them, and `schema` gives the schema of \
     each kind of file.";

/// What the server tells the drafter's assistant, after [`INSTRUCTIONS`].
const ASSISTANT_INSTRUCTIONS: &str =
    "You are the drafter's assistant in AgentDrafter's drafting app, drafting with them on \
     their drawing set. You never change the set yourself: `change`, `undo`, `accept`, `keep` \
     and `discard` are not your tools. To change it, call `propose` with the change set \
     `change` would take: it waits, drawn dashed on the drafter's drawing, and lands only when \
     the drafter keeps it. `operations` lists what a change set can hold. When the drafter \
     says \"this\" or asks what something is, `looking-at` gives the sheet they have open and \
     the objects they selected, by their IDs; `query` tells you about them. When they ask how \
     to do something, call `guide` with the steps for them to do themselves on their own \
     drawing, each with its drawing tool, the points to click and the operation that \
     completes it; they start it from the chat.";

/// Runs `agentdrafter mcp`: serves messages from standard input until it
/// closes.
pub fn serve(args: Args) -> ExitCode {
    let server = match server_for(args) {
        Ok(server) => server,
        Err(error) => {
            let (line, code) = send(Reply::<()>::from(Err(error)));
            println!("{line}");
            return code;
        }
    };
    let settings = Settings::from_env();
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    answer_lines(
        &mut stdin.lock(),
        &mut stdout.lock(),
        &settings,
        MOST_MESSAGE,
        server,
    );
    ExitCode::SUCCESS
}

/// The server `mcp`'s arguments ask for: the full one, or the assistant's,
/// bound to a set and given the message's `looking-at` reply.
fn server_for(mut args: Args) -> Result<Server, Error> {
    let usage = |message: String| Error::usage(format!("{message}; usage: {MCP_USAGE}"));
    let mut server = Server::default();
    let mut snapshot: Option<std::path::PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--assistant") if !server.assistant => server.assistant = true,
            Some("--set") if server.set.is_none() => {
                let set = args
                    .next()
                    .ok_or_else(|| usage("--set needs a drawing set's folder".into()))?;
                let set = std::fs::canonicalize(&set).map_err(|e| {
                    usage(format!(
                        "--set `{}` cannot be found: {e}",
                        set.to_string_lossy()
                    ))
                })?;
                server.set = Some(set);
            }
            Some("--looking-at") if snapshot.is_none() => {
                snapshot = Some(
                    args.next()
                        .ok_or_else(|| usage("--looking-at needs a file".into()))?
                        .into(),
                );
            }
            _ => {
                return Err(usage(format!(
                    "mcp takes --assistant, --set and --looking-at once each, not `{}`",
                    arg.to_string_lossy()
                )))
            }
        }
    }
    if !server.assistant && (server.set.is_some() || snapshot.is_some()) {
        return Err(usage(
            "--set and --looking-at are the assistant's: give --assistant".into(),
        ));
    }
    if let Some(file) = snapshot {
        let text = std::fs::read_to_string(&file).map_err(|e| {
            usage(format!(
                "--looking-at `{}` cannot be read: {e}",
                file.display()
            ))
        })?;
        let reply: Value = serde_json::from_str(&text).map_err(|e| {
            usage(format!(
                "--looking-at `{}` is not a reply: {e}",
                file.display()
            ))
        })?;
        server.looking_at = Some(reply);
    }
    Ok(server)
}

/// The most one message may hold, as the HTTP API's body may.
const MOST_MESSAGE: usize = 64 * 1024 * 1024;

/// Answers each message on `input`, one to a line and each of `most`
/// bytes at most, on `output`, until `input` ends or `output` closes. A
/// longer message is not read into memory: it is answered with an error and
/// passed over to the end of its line.
fn answer_lines(
    input: &mut impl BufRead,
    output: &mut impl Write,
    settings: &Settings,
    most: usize,
    mut server: Server,
) {
    let mut line = Vec::new();
    loop {
        line.clear();
        // One byte more than a message may hold tells a long one apart.
        let limit = u64::try_from(most).unwrap_or(u64::MAX).saturating_add(1);
        match input.by_ref().take(limit).read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        let answer = if line.len() > most && line.last() != Some(&b'\n') {
            if pass_line(input).is_err() {
                break;
            }
            Some(failure(
                Value::Null,
                (
                    -32600,
                    format!("Invalid Request: a message holds at most {most} bytes"),
                ),
            ))
        } else if line.iter().all(u8::is_ascii_whitespace) {
            None
        } else {
            server.handle(&line, settings)
        };
        if let Some(answer) = answer {
            let sent = writeln!(output, "{answer}").and_then(|()| output.flush());
            if sent.is_err() {
                break;
            }
        }
    }
}

/// Passes over what is left of a line on `input`, its newline too, holding
/// none of it.
fn pass_line(input: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        match buffer.iter().position(|&b| b == b'\n') {
            Some(end) => {
                input.consume(end + 1);
                return Ok(());
            }
            None => {
                let length = buffer.len();
                input.consume(length);
            }
        }
    }
}

/// The server's state: the handshake revision a client chose, if one did,
/// and whether it serves the drafter's assistant, the set it is bound to
/// and the `looking-at` reply it gives.
#[derive(Default)]
struct Server {
    handshake: Option<&'static str>,
    assistant: bool,
    /// The drawing set every call must name, canonical.
    set: Option<std::path::PathBuf>,
    /// The `looking-at` reply for the message being answered.
    looking_at: Option<Value>,
}

/// A JSON-RPC error: its code and message.
type Failure = (i64, String);

impl Server {
    /// The answer to one message, or none for a notification.
    fn handle(&mut self, message: &[u8], settings: &Settings) -> Option<Value> {
        let message: Value = match serde_json::from_slice(message) {
            Ok(message) => message,
            Err(e) => return Some(failure(Value::Null, (-32700, format!("Parse error: {e}")))),
        };
        let Value::Object(message) = message else {
            return Some(failure(
                Value::Null,
                (
                    -32600,
                    "Invalid Request: each message is one JSON object".to_owned(),
                ),
            ));
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response, which this server never asks for, needs no answer.
            let response = message.contains_key("result") || message.contains_key("error");
            return match id {
                Some(id) if !response => Some(failure(
                    id,
                    (
                        -32600,
                        "Invalid Request: a request names its method".to_owned(),
                    ),
                )),
                _ => None,
            };
        };
        // A notification, such as notifications/initialized, needs no
        // answer.
        let id = id?;
        if message.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
            return Some(failure(
                id,
                (-32600, "Invalid Request: jsonrpc is \"2.0\"".to_owned()),
            ));
        }
        let params = match message.get("params") {
            None | Some(Value::Null) => json!({}),
            Some(params @ Value::Object(_)) => params.clone(),
            Some(_) => {
                return Some(failure(
                    id,
                    (-32602, "Invalid params: params is an object".to_owned()),
                ))
            }
        };
        let modern = match params.get("_meta").and_then(|meta| meta.get(VERSION_KEY)) {
            None => false,
            Some(Value::String(version)) if version == MODERN => true,
            Some(requested) => return Some(unsupported(id, requested)),
        };
        let outcome = match method {
            "initialize" => Ok(self.initialize(&params)),
            "server/discover" => Ok(discover(self.assistant)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools(self.assistant) })),
            "tools/call" => self.call(&params, modern, settings),
            other => Err((-32601, format!("Method not found: {other}"))),
        };
        Some(match outcome {
            Ok(mut result) => {
                if modern || method == "server/discover" {
                    result["resultType"] = json!("complete");
                }
                json!({ "jsonrpc": "2.0", "id": id, "result": result })
            }
            Err(error) => failure(id, error),
        })
    }

    /// The handshake: the revision the client asked for when the server
    /// speaks it, or else the newest it does.
    fn initialize(&mut self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = HANDSHAKE
            .iter()
            .find(|version| Some(**version) == asked)
            .copied()
            .unwrap_or(HANDSHAKE[0]);
        self.handshake = Some(version);
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": server_info(),
            "instructions": instructions(self.assistant),
        })
    }

    /// Calls a tool: runs the command of its name on its arguments.
    fn call(&self, params: &Value, modern: bool, settings: &Settings) -> Result<Value, Failure> {
        let name = params.get("name").and_then(Value::as_str).ok_or_else(|| {
            (
                -32602,
                "Invalid params: tools/call names its tool under `name`".to_owned(),
            )
        })?;
        let command = find_command(name)
            .filter(|command| !self.assistant || command.access() != Access::Writes)
            .ok_or_else(|| {
                let withheld = self.assistant && find_command(name).is_some();
                let message = if withheld {
                    format!(
                        "Unknown tool: {name}. The drafter's assistant does not change the set \
                         itself: call `propose` with the change set, and the drafter keeps or \
                         discards it"
                    )
                } else {
                    format!("Unknown tool: {name}")
                };
                (-32602, message)
            })?;
        let mut arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments @ Value::Object(_)) => arguments.clone(),
            Some(_) => {
                return Err((
                    -32602,
                    "Invalid params: a tool's `arguments` are an object".to_owned(),
                ))
            }
        };
        self.within_set(&arguments)?;
        // The assistant's proposal, or guide, is the assistant's when it
        // names no one else.
        if self.assistant && matches!(command.name(), "propose" | "guide") {
            if let Some(arguments) = arguments.as_object_mut() {
                arguments
                    .entry("by")
                    .or_insert_with(|| Value::String(ASSISTANT.to_owned()));
            }
        }
        // What the drafter was looking at when they sent the message is the
        // reply serve gave this server, not what the viewer records since.
        let reply: Value = match (&self.looking_at, command.name()) {
            (Some(snapshot), "looking-at") => snapshot.clone(),
            // A reply holds only strings, numbers, lists and string-keyed
            // maps, which always serialize.
            _ => serde_json::to_value(command.run(arguments, settings, Interface::Mcp))
                .expect("a reply serializes to JSON"),
        };
        let ok = reply.get("ok").and_then(Value::as_bool) == Some(true);
        let text = reply.to_string();
        let mut result = json!({
            "content": [{ "type": "text", "text": text }],
            "isError": !ok,
        });
        // Structured content came in with revision 2025-06-18.
        let structured = modern || !matches!(self.handshake, Some("2025-03-26" | "2024-11-05"));
        if structured {
            result["structuredContent"] = reply;
        }
        Ok(result)
    }

    /// Refuses a call of the assistant's server bound to a set that names
    /// another set under `set`, the bound one alone, however it is spelled;
    /// or a file outside it under `source` or `output`, which may name the
    /// set or a file in it.
    fn within_set(&self, arguments: &Value) -> Result<(), Failure> {
        let Some(bound) = &self.set else {
            return Ok(());
        };
        for key in ["set", "source", "output"] {
            let Some(named) = arguments.get(key).and_then(Value::as_str) else {
                continue;
            };
            let inside = std::fs::canonicalize(named).is_ok_and(|path| {
                if key == "set" {
                    path == *bound
                } else {
                    path.starts_with(bound)
                }
            });
            if !inside {
                return Err((
                    -32602,
                    format!(
                        "Invalid params: the drafter's assistant works on the drawing set `{}` \
                         alone, and `{key}` names `{named}`",
                        bound.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

/// The answer to `server/discover`.
fn discover(assistant: bool) -> Value {
    json!({
        "supportedVersions": [MODERN],
        "capabilities": { "tools": {} },
        "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
        "instructions": instructions(assistant),
    })
}

/// What the server tells an agent's model about itself, and, serving the
/// drafter's assistant, what it may do.
fn instructions(assistant: bool) -> String {
    if assistant {
        format!("{INSTRUCTIONS} {ASSISTANT_INSTRUCTIONS}")
    } else {
        INSTRUCTIONS.to_owned()
    }
}

fn server_info() -> Value {
    json!({ "name": "agentdrafter", "version": env!("CARGO_PKG_VERSION") })
}

/// Every command as a tool, in the registry's order; serving the drafter's
/// assistant, those the core gives it.
fn tools(assistant: bool) -> Vec<Value> {
    let served: Vec<&Command> = if assistant {
        assistant_commands().collect()
    } else {
        commands().iter().collect()
    };
    served
        .into_iter()
        .map(|command| {
            json!({
                "name": command.name(),
                "description": command.does(),
                "inputSchema": command.input_schema(),
            })
        })
        .collect()
}

fn failure(id: Value, (code, message): Failure) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// The error for a request that names a revision the server does not
/// serve request by request.
fn unsupported(id: Value, requested: &Value) -> Value {
    let mut supported = vec![MODERN];
    supported.extend(HANDSHAKE);
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": -32022,
            "message": "Unsupported protocol version",
            "data": { "supported": supported, "requested": requested },
        },
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn ask(server: &mut Server, message: Value) -> Option<Value> {
        server.handle(message.to_string().as_bytes(), &Settings::default())
    }

    /// The answers `answer_lines` writes for `input`, with messages of
    /// `most` bytes at most.
    fn answers(input: impl std::io::Read, most: usize) -> Vec<Value> {
        let mut output = Vec::new();
        let mut input = std::io::BufReader::new(input);
        answer_lines(
            &mut input,
            &mut output,
            &Settings::default(),
            most,
            Server::default(),
        );
        String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    const PING: &str = r#"{"jsonrpc":"2.0","id":7,"method":"ping"}"#;

    #[test]
    fn the_assistants_server_takes_its_own_set_alone_not_one_nested_in_it() {
        let folder =
            std::env::temp_dir().join(format!("agentdrafter-bound-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let nested = folder.join("set").join("nested-copy");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(
            folder.join("set").join("set.yaml"),
            "units: in
",
        )
        .unwrap();
        std::fs::write(
            nested.join("set.yaml"),
            "units: in
",
        )
        .unwrap();
        std::fs::write(folder.join("set").join("rows.yaml"), "").unwrap();
        let bound = std::fs::canonicalize(folder.join("set")).unwrap();
        let server = Server {
            assistant: true,
            set: Some(bound),
            ..Server::default()
        };
        let set = folder.join("set");
        let named = |key: &str, path: &Path| json!({ key: path.to_string_lossy() });
        // The set itself, however it is spelled.
        assert!(server.within_set(&named("set", &set)).is_ok());
        assert!(server.within_set(&named("set", &set.join("."))).is_ok());
        // A copy of a set nested inside it is another set.
        let refused = server.within_set(&named("set", &nested)).unwrap_err();
        assert_eq!(refused.0, -32602);
        assert!(refused.1.contains("alone"), "{}", refused.1);
        // A file inside the set may be a source or an output; one outside
        // may not.
        assert!(server
            .within_set(&named("source", &set.join("rows.yaml")))
            .is_ok());
        assert!(server.within_set(&named("source", &nested)).is_ok());
        assert!(server.within_set(&named("source", &folder)).is_err());
        assert!(server.within_set(&named("set", &folder)).is_err());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn the_assistants_server_lists_and_runs_only_what_reads_and_proposes() {
        let mut server = Server {
            assistant: true,
            ..Server::default()
        };
        let init = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}),
        )
        .unwrap();
        let instructions = init["result"]["instructions"].as_str().unwrap();
        assert!(instructions.contains("call `propose`"), "{instructions}");
        assert!(instructions.contains("call `guide`"), "{instructions}");
        let listed = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .unwrap();
        let names: Vec<&str> = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        let given: Vec<&str> = assistant_commands().map(Command::name).collect();
        assert_eq!(names, given);
        for name in ["propose", "guide", "guides", "query", "looking-at"] {
            assert!(names.contains(&name), "{name}");
        }
        for name in [
            "change", "undo", "accept", "keep", "discard", "look-at", "build",
        ] {
            assert!(!names.contains(&name), "{name}");
            let refused = ask(
                &mut server,
                json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": name, "arguments": {"set": "missing"}}}),
            )
            .unwrap();
            assert_eq!(refused["error"]["code"], -32602, "{refused}");
            let message = refused["error"]["message"].as_str().unwrap();
            assert!(message.contains("call `propose`"), "{message}");
        }

        // The full server lists every command, and says nothing of the
        // assistant.
        let mut full = Server::default();
        let init = ask(
            &mut full,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        )
        .unwrap();
        assert!(!init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains("assistant"));
        let listed = ask(
            &mut full,
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        )
        .unwrap();
        assert_eq!(
            listed["result"]["tools"].as_array().unwrap().len(),
            commands().len()
        );
    }

    #[test]
    fn a_message_over_the_cap_is_refused_and_the_next_one_answered() {
        // A line of more than 64 MiB, then a ping.
        let long = std::io::repeat(b'x').take(MOST_MESSAGE as u64 + 1000);
        let rest = std::io::Cursor::new(format!("\n{PING}\n"));
        let got = answers(long.chain(rest), MOST_MESSAGE);
        assert_eq!(got.len(), 2, "{got:?}");
        assert_eq!(got[0]["error"]["code"], -32600);
        assert_eq!(got[0]["id"], Value::Null);
        assert!(got[0]["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&MOST_MESSAGE.to_string()));
        assert_eq!(got[1], json!({"jsonrpc": "2.0", "id": 7, "result": {}}));

        // A message of the cap is taken; one byte more is not.
        let most = PING.len() + 4;
        let fits = format!("{PING}    \n");
        let over = format!("{PING}     \n{PING}\n");
        assert_eq!(answers(fits.as_bytes(), most)[0]["result"], json!({}));
        let got = answers(over.as_bytes(), most);
        assert_eq!(got[0]["error"]["code"], -32600);
        assert_eq!(got[1]["result"], json!({}));
        // A long last line with no newline ends the input.
        let got = answers(format!("{PING}{}", " ".repeat(100)).as_bytes(), most);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0]["error"]["code"], -32600);
    }

    #[test]
    fn the_handshake_chooses_a_revision_the_server_speaks() {
        for (asked, chosen) in [
            ("2025-11-25", "2025-11-25"),
            ("2025-06-18", "2025-06-18"),
            ("2024-11-05", "2024-11-05"),
            ("2099-01-01", "2025-11-25"),
        ] {
            let mut server = Server::default();
            let answer = ask(
                &mut server,
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
                    "protocolVersion": asked, "capabilities": {},
                    "clientInfo": {"name": "test", "version": "0"}}}),
            )
            .unwrap();
            assert_eq!(answer["result"]["protocolVersion"], chosen);
            assert!(answer["result"]["capabilities"]["tools"].is_object());
            assert_eq!(answer["result"]["serverInfo"]["name"], "agentdrafter");
            assert!(answer["result"].get("resultType").is_none());
        }
    }

    #[test]
    fn structured_content_goes_only_to_revisions_that_have_it() {
        let call = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call",
            "params": {"name": "operations", "arguments": {}}});
        for (version, structured) in [("2025-11-25", true), ("2025-03-26", false)] {
            let mut server = Server::default();
            ask(
                &mut server,
                json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": version}}),
            );
            let answer = ask(&mut server, call.clone()).unwrap();
            assert_eq!(answer["result"]["isError"], false);
            assert_eq!(
                answer["result"].get("structuredContent").is_some(),
                structured,
                "{version}"
            );
        }
    }

    #[test]
    fn a_modern_request_is_served_without_a_handshake() {
        let mut server = Server::default();
        let meta = json!({ VERSION_KEY: MODERN });
        let answer = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": "d", "method": "server/discover",
                "params": {"_meta": meta}}),
        )
        .unwrap();
        assert_eq!(answer["result"]["supportedVersions"], json!([MODERN]));
        assert_eq!(answer["result"]["resultType"], "complete");
        assert_eq!(
            answer["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "agentdrafter"
        );
        let answer = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"_meta": meta, "name": "schema", "arguments": {"kind": "rows"}}}),
        )
        .unwrap();
        assert_eq!(answer["result"]["resultType"], "complete");
        assert_eq!(
            answer["result"]["structuredContent"]["result"]["title"],
            "AgentDrafter drawing set rows"
        );

        let answer = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 4, "method": "tools/list",
                "params": {"_meta": {VERSION_KEY: "1900-01-01"}}}),
        )
        .unwrap();
        assert_eq!(answer["error"]["code"], -32022);
        assert_eq!(answer["error"]["data"]["requested"], "1900-01-01");
        assert_eq!(answer["error"]["data"]["supported"][0], MODERN);
    }

    #[test]
    fn a_wrong_message_gets_a_json_rpc_error_and_a_notification_none() {
        let mut server = Server::default();
        let settings = Settings::default();
        let parse = server.handle(b"{not json", &settings).unwrap();
        assert_eq!(parse["error"]["code"], -32700);
        assert_eq!(parse["id"], Value::Null);
        for (message, code) in [
            (
                json!([{"jsonrpc": "2.0", "id": 1, "method": "ping"}]),
                -32600,
            ),
            (json!({"jsonrpc": "1.0", "id": 1, "method": "ping"}), -32600),
            (json!({"jsonrpc": "2.0", "id": 1}), -32600),
            (
                json!({"jsonrpc": "2.0", "id": 1, "method": "resources/list"}),
                -32601,
            ),
            (
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "draw"}}),
                -32602,
            ),
            (
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "build", "arguments": []}}),
                -32602,
            ),
            (
                json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": []}),
                -32602,
            ),
        ] {
            let answer = ask(&mut server, message.clone()).unwrap();
            assert_eq!(answer["error"]["code"], code, "{message}");
        }
        for silent in [
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": 1}}),
            json!({"jsonrpc": "2.0", "id": 9, "result": {}}),
        ] {
            assert_eq!(ask(&mut server, silent.clone()), None, "{silent}");
        }
        // Arguments that do not fit a tool's schema are the tool's error,
        // which the agent can fix, not the protocol's.
        let answer = ask(
            &mut server,
            json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
                "params": {"name": "journal", "arguments": {"folder": "x"}}}),
        )
        .unwrap();
        assert_eq!(answer["result"]["isError"], true);
        assert_eq!(
            answer["result"]["structuredContent"]["error"]["code"],
            "usage"
        );
    }
}
