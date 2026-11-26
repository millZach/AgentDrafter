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

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::{AgentEvent, McpServer, SERVER};

/// How long one preflight probe (`exec --help`, `--version`, `login status`)
/// may run before it is stopped (#229).
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Checks that `program` is a Codex build that can run an assistant turn
/// under `--ignore-user-config` (supports the flag and is signed in without
/// relying on a user-configured credential store such as `keyring`, #229).
pub(super) fn check(program: &Path) -> Result<(), String> {
    let has_api_key = std::env::var_os("CODEX_API_KEY").is_some_and(|v| !v.is_empty());
    check_with_api_key(program, has_api_key)
}

fn check_with_api_key(program: &Path, has_api_key: bool) -> Result<(), String> {
    let help = run_probe(program, &["exec", "--help"], PROBE_TIMEOUT).map_err(|_| {
        "Codex could not be checked (`codex exec --help` failed). Update or reinstall Codex and look again."
            .to_owned()
    })?;
    if !help.success {
        return Err(
            "Codex could not be checked (`codex exec --help` failed). Update or reinstall Codex and look again."
                .to_owned(),
        );
    }
    if !help.output.contains("--ignore-user-config") {
        let label = version_label(program);
        return Err(format!(
            "{label} does not support --ignore-user-config, which AgentDrafter needs to keep your other MCP servers off. Update Codex (`npm install -g @openai/codex`) and look again."
        ));
    }
    if has_api_key {
        return Ok(());
    }
    let file_login = run_probe(
        program,
        &[
            "login",
            "status",
            "-c",
            "cli_auth_credentials_store=\"file\"",
        ],
        PROBE_TIMEOUT,
    )
    .map_err(|_| {
        "Codex sign-in could not be checked. Run `codex login status` to check it, then look again."
            .to_owned()
    })?;
    if file_login.success {
        return Ok(());
    }
    if let Ok(default_login) = run_probe(program, &["login", "status"], PROBE_TIMEOUT) {
        if default_login.success {
            return Err(
                "Codex is signed in only through its user-configured credential store (such as keyring), which --ignore-user-config ignores. Sign in with file credentials (`codex login -c cli_auth_credentials_store=file`) or set CODEX_API_KEY, then look again."
                    .to_owned(),
            );
        }
    }
    Err(
        "Codex is not signed in. Run `codex login` (or set CODEX_API_KEY) and look again."
            .to_owned(),
    )
}

fn version_label(program: &Path) -> String {
    let Ok(out) = run_probe(program, &["--version"], PROBE_TIMEOUT) else {
        return "This Codex".to_owned();
    };
    if !out.success {
        return "This Codex".to_owned();
    }
    let Some(line) = out.output.lines().find(|l| !l.trim().is_empty()) else {
        return "This Codex".to_owned();
    };
    let token = line
        .split_whitespace()
        .find(|part| part.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .map(|part| part.trim_start_matches('v'));
    match token {
        Some(ver) => format!("Codex {ver}"),
        None => "This Codex".to_owned(),
    }
}

struct ProbeOutput {
    success: bool,
    output: String,
}

fn run_probe(program: &Path, args: &[&str], limit: Duration) -> Result<ProbeOutput, String> {
    let mut cmd = super::program(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    super::own_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let pid = child.id();
    let _active = super::ActiveChild::track(pid);
    let stdout = child.stdout.take().expect("stdout piped");
    let stderr = child.stderr.take().expect("stderr piped");
    let read_pipe = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                if buf.len() < 1 << 18 {
                    buf.extend_from_slice(&chunk[..n]);
                }
            }
            String::from_utf8_lossy(&buf).into_owned()
        })
    };
    let out_handle = read_pipe(Box::new(stdout));
    let err_handle = read_pipe(Box::new(stderr));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if started.elapsed() >= limit {
                    super::kill_tree(pid);
                    let _ = child.wait();
                    break Err("timed out".to_owned());
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => {
                super::kill_tree(pid);
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    }?;
    let mut output = out_handle.join().unwrap_or_default();
    let err = err_handle.join().unwrap_or_default();
    if !err.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&err);
    }
    Ok(ProbeOutput {
        success: status.success(),
        output,
    })
}

/// Checks Codex's system configuration directories for system MCP servers,
/// hooks, or requirements that `--ignore-user-config` still loads (#230).
pub(super) fn managed_warnings() -> Vec<String> {
    let mut warnings = Vec::new();
    for dir in system_dirs() {
        warnings.extend(managed_warnings_in(&dir));
    }
    warnings
}

