//! The adapter for Claude Code's command line, `claude`.
//!
//! Each message is one run of `claude -p`, its prompt on standard input,
//! printing its work as JSON lines (`--output-format stream-json`). It gets
//! no built-in tools (`--tools ""`) and one MCP server, the assistant's
//! (`--mcp-config`, `--strict-mcp-config`), whose tools it may call without
//! asking (`--allowedTools mcp__agentdrafter`). The next message resumes the
//! session the first one's lines named (`--resume`).

use std::ffi::OsString;
use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use super::{AgentEvent, SERVER};

/// The command that runs one message: `program` is where `claude` was
/// found, `config` the MCP configuration file naming the assistant's
/// server, `session` the session to resume.
pub(super) fn command(program: &Path, config: &Path, session: Option<&str>) -> Command {
    let mut command = super::program(program);
    command
        .arg("-p")
        .args(["--output-format", "stream-json", "--verbose"])
        .arg("--mcp-config")
        .arg(config)
        .arg("--strict-mcp-config")
        .args(["--tools", ""])
        .args(["--allowedTools", &format!("mcp__{SERVER}")]);
    if let Some(session) = session {
        command.args(["--resume", session]);
    }
    command
}

/// The MCP configuration file's contents: the assistant's server, started
/// by `own`, the command that runs AgentDrafter.
pub(super) fn config(own: &[OsString]) -> Value {
    let (program, args) = own.split_first().expect("a command has a program");
    let mut args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    args.extend(["mcp".to_owned(), "--assistant".to_owned()]);
    json!({
        "mcpServers": {
            SERVER: { "command": program.to_string_lossy(), "args": args }
        }
    })
}

/// What one line of `claude`'s output says.
pub(super) fn read(line: &str) -> Vec<AgentEvent> {
    let Ok(message) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let mut events = Vec::new();
    if let Some(session) = message.get("session_id").and_then(Value::as_str) {
        events.push(AgentEvent::Session(session.to_owned()));
    }
    let content = message
        .pointer("/message/content")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    match message.get("type").and_then(Value::as_str) {
        Some("assistant") => {
            for part in content {
                match part.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        let text = part.get("text").and_then(Value::as_str).unwrap_or("");
                        if !text.trim().is_empty() {
                            events.push(AgentEvent::Words(text.to_owned()));
                        }
                    }
                    Some("tool_use") => events.push(AgentEvent::Started {
                        id: text_of(&part, "id"),
                        tool: tool_name(&text_of(&part, "name")),
                        input: part.get("input").cloned().unwrap_or(json!({})),
                    }),
                    _ => {}
                }
            }
        }
        Some("user") => {
            for part in content {
                if part.get("type").and_then(Value::as_str) != Some("tool_result") {
                    continue;
                }
                let said = match part.get("content") {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Array(parts)) => parts
                        .iter()
                        .filter_map(|p| p.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    _ => String::new(),
                };
                let refused = part
                    .get("is_error")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                events.push(AgentEvent::done(
                    text_of(&part, "tool_use_id"),
                    &said,
                    refused,
                ));
            }
        }
        Some("result") => {
            let failed = message
                .get("is_error")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if failed {
                let why = message
                    .get("result")
                    .and_then(Value::as_str)
                    .filter(|r| !r.trim().is_empty())
                    .or_else(|| message.get("subtype").and_then(Value::as_str))
                    .unwrap_or("it reported an error");
                events.push(AgentEvent::Failed(why.to_owned()));
            }
        }
        _ => {}
    }
    events
}

/// A tool's name as the registry names it: `mcp__agentdrafter__query` is
/// `query`; any other tool keeps its own name.
fn tool_name(name: &str) -> String {
    name.strip_prefix(&format!("mcp__{SERVER}__"))
        .unwrap_or(name)
        .to_owned()
}

fn text_of(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_lines_say_the_session_words_and_each_command_with_its_reply() {
        let init = r#"{"type":"system","subtype":"init","session_id":"s-1","tools":[]}"#;
        assert_eq!(read(init), vec![AgentEvent::Session("s-1".into())]);
        let used = r#"{"type":"assistant","session_id":"s-1","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","id":"t1","name":"mcp__agentdrafter__looking-at","input":{"set":"demo"}},{"type":"tool_use","id":"t2","name":"Read","input":{}}]}}"#;
        assert_eq!(
            read(used),
            vec![
                AgentEvent::Session("s-1".into()),
                AgentEvent::Words("Looking.".into()),
                AgentEvent::Started {
                    id: "t1".into(),
                    tool: "looking-at".into(),
                    input: json!({"set": "demo"})
                },
                AgentEvent::Started {
                    id: "t2".into(),
                    tool: "Read".into(),
                    input: json!({})
                },
            ]
        );
        let result = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":[{"type":"text","text":"{\"ok\":true,\"result\":{\"sheet\":\"A\"}}"}]}]}}"#;
        assert_eq!(
            read(result),
            vec![AgentEvent::Done {
                id: "t1".into(),
                reply: json!({"ok": true, "result": {"sheet": "A"}}),
                ok: true
            }]
        );
        let refused = r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"t2","content":"No such tool","is_error":true}]}}"#;
        assert_eq!(
            read(refused),
            vec![AgentEvent::Done {
                id: "t2".into(),
                reply: json!({"ok": false, "error": {"message": "No such tool"}}),
                ok: false
            }]
        );
        let failed = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"session_id":"s-1"}"#;
        assert_eq!(
            read(failed),
            vec![
                AgentEvent::Session("s-1".into()),
                AgentEvent::Failed("error_during_execution".into())
            ]
        );
        assert!(read("not JSON").is_empty());
    }

    #[test]
    fn it_runs_with_no_tool_of_its_own_and_the_assistants_server_alone() {
        let command = command(
            Path::new("/bin/claude"),
            Path::new("/tmp/mcp.json"),
            Some("s-1"),
        );
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        for wanted in [
            "-p",
            "stream-json",
            "--strict-mcp-config",
            "mcp__agentdrafter",
        ] {
            assert!(args.iter().any(|a| a == wanted), "{wanted} in {args:?}");
        }
        let tools = args.iter().position(|a| a == "--tools").unwrap();
        assert_eq!(args[tools + 1], "", "no built-in tool");
        let resume = args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(args[resume + 1], "s-1");
        let config = config(&[OsString::from("/bin/agentdrafter")]);
        assert_eq!(
            config["mcpServers"]["agentdrafter"],
            json!({"command": "/bin/agentdrafter", "args": ["mcp", "--assistant"]})
        );
    }
}
