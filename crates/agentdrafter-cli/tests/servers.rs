//! The HTTP API and the MCP server over the same core as the command line:
//! every command of the registry is reachable through all three, and gives
//! the same reply through each; the same change set gives the same journal
//! entry, apart from its time; the HTTP API listens on the local machine
//! unless told otherwise; and the MCP handshake works.
//!
//! Every server a test starts listens on the local machine only and is
//! stopped by the test, when the value that holds it is dropped.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, ExitStatus, Stdio};

use serde_json::{json, Value};

#[path = "../../agentdrafter-core/tests/support/demo_set.rs"]
mod demo_set;
#[path = "../../agentdrafter-core/tests/support/snapshot.rs"]
mod snapshot;

const BIN: &str = env!("CARGO_BIN_EXE_agentdrafter");

/// An empty folder for one test, under Cargo's scratch folder in `target/`.
fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("servers")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `agentdrafter` in `cwd` and returns its exit code and the JSON it
/// printed.
fn cli(args: &[&str], cwd: &Path) -> (i32, Value) {
    let out = Command::new(BIN)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    assert!(
        out.stderr.is_empty(),
        "nothing on standard error: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert_eq!(stdout.lines().count(), 1, "one line of JSON: {stdout}");
    (
        out.status.code().unwrap(),
        serde_json::from_str(&stdout).unwrap(),
    )
}

/// `agentdrafter serve`, started in a folder; stopped when dropped.
struct Http {
    child: Child,
    address: SocketAddr,
    started: Value,
}

impl Http {
    fn start(cwd: &Path, args: &[&str]) -> Self {
        let mut child = Command::new(BIN)
            .arg("serve")
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let started: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(started["ok"], true, "{started}");
        let address = started["result"]["address"]
            .as_str()
            .unwrap()
            .parse()
            .unwrap();
        Self {
            child,
            address,
            started,
        }
    }

    /// Sends one request and returns its status and JSON.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> (u16, Value) {
        let body = body.map(Value::to_string).unwrap_or_default();
        let mut stream = TcpStream::connect(self.address).unwrap();
        write!(
            stream,
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            self.address,
            body.len()
        )
        .unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("Content-Type: application/json"), "{head}");
        let status = head.split(' ').nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap())
    }

    fn run(&self, command: &str, input: &Value) -> (u16, Value) {
        self.send("POST", &format!("/commands/{command}"), Some(input))
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `agentdrafter mcp`, started in a folder; stopped when dropped, or by
/// closing its input.
struct Mcp {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    next_id: u64,
}

impl Mcp {
    fn start(cwd: &Path) -> Self {
        let mut child = Command::new(BIN)
            .arg("mcp")
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            input: child.stdin.take(),
            output: BufReader::new(child.stdout.take().unwrap()),
            child,
            next_id: 0,
        }
    }

    fn write(&mut self, message: &Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
    }

    /// Sends a request and returns its answer, which carries its ID.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        let mut line = String::new();
        self.output.read_line(&mut line).unwrap();
        let answer: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(answer["jsonrpc"], "2.0");
        assert_eq!(answer["id"], id, "{answer}");
        answer
    }

    fn notify(&mut self, method: &str) {
        self.write(&json!({"jsonrpc": "2.0", "method": method}));
    }

    /// The legacy handshake, as a 2025-11-25 client opens.
    fn initialize(&mut self) -> Value {
        let answer = self.request(
            "initialize",
            json!({"protocolVersion": "2025-11-25", "capabilities": {},
                   "clientInfo": {"name": "servers-test", "version": "0"}}),
        );
        self.notify("notifications/initialized");
        answer
    }

    /// Calls a tool and returns its result, checking that its text and its
    /// structured content are the same reply.
    fn call(&mut self, tool: &str, arguments: &Value) -> Value {
        let answer = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &answer["result"];
        let text = result["content"][0]["text"].as_str().unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        let reply: Value = serde_json::from_str(text).unwrap();
        assert_eq!(reply, result["structuredContent"]);
        assert_eq!(result["isError"], reply["ok"] == false);
        result.clone()
    }

    /// Closes the server's input, which stops it, and waits for it.
    fn finish(mut self) -> ExitStatus {
        drop(self.input.take());
        self.child.wait().unwrap()
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        drop(self.input.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// `value` with every journal time taken out, the one thing two runs of the
/// same change set may differ in.
fn without_times(value: &Value) -> Value {
    match value {
        Value::Object(map) => map
            .iter()
            .filter(|(key, _)| key.as_str() != "at")
            .map(|(key, value)| (key.clone(), without_times(value)))
            .collect::<serde_json::Map<_, _>>()
            .into(),
        Value::Array(items) => items.iter().map(without_times).collect(),
        other => other.clone(),
    }
}

/// The calls the interfaces' tests make, shared with the Python library's
/// (`python/tests/test_interfaces.py`), so every interface is held to the
/// same ones: the change sets they apply, by file name, and one call of each
/// command, as the command line's arguments and as the input the other
/// interfaces take.
const CALLS: &str = include_str!("calls.json");

fn shared() -> Value {
    serde_json::from_str(CALLS).unwrap()
}

/// The shared change set of this file name.
fn change_set(file: &str) -> Value {
    let change_set = shared()["change_sets"][file].clone();
    assert!(change_set.is_object(), "no change set {file} in calls.json");
    change_set
}

/// A folder for one interface, holding a copy of the demo set as
/// `demo-set` and the change sets as files, so every interface is given
/// the same relative paths.
fn interface_folder(test: &str, interface: &str) -> PathBuf {
    let dir = scratch(&format!("{test}/{interface}"));
    demo_set::copy_demo_set(&dir.join("demo-set"));
    for (file, change_set) in shared()["change_sets"].as_object().unwrap() {
        fs::write(dir.join(file), change_set.to_string()).unwrap();
    }
    dir
}

/// One call of each command, as the command line's arguments and as the
/// input the servers take. Run in the registry's order on a fresh copy of
/// the demo set, each finds what the ones before it made.
fn calls() -> BTreeMap<String, (Vec<String>, Value)> {
    shared()["calls"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(name, call)| {
            let args = serde_json::from_value(call["args"].clone()).unwrap();
            (name.clone(), (args, call["input"].clone()))
        })
        .collect()
}

#[test]
fn every_command_of_the_registry_is_reachable_through_every_interface_with_the_same_reply() {
    let (cli_dir, http_dir, mcp_dir) = (
        interface_folder("every-command", "cli"),
        interface_folder("every-command", "http"),
        interface_folder("every-command", "mcp"),
    );
    let http = Http::start(&http_dir, &["--listen", "127.0.0.1:0"]);
    let mut mcp = Mcp::start(&mcp_dir);
    mcp.initialize();

    // Each server lists every command of the registry, with its schema.
    let registry = agentdrafter_core::commands();
    let (status, listed) = http.send("GET", "/commands", None);
    assert_eq!(status, 200, "{listed}");
    let listed = listed["result"]["commands"].as_array().unwrap().clone();
    let tools = mcp.request("tools/list", json!({}))["result"]["tools"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(listed.len(), registry.len());
    assert_eq!(tools.len(), registry.len());
    for ((command, listed), tool) in registry.iter().zip(&listed).zip(&tools) {
        assert_eq!(listed["name"], command.name());
        assert_eq!(listed["does"], command.does());
        assert_eq!(listed["input_schema"], command.input_schema());
        assert_eq!(tool["name"], command.name());
        assert_eq!(tool["description"], command.does());
        assert_eq!(tool["inputSchema"], command.input_schema());
    }

    // Each command, run through each interface, gives the same reply.
    let calls = calls();
    for command in registry {
        let name = command.name();
        let Some((args, input)) = calls.get(name) else {
            panic!("`{name}` is in the registry: add a call of it to tests/calls.json");
        };
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let (code, by_cli) = cli(&args, &cli_dir);
        let (status, by_http) = http.run(name, input);
        let by_mcp = mcp.call(name, input)["structuredContent"].clone();
        assert_eq!(by_cli["ok"], true, "{name}: {by_cli}");
        assert_eq!((code, status), (0, 200), "{name}");
        assert_eq!(
            without_times(&by_http),
            without_times(&by_cli),
            "`{name}` through the HTTP API and the command line"
        );
        assert_eq!(
            without_times(&by_mcp),
            without_times(&by_cli),
            "`{name}` through the MCP server and the command line"
        );
    }
    assert!(
        mcp.finish().success(),
        "the MCP server stops when its input closes"
    );
    for dir in [&http_dir, &mcp_dir] {
        assert_eq!(
            snapshot::snapshot(&dir.join("demo-set"))
                .into_iter()
                .map(|(path, bytes)| (path.strip_prefix(dir).unwrap().to_owned(), bytes))
                .collect::<BTreeMap<_, _>>(),
            snapshot::snapshot(&cli_dir.join("demo-set"))
                .into_iter()
                .map(|(path, bytes)| (path.strip_prefix(&cli_dir).unwrap().to_owned(), bytes))
                .collect::<BTreeMap<_, _>>(),
            "every file of the set is the same after the same commands"
        );
    }
}

#[test]
fn the_same_change_set_gives_the_same_reply_and_journal_entry_through_every_interface() {
    let (cli_dir, http_dir, mcp_dir) = (
        interface_folder("same-change-set", "cli"),
        interface_folder("same-change-set", "http"),
        interface_folder("same-change-set", "mcp"),
    );
    for dir in [&cli_dir, &http_dir, &mcp_dir] {
        let (code, json) = cli(&["build", "demo-set"], dir);
        assert_eq!(code, 0, "{json}");
    }
    let http = Http::start(&http_dir, &["--listen", "127.0.0.1:0"]);
    let mut mcp = Mcp::start(&mcp_dir);
    mcp.initialize();

    // A template edit, which lands on every sheet, then a change set that
    // lands on one sheet and fails on another.
    for (file, ok) in [("move.json", true), ("partial.json", false)] {
        let input = json!({"set": "demo-set", "change_set": change_set(file)});
        let (code, by_cli) = cli(&["change", "demo-set", file], &cli_dir);
        let (status, by_http) = http.run("change", &input);
        let result = mcp.call("change", &input);
        assert_eq!(result["isError"], !ok);
        assert_eq!(by_cli["ok"], ok, "{by_cli}");
        assert_eq!((code, status), if ok { (0, 200) } else { (1, 422) });
        assert_eq!(
            by_http, by_cli,
            "the HTTP API's reply is the command line's"
        );
        assert_eq!(
            result["structuredContent"], by_cli,
            "the MCP server's reply is the command line's"
        );
    }
    assert!(mcp.finish().success());
    drop(http);

    let journal = |dir: &Path| -> Vec<Value> {
        fs::read_to_string(dir.join("demo-set").join(agentdrafter_core::JOURNAL_FILE))
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    };
    let by_cli = journal(&cli_dir);
    assert_eq!(by_cli.len(), 2);
    for entry in &by_cli {
        assert!(entry["at"].as_str().unwrap().ends_with('Z'), "{entry}");
    }
    for dir in [&http_dir, &mcp_dir] {
        let entries = journal(dir);
        assert_eq!(
            entries.iter().map(without_times).collect::<Vec<_>>(),
            by_cli.iter().map(without_times).collect::<Vec<_>>(),
            "the same journal entries, apart from their times"
        );
    }
}

#[test]
fn serve_listens_on_the_local_machine_and_needs_no_token_there() {
    let dir = scratch("local");
    for listen in ["127.0.0.1:0", "localhost:0"] {
        let http = Http::start(&dir, &["--listen", listen]);
        let started = &http.started["result"];
        assert!(http.address.ip().is_loopback(), "{listen}: {started}");
        assert_eq!(started["local_only"], true);
        assert_eq!(started["token_required"], false);
        assert!(started.get("token").is_none());
        assert_eq!(started["listening"], format!("http://{}", http.address));
        let (status, json) = http.send("GET", "/commands", None);
        assert_eq!(
            status, 200,
            "no token is needed on the local machine: {json}"
        );
        let (status, json) = http.run("schema", &json!({"kind": "set"}));
        assert_eq!(status, 200, "{json}");
        assert_eq!(json["result"]["title"], "AgentDrafter drawing set settings");
        // A key the schema does not name is refused, as the command line
        // refuses it.
        let (status, json) = http.run(
            "query",
            &json!({"source": ".", "query": {"kind": "extents", "typo": 1}}),
        );
        assert_eq!(status, 400, "{json}");
        assert_eq!(json["error"]["code"], "usage");
    }

    // A wrong address, or a port another program holds, is a JSON error.
    let (code, json) = cli(&["serve", "--listen", "not an address"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");
    let held = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port();
    let (code, json) = cli(&["serve", "--listen", &format!("127.0.0.1:{port}")], &dir);
    assert_eq!(code, 1, "{json}");
    assert_eq!(json["error"]["code"], "listen_failed");
    let (code, json) = cli(&["mcp", "extra"], &dir);
    assert_eq!(code, 2, "{json}");
    assert_eq!(json["error"]["code"], "usage");

    let (_, help) = cli(&["help"], &dir);
    assert!(help["result"]["commands"]["serve"]["usage"]
        .as_str()
        .unwrap()
        .starts_with("agentdrafter serve"));
    assert_eq!(
        help["result"]["commands"]["mcp"]["usage"],
        "agentdrafter mcp"
    );
}

#[test]
fn the_mcp_handshake_works_and_tools_are_listed_and_called() {
    let dir = scratch("mcp");
    demo_set::copy_demo_set(&dir.join("demo-set"));
    let mut mcp = Mcp::start(&dir);

    let answer = mcp.initialize();
    let result = &answer["result"];
    assert_eq!(result["protocolVersion"], "2025-11-25");
    assert_eq!(result["serverInfo"]["name"], "agentdrafter");
    assert_eq!(result["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"]
        .as_str()
        .unwrap()
        .contains("agentdrafter"));
    assert_eq!(mcp.request("ping", json!({}))["result"], json!({}));

    let tools = mcp.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    let registry: Vec<&str> = agentdrafter_core::commands()
        .iter()
        .map(|command| command.name())
        .collect();
    assert_eq!(names, registry);
    for tool in tools["result"]["tools"].as_array().unwrap() {
        assert_eq!(tool["inputSchema"]["type"], "object", "{}", tool["name"]);
        assert!(tool["description"].as_str().unwrap().len() > 40);
    }

    let result = mcp.call("build", &json!({"source": "demo-set"}));
    assert_eq!(result["isError"], false, "{result}");
    assert_eq!(result["structuredContent"]["result"]["built"], 3);
    assert!(dir.join("demo-set/dwg/DEMO-003.dwg").is_file());
    let result = mcp.call(
        "query",
        &json!({"source": "demo-set", "query": {"kind": "blocks"}, "sheet": "DEMO-002"}),
    );
    assert_eq!(
        result["structuredContent"]["result"]["sheets"],
        json!(["DEMO-002"])
    );
    // A key the query's kind does not take is refused, not passed over.
    let result = mcp.call(
        "query",
        &json!({"source": "demo-set", "query": {"kind": "blocks", "name": "x"}}),
    );
    assert_eq!(result["isError"], true, "{result}");
    assert_eq!(result["structuredContent"]["error"]["code"], "usage");

    // A command's own error is the tool's, for the agent to read and fix.
    let result = mcp.call("journal", &json!({"set": "demo-set", "folder": "x"}));
    assert_eq!(result["isError"], true);
    assert_eq!(result["structuredContent"]["error"]["code"], "usage");
    let result = mcp.call("build", &json!({"source": "demo-set"}));
    assert_eq!(
        result["isError"], true,
        "the DWGs are there and no overwrite is asked"
    );
    let answer = mcp.request("tools/call", json!({"name": "draw", "arguments": {}}));
    assert_eq!(answer["error"]["code"], -32602);

    // A client of the newer revision, which sends no handshake, is served
    // on the same process.
    let meta = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28"});
    let answer = mcp.request("server/discover", json!({"_meta": meta}));
    assert_eq!(answer["result"]["supportedVersions"], json!(["2026-07-28"]));
    let answer = mcp.request(
        "tools/call",
        json!({"_meta": meta, "name": "operations", "arguments": {}}),
    );
    assert_eq!(answer["result"]["resultType"], "complete");
    assert_eq!(answer["result"]["isError"], false);
    assert!(answer["result"]["structuredContent"]["result"]["operations"].is_array());

    assert!(
        mcp.finish().success(),
        "the MCP server stops when its input closes"
    );
}