fn system_dirs() -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        vec![PathBuf::from("/etc/codex")]
    }
    #[cfg(windows)]
    {
        let pd = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
        vec![pd.join("OpenAI").join("Codex")]
    }
}

/// Inspects a single Codex system-configuration directory for MCP servers,
/// hooks, or managed requirements that stay reachable with
/// `--ignore-user-config` (#230).
pub(super) fn managed_warnings_in(dir: &Path) -> Vec<String> {
    let mut warnings = Vec::new();
    for name in ["config.toml", "managed_config.toml"] {
        let path = dir.join(name);
        if !path.exists() {
            continue;
        }
        match std::fs::read_to_string(&path) {
            Err(_) => warnings.push(format!(
                "Codex system configuration at {} could not be read, so proposal-only cannot be guaranteed.",
                path.display()
            )),
            Ok(text) => {
                let found = scan_toml(&text);
                if found.mcp_servers {
                    warnings.push(format!(
                        "Codex has system MCP servers in {} that --ignore-user-config still loads, so proposal-only cannot be guaranteed.",
                        path.display()
                    ));
                }
                if found.hooks {
                    warnings.push(format!(
                        "Codex has system hooks in {} that --ignore-user-config still loads, so proposal-only cannot be guaranteed.",
                        path.display()
                    ));
                }
            }
        }
    }
    let requirements = dir.join("requirements.toml");
    if requirements.exists() {
        match std::fs::read_to_string(&requirements) {
            Err(_) => warnings.push(format!(
                "Codex system requirements at {} could not be read, so proposal-only cannot be guaranteed.",
                requirements.display()
            )),
            Ok(text) => {
                let found = scan_toml(&text);
                if found.mcp_servers || found.hooks || found.managed_hooks_policy {
                    warnings.push(format!(
                        "Codex has system requirements in {} that may keep extra MCP servers or hooks active, so proposal-only cannot be guaranteed.",
                        requirements.display()
                    ));
                }
            }
        }
    }
    warnings
}

#[derive(Default)]
struct TomlFindings {
    mcp_servers: bool,
    hooks: bool,
    managed_hooks_policy: bool,
}

/// Minimal TOML line scanner detecting system `[mcp_servers.*]` tables/keys
/// (other than `agentdrafter`), `[hooks]` definitions, and managed-hook policy
/// flags without adding a TOML parser dependency.
fn scan_toml(text: &str) -> TomlFindings {
    let mut found = TomlFindings::default();
    let mut section = String::new();
    for raw in text.lines() {
        let line = strip_toml_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let is_array = line.starts_with("[[") && line.ends_with("]]");
            let inner = if is_array {
                &line[2..line.len() - 2]
            } else {
                &line[1..line.len() - 1]
            }
            .trim();
            section = inner.to_owned();
            if let Some(rest) = section.strip_prefix("mcp_servers.") {
                let name = first_toml_segment(rest);
                if name != SERVER {
                    found.mcp_servers = true;
                }
            } else if is_array && (section == "hooks" || section.starts_with("hooks.")) {
                found.hooks = true;
            }
            continue;
        }
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        let key = lhs.trim();
        let val = rhs.trim();
        if section == "mcp_servers" {
            let name = first_toml_segment(key);
            if name != SERVER && !is_empty_toml_container(val) {
                found.mcp_servers = true;
            }
        } else if section.is_empty() {
            if let Some(rest) = key.strip_prefix("mcp_servers.") {
                let name = first_toml_segment(rest);
                if name != SERVER {
                    found.mcp_servers = true;
                }
            } else if key == "mcp_servers" && !is_empty_toml_container(val) {
                found.mcp_servers = true;
            } else if (key == "hooks" || key.starts_with("hooks.")) && !is_empty_toml_container(val)
            {
                found.hooks = true;
            } else if key == "allow_managed_hooks_only" && val == "true" {
                found.managed_hooks_policy = true;
            }
        } else if section == "hooks" || section.starts_with("hooks.") {
            if !is_empty_toml_container(val) {
                found.hooks = true;
            }
        } else if key == "allow_managed_hooks_only" && val == "true" {
            found.managed_hooks_policy = true;
        }
    }
    found
}

