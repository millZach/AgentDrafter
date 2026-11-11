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

use std::io::{BufRead, Write};
use std::process::ExitCode;

use agentdrafter_core::{commands, find_command, Error, Reply, Settings};
use serde_json::{json, Value};

use crate::{send, Args};

pub const MCP_USAGE: &str = "agentdrafter mcp";

/// What `mcp` does, for the command line's help.
pub const MCP_DOES: &str =
    "Serves every command as a tool of an MCP server over standard input and output, one \
     JSON-RPC message to a line, for an agent's MCP client to start. Each tool takes the \
     command's input as its arguments and replies with the JSON the command line prints. Paths \
     are taken from the folder the server started in. It stops when its standard input closes.";

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

/// Runs `agentdrafter mcp`: serves messages from standard input until it
/// closes.
pub fn serve(mut args: Args) -> ExitCode {
    if let Some(extra) = args.next() {
        let error = Error::usage(format!(
            "mcp takes no argument, found `{}`; usage: {MCP_USAGE}",
            extra.to_string_lossy()
        ));
        let (line, code) = send(Reply::<()>::from(Err(error)));
        println!("{line}");
        return code;
    }
    let settings = Settings::from_env();
    let mut server = Server::default();
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    let mut line = Vec::new();
    loop {
        line.clear();
        match input.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Some(answer) = server.handle(&line, &settings) {
            let sent = writeln!(output, "{answer}").and_then(|()| output.flush());
            if sent.is_err() {
                break;
            }
        }
    }
    ExitCode::SUCCESS
}

/// The server's state: the handshake revision a client chose, if one did.
#[derive(Default)]
struct Server {
    handshake: Option<&'static str>,
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
            "server/discover" => Ok(discover()),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
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
            "instructions": INSTRUCTIONS,
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
        let command =
            find_command(name).ok_or_else(|| (-32602, format!("Unknown tool: {name}")))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(arguments @ Value::Object(_)) => arguments.clone(),
            Some(_) => {
                return Err((
                    -32602,
                    "Invalid params: a tool's `arguments` are an object".to_owned(),
                ))
            }
        };
        let reply = command.run(arguments, settings);
        // A reply holds only strings, numbers, lists and string-keyed maps,
        // which always serialize.
        let text = serde_json::to_string(&reply).expect("a reply serializes to JSON");
        let mut result = json!({
            "content": [{ "type": "text", "text": text }],
            "isError": !reply.ok,
        });
        // Structured content came in with revision 2025-06-18.
        let structured = modern || !matches!(self.handshake, Some("2025-03-26" | "2024-11-05"));
        if structured {
            result["structuredContent"] =
                serde_json::to_value(&reply).expect("a reply serializes to JSON");
        }
        Ok(result)
    }
}

/// The answer to `server/discover`.
fn discover() -> Value {
    json!({
        "supportedVersions": [MODERN],
        "capabilities": { "tools": {} },
        "_meta": { "io.modelcontextprotocol/serverInfo": server_info() },
        "instructions": INSTRUCTIONS,
    })
}

fn server_info() -> Value {
    json!({ "name": "agentdrafter", "version": env!("CARGO_PKG_VERSION") })
}

/// Every command as a tool, in the registry's order.
fn tools() -> Vec<Value> {
    commands()
        .iter()
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
    use super::*;

    fn ask(server: &mut Server, message: Value) -> Option<Value> {
        server.handle(message.to_string().as_bytes(), &Settings::default())
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
