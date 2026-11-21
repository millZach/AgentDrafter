//! The adapter for Codex's command line, `codex`.
//!
//! Each message is one run of `codex exec`, its prompt on standard input
//! (`-`), printing its work as JSON lines (`--json`). It has no shell
//! (`shell_tool` and `unified_exec` off), so no command rule the drafter
//! saved (`.rules`, which load even without their configuration) can let a
//! command run; what else could write, such as `apply_patch`, is held by the
//! read-only sandbox (`--sandbox read-only`), it asks no one
//! (`approval_policy` `never`), and it gets the assistant's MCP server by configuration
//! (`-c mcp_servers.agentdrafter...`), whose tools it calls without asking
//! (`default_tools_approval_mode`): they only read, or propose. The
//! drafter's own configuration, `config.toml`, with the MCP servers it
//! names, is not loaded (`--ignore-user-config`), so the assistant's server
//! is the only one; the drafter's sign-in still is. Its apps, browser,
//! computer use, hooks, image viewing and image making are off too
//! (`--disable`), so it has no tool that reads a file beyond the set. The next message resumes
//! the thread the first one's lines named (`exec resume <thread>`).

use std::path::Path;
use std::process::Command;

use serde_json::{json, Value};

use super::{AgentEvent, McpServer, SERVER};

/// The features of Codex the assistant runs without: its shell, and those
/// that reach beyond its MCP server. Codex 0.171's names.
const DISABLED: [&str; 9] = [
    "shell_tool",
    "unified_exec",
    "apps",
    "browser_use",
    "computer_use",
    "in_app_browser",
    "hooks",
    "view_image",
    "image_generation",
];

/// The command that runs one message: `program` is where `codex` was found,
/// `server` the assistant's MCP server, `thread` the thread to resume.
pub(super) fn command(program: &Path, server: &McpServer, thread: Option<&str>) -> Command {
    let mut command = super::program(program);
    command
        .arg("exec")
        .arg("--json")
        .arg("--skip-git-repo-check")
        .arg("--ignore-user-config")
        .args(["--sandbox", "read-only"])
        .args(["-c", "approval_policy=never"]);
    for feature in DISABLED {
        command.args(["--disable", feature]);
    }
    for setting in config(server) {
        command.arg("-c").arg(setting);
    }
    if let Some(thread) = thread {
        command.args(["resume", thread]);
    }
    command.arg("-");
    command
}

/// The `-c` settings that give Codex the assistant's MCP server, as TOML
/// values: literal strings, which take a Windows path's backslashes as they
/// are.
fn config(server: &McpServer) -> Vec<String> {
    let literal = |text: &str| -> String {
        if text.contains('\'') || text.contains('\n') {
            // A literal string cannot hold a quote; a basic string, escaped,
            // can.
            serde_json::to_string(text).expect("text serializes to JSON")
        } else {
            format!("'{text}'")
        }
    };
    let listed: Vec<String> = server.args.iter().map(|a| literal(a)).collect();
    vec![
        format!("mcp_servers.{SERVER}.command={}", literal(&server.command)),
        format!("mcp_servers.{SERVER}.args=[{}]", listed.join(",")),
        format!("mcp_servers.{SERVER}.default_tools_approval_mode=approve"),
    ]
}