fn strip_toml_comment(line: &str) -> &str {
    let mut in_double = false;
    let mut in_single = false;
    let mut escaped = false;
    for (idx, ch) in line.char_indices() {
        if in_double {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_double = false;
            }
        } else if in_single {
            if ch == '\'' {
                in_single = false;
            }
        } else {
            match ch {
                '"' => in_double = true,
                '\'' => in_single = true,
                '#' => return &line[..idx],
                _ => {}
            }
        }
    }
    line
}

fn first_toml_segment(path: &str) -> &str {
    let trimmed = path.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().unwrap_or("")
    } else if let Some(rest) = trimmed.strip_prefix('\'') {
        rest.split('\'').next().unwrap_or("")
    } else {
        trimmed.split('.').next().unwrap_or("").trim()
    }
}

fn is_empty_toml_container(val: &str) -> bool {
    let compact: String = val.chars().filter(|c| !c.is_whitespace()).collect();
    compact == "{}" || compact == "[]"
}

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

    #[test]
    fn managed_warnings_detect_system_mcp_servers_hooks_and_requirements() {
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-codex-managed-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Comments and benign model/agentdrafter-only settings produce no warnings.
        std::fs::write(
            dir.join("config.toml"),
            "\
# [mcp_servers.commented_out]
model = \"gpt-5\"
[mcp_servers.agentdrafter]
command = \"agentdrafter\"
",
        )
        .unwrap();
        assert!(managed_warnings_in(&dir).is_empty());

        // System MCP server + system hooks + requirements policy all warn.
        std::fs::write(
            dir.join("managed_config.toml"),
            "\
[mcp_servers.\"corp-shell\"]
command = \"shell-mcp\"

[hooks]
pre_turn = [\"audit.sh\"]
",
        )
        .unwrap();
        std::fs::write(
            dir.join("requirements.toml"),
            "allow_managed_hooks_only = true\n",
        )
        .unwrap();
        let warnings = managed_warnings_in(&dir);
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert!(warnings[0].contains("managed_config.toml") && warnings[0].contains("MCP servers"));
        assert!(warnings[1].contains("managed_config.toml") && warnings[1].contains("hooks"));
        assert!(warnings[2].contains("requirements.toml"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn check_rejects_old_keyring_only_and_signed_out_codex() {
        let _guard = super::super::CHILD_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir =
            std::env::temp_dir().join(format!("agentdrafter-codex-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fake-agent")
            .join("fake_agent.py");
        let make_shim = |name: &str, envs: &[(&str, &str)]| -> PathBuf {
            if cfg!(windows) {
                let shim = dir.join(format!("{name}.cmd"));
                let mut body = String::from("@setlocal\r\n");
                for (k, v) in envs {
                    body.push_str(&format!("@set {k}={v}\r\n"));
                }
                body.push_str(&format!("@python \"{}\" codex %*\r\n", script.display()));
                std::fs::write(&shim, body).unwrap();
                shim
            } else {
                let shim = dir.join(name);
                let mut body = String::from("#!/bin/sh\n");
                for (k, v) in envs {
                    body.push_str(&format!("export {k}={v}\n"));
                }
                body.push_str(&format!(
                    "exec python3 \"{}\" codex \"$@\"\n",
                    script.display()
                ));
                std::fs::write(&shim, body).unwrap();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755))
                        .unwrap();
                }
                shim
            }
        };

        let ok_shim = make_shim("codex-ok", &[("FAKE_CODEX_AUTH", "file")]);
        assert_eq!(check_with_api_key(&ok_shim, false), Ok(()));

        let old_shim = make_shim("codex-old", &[("FAKE_CODEX_OLD", "0.120.0")]);
        let err = check_with_api_key(&old_shim, false).unwrap_err();
        assert!(
            err.contains("Codex 0.120.0") && err.contains("--ignore-user-config"),
            "{err}"
        );

        let keyring_shim = make_shim("codex-keyring", &[("FAKE_CODEX_AUTH", "keyring")]);
        let err = check_with_api_key(&keyring_shim, false).unwrap_err();
        assert!(
            err.contains("keyring") && err.contains("cli_auth_credentials_store=file"),
            "{err}"
        );

        let signed_out_shim = make_shim("codex-none", &[("FAKE_CODEX_AUTH", "none")]);
        let err = check_with_api_key(&signed_out_shim, false).unwrap_err();
        assert!(err.contains("codex login"), "{err}");
        // CODEX_API_KEY lets exec run without file credentials.
        assert_eq!(check_with_api_key(&signed_out_shim, true), Ok(()));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