/// What one line of `codex exec --json`'s output says.
pub(super) fn read(line: &str) -> Vec<AgentEvent> {
    let Ok(event) = serde_json::from_str::<Value>(line) else {
        return Vec::new();
    };
    let item = event.get("item").cloned().unwrap_or(Value::Null);
    let id = item
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match event.get("type").and_then(Value::as_str) {
        Some("thread.started") => event
            .get("thread_id")
            .and_then(Value::as_str)
            .map(|thread| vec![AgentEvent::Session(thread.to_owned())])
            .unwrap_or_default(),
        Some("item.started") => match item.get("type").and_then(Value::as_str) {
            Some("mcp_tool_call") => vec![AgentEvent::Started {
                id,
                tool: tool_name(&item),
                input: item.get("arguments").cloned().unwrap_or(json!({})),
            }],
            Some("command_execution") => vec![AgentEvent::Started {
                id,
                tool: "shell".to_owned(),
                input: json!({ "command": item.get("command").cloned().unwrap_or(Value::Null) }),
            }],
            _ => Vec::new(),
        },
        Some("item.completed") => match item.get("type").and_then(Value::as_str) {
            Some("agent_message") => item
                .get("text")
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .map(|text| vec![AgentEvent::Words(text.to_owned())])
                .unwrap_or_default(),
            Some("mcp_tool_call") => {
                let failed = item.get("status").and_then(Value::as_str) == Some("failed");
                let error = item
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let said = item
                    .pointer("/result/content")
                    .and_then(Value::as_array)
                    .map(|parts| {
                        parts
                            .iter()
                            .filter_map(|p| p.get("text").and_then(Value::as_str))
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .filter(|said| !said.is_empty())
                    .or(error)
                    .unwrap_or_default();
                vec![AgentEvent::done(id, &said, failed)]
            }
            Some("command_execution") => {
                let ok = item.get("exit_code").and_then(Value::as_i64) == Some(0);
                let output = item
                    .get("aggregated_output")
                    .cloned()
                    .unwrap_or(Value::Null);
                vec![AgentEvent::Done {
                    id,
                    reply: json!({ "ok": ok, "output": output }),
                    ok,
                }]
            }
            Some("error") => item
                .get("message")
                .and_then(Value::as_str)
                .map(|m| vec![AgentEvent::Failed(m.to_owned())])
                .unwrap_or_default(),
            _ => Vec::new(),
        },
        Some("turn.failed") => vec![AgentEvent::Failed(
            event
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or("the turn failed")
                .to_owned(),
        )],
        Some("error") => event
            .get("message")
            .and_then(Value::as_str)
            .map(|m| vec![AgentEvent::Failed(m.to_owned())])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// A tool's name as the registry names it, for one of the assistant's
/// server; any other server's tool as `server/tool`.
fn tool_name(item: &Value) -> String {
    let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
    match item.get("server").and_then(Value::as_str) {
        Some(server) if server != SERVER => format!("{server}/{tool}"),
        _ => tool.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn its_lines_say_the_thread_words_and_each_command_with_its_reply() {
        assert_eq!(
            read(r#"{"type":"thread.started","thread_id":"th-1"}"#),
            vec![AgentEvent::Session("th-1".into())]
        );
        let started = r#"{"type":"item.started","item":{"id":"item_1","type":"mcp_tool_call","server":"agentdrafter","tool":"propose","arguments":{"set":"demo"},"status":"in_progress"}}"#;
        assert_eq!(
            read(started),
            vec![AgentEvent::Started {
                id: "item_1".into(),
                tool: "propose".into(),
                input: json!({"set": "demo"})
            }]
        );
        let done = r#"{"type":"item.completed","item":{"id":"item_1","type":"mcp_tool_call","server":"agentdrafter","tool":"propose","arguments":{},"result":{"content":[{"type":"text","text":"{\"ok\":true,\"result\":{\"proposal\":1}}"}]},"status":"completed"}}"#;
        assert_eq!(
            read(done),
            vec![AgentEvent::Done {
                id: "item_1".into(),
                reply: json!({"ok": true, "result": {"proposal": 1}}),
                ok: true
            }]
        );
        let words = r#"{"type":"item.completed","item":{"id":"item_2","type":"agent_message","text":"Proposed."}}"#;
        assert_eq!(read(words), vec![AgentEvent::Words("Proposed.".into())]);
        let failed = r#"{"type":"turn.failed","error":{"message":"stream disconnected"}}"#;
        assert_eq!(
            read(failed),
            vec![AgentEvent::Failed("stream disconnected".into())]
        );
        assert!(read(r#"{"type":"turn.completed","usage":{}}"#).is_empty());
    }

    #[test]
    fn its_shell_only_reads_and_it_gets_the_assistants_server() {
        let server = McpServer {
            command: r"C:\Program Files\agentdrafter.exe".into(),
            args: vec!["mcp".into(), "--assistant".into()],
        };
        let command = command(Path::new("codex"), &server, Some("th-1"));
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[0], "exec");
        assert!(
            args.contains(&"--ignore-user-config".to_owned()),
            "none of the drafter's own MCP servers"
        );
        for feature in DISABLED {
            assert!(
                args.windows(2).any(|w| w == ["--disable", feature]),
                "{feature}"
            );
        }
        let sandbox = args.iter().position(|a| a == "--sandbox").unwrap();
        assert_eq!(args[sandbox + 1], "read-only");
        assert!(args.contains(
            &r"mcp_servers.agentdrafter.command='C:\Program Files\agentdrafter.exe'".to_owned()
        ));
        assert!(args.contains(&"mcp_servers.agentdrafter.args=['mcp','--assistant']".to_owned()));
        assert!(args
            .contains(&"mcp_servers.agentdrafter.default_tools_approval_mode=approve".to_owned()));
        assert_eq!(&args[args.len() - 3..], ["resume", "th-1", "-"]);
    }
}
